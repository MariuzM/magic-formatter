use std::sync::{Arc, Mutex};

use lsp_types::TextDocumentContentChangeEvent;
use magic_core::Language;
use magic_core::text::{LineIndex, point_after};
use tree_sitter::{InputEdit, Parser, Tree};

#[derive(Default)]
pub struct Tokens {
    pub revision: Option<u64>,
    pub data:     Arc<Vec<u32>>,
    pub sent:     Option<(String, Arc<Vec<u32>>)>,
}

pub struct Document {
    pub lang:     &'static dyn Language,
    pub text:     Arc<String>,
    pub lines:    Arc<LineIndex>,
    pub tree:     Option<Tree>,
    pub revision: u64,
    pub tokens:   Arc<Mutex<Tokens>>,
}

impl Document {
    pub fn new(lang: &'static dyn Language, text: String) -> Self {
        let lines = LineIndex::new(&text);
        Self {
            lang,
            text: Arc::new(text),
            lines: Arc::new(lines),
            tree: None,
            revision: 0,
            tokens: Arc::default(),
        }
    }

    pub fn apply(&mut self, changes: Vec<TextDocumentContentChangeEvent>) {
        self.revision += 1;
        for change in changes {
            match change.range {
                Some(r) => {
                    let start = self.lines.offset(&self.text, r.start.line, r.start.character);
                    let end   = self.lines.offset(&self.text, r.end.line, r.end.character).max(start);
                    let edit  = InputEdit {
                        start_byte:       start,
                        old_end_byte:     end,
                        new_end_byte:     start + change.text.len(),
                        start_position:   self.lines.point(start),
                        old_end_position: self.lines.point(end),
                        new_end_position: point_after(self.lines.point(start), &change.text),
                    };
                    Arc::make_mut(&mut self.text).replace_range(start..end, &change.text);
                    Arc::make_mut(&mut self.lines).edit(start, end, &change.text);
                    if let Some(tree) = &mut self.tree {
                        tree.edit(&edit);
                    }
                }
                None => {
                    self.lines = Arc::new(LineIndex::new(&change.text));
                    self.text  = Arc::new(change.text);
                    self.tree  = None;
                }
            }
        }
    }

    pub fn reparse(&mut self, parser: &mut Parser) {
        if let Some(analysis) = self.lang.analysis()
            && parser.set_language(&analysis.grammar()).is_ok()
        {
            let masked = analysis.mask(&self.text);
            self.tree  = parser.parse(masked.as_deref().unwrap_or(&self.text), self.tree.as_ref());
        }
    }
}
