use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Index {
    paths:     Vec<PathBuf>,
    ids:       HashMap<PathBuf, u32>,
    file_syms: Vec<Vec<u32>>,
    symbols:   HashMap<Box<str>, u32>,
    postings:  Vec<Vec<u32>>,
}

impl Index {
    pub fn update(&mut self, path: &Path, text: &str) {
        let id = self.remove(path).unwrap_or_else(|| {
            let id = self.paths.len() as u32;
            self.paths.push(path.to_path_buf());
            self.ids.insert(path.to_path_buf(), id);
            self.file_syms.push(Vec::new());
            id
        });
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
        for sym in std::mem::take(&mut self.file_syms[id as usize]) {
            self.postings[sym as usize].retain(|&f| f != id);
        }
        Some(id)
    }

    pub fn files_with(&self, name: &str) -> Vec<PathBuf> {
        self.symbols
            .get(identifiers(name).next().unwrap_or(name))
            .map(|&s| self.postings[s as usize].iter().map(|&f| self.paths[f as usize].clone()).collect())
            .unwrap_or_default()
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
