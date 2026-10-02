use tree_sitter::{Node, Tree};

use crate::language::Analysis;
use crate::locals::{Def, Locals};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Local(Def),
    Global,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub name:   String,
    pub target: Target,
}

pub fn identifier_at<'t>(lang: &dyn Analysis, tree: &'t Tree, offset: usize) -> Option<Node<'t>> {
    let kinds = lang.identifier_kinds();
    let root  = tree.root_node();
    [offset, offset.saturating_sub(1)]
        .into_iter()
        .filter_map(|o| root.descendant_for_byte_range(o, o))
        .find(|n| kinds.contains(&n.kind()))
}

pub fn symbol_at(lang: &dyn Analysis, tree: &Tree, src: &str, offset: usize) -> Option<Symbol> {
    let node   = identifier_at(lang, tree, offset)?;
    let name   = node.utf8_text(src.as_bytes()).ok()?.to_string();
    let target = if lang.resolves_locally(node) {
        Locals::collect(lang, tree, src)
            .resolve(&name, node.start_byte())
            .map_or(Target::Global, |d| Target::Local(d.clone()))
    } else {
        Target::Global
    };
    Some(Symbol { name, target })
}

pub fn occurrences(
    lang: &dyn Analysis,
    tree: &Tree,
    src: &str,
    symbol: &Symbol,
    include_declaration: bool,
) -> Vec<(usize, usize)> {
    let bytes      = src.as_bytes();
    let kinds      = lang.identifier_kinds();
    let locals     = Locals::collect(lang, tree, src);
    let mut out    = Vec::new();
    let mut cursor = tree.walk();

    'walk: loop {
        let node = cursor.node();
        if node.child_count() == 0 && kinds.contains(&node.kind()) && node.utf8_text(bytes).is_ok_and(|t| t == symbol.name) {
            let local = if lang.resolves_locally(node) {
                locals.resolve(&symbol.name, node.start_byte())
            } else {
                None
            };
            let hit = match &symbol.target {
                Target::Local(def) => local == Some(def) && (include_declaration || node.start_byte() != def.start),
                Target::Global => local.is_none(),
            };
            if hit {
                out.push((node.start_byte(), node.end_byte()));
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
    out
}
