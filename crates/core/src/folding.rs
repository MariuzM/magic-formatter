use std::collections::BTreeMap;

use tree_sitter::{Node, Tree};

use crate::language::Analysis;
use crate::text::LineIndex;

const CLOSERS: &[&str] = &[")", "]", "}"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FoldKind {
    Region,
    Comment,
    Imports,
}

fn starts_line(src: &str, node: Node) -> bool {
    src[..node.start_byte()].rsplit('\n').next().unwrap_or_default().trim().is_empty()
}

fn last_leaf(node: Node) -> Node {
    let mut node = node;
    while let Some(child) = node.child(node.child_count().wrapping_sub(1) as _) {
        node = child;
    }
    node
}

pub fn folding_ranges(lang: &dyn Analysis, tree: &Tree, src: &str) -> Vec<(usize, usize, FoldKind)> {
    let lines                                         = LineIndex::new(src);
    let comments                                      = lang.comment_kinds();
    let imports                                       = lang.import_kinds();
    let mut folds: BTreeMap<usize, (usize, FoldKind)> = BTreeMap::new();
    let mut add                                       = |start: usize, end: usize, kind: FoldKind| {
        if end > start {
            let entry = folds.entry(start).or_insert((end, kind));
            if end > entry.0 {
                *entry = (end, kind);
            }
        }
    };
    let end_line = |node: Node| {
        let last = last_leaf(node);
        let line = lines.line_of(node.end_byte().saturating_sub(1).max(node.start_byte()));
        if CLOSERS.contains(&last.kind()) && starts_line(src, last) { line.saturating_sub(1) } else { line }
    };

    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let n    = node.child_count();
        if n >= 2
            && let (Some(first), Some(last)) = (node.child(0), node.child(n - 1))
            && matches!((first.kind(), last.kind()), ("(", ")") | ("[", "]") | ("{", "}"))
        {
            add(lines.line_of(first.start_byte()), end_line(node), FoldKind::Region);
        }
        if let Some((start, end)) = lang.fold(node) {
            let line = lines.line_of(end.saturating_sub(1).max(start));
            let last = last_leaf(node);
            let line = if node.end_byte() == end && CLOSERS.contains(&last.kind()) && starts_line(src, last) {
                line.saturating_sub(1)
            } else {
                line
            };
            add(lines.line_of(start), line, FoldKind::Region);
        }
        if comments.contains(&node.kind()) {
            add(lines.line_of(node.start_byte()), lines.line_of(node.end_byte().saturating_sub(1)), FoldKind::Comment);
        }
        if n > 0 {
            let category = |c: Node| {
                if comments.contains(&c.kind()) && starts_line(src, c) {
                    Some(FoldKind::Comment)
                } else if imports.contains(&c.kind()) {
                    Some(FoldKind::Imports)
                } else {
                    None
                }
            };
            let mut c               = node.walk();
            let children: Vec<Node> = node.children(&mut c).collect();
            let mut i               = 0;
            while i < children.len() {
                let Some(kind) = category(children[i]) else {
                    i += 1;
                    continue;
                };
                let mut j = i;
                while j + 1 < children.len()
                    && category(children[j + 1]) == Some(kind)
                    && lines.line_of(children[j + 1].start_byte()) <= lines.line_of(children[j].end_byte()) + 1
                {
                    j += 1;
                }
                if j > i {
                    add(lines.line_of(children[i].start_byte()), lines.line_of(children[j].end_byte().saturating_sub(1)), kind);
                }
                i = j + 1;
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
    folds.into_iter().map(|(start, (end, kind))| (start, end, kind)).collect()
}

pub fn selection_ranges(tree: &Tree, offset: usize) -> Vec<(usize, usize)> {
    let mut out  = Vec::new();
    let mut node = tree.root_node().descendant_for_byte_range(offset, offset);
    while let Some(n) = node {
        let range = (n.start_byte(), n.end_byte());
        if out.last() != Some(&range) {
            out.push(range);
        }
        node = n.parent();
    }
    out
}
