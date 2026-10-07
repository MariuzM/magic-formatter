use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::symbols::{IndexedSymbol, SymbolKind};

#[derive(Default)]
pub struct Index {
    paths:      Vec<Arc<Path>>,
    ids:        HashMap<Arc<Path>, u32>,
    file_syms:  Vec<Vec<u32>>,
    symbols:    HashMap<Box<str>, u32>,
    postings:   Vec<Vec<u32>>,
    file_enums: Vec<Vec<Box<str>>>,
    enums:      HashMap<Box<str>, u32>,
    functions:  HashMap<Box<str>, u32>,
    outlines:   Vec<Vec<IndexedSymbol>>,
    file_defs:  Vec<Option<Vec<u32>>>,
}

impl Index {
    pub fn update(&mut self, path: &Path, text: &str, outline: Vec<IndexedSymbol>, defined: Option<Vec<(usize, usize)>>) {
        let id = self.remove(path).unwrap_or_else(|| {
            let id              = self.paths.len() as u32;
            let path: Arc<Path> = Arc::from(path);
            self.paths.push(path.clone());
            self.ids.insert(path, id);
            self.file_syms.push(Vec::new());
            self.file_enums.push(Vec::new());
            self.outlines.push(Vec::new());
            self.file_defs.push(None);
            id
        });

        for symbol in outline.iter().filter(|s| s.kind == SymbolKind::Function) {
            *self.functions.entry(symbol.name.clone()).or_default() += 1;
        }

        self.outlines[id as usize] = outline;

        let declared: Vec<Box<str>> = enum_declarations(text).map(Box::from).collect();

        for name in &declared {
            *self.enums.entry(name.clone()).or_default() += 1;
        }

        self.file_enums[id as usize] = declared;

        let unique: HashSet<&str> = identifiers(text).collect();
        let mut syms              = Vec::with_capacity(unique.len());

        for name in unique {
            let sym = match self.symbols.get(name) {
                Some(&s) => s,
                None => {
                    let s = self.postings.len() as u32;
                    self.symbols.insert(name.into(), s);
                    self.postings.push(Vec::new());
                    s
                }
            };
            let postings = &mut self.postings[sym as usize];
            if let Err(at) = postings.binary_search(&id) {
                postings.insert(at, id);
            }
            syms.push(sym);
        }
        self.file_syms[id as usize] = syms;

        self.file_defs[id as usize] = defined.map(|ranges| {
            let mut defs: Vec<u32> =
                ranges.into_iter().filter_map(|(start, end)| self.symbols.get(identifiers(text.get(start..end)?).next()?).copied()).collect();
            defs.sort_unstable();
            defs.dedup();
            defs
        });
    }

    pub fn remove(&mut self, path: &Path) -> Option<u32> {
        let id = *self.ids.get(path)?;

        for symbol in std::mem::take(&mut self.outlines[id as usize]) {
            if symbol.kind == SymbolKind::Function
                && let Some(n) = self.functions.get_mut(&symbol.name)
            {
                *n -= 1;
                if *n == 0 {
                    self.functions.remove(&symbol.name);
                }
            }
        }

        self.file_defs[id as usize] = None;

        for sym in std::mem::take(&mut self.file_syms[id as usize]) {
            let postings = &mut self.postings[sym as usize];
            if let Ok(at) = postings.binary_search(&id) {
                postings.remove(at);
            }
        }

        for name in std::mem::take(&mut self.file_enums[id as usize]) {
            if let Some(n) = self.enums.get_mut(&name) {
                *n -= 1;
                if *n == 0 {
                    self.enums.remove(&name);
                }
            }
        }
        Some(id)
    }

    pub fn is_enum(&self, name: &str) -> bool {
        self.enums.contains_key(name)
    }

    pub fn is_function(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }

    pub fn files_with(&self, name: &str) -> Vec<Arc<Path>> {
        self.files_where(name, |_, _| true)
    }

    pub fn files_defining(&self, name: &str) -> Vec<Arc<Path>> {
        self.files_where(name, |sym, file| self.file_defs[file as usize].as_ref().is_none_or(|defs| defs.binary_search(&sym).is_ok()))
    }

    fn files_where(&self, name: &str, keep: impl Fn(u32, u32) -> bool) -> Vec<Arc<Path>> {
        let Some(&sym) = self.symbols.get(identifiers(name).next().unwrap_or(name)) else {
            return Vec::new();
        };
        let mut files: Vec<Arc<Path>> =
            self.postings[sym as usize].iter().filter(|&&f| keep(sym, f)).map(|&f| self.paths[f as usize].clone()).collect();
        files.sort_unstable();
        files
    }

    pub fn outline(&self) -> impl Iterator<Item = (&Path, &IndexedSymbol)> {
        let mut order: Vec<usize> = (0..self.paths.len()).collect();
        order.sort_unstable_by(|&a, &b| self.paths[a].cmp(&self.paths[b]));
        order.into_iter().flat_map(move |i| self.outlines[i].iter().map(move |s| (&*self.paths[i], s)))
    }

    pub fn file_count(&self) -> usize {
        self.file_syms.iter().filter(|s| !s.is_empty()).count()
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

pub fn identifiers(text: &str) -> impl Iterator<Item = &str> {
    let bytes = text.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < bytes.len() {
            if is_ident_byte(bytes[i]) && !bytes[i].is_ascii_digit() {
                let start = i;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                return Some(&text[start..i]);
            }
            if bytes[i].is_ascii_digit() {
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                continue;
            }
            i += 1;
        }
        None
    })
}

pub fn declarations<'a>(text: &'a str, keywords: &'a [&str]) -> impl Iterator<Item = &'a str> {
    let mut prev = "";
    identifiers(text).filter_map(move |name| {
        let hit = keywords.contains(&prev) && name.starts_with(|c: char| c.is_ascii_uppercase());
        prev    = name;
        hit.then_some(name)
    })
}

pub fn enum_declarations(text: &str) -> impl Iterator<Item = &str> {
    declarations(text, &["enum"])
}

pub fn type_declarations(text: &str) -> impl Iterator<Item = &str> {
    declarations(text, &["struct", "union", "trait", "type", "class", "protocol", "actor", "typealias"])
}
