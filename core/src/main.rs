use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{exit, Command, Stdio};

const MARK: &str          = concat!("// __rustfmt_", "magic__");
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

fn find_edition() -> String {
    let mut dir = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
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
        } else if c == '=' && rec.eq.is_none() && depth == rec.start_depth && !rec.dirty && (i == 0 || !"=!<>+-*/%&|^".contains(s[i - 1])) && s.get(i + 1).map_or(true, |&x| x != '=' && x != '>') {
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

fn rustfmt_path() -> PathBuf {
    if let Ok(p) = env::var("RUSTFMT_MAGIC_RUSTFMT") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(home) = env::var_os("HOME") {
        let p = PathBuf::from(home).join(".cargo").join("bin").join("rustfmt");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("rustfmt")
}

fn main() {
    let mut src = String::new();
    if io::stdin().read_to_string(&mut src).is_err() {
        exit(1);
    }
    let sigs   = inline_if_signatures(&src);
    let marked = add_marks(&src);

    let mut child = match Command::new(rustfmt_path())
        .args(["+nightly", "--edition", &find_edition()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rustfmt-magic: failed to run rustfmt: {e}");
            exit(1);
        }
    };
    child.stdin.take().unwrap().write_all(marked.as_bytes()).ok();
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("rustfmt-magic: {e}");
            exit(1);
        }
    };
    io::stderr().write_all(&output.stderr).ok();
    if !output.status.success() {
        exit(output.status.code().unwrap_or(1));
    }
    let formatted = String::from_utf8_lossy(&output.stdout);
    let result    = align_assignments(&join_inline_ifs(&strip_marks(&formatted), &sigs));
    print!("{result}");
}
