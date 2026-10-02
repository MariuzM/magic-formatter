use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use lsp_types::Uri;
use magic_core::index::Index;
use magic_core::symbols::{IndexedSymbol, indexed};
use tree_sitter::Parser;

use crate::language_for_path;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    url::Url::parse(uri.as_str()).ok()?.to_file_path().ok()
}

pub fn path_to_uri(path: &Path) -> Option<Uri> {
    Uri::from_str(url::Url::from_file_path(path).ok()?.as_str()).ok()
}

pub fn read_source(path: &Path) -> Option<String> {
    if fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    Some(String::from_utf8_lossy(&fs::read(path).ok()?).into_owned())
}

pub fn outline(path: &Path, text: &str) -> Vec<IndexedSymbol> {
    let Some(analysis) = language_for_path(path).and_then(|l| l.analysis()).filter(|a| a.language_server()) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&analysis.grammar()).is_err() {
        return Vec::new();
    }
    parser.parse(text, None).map(|tree| indexed(analysis, &tree, text)).unwrap_or_default()
}

pub fn index_roots(roots: Vec<PathBuf>, extensions: Vec<&'static str>, index: Arc<RwLock<Index>>) {
    if roots.is_empty() || extensions.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let started   = Instant::now();
        let mut count = 0;
        for root in &roots {
            for entry in ignore::WalkBuilder::new(root).build().flatten() {
                let path   = entry.path();
                let wanted = path.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.contains(&e));
                if !wanted || !entry.file_type().is_some_and(|t| t.is_file()) {
                    continue;
                }
                if let Some(text) = read_source(path) {
                    let outline = outline(path, &text);
                    index.write().unwrap().update(path, &text, outline);
                    count += 1;
                }
            }
        }
        eprintln!("magic-formatter: indexed {count} files in {:.0?}", started.elapsed());
    });
}
