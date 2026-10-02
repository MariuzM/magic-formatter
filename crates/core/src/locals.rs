use std::collections::HashMap;

use tree_sitter::Tree;

use crate::language::Analysis;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Def {
    pub name:        String,
    pub start:       usize,
    pub end:         usize,
    pub visible:     usize,
    pub scope_start: usize,
    pub scope_end:   usize,
}

#[derive(Default)]
pub struct Locals {
    defs:    Vec<Def>,
    by_name: HashMap<String, Vec<usize>>,
}

impl Locals {
    pub fn collect(lang: &dyn Analysis, tree: &Tree, src: &str) -> Self {
        let bytes                           = src.as_bytes();
        let scope_kinds                     = lang.scope_kinds();
        let mut locals                      = Self::default();
        let mut scopes: Vec<(usize, usize)> = Vec::new();
        let mut found                       = Vec::new();
        let mut cursor                      = tree.walk();

        'walk: loop {
            let node = cursor.node();
            if scope_kinds.contains(&node.kind()) {
                scopes.push((node.start_byte(), node.end_byte()));
            }
            found.clear();
            lang.collect_definitions(node, bytes, &mut found);
            if let Some(&(scope_start, scope_end)) = scopes.last() {
                for &(def, visible) in &found {
                    let Ok(name) = def.utf8_text(bytes) else { continue };
                    locals.by_name.entry(name.to_string()).or_default().push(locals.defs.len());
                    locals.defs.push(Def {
                        name: name.to_string(),
                        start: def.start_byte(),
                        end: def.end_byte(),
                        visible,
                        scope_start,
                        scope_end,
                    });
                }
            }
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                if scope_kinds.contains(&cursor.node().kind()) {
                    scopes.pop();
                }
                if cursor.goto_next_sibling() {
                    continue 'walk;
                }
                if !cursor.goto_parent() {
                    break 'walk;
                }
            }
        }
        locals
    }

    pub fn resolve(&self, name: &str, offset: usize) -> Option<&Def> {
        self.by_name
            .get(name)?
            .iter()
            .map(|&i| &self.defs[i])
            .filter(|d| d.start == offset || d.scope_start <= offset && offset < d.scope_end && d.visible <= offset)
            .max_by(|a, b| {
                (a.start == offset)
                    .cmp(&(b.start == offset))
                    .then(a.scope_start.cmp(&b.scope_start))
                    .then(b.scope_end.cmp(&a.scope_end))
                    .then(a.start.cmp(&b.start))
            })
    }

    pub fn visible(&self, offset: usize) -> impl Iterator<Item = &Def> {
        self.defs.iter().filter(move |d| d.scope_start <= offset && offset <= d.scope_end && d.visible <= offset)
    }
}
