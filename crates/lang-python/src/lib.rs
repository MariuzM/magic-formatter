mod features;
pub mod format;
mod imports;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use magic_core::completion::grammar_keywords;
use magic_core::describe::{Call, Description};
use magic_core::highlight::{Highlighter, Precedence, Span};
use magic_core::locals::Locals;
use magic_core::symbols::{SymbolInfo, SymbolKind};
use magic_core::{Analysis, FormatContext, Formatted, Language};
use tree_sitter::{Node, Tree};

pub struct Python {
    highlighter: OnceLock<Highlighter>,
}

pub static PYTHON: Python = Python {
    highlighter: OnceLock::new(),
};

const NESTED_SCOPES: &[&str] = &[
    "lambda",
    "list_comprehension",
    "set_comprehension",
    "dictionary_comprehension",
    "generator_expression",
];

fn is_capture(dotted: Node) -> bool {
    let Some(parent) = dotted.parent() else { return false };
    dotted.named_child_count() == 1
        && match parent.kind() {
            "case_pattern" => true,
            "keyword_pattern" => parent.named_child(0) != Some(dotted),
            _ => false,
        }
}

fn targets<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    match node.kind() {
        "identifier" => out.push(node),
        "pattern_list" | "tuple_pattern" | "list_pattern" | "list_splat_pattern" | "dictionary_splat_pattern" | "splat_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                targets(child, out);
            }
        }
        "typed_parameter" => {
            if let Some(first) = node.named_child(0) {
                targets(first, out);
            }
        }
        "default_parameter" | "typed_default_parameter" => {
            if let Some(name) = node.child_by_field_name("name") {
                targets(name, out);
            }
        }
        _ => {}
    }
}

fn parameters<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        targets(child, out);
    }
}

fn bindings<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>, excluded: &mut Vec<Node<'t>>) {
    let field = |f: &str| node.child_by_field_name(f);
    match node.kind() {
        "function_definition" | "class_definition" => {
            out.extend(field("name"));
            return;
        }
        k if NESTED_SCOPES.contains(&k) => return,
        "assignment" | "augmented_assignment" | "for_statement" => {
            if let Some(left) = field("left") {
                targets(left, out);
            }
        }
        "as_pattern_target" | "splat_pattern" => targets(node.named_child(0).unwrap_or(node), out),
        "as_pattern" if node.parent().is_some_and(|p| p.kind() == "case_pattern") => {
            let mut cursor = node.walk();
            out.extend(node.named_children(&mut cursor).filter(|c| c.kind() == "identifier"));
        }
        "named_expression" => out.extend(field("name")),
        "dotted_name" if is_capture(node) => out.extend(node.named_child(0)),
        "aliased_import" => out.extend(field("alias")),
        "import_statement" | "import_from_statement" => {
            let mut cursor = node.walk();
            for name in node.children_by_field_name("name", &mut cursor).filter(|n| n.kind() == "dotted_name") {
                out.extend(name.named_child(0));
            }
        }
        "global_statement" | "nonlocal_statement" => {
            let mut cursor = node.walk();
            excluded.extend(node.named_children(&mut cursor).filter(|c| c.kind() == "identifier"));
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        bindings(child, out, excluded);
    }
}

impl Analysis for Python {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }

    fn highlighter(&self) -> &Highlighter {
        self.highlighter
            .get_or_init(|| Highlighter::new(&self.grammar(), include_str!("../queries/highlights.scm"), Precedence::LastWins, &[]))
    }

    fn identifier_kinds(&self) -> &'static [&'static str] {
        &["identifier"]
    }

    fn scope_kinds(&self) -> &'static [&'static str] {
        &[
            "function_definition",
            "lambda",
            "list_comprehension",
            "set_comprehension",
            "dictionary_comprehension",
            "generator_expression",
        ]
    }

    fn collect_definitions<'t>(&self, node: Node<'t>, src: &[u8], out: &mut Vec<(Node<'t>, usize)>) {
        let mut found    = Vec::new();
        let mut excluded = Vec::new();
        match node.kind() {
            "function_definition" => {
                if let Some(params) = node.child_by_field_name("parameters") {
                    parameters(params, &mut found);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    bindings(body, &mut found, &mut excluded);
                }
            }
            "lambda" => {
                if let Some(params) = node.child_by_field_name("parameters") {
                    parameters(params, &mut found);
                }
            }
            k if NESTED_SCOPES.contains(&k) => {
                let mut cursor = node.walk();
                for clause in node.named_children(&mut cursor).filter(|c| c.kind() == "for_in_clause") {
                    if let Some(left) = clause.child_by_field_name("left") {
                        targets(left, &mut found);
                    }
                }
            }
            _ => return,
        }
        let excluded: Vec<&str> = excluded.iter().filter_map(|n| n.utf8_text(src).ok()).collect();
        let mut seen: Vec<&str> = Vec::new();
        for def in found {
            let Ok(name) = def.utf8_text(src) else { continue };
            if !excluded.contains(&name) && !seen.contains(&name) {
                seen.push(name);
                out.push((def, node.start_byte()));
            }
        }
    }

    fn resolves_locally(&self, node: Node, parent: Option<Node>) -> bool {
        if node.kind() != "identifier" {
            return false;
        }
        let Some(parent) = parent else { return true };
        match parent.kind() {
            "attribute" => parent.child_by_field_name("attribute") != Some(node),
            "keyword_argument" => parent.child_by_field_name("name") != Some(node),
            "dotted_name" => {
                let first = parent.named_child(0) == Some(node);
                first
                    && (is_capture(parent)
                        || parent.parent().is_some_and(|g| match g.kind() {
                            "import_statement" => true,
                            "import_from_statement" => g.child_by_field_name("module_name") != Some(parent),
                            _ => false,
                        }))
            }
            _ => true,
        }
    }

    fn definition_kinds(&self, _node: Node) -> &'static [(&'static str, &'static str)] {
        &[
            ("function_definition", "name"),
            ("class_definition", "name"),
            ("assignment", "left"),
        ]
    }

    fn import_definitions(&self, tree: &Tree, src: &str, offset: usize, file: &Path, roots: &[PathBuf]) -> Vec<(PathBuf, usize, usize)> {
        imports::definitions(tree, src, offset, file, roots)
    }

    fn language_server(&self) -> bool {
        true
    }

    fn fence(&self) -> &'static str {
        "python"
    }

    fn symbol(&self, node: Node, src: &str) -> Option<SymbolInfo> {
        features::symbol(node, src)
    }

    fn describe(&self, node: Node, src: &str) -> Option<Description> {
        features::describe(node, src)
    }

    fn call_at<'t>(&self, node: Node<'t>, offset: usize) -> Option<Call<'t>> {
        features::call_at(node, offset)
    }

    fn fold(&self, node: Node) -> Option<(usize, usize)> {
        features::fold(node)
    }

    fn import_kinds(&self) -> &'static [&'static str] {
        &["import_statement", "import_from_statement", "future_import_statement"]
    }

    fn keywords(&self) -> Vec<String> {
        grammar_keywords(&self.grammar()).into_iter().filter(|k| !matches!(k.as_str(), "print" | "exec")).collect()
    }

    fn builtins(&self) -> &'static [&'static str] {
        features::BUILTINS
    }

    fn syntax_diagnostics(&self) -> bool {
        true
    }

    fn refine(&self, tree: &Tree, src: &str, spans: &mut [Span], _locals: &Locals) {
        features::refine(tree, src, spans);
    }

    fn member_completions(
        &self,
        tree: &Tree,
        src: &str,
        offset: usize,
        file: &Path,
        roots: &[PathBuf],
    ) -> Option<Vec<(String, SymbolKind)>> {
        imports::members(tree, src, offset, file, roots)
    }
}

impl Language for Python {
    fn id(&self) -> &'static str {
        "python"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["py", "pyi", "pyw"]
    }

    fn analysis(&self) -> Option<&dyn Analysis> {
        Some(self)
    }

    fn format(&self, src: &str, ctx: &FormatContext) -> Result<Formatted, String> {
        let opts = format::Options {
            indent:    ctx.indent.clone(),
            max_width: 100,
        };
        Ok(Formatted {
            text:    format::format(src, &opts)?,
            warning: None,
        })
    }
}
