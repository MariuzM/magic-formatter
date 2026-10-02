use tree_sitter::Node;

pub struct Description {
    pub signature: String,
    pub docs:      Option<String>,
    pub params:    Vec<String>,
}

pub struct Call<'t> {
    pub callee: Node<'t>,
    pub args:   Node<'t>,
}

pub fn collapse_ws(s: &str) -> String {
    let joined    = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out   = String::with_capacity(joined.len());
    let mut chars = joined.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ' ' && chars.peek().is_some_and(|n| matches!(n, ')' | ']' | ',')) {
            continue;
        }
        out.push(c);
        if matches!(c, '(' | '[') && chars.peek() == Some(&' ') {
            chars.next();
        }
    }
    out.replace(",)", ")").replace(",]", "]")
}

pub fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or_default().trim_end();
    let more = s.contains('\n');
    match line.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None if more => format!("{line} …"),
        None => line.to_string(),
    }
}

fn strip_comment(text: &str) -> Vec<String> {
    let text = text.trim();
    if let Some(body) = text.strip_prefix("/**").or_else(|| text.strip_prefix("/*")) {
        let body = body.strip_suffix("*/").unwrap_or(body);
        return body
            .lines()
            .map(|l| {
                let l = l.trim_start();
                l.strip_prefix("* ").or_else(|| l.strip_prefix('*')).unwrap_or(l).trim_end().to_string()
            })
            .collect();
    }
    let body = ["///", "//!", "//", "#"].iter().find_map(|p| text.strip_prefix(p)).unwrap_or(text);
    vec![body.strip_prefix(' ').unwrap_or(body).trim_end().to_string()]
}

pub fn comments_before(node: Node, src: &str, kinds: &[&str]) -> Option<String> {
    let mut lines = Vec::new();
    let mut next  = node;
    while let Some(prev) = next.prev_sibling().filter(|p| kinds.contains(&p.kind())) {
        let gap = &src[prev.end_byte()..next.start_byte()];
        if gap.matches('\n').count() > 1 || !src[..prev.start_byte()].rsplit('\n').next().unwrap_or_default().trim().is_empty() {
            break;
        }
        let text = &src[prev.start_byte()..prev.end_byte()];
        if text.starts_with("#!") {
            break;
        }
        lines.splice(0..0, strip_comment(text));
        next = prev;
    }
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

pub fn escape_markdown(plain: &str) -> String {
    plain
        .lines()
        .map(|line| {
            let indent  = line.len() - line.trim_start().len();
            let mut out = "&nbsp;".repeat(indent);
            for c in line.trim_start().chars() {
                if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|') {
                    out.push('\\');
                }
                out.push(c);
            }
            out
        })
        .collect::<Vec<_>>()
        .join("  \n")
}

pub fn markdown(fence: &str, d: &Description) -> String {
    let mut out = format!("```{fence}\n{}\n```", d.signature);
    if let Some(docs) = &d.docs {
        out.push_str("\n\n---\n\n");
        out.push_str(docs);
    }
    out
}

pub fn active_parameter(args: Node, offset: usize) -> usize {
    let mut cursor = args.walk();
    args.children(&mut cursor).filter(|c| c.kind() == "," && c.end_byte() <= offset).count()
}
