use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::symbols::IndexedSymbol;

#[derive(Default)]
pub struct Index {
    paths:      Vec<PathBuf>,
    ids:        HashMap<PathBuf, u32>,
    file_syms:  Vec<Vec<u32>>,
    symbols:    HashMap<Box<str>, u32>,
    postings:   Vec<Vec<u32>>,
    file_enums: Vec<Vec<Box<str>>>,
    enums:      HashMap<Box<str>, u32>,
    outlines:   Vec<Vec<IndexedSymbol>>,
}

impl Index {
    pub fn update(&mut self, path: &Path, text: &str, outline: Vec<IndexedSymbol>) {
        let id = self.remove(path).unwrap_or_else(|| {
            let id = self.paths.len() as u32;
            self.paths.push(path.to_path_buf());
            self.ids.insert(path.to_path_buf(), id);
            self.file_syms.push(Vec::new());
            self.file_enums.push(Vec::new());
            self.outlines.push(Vec::new());
            id
        });

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
            self.postings[sym as usize].push(id);
            syms.push(sym);
        }
        self.file_syms[id as usize] = syms;
    }

    pub fn remove(&mut self, path: &Path) -> Option<u32> {
        let id = *self.ids.get(path)?;

        self.outlines[id as usize].clear();

        for sym in std::mem::take(&mut self.file_syms[id as usize]) {
            self.postings[sym as usize].retain(|&f| f != id);
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

    pub fn files_with(&self, name: &str) -> Vec<PathBuf> {
        self.symbols
            .get(identifiers(name).next().unwrap_or(name))
            .map(|&s| self.postings[s as usize].iter().map(|&f| self.paths[f as usize].clone()).collect())
            .unwrap_or_default()
    }

    pub fn outline(&self) -> impl Iterator<Item = (&Path, &IndexedSymbol)> {
        self.paths.iter().zip(&self.outlines).flat_map(|(p, syms)| syms.iter().map(move |s| (p.as_path(), s)))
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
