use lsp_types::TextDocumentContentChangeEvent;
use magic_core::Language;
use magic_core::text::{LineIndex, point_after};
use tree_sitter::{InputEdit, Parser, Tree};

pub struct Document {
    pub lang:  &'static dyn Language,
    pub text:  String,
    pub lines: LineIndex,
    pub tree:  Option<Tree>,
}

impl Document {
    pub fn new(lang: &'static dyn Language, text: String) -> Self {
        let lines = LineIndex::new(&text);
        Self { lang, text, lines, tree: None }
    }

    pub fn apply(&mut self, changes: Vec<TextDocumentContentChangeEvent>) {
        for change in changes {
            match change.range {
                Some(r) => {
                    let start = self.lines.offset(&self.text, r.start.line, r.start.character);
                    let end   = self.lines.offset(&self.text, r.end.line, r.end.character).max(start);
                    let edit  = InputEdit {
                        start_byte:          start,
                        old_end_byte:        end,
                        new_end_byte:        start + change.text.len(),
                        start_position:      self.lines.point(start),
                        old_end_position:    self.lines.point(end),
                        new_end_position:    point_after(self.lines.point(start), &change.text),
                    };
                    self.text.replace_range(start..end, &change.text);
                    if let Some(tree) = &mut self.tree {
                        tree.edit(&edit);
                    }
                }
                None => {
                    self.text = change.text;
                    self.tree = None;
                }
            }
            self.lines = LineIndex::new(&self.text);
        }
    }

    pub fn reparse(&mut self, parser: &mut Parser) {
        if parser.set_language(&self.lang.grammar()).is_ok() {
            self.tree = parser.parse(&self.text, self.tree.as_ref());
        }
    }
}
