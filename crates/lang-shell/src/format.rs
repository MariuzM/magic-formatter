use tree_sitter::{Node, Parser, Tree};

pub struct Options {
    pub indent: String,
}

const ATOMIC: &[&str] = &["string", "raw_string", "ansi_c_string", "translated_string", "heredoc_body"];

const HEREDOC: &[&str] = &["heredoc_body", "heredoc_end"];

const CLOSERS: &[&str] = &["fi", "done", "esac", "}", ")", "]", "]]", "))"];

fn errors<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    if node.is_error() || node.is_missing() {
        out.push(node);
        return;
    }
    if !node.has_error() {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        errors(child, out);
    }
}

fn error_count(tree: &Tree) -> usize {
    let mut out = Vec::new();
    errors(tree.root_node(), &mut out);
    out.len()
}

fn parse(src: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_bash::LANGUAGE.into()).map_err(|e| e.to_string())?;
    parser.parse(src, None).ok_or_else(|| "parser failed".into())
}

fn verbatim_spans(tree: &Tree, src: &str) -> Vec<(usize, usize)> {
    let mut out    = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let kind = node.kind();
        let atom = ATOMIC.contains(&kind);
        if HEREDOC.contains(&kind) {
            out.push(((node.start_byte() - node.start_position().column).saturating_sub(1), node.end_byte()));
        } else if (atom || node.child_count() == 0) && src[node.start_byte()..node.end_byte()].contains('\n') {
            out.push((node.start_byte(), node.end_byte()));
        }
        if !atom && cursor.goto_first_child() {
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
    out
}

fn is_delimiter(anchor: Node, node: Node) -> bool {
    match node.kind() {
        "then" | "do" | "done" | "fi" | "esac" | "in" | "}" | ")" | "]" | "]]" | "))" | "elif_clause" | "else_clause" => true,
        "do_group" | "compound_statement" | "subshell" => anchor.child_by_field_name("body") == Some(node),
        _ => false,
    }
}

fn level_of(first: Node, row: usize, levels: &[usize]) -> usize {
    let mut node = first;
    while let Some(parent) = node.parent() {
        if parent.start_position().row < row {
            if parent.kind() == "program" {
                return 0;
            }
            let base = levels[parent.start_position().row];
            return if is_delimiter(parent, node) { base } else { base + 1 };
        }
        node = parent;
    }
    0
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blank,
    Verbatim,
    Comment,
    Code,
}

fn strip_ws(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().filter(|c| !c.is_whitespace())
}

fn mask_expansion_flags(bytes: &mut [u8]) {
    let mut i = 0;
    while i + 3 < bytes.len() {
        if !bytes[i..].starts_with(b"${(") {
            i += 1;
            continue;
        }
        let Some(len) = bytes[i + 3..].iter().take(40).position(|&b| b == b')' || b == b'\n') else {
            i += 1;
            continue;
        };
        let close = i + 3 + len;
        if bytes[close] != b')' || len == 0 {
            i += 1;
            continue;
        }
        let next = bytes.get(close + 1).copied().unwrap_or(b' ');
        if next.is_ascii_alphanumeric() || next == b'_' {
            bytes[i..close - 1].fill(b' ');
            bytes[close - 1] = b'$';
            bytes[close]     = b'{';
        } else {
            bytes[i..=close].fill(b' ');
            let mut depth = 1;
            let mut j     = close + 1;
            while j < bytes.len() {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            bytes[j] = b' ';
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
        }
        i = close + 1;
    }
}

pub fn mask_zsh(src: &str) -> String {
    let mut bytes = src.as_bytes().to_vec();
    mask_expansion_flags(&mut bytes);
    mask_glob_qualifiers(&mut bytes);
    String::from_utf8(bytes).unwrap_or_else(|_| src.to_string())
}

fn mask_glob_qualifiers(bytes: &mut [u8]) {
    let mut i = 1;
    while i < bytes.len() {
        let attached = !bytes[i - 1].is_ascii_whitespace() && !b"$=()|&;<>!`".contains(&bytes[i - 1]);
        if bytes[i] == b'(' && attached {
            let body = bytes[i + 1..].iter().take_while(|&&b| b.is_ascii_alphanumeric() || b"[],.:^-/@+".contains(&b)).count();
            let end  = i + 1 + body;
            let next = bytes.get(end + 1).copied().unwrap_or(b' ');
            if body > 0 && bytes.get(end) == Some(&b')') && !(next.is_ascii_alphanumeric() || next == b'_') {
                bytes[i]   = b'_';
                bytes[end] = b'_';
                i          = end;
            }
        }
        i += 1;
    }
}

pub fn format(src: &str, opts: &Options) -> Result<String, String> {
    let tree       = parse(&mask_zsh(src))?;
    let root       = tree.root_node();
    let mut broken = Vec::new();

    errors(root, &mut broken);

    let regions: Vec<(usize, usize)> = broken
        .iter()
        .map(|&e| {
            let mut n = e;
            while let Some(p) = n.parent().filter(|p| p.kind() != "program") {
                n = p;
            }
            (n.start_position().row, n.end_position().row)
        })
        .collect();

    let unparsed = |row: usize| regions.iter().any(|&(a, b)| a <= row && row <= b);
    let spans    = verbatim_spans(&tree, src);
    let inside   = |s: usize| spans.iter().any(|&(a, b)| a < s && s < b);

    let mut starts = vec![0];
    starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
    let raw: Vec<&str> = src.split('\n').collect();

    let mut kinds  = vec![Kind::Blank; raw.len()];
    let mut levels = vec![0; raw.len()];
    let mut closer = vec![false; raw.len()];

    for (row, line) in raw.iter().enumerate() {
        let start   = starts[row];
        let content = line.trim_start_matches([' ', '\t']);
        let prev    = row.checked_sub(1).map(|r| raw[r].trim_end_matches('\r'));

        let glued = prev
            .is_some_and(|p| p.ends_with('\\') && !p[..p.len() - 1].ends_with(char::is_whitespace))
            && content.len() == line.len();

        if row > 0 && (inside(start) || glued) {
            kinds[row]  = Kind::Verbatim;
            levels[row] = levels[row - 1];
            continue;
        }

        if content.trim().is_empty() {
            continue;
        }

        if unparsed(row) {
            kinds[row]  = Kind::Verbatim;
            levels[row] = row.checked_sub(1).map_or(0, |r| levels[r]);
            continue;
        }

        let p     = start + (line.len() - content.len());
        let first = root.descendant_for_byte_range(p, p + 1).unwrap_or(root);
        if first.kind() == "comment" {
            kinds[row] = Kind::Comment;
            continue;
        }

        kinds[row]  = Kind::Code;
        closer[row] = CLOSERS.contains(&first.kind());
        levels[row] = level_of(first, row, &levels);
    }

    for row in (0..raw.len()).rev() {
        if kinds[row] != Kind::Comment {
            continue;
        }
        let next    = (row + 1..raw.len()).find(|&r| matches!(kinds[r], Kind::Code | Kind::Verbatim));
        levels[row] = match next {
            Some(r) if closer[r] => levels[r] + 1,
            Some(r) => levels[r],
            None => 0,
        };
    }

    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut pending          = false;

    for (row, line) in raw.iter().enumerate() {
        match kinds[row] {
            Kind::Blank => pending = !out.is_empty(),
            Kind::Verbatim => {
                if pending {
                    out.push(String::new());
                    pending = false;
                }
                out.push(line.to_string());
            }
            Kind::Comment | Kind::Code => {
                if pending {
                    out.push(String::new());
                    pending = false;
                }
                let end     = starts[row] + line.len();
                let content = line.trim_start_matches([' ', '\t']);
                let trimmed = content.trim_end();
                let keep    = spans.iter().any(|&(a, b)| a <= end && end < b) || trimmed.len() < content.len() && trimmed.ends_with('\\');
                let body    = if keep { content } else { trimmed };
                out.push(format!("{}{}", opts.indent.repeat(levels[row]), body));
            }
        }
    }

    if out.is_empty() {
        return Ok(String::new());
    }

    let mut result = out.join("\n");
    result.push('\n');

    if !strip_ws(src).eq(strip_ws(&result)) {
        return Err("formatter safety check failed: output changed non-whitespace content".into());
    }

    if error_count(&parse(&mask_zsh(&result))?) > broken.len() {
        return Err("formatter safety check failed: output introduced a syntax error".into());
    }

    Ok(result)
}
