use std::collections::HashSet;

use tree_sitter::Tree;

use crate::index::identifiers;
use crate::language::Analysis;
use crate::locals::Locals;
use crate::symbols::{SymbolKind, document_symbols, flatten};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Keyword,
    Builtin,
    Symbol(SymbolKind),
    Text,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub label:  String,
    pub kind:   ItemKind,
    pub detail: Option<String>,
}

pub fn grammar_keywords(lang: &tree_sitter::Language) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in 0..lang.node_kind_count() as u16 {
        if lang.node_kind_is_named(id) || !lang.node_kind_is_visible(id) {
            continue;
        }
        if let Some(kind) = lang.node_kind_for_id(id)
            && kind.len() > 1
            && kind.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
            && !out.iter().any(|k| k == kind)
        {
            out.push(kind.to_string());
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn word_start(src: &str, offset: usize) -> usize {
    src[..offset].char_indices().rev().take_while(|(_, c)| is_ident(*c)).last().map_or(offset, |(i, _)| i)
}

pub fn is_member_access(src: &str, offset: usize) -> bool {
    src[..word_start(src, offset)].ends_with('.')
}

struct Items {
    seen: HashSet<String>,
    out:  Vec<Item>,
    skip: String,
}

impl Items {
    fn push(&mut self, label: &str, kind: ItemKind, detail: Option<String>) {
        if label != self.skip && !label.is_empty() && self.seen.insert(label.to_string()) {
            self.out.push(Item {
                label: label.to_string(),
                kind,
                detail,
            });
        }
    }
}

pub fn complete(lang: &dyn Analysis, tree: &Tree, src: &str, offset: usize, extra: Vec<Item>) -> Vec<Item> {
    let mut items = Items {
        seen: HashSet::new(),
        out:  Vec::new(),
        skip: src[word_start(src, offset)..offset].to_string(),
    };
    for def in Locals::collect(lang, tree, src).visible(offset) {
        items.push(&def.name, ItemKind::Symbol(SymbolKind::Variable), None);
    }
    let symbols  = document_symbols(lang, tree, src);
    let mut flat = Vec::new();
    flatten(&symbols, None, &mut flat);
    for (symbol, container) in flat.iter().filter(|(s, _)| !s.kind.is_member()) {
        items.push(&symbol.name, ItemKind::Symbol(symbol.kind), container.map(String::from));
    }
    for keyword in lang.keywords() {
        items.push(&keyword, ItemKind::Keyword, None);
    }
    for builtin in lang.builtins() {
        items.push(builtin, ItemKind::Builtin, None);
    }
    for item in extra {
        items.push(&item.label, item.kind, item.detail);
    }
    for word in identifiers(src) {
        items.push(word, ItemKind::Text, None);
    }
    items.out
}

pub fn members(lang: &dyn Analysis, tree: &Tree, src: &str, offset: usize, extra: Vec<Item>) -> Vec<Item> {
    let mut items = Items {
        seen: HashSet::new(),
        out:  Vec::new(),
        skip: src[word_start(src, offset)..offset].to_string(),
    };
    let symbols  = document_symbols(lang, tree, src);
    let mut flat = Vec::new();
    flatten(&symbols, None, &mut flat);
    for (symbol, container) in flat.iter().filter(|(s, _)| s.kind.is_member()) {
        items.push(&symbol.name, ItemKind::Symbol(symbol.kind), container.map(String::from));
    }
    for item in extra {
        items.push(&item.label, item.kind, item.detail);
    }
    let bytes = src.as_bytes();
    for (i, _) in src.match_indices('.') {
        let rest = &src[i + 1..];
        let len  = rest.find(|c: char| !is_ident(c)).unwrap_or(rest.len());
        if len > 0 && !bytes[i + 1].is_ascii_digit() {
            items.push(&rest[..len], ItemKind::Text, None);
        }
    }
    items.out
}
