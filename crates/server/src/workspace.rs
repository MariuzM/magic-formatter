use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use ignore::WalkState;
use lsp_types::Uri;
use magic_core::Language;
use magic_core::definitions::name_ranges;
use magic_core::index::Index;
use magic_core::symbols::{IndexedSymbol, indexed};
use tree_sitter::{Parser, Tree};

use crate::language_for_path;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

const INDEX_BATCH: usize = 64;

const INDEX_THREADS: usize = 4;

thread_local! {
    static PARSER: RefCell<Parser> = RefCell::new(Parser::new());
}

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

pub fn parse(lang: &dyn Language, text: &str) -> Option<Tree> {
    let analysis = lang.analysis()?;
    PARSER.with_borrow_mut(|parser| {
        parser.set_language(&analysis.grammar()).ok()?;
        let masked = analysis.mask(text);
        parser.parse(masked.as_deref().unwrap_or(text), None)
    })
}

pub fn parsed(lang: &dyn Language, path: &Path) -> Option<(Arc<String>, Tree)> {
    let text = read_source(path)?;
    let tree = parse(lang, &text)?;
    Some((Arc::new(text), tree))
}

pub type Scan = (Vec<IndexedSymbol>, Option<Vec<(usize, usize)>>);

pub fn scan(path: &Path, text: &str) -> Scan {
    let Some(analysis) = language_for_path(path).and_then(|l| l.analysis()).filter(|a| a.language_server()) else {
        return (Vec::new(), None);
    };
    PARSER.with_borrow_mut(|parser| {
        if parser.set_language(&analysis.grammar()).is_err() {
            return (Vec::new(), None);
        }
        let Some(tree) = parser.parse(text, None) else {
            return (Vec::new(), None);
        };
        let kinds = analysis.definition_kinds(tree.root_node());
        (indexed(analysis, &tree, text), Some(name_ranges(&tree, kinds)))
    })
}

fn flush(index: &RwLock<Index>, batch: &mut Vec<(PathBuf, String, Scan)>) {
    if batch.is_empty() {
        return;
    }
    let mut index = index.write().unwrap();
    for (path, text, (outline, defined)) in batch.drain(..) {
        index.update(&path, &text, outline, defined);
    }
}

pub fn index_roots(roots: Vec<PathBuf>, languages: Vec<&'static str>, warm: Vec<&'static str>, index: Arc<RwLock<Index>>) {
    if roots.is_empty() || languages.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let started        = Instant::now();
        let (tx, rx)       = crossbeam_channel::bounded(INDEX_BATCH * 4);
        let warmed         = Mutex::new(HashSet::new());
        let writer         = std::thread::spawn({
            let index = index.clone();
            move || {
                let mut count = 0;
                let mut batch = Vec::with_capacity(INDEX_BATCH);
                for item in rx {
                    batch.push(item);
                    count += 1;
                    if batch.len() >= INDEX_BATCH {
                        flush(&index, &mut batch);
                    }
                }
                flush(&index, &mut batch);
                count
            }
        });
        let mut walk = ignore::WalkBuilder::new(&roots[0]);
        for root in &roots[1..] {
            walk.add(root);
        }
        walk.threads(std::thread::available_parallelism().map_or(1, |n| n.get()).min(INDEX_THREADS));
        walk.build_parallel().run(|| {
            let (tx, languages, warm, warmed) = (tx.clone(), &languages, &warm, &warmed);
            Box::new(move |entry| {
                let Ok(entry)  = entry else { return WalkState::Continue };
                let path       = entry.path();
                let lang       = language_for_path(path).filter(|l| languages.contains(&l.id()));
                let Some(lang) = lang.filter(|_| entry.file_type().is_some_and(|t| t.is_file())) else {
                    return WalkState::Continue;
                };
                if warm.contains(&lang.id()) && warmed.lock().unwrap().insert(lang.id()) {
                    std::thread::spawn(move || lang.analysis().map(|a| a.highlighter()).is_some());
                }
                if let Some(text) = read_source(path) {
                    let scan = scan(path, &text);
                    tx.send((path.to_path_buf(), text, scan)).ok();
                }
                WalkState::Continue
            })
        });
        drop(tx);
        let count = writer.join().unwrap_or_default();
        eprintln!("magic-formatter: indexed {count} files in {:.0?}", started.elapsed());
    });
}
