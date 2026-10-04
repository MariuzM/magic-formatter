use tree_sitter::{Node, Parser, Tree};

pub struct Options {
    pub indent: String,
}

const HEREDOC_INSTRUCTIONS: &[&str] = &["RUN", "COPY", "ADD"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

struct Line {
    text:     String,
    verbatim: bool,
}

fn parse(src: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_containerfile::LANGUAGE.into()).map_err(|e| e.to_string())?;
    parser.parse(src, None).ok_or_else(|| "parser failed".into())
}

fn error_count(node: Node) -> usize {
    if node.is_error() || node.is_missing() {
        return 1;
    }
    if !node.has_error() {
        return 0;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).map(error_count).sum()
}

fn strip_ws(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().filter(|c| !c.is_whitespace())
}

fn escape_char(src: &str) -> char {
    for line in src.lines() {
        let Some((key, value)) = line.trim().strip_prefix('#').and_then(|d| d.split_once('=')) else {
            break;
        };
        let key = key.trim();
        if key.is_empty() || key.contains(char::is_whitespace) {
            break;
        }
        if key.eq_ignore_ascii_case("escape") {
            return if value.trim() == "`" { '`' } else { '\\' };
        }
    }
    '\\'
}

fn continues(line: &str, esc: char) -> bool {
    let mut chars = line.trim_end_matches([' ', '\t', '\r']).chars().rev();
    chars.next() == Some(esc) && chars.next().is_some_and(|c| c != esc)
}

fn is_skipped(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

fn scan(line: &str, esc: char, mut q: Quote) -> Quote {
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        q = match (q, c) {
            (Quote::None | Quote::Double, c) if c == esc => {
                chars.next();
                q
            }
            (Quote::None, '\'') => Quote::Single,
            (Quote::None, '"') => Quote::Double,
            (Quote::Single, '\'') | (Quote::Double, '"') => Quote::None,
            _ => q,
        };
    }
    q
}

fn heredoc_names(line: &str, out: &mut Vec<(String, bool)>) {
    for word in line.split_whitespace() {
        let Some(rest) = word.trim_start_matches(|c: char| c.is_ascii_digit()).strip_prefix("<<") else {
            continue;
        };
        let (strip, rest) = match rest.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, rest),
        };
        if rest.contains('<') {
            continue;
        }
        let name = ['"', '\'']
            .iter()
            .find_map(|q| rest.strip_prefix(*q).and_then(|r| r.strip_suffix(*q)))
            .unwrap_or(rest);
        if !name.is_empty() {
            out.push((name.to_string(), strip));
        }
    }
}

fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

fn rebase(line: &str, min: usize, prefix: &str) -> String {
    let extra: String = indent_of(line).chars().skip(min).collect();
    format!("{prefix}{extra}{}", line.trim())
}

fn normalize_arrays(src: &str, tree: &Tree) -> String {
    let mut edits  = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        if node.kind() == "json_string_array" && !node.has_error() && !src[node.byte_range()].contains('\n') {
            let mut c               = node.walk();
            let children: Vec<Node> = node.named_children(&mut c).collect();
            if children.iter().all(|n| n.kind() == "json_string") {
                let items: Vec<&str> = children.iter().map(|n| &src[n.byte_range()]).collect();
                edits.push((node.start_byte(), node.end_byte(), format!("[{}]", items.join(", "))));
            }
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    let mut out = String::with_capacity(src.len());
    let mut pos = 0;
    for (start, end, text) in edits {
        out.push_str(&src[pos..start]);
        out.push_str(&text);
        pos = end;
    }
    out.push_str(&src[pos..]);
    out
}

fn instruction(src: &[&str], i: usize, esc: char, opts: &Options, out: &mut Vec<Line>) -> usize {
    let mut end  = i;
    let mut cont = continues(src[i], esc);
    while cont && end + 1 < src.len() {
        end += 1;
        if !is_skipped(src[end]) {
            cont = continues(src[end], esc);
        }
    }

    let head       = src[i].trim();
    let keyword    = head.split_whitespace().next().unwrap_or_default();
    let mut q      = scan(src[i], esc, Quote::None);
    let mut quoted = vec![false; end + 1 - i];
    for k in i + 1..=end {
        if is_skipped(src[k]) {
            continue;
        }
        quoted[k - i] = q != Quote::None;
        q             = scan(src[k], esc, q);
    }
    let min = (i + 1..=end)
        .filter(|&k| !is_skipped(src[k]) && !quoted[k - i])
        .map(|k| indent_of(src[k]).chars().count())
        .min()
        .unwrap_or(0);

    out.push(Line {
        text:     match head.split_once(char::is_whitespace) {
            Some((kw, rest)) => format!("{kw} {}", rest.trim_start()),
            None => head.to_string(),
        },
        verbatim: false,
    });
    for k in i + 1..=end {
        let line = src[k];
        let text = if line.trim().is_empty() {
            String::new()
        } else if quoted[k - i] {
            line.trim_end().to_string()
        } else if line.trim_start().starts_with('#') && indent_of(line).chars().count() < min {
            format!("{}{}", opts.indent, line.trim())
        } else {
            rebase(line, min, &opts.indent)
        };
        out.push(Line { text, verbatim: false });
    }

    let mut heredocs = Vec::new();
    if HEREDOC_INSTRUCTIONS.iter().any(|h| h.eq_ignore_ascii_case(keyword)) {
        for line in src[i..=end].iter().filter(|l| !is_skipped(l)) {
            heredoc_names(line, &mut heredocs);
        }
    }
    let mut next = end + 1;
    for (name, strip) in heredocs {
        while next < src.len() && !(next + 1 == src.len() && src[next].is_empty()) {
            let line = src[next];
            out.push(Line {
                text:     line.to_string(),
                verbatim: true,
            });
            next += 1;
            let body = line.trim_end_matches('\r');
            if body == name || strip && body.trim_start_matches('\t') == name {
                break;
            }
        }
    }
    next
}

fn layout(src: &str, opts: &Options) -> String {
    let esc              = escape_char(src);
    let lines: Vec<&str> = src.split('\n').collect();
    let mut out          = Vec::new();
    let mut i            = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push(Line {
                text:     trimmed.to_string(),
                verbatim: false,
            });
            i += 1;
        } else {
            i = instruction(&lines, i, esc, opts, &mut out);
        }
    }

    let blank                  = |l: &Line| !l.verbatim && l.text.is_empty();
    let mut result: Vec<&Line> = Vec::with_capacity(out.len());
    for line in &out {
        if blank(line) && result.last().is_none_or(|l| blank(l)) {
            continue;
        }
        result.push(line);
    }
    while result.last().is_some_and(|l| blank(l)) {
        result.pop();
    }
    let mut text = result.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
    text.push('\n');
    text
}

pub fn format(src: &str, opts: &Options) -> Result<String, String> {
    if src.trim().is_empty() {
        return Ok(String::new());
    }
    let tree       = parse(src)?;
    let errors     = error_count(tree.root_node());
    let normalized = normalize_arrays(src, &tree);
    let out        = layout(&normalized, opts);

    if !strip_ws(src).eq(strip_ws(&out)) {
        return Err("formatter safety check failed: output changed non-whitespace content".into());
    }
    if error_count(parse(&out)?.root_node()) > errors {
        return Err("formatter safety check failed: output introduced a syntax error".into());
    }
    Ok(out)
}
