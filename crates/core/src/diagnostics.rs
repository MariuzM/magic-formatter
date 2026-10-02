use tree_sitter::{Node, Tree};

const LIMIT: usize = 100;

fn collect(node: Node, out: &mut Vec<(usize, usize, String)>) {
    if out.len() >= LIMIT {
        return;
    }
    if node.is_missing() {
        out.push((node.start_byte(), node.end_byte(), format!("Missing `{}`", node.kind())));
        return;
    }
    if node.is_error() {
        out.push((node.start_byte(), node.end_byte(), "Syntax error".into()));
        return;
    }
    if !node.has_error() {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, out);
    }
}

pub fn syntax_errors(tree: &Tree) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    collect(tree.root_node(), &mut out);
    out
}
