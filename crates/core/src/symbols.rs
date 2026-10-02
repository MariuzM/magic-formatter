use tree_sitter::{Node, Tree};

use crate::language::Analysis;
use crate::text::LineIndex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Module        = 2,
    Namespace     = 3,
    Class         = 5,
    Method        = 6,
    Property      = 7,
    Field         = 8,
    Constructor   = 9,
    Enum          = 10,
    Interface     = 11,
    Function      = 12,
    Variable      = 13,
    Constant      = 14,
    EnumMember    = 22,
    Struct        = 23,
    TypeParameter = 26,
}

impl SymbolKind {
    pub fn is_member(self) -> bool {
        matches!(self, Self::Method | Self::Property | Self::Field | Self::EnumMember)
    }
}

pub struct SymbolInfo {
    pub name:      String,
    pub kind:      SymbolKind,
    pub selection: (usize, usize),
    pub detail:    Option<String>,
}

pub struct Symbol {
    pub name:      String,
    pub kind:      SymbolKind,
    pub detail:    Option<String>,
    pub range:     (usize, usize),
    pub selection: (usize, usize),
    pub children:  Vec<Symbol>,
}

#[derive(Clone, Debug)]
pub struct IndexedSymbol {
    pub name:      Box<str>,
    pub kind:      SymbolKind,
    pub container: Option<Box<str>>,
    pub line:      u32,
    pub start:     u32,
    pub end:       u32,
}

fn walk(lang: &dyn Analysis, node: Node, src: &str, out: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match lang.symbol(child, src) {
            Some(info) => {
                let mut symbol = Symbol {
                    name:      info.name,
                    kind:      info.kind,
                    detail:    info.detail,
                    range:     (child.start_byte(), child.end_byte()),
                    selection: info.selection,
                    children:  Vec::new(),
                };
                walk(lang, child, src, &mut symbol.children);
                out.push(symbol);
            }
            None => walk(lang, child, src, out),
        }
    }
}

pub fn document_symbols(lang: &dyn Analysis, tree: &Tree, src: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    walk(lang, tree.root_node(), src, &mut out);
    out
}

pub fn flatten<'s>(symbols: &'s [Symbol], container: Option<&'s str>, out: &mut Vec<(&'s Symbol, Option<&'s str>)>) {
    for symbol in symbols {
        out.push((symbol, container));
        flatten(&symbol.children, Some(&symbol.name), out);
    }
}

pub fn indexed(lang: &dyn Analysis, tree: &Tree, src: &str) -> Vec<IndexedSymbol> {
    let symbols  = document_symbols(lang, tree, src);
    let lines    = LineIndex::new(src);
    let mut flat = Vec::new();
    flatten(&symbols, None, &mut flat);
    flat.into_iter()
        .map(|(s, container)| {
            let (line, start) = lines.position(src, s.selection.0);
            let (_, end)      = lines.position(src, s.selection.1);
            IndexedSymbol {
                name: s.name.as_str().into(),
                kind: s.kind,
                container: container.map(Box::from),
                line,
                start,
                end,
            }
        })
        .collect()
}
