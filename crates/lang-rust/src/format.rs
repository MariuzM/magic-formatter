use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MARK: &str          = concat!("// __magic_", "formatter__");
const KEYWORDS: [&str; 6] = ["if", "while", "for", "match", "loop", "else"];

fn char_literal_end(s: &[char], i: usize) -> Option<usize> {
    if s.get(i).copied() != Some('\'') {
        return None;
    }
    match s.get(i + 1).copied() {
        Some('\\') => {
            if s.get(i + 2).is_some() && s.get(i + 3).copied() == Some('\'') {
                Some(i + 4)
            } else {
                None
            }
        }
        Some(c) if c != '\'' && c != '\\' => {
            if s.get(i + 2).copied() == Some('\'') {
                Some(i + 3)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn raw_string_open(s: &[char], i: usize) -> Option<(usize, usize)> {
    let mut j = i;
    if s.get(j).copied() == Some('b') {
        j += 1;
    }
    if s.get(j).copied() != Some('r') {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while s.get(j).copied() == Some('#') {
        hashes += 1;
        j += 1;
    }
    if s.get(j).copied() != Some('"') {
        return None;
    }
    Some((j + 1, hashes))
}

fn raw_string_close(s: &[char], from: usize, hashes: usize) -> usize {
    let mut i = from;
    while i < s.len() {
        if s[i] == '"' {
            let mut k = 0;
            while k < hashes && s.get(i + 1 + k).copied() == Some('#') {
                k += 1;
            }
            if k == hashes {
                return i + 1 + hashes;
            }
        }
        i += 1;
    }
    s.len()
}

fn starts_at(s: &[char], i: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, c)| s.get(i + k).copied() == Some(c))
}

fn find_edition(start: &Path) -> String {
    let mut dir = start.to_path_buf();
    for _ in 0..8 {
        let manifest = dir.join("Cargo.toml");
        if let Ok(text) = fs::read_to_string(&manifest) {
            for line in text.lines() {
                let t              = line.trim_start();
                let Some(rest)     = t.strip_prefix("edition") else { continue };
                let rest           = rest.trim_start();
                let Some(rest)     = rest.strip_prefix('=') else { continue };
                let rest           = rest.trim_start();
                let Some(rest)     = rest.strip_prefix('"') else { continue };
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if digits.len() == 4 && rest[digits.len()..].starts_with('"') {
                    return digits;
                }
            }
        }
        if !dir.pop() {
            break;
        }
    }
    "2021".to_string()
}

struct Frame {
    opener:         char,
    pos:            usize,
    start_line:     usize,
    commas:         Vec<usize>,
    trailing:       bool,
    awaiting_first: bool,
    first_on_next:  bool,
}

#[derive(Default)]
struct ChainFrame {
    multiline:               Option<bool>,
    last_token_line:         Option<usize>,
    generic_depth:           usize,
    generic_delimiter_depth: usize,
}

fn looks_like_tuple(s: &[char], opener_pos: usize) -> bool {
    let mut j = opener_pos as isize - 1;
    while j >= 0 && matches!(s[j as usize], ' ' | '\t' | '\n') {
        j -= 1;
    }
    if j < 0 {
        return true;
    }
    let c = s[j as usize];
    !(c.is_ascii_alphanumeric() || matches!(c, '_' | '!' | ')' | ']' | '>'))
}

fn touch(stack: &mut [Frame], line: usize, is_comma: bool) {
    if let Some(top) = stack.last_mut() {
        if top.awaiting_first {
            top.awaiting_first = false;
            top.first_on_next  = line > top.start_line;
        }
        top.trailing = is_comma;
    }
}

fn find_insert_points(s: &[char]) -> Vec<usize> {
    let n                      = s.len();
    let mut i                  = 0;
    let mut line               = 0;
    let mut stack: Vec<Frame>  = Vec::new();
    let mut points: Vec<usize> = Vec::new();

    while i < n {
        let c = s[i];
        if let Some((content, hashes)) = raw_string_open(s, i) {
            touch(&mut stack, line, false);
            let end = raw_string_close(s, content, hashes);
            line += s[i..end].iter().filter(|&&x| x == '\n').count();
            i = end;
            continue;
        }
        if c == '"' {
            touch(&mut stack, line, false);
            i += 1;
            while i < n && s[i] != '"' {
                if s[i] == '\n' {
                    line += 1;
                }
                i += if s[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            continue;
        }
        if c == '\'' {
            if let Some(end) = char_literal_end(s, i) {
                touch(&mut stack, line, false);
                i = end;
                continue;
            }
        }
        if starts_at(s, i, "//") {
            while i < n && s[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if starts_at(s, i, "/*") {
            let mut depth = 1;
            i += 2;
            while i < n && depth > 0 {
                if starts_at(s, i, "/*") {
                    depth += 1;
                    i += 2;
                } else if starts_at(s, i, "*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    if s[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        if matches!(c, '(' | '[' | '{') {
            touch(&mut stack, line, false);
            stack.push(Frame {
                opener:         c,
                pos:            i,
                start_line:     line,
                commas:         Vec::new(),
                trailing:       false,
                awaiting_first: true,
                first_on_next:  false,
            });
            i += 1;
            continue;
        }
        if matches!(c, ')' | ']' | '}') {
            if let Some(frame) = stack.pop() {
                let multiline = line > frame.start_line;
                if matches!(frame.opener, '(' | '[') && !frame.commas.is_empty() {
                    let expand = if multiline {
                        frame.first_on_next
                    } else {
                        frame.trailing && !(frame.opener == '(' && frame.commas.len() == 1 && looks_like_tuple(s, frame.pos))
                    };
                    if expand {
                        points.extend(frame.commas);
                    }
                }
            }
            touch(&mut stack, line, false);
            i += 1;
            continue;
        }
        if c == ',' {
            if let Some(top) = stack.last_mut() {
                top.commas.push(i);
            }
            touch(&mut stack, line, true);
            i += 1;
            continue;
        }
        if c == '\n' {
            line += 1;
        } else if !matches!(c, ' ' | '\t' | '\r') {
            touch(&mut stack, line, false);
        }
        i += 1;
    }
    points.sort_unstable();
    points
}

fn is_chain_dot(s: &[char], i: usize) -> bool {
    if s.get(i).copied() != Some('.') || s.get(i + 1).copied() == Some('.') || i > 0 && s[i - 1] == '.' {
        return false;
    }
    let Some(next) = s.get(i + 1).copied() else { return false };
    if !(next == '_' || next.is_alphanumeric()) {
        return false;
    }
    !(next.is_ascii_digit() && i > 0 && s[i - 1].is_ascii_digit())
}

fn find_chain_insert_points(s: &[char]) -> Vec<usize> {
    let n                      = s.len();
    let mut i                  = 0;
    let mut line               = 0;
    let mut stack              = vec![ChainFrame::default()];
    let mut points: Vec<usize> = Vec::new();

    while i < n {
        if let Some((content, hashes)) = raw_string_open(s, i) {
            let end = raw_string_close(s, content, hashes);
            line += s[i..end].iter().filter(|&&c| c == '\n').count();
            stack.last_mut().unwrap().last_token_line = Some(line);
            i                                         = end;
            continue;
        }
        let c = s[i];
        if c == '"' {
            i += 1;
            while i < n && s[i] != '"' {
                if s[i] == '\n' {
                    line += 1;
                }
                i += if s[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            stack.last_mut().unwrap().last_token_line = Some(line);
            continue;
        }
        if c == '\''
            && let Some(end) = char_literal_end(s, i)
        {
            stack.last_mut().unwrap().last_token_line = Some(line);
            i                                         = end;
            continue;
        }
        if starts_at(s, i, "//") {
            while i < n && s[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if starts_at(s, i, "/*") {
            let mut depth = 1;
            i += 2;
            while i < n && depth > 0 {
                if starts_at(s, i, "/*") {
                    depth += 1;
                    i += 2;
                } else if starts_at(s, i, "*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    if s[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if matches!(c, ' ' | '\t' | '\r') {
            i += 1;
            continue;
        }
        if starts_at(s, i, "::<") {
            let frame = stack.last_mut().unwrap();
            frame.generic_depth += 1;
            frame.last_token_line = Some(line);
            i += 3;
            continue;
        }
        if stack.last().unwrap().generic_depth > 0 {
            let frame = stack.last_mut().unwrap();
            if matches!(c, '(' | '[' | '{') {
                frame.generic_delimiter_depth += 1;
            } else if matches!(c, ')' | ']' | '}') && frame.generic_delimiter_depth > 0 {
                frame.generic_delimiter_depth -= 1;
            } else if c == '<' && frame.generic_delimiter_depth == 0 {
                frame.generic_depth += 1;
            } else if c == '>' && frame.generic_delimiter_depth == 0 && (i == 0 || s[i - 1] != '-') {
                frame.generic_depth -= 1;
            }
            frame.last_token_line = Some(line);
            i += 1;
            continue;
        }
        if matches!(c, '(' | '[' | '{') {
            let frame = stack.last_mut().unwrap();
            if c == '{' {
                frame.multiline = None;
            }
            frame.last_token_line = Some(line);
            stack.push(ChainFrame::default());
            i += 1;
            continue;
        }
        if matches!(c, ')' | ']' | '}') {
            if stack.len() > 1 {
                stack.pop();
            }
            stack.last_mut().unwrap().last_token_line = Some(line);
            i += 1;
            continue;
        }
        if is_chain_dot(s, i) {
            let frame     = stack.last_mut().unwrap();
            let multiline = *frame.multiline.get_or_insert_with(|| frame.last_token_line.is_some_and(|l| line > l));
            if multiline {
                points.push(i);
            }
            frame.last_token_line = Some(line);
            i += 1;
            continue;
        }
        let frame = stack.last_mut().unwrap();
        if c == ':' && s.get(i + 1).copied() == Some(':') {
            frame.last_token_line = Some(line);
            i += 2;
            continue;
        }
        if c == '.' || matches!(c, ',' | ';' | ':' | '=' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '!' | '<' | '>') {
            frame.multiline = None;
        }
        frame.last_token_line = Some(line);
        i += 1;
    }
    points
}

fn add_chain_marks(src: &str) -> String {
    let s: Vec<char> = src.chars().collect();
    let points       = find_chain_insert_points(&s);
    let mut out      = String::with_capacity(src.len() + points.len() * (MARK.len() + 2));
    let mut prev     = 0;
    for p in points {
        out.extend(&s[prev..p]);
        out.push(' ');
        out.push_str(MARK);
        out.push('\n');
        prev = p;
    }
    out.extend(&s[prev..]);
    out
}

fn add_marks(src: &str) -> String {
    let s: Vec<char> = src.chars().collect();
    let points       = find_insert_points(&s);
    let mut out      = String::with_capacity(src.len() + points.len() * (MARK.len() + 2));
    let mut prev     = 0;
    for p in points {
        out.extend(&s[prev..=p]);
        let mut j = p + 1;
        while j < s.len() && matches!(s[j], ' ' | '\t') {
            j += 1;
        }
        if j >= s.len() || s[j] == '\n' || starts_at(&s, j, "//") {
            out.push(' ');
            out.push_str(MARK);
        } else {
            out.push(' ');
            out.push_str(MARK);
            out.push('\n');
        }
        prev = p + 1;
    }
    out.extend(&s[prev..]);
    out
}

fn strip_marks(txt: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in txt.split('\n') {
        if line.contains(MARK) {
            let mut l = line.to_string();
            while let Some(pos) = l.find(MARK) {
                let mut start = pos;
                while start > 0 && matches!(l.as_bytes()[start - 1], b' ' | b'\t') {
                    start -= 1;
                }
                l.replace_range(start..pos + MARK.len(), "");
            }
            if l.trim().is_empty() {
                continue;
            }
            out.push(l);
        } else {
            out.push(line.to_string());
        }
    }
    out.join("\n")
}

#[derive(Clone)]
struct LineRec {
    start:       usize,
    end:         usize,
    start_depth: i32,
    eq:          Option<usize>,
    dirty:       bool,
}

#[allow(unused_assignments)]
fn line_records(s: &[char]) -> Vec<LineRec> {
    let n                      = s.len();
    let mut i                  = 0;
    let mut depth: i32         = 0;
    let mut recs: Vec<LineRec> = Vec::new();
    let mut rec = LineRec {
        start:       0,
        end:         0,
        start_depth: 0,
        eq:          None,
        dirty:       false,
    };

    macro_rules! close {
        ($pos:expr) => {{
            rec.end = $pos;
            recs.push(rec.clone());
            rec = LineRec {
                start:       $pos + 1,
                end:         $pos + 1,
                start_depth: depth,
                eq:          None,
                dirty:       false,
            };
        }};
    }

    while i < n {
        let c = s[i];
        if c == '\n' {
            close!(i);
            i += 1;
            continue;
        }
        if let Some((content, hashes)) = raw_string_open(s, i) {
            let end = raw_string_close(s, content, hashes);
            while i < end {
                if s[i] == '\n' {
                    close!(i);
                    rec.dirty = true;
                }
                i += 1;
            }
            continue;
        }
        if c == '"' {
            i += 1;
            while i < n && s[i] != '"' {
                if s[i] == '\n' {
                    close!(i);
                    rec.dirty = true;
                    i += 1;
                } else {
                    i += if s[i] == '\\' { 2 } else { 1 };
                }
            }
            i += 1;
            continue;
        }
        if c == '\'' {
            if let Some(end) = char_literal_end(s, i) {
                i = end;
                continue;
            }
        }
        if starts_at(s, i, "//") {
            while i < n && s[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if starts_at(s, i, "/*") {
            let mut d = 1;
            i += 2;
            while i < n && d > 0 {
                if s[i] == '\n' {
                    close!(i);
                    rec.dirty = true;
                    i += 1;
                } else if starts_at(s, i, "/*") {
                    d += 1;
                    i += 2;
                } else if starts_at(s, i, "*/") {
                    d -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if matches!(c, '(' | '[' | '{') {
            depth += 1;
        } else if matches!(c, ')' | ']' | '}') {
            depth -= 1;
        } else if c == '='
            && rec.eq.is_none()
            && depth == rec.start_depth
            && !rec.dirty
            && (i == 0 || !"=!<>+-*/%&|^".contains(s[i - 1]))
            && s.get(i + 1).map_or(true, |&x| x != '=' && x != '>')
        {
            rec.eq = Some(i);
        }
        i += 1;
    }
    close!(n);
    recs
}

fn brace_net(line: &[char]) -> (i32, i32) {
    let n         = line.len();
    let mut i     = 0;
    let mut net   = 0;
    let mut opens = 0;
    while i < n {
        let c = line[i];
        if c == '"' {
            i += 1;
            while i < n && line[i] != '"' {
                i += if line[i] == '\\' { 2 } else { 1 };
            }
        } else if c == '\'' && char_literal_end(line, i).is_some() {
            i = char_literal_end(line, i).unwrap() - 1;
        } else if starts_at(line, i, "//") {
            break;
        } else if c == '{' {
            net += 1;
            opens += 1;
        } else if c == '}' {
            net -= 1;
        }
        i += 1;
    }
    (net, opens)
}

fn signature(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace() && *c != ';').collect()
}

fn first_word(line: &str) -> String {
    line.trim_start().chars().take_while(|c| c.is_ascii_alphabetic() || *c == '_').collect()
}

fn inline_if_signatures(src: &str) -> Vec<String> {
    let s: Vec<char> = src.chars().collect();
    let mut sigs     = Vec::new();
    for rec in line_records(&s) {
        if rec.dirty {
            continue;
        }
        let text: String = s[rec.start..rec.end].iter().collect();
        let t            = text.trim();
        if !t.starts_with("if ") || !(t.ends_with('}') || t.ends_with("};")) {
            continue;
        }
        let chars: Vec<char> = t.chars().collect();
        let (net, opens)     = brace_net(&chars);
        if opens > 0 && net == 0 {
            sigs.push(signature(t));
        }
    }
    sigs
}

fn join_inline_ifs(txt: &str, sigs: &[String]) -> String {
    if sigs.is_empty() {
        return txt.to_string();
    }
    let s: Vec<char>           = txt.chars().collect();
    let recs                   = line_records(&s);
    let src_lines: Vec<String> = recs.iter().map(|r| s[r.start..r.end].iter().collect()).collect();
    let mut out: Vec<String>   = Vec::new();
    let mut i                  = 0;
    while i < recs.len() {
        let line = &src_lines[i];
        let t    = line.trim();
        if !recs[i].dirty && t.starts_with("if ") && t.ends_with('{') {
            let chars: Vec<char> = t.chars().collect();
            let (mut net, _)     = brace_net(&chars);
            if net > 0 {
                let mut k     = i + 1;
                let mut parts = vec![t.to_string()];
                let mut ok    = true;
                while k < recs.len() && net > 0 {
                    if recs[k].dirty {
                        ok = false;
                        break;
                    }
                    let part                  = src_lines[k].trim().to_string();
                    let part_chars: Vec<char> = part.chars().collect();
                    let (d, _)                = brace_net(&part_chars);
                    parts.push(part);
                    net += d;
                    k += 1;
                }
                if ok && net == 0 {
                    let joined = parts.join(" ");
                    if sigs.contains(&signature(&joined)) {
                        let indent: String = line.chars().take_while(|c| matches!(c, ' ' | '\t')).collect();
                        out.push(format!("{indent}{joined}"));
                        i = k;
                        continue;
                    }
                }
            }
        }
        out.push(line.clone());
        i += 1;
    }
    out.join("\n")
}

#[allow(unused_assignments)]
fn align_assignments(txt: &str) -> String {
    let s: Vec<char>                   = txt.chars().collect();
    let recs                           = line_records(&s);
    let mut lines: Vec<String>         = recs.iter().map(|r| s[r.start..r.end].iter().collect()).collect();
    let mut groups: Vec<Vec<usize>>    = Vec::new();
    let mut cur: Vec<usize>            = Vec::new();
    let mut cur_indent: Option<String> = None;

    macro_rules! flush {
        () => {{
            if cur.len() > 1 {
                groups.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            cur_indent = None;
        }};
    }

    for (idx, rec) in recs.iter().enumerate() {
        let line = &lines[idx];
        let eligible = rec.eq.is_some()
            && !rec.dirty
            && rec.eq.map_or(false, |eq| {
                let after: String   = s[eq + 1..rec.end].iter().collect();
                let from_eq: String = s[eq..rec.end].iter().collect();
                !after.trim().is_empty() && from_eq.contains(';')
            })
            && !KEYWORDS.contains(&first_word(line).as_str());
        if eligible {
            let indent: String = line.chars().take_while(|c| matches!(c, ' ' | '\t')).collect();
            if cur_indent.is_some() && cur_indent.as_deref() != Some(indent.as_str()) {
                flush!();
            }
            cur.push(idx);
            cur_indent = Some(indent);
        } else {
            flush!();
        }
    }
    flush!();

    for group in groups {
        let widths: Vec<usize> = group
            .iter()
            .map(|&idx| {
                let eq             = recs[idx].eq.unwrap();
                let prefix: String = s[recs[idx].start..eq].iter().collect();
                prefix.trim_end().chars().count()
            })
            .collect();
        let target = widths.iter().max().unwrap() + 1;
        for (&idx, &w) in group.iter().zip(widths.iter()) {
            let eq               = recs[idx].eq.unwrap();
            let eq_offset        = eq - recs[idx].start;
            let line             = &lines[idx];
            let chars: Vec<char> = line.chars().collect();
            let prefix: String   = chars[..eq_offset].iter().collect();
            let rest: String     = chars[eq_offset..].iter().collect();
            lines[idx]           = format!("{}{}{}", prefix.trim_end(), " ".repeat(target - w), rest);
        }
    }
    lines.join("\n")
}

fn cargo_bin(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let p   = PathBuf::from(env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))?).join(".cargo").join("bin").join(exe);
    p.exists().then_some(p)
}

fn rustfmt_path(configured: Option<&Path>) -> PathBuf {
    if let Some(p) = configured {
        return p.to_path_buf();
    }
    if let Ok(p) = env::var("MAGIC_FORMATTER_RUSTFMT") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    cargo_bin("rustfmt").unwrap_or_else(|| PathBuf::from("rustfmt"))
}

fn run(binary: &Path, args: &[&str], input: &str, cwd: &Path) -> Result<String, String> {
    let mut child = Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run {}: {e}", binary.display()))?;
    child.stdin.take().unwrap().write_all(input.as_bytes()).ok();
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn format(src: &str, dir: &Path, rustfmt: Option<&Path>) -> Result<String, String> {
    let sigs      = inline_if_signatures(src);
    let marked    = add_marks(&add_chain_marks(src));
    let edition   = find_edition(dir);
    let formatted = run(&rustfmt_path(rustfmt), &["+nightly", "--edition", &edition], &marked, dir)?;
    Ok(align_assignments(&join_inline_ifs(&strip_marks(&formatted), &sigs)))
}

fn find_up(start: &Path, name: &str) -> Option<PathBuf> {
    start.ancestors().find(|d| d.join(name).exists()).map(Path::to_path_buf)
}

pub fn topcoat(src: &str, dir: &Path, topcoat: Option<&Path>) -> Option<Result<String, String>> {
    let root   = find_up(dir, "Topcoat.toml")?;
    let binary = topcoat.map(Path::to_path_buf).or_else(|| cargo_bin("topcoat")).unwrap_or_else(|| PathBuf::from("topcoat"));
    Some(run(&binary, &["fmt", "--stdin"], src, &root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked_chain_lines(src: &str) -> Vec<String> {
        let s: Vec<char> = src.chars().collect();
        find_chain_insert_points(&s)
            .iter()
            .map(|&i| s[i..].iter().take_while(|&&c| c != '\n').collect())
            .collect()
    }

    #[test]
    fn marks_every_suffix_when_first_suffix_starts_on_next_line() {
        let src = r#"let port = std::env::var("PORT")
    .ok()
    .and_then(|p| p.parse::<u16>().ok())
    .unwrap_or(4002);"#;

        assert_eq!(
            marked_chain_lines(src),
            [
                ".ok()".to_string(),
                ".and_then(|p| p.parse::<u16>().ok())".to_string(),
                ".unwrap_or(4002);".to_string(),
            ]
        );
    }

    #[test]
    fn marks_inline_suffixes_after_a_multiline_chain_trigger() {
        let src = "let value = input\n    .parse::<Option<Result<u16, E>>>().ok();";

        assert_eq!(marked_chain_lines(src), [".parse::<Option<Result<u16, E>>>().ok();", ".ok();"]);
    }

    #[test]
    fn leaves_chain_alone_when_first_suffix_is_inline() {
        let src = "let value = input.parse()\n    .ok();";

        assert!(marked_chain_lines(src).is_empty());
    }

    #[test]
    fn handles_independent_chains_at_the_same_depth() {
        let src = "let first = input.parse();\nlet second = other\n    .parse().ok();";

        assert_eq!(marked_chain_lines(src), [".parse().ok();", ".ok();"]);
    }
}
