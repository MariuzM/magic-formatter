mod features;
pub mod format;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use magic_core::completion::word_start;
use magic_core::describe::{Call, Description};
use magic_core::highlight::{Highlighter, Precedence, Span};
use magic_core::symbols::{SymbolInfo, SymbolKind};
use magic_core::{Analysis, FormatContext, Formatted, Language};
use tree_sitter::{Node, Tree};

pub struct Kotlin {
    highlighter: OnceLock<Highlighter>,
}

pub static KOTLIN: Kotlin = Kotlin {
    highlighter: OnceLock::new(),
};

const NON_LOCAL_PARENTS: &[&str] = &[
    "user_type",
    "class_declaration",
    "object_declaration",
    "companion_object",
    "function_declaration",
    "type_alias",
    "qualified_identifier",
    "enum_entry",
    "class_parameter",
    "type_parameter",
    "type_constraint",
    "callable_reference",
    "this_expression",
    "super_expression",
    "return_expression",
    "labeled_expression",
];

fn identifier(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|c| c.kind() == "identifier")
}

fn declaration_visibility(decl: Node) -> Option<usize> {
    let parent = decl.parent()?;
    match parent.kind() {
        "property_declaration" => Some(parent.end_byte()),
        "multi_variable_declaration" => match parent.parent()? {
            p if p.kind() == "property_declaration" => Some(p.end_byte()),
            _ => Some(decl.start_byte()),
        },
        "for_statement" | "lambda_parameters" | "when_subject" => Some(decl.start_byte()),
        _ => None,
    }
}

impl Analysis for Kotlin {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_kotlin_ng::LANGUAGE.into()
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
            "function_declaration",
            "secondary_constructor",
            "anonymous_function",
            "anonymous_initializer",
            "getter",
            "setter",
            "lambda_literal",
            "block",
            "for_statement",
            "when_expression",
            "catch_block",
        ]
    }

    fn collect_definitions<'t>(&self, node: Node<'t>, _src: &[u8], out: &mut Vec<(Node<'t>, usize)>) {
        match node.kind() {
            "parameter" if node.parent().is_some_and(|p| p.kind() == "function_value_parameters") => {
                out.extend(identifier(node).map(|n| (n, n.start_byte())));
            }
            "variable_declaration" => {
                if let (Some(name), Some(visible)) = (identifier(node), declaration_visibility(node)) {
                    out.push((name, visible));
                }
            }
            "catch_block" => out.extend(identifier(node).map(|n| (n, n.start_byte()))),
            _ => {}
        }
    }

    fn resolves_locally(&self, node: Node) -> bool {
        if node.kind() != "identifier" {
            return false;
        }
        let Some(parent) = node.parent() else { return true };
        match parent.kind() {
            k if NON_LOCAL_PARENTS.contains(&k) => false,
            "navigation_expression" => node.prev_sibling().is_none(),
            "infix_expression" => parent.named_child(1) != Some(node),
            "value_argument" => node.next_sibling().is_none_or(|n| n.kind() != "="),
            _ => true,
        }
    }

    fn definition_kinds(&self, _node: Node) -> &'static [(&'static str, &'static str)] {
        &[
            ("class_declaration", "name"),
            ("object_declaration", "name"),
            ("companion_object", "name"),
            ("function_declaration", "name"),
            ("type_alias", "type"),
            ("variable_declaration", ""),
            ("class_parameter", ""),
            ("enum_entry", ""),
        ]
    }

    fn is_definition(&self, name: Node) -> bool {
        features::is_definition(name)
    }

    fn implementations(&self, tree: &Tree, src: &str, name: &str) -> Option<Vec<(usize, usize)>> {
        Some(features::implementations(tree, src, name))
    }

    fn import_fallback(&self, tree: &Tree, src: &str, name: &str) -> Option<(usize, usize)> {
        features::import_of(tree, src, name)
    }

    fn language_server(&self) -> bool {
        true
    }

    fn fence(&self) -> &'static str {
        "kotlin"
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

    fn comment_kinds(&self) -> &'static [&'static str] {
        &["line_comment", "block_comment"]
    }

    fn import_kinds(&self) -> &'static [&'static str] {
        &["import"]
    }

    fn builtins(&self) -> &'static [&'static str] {
        features::BUILTINS
    }

    fn syntax_diagnostics(&self) -> bool {
        true
    }

    fn refine(&self, tree: &Tree, src: &str, spans: &mut [Span]) {
        features::refine(tree, src, spans);
    }

    fn member_completions(
        &self,
        tree: &Tree,
        src: &str,
        offset: usize,
        _file: &Path,
        _roots: &[PathBuf],
    ) -> Option<Vec<(String, SymbolKind)>> {
        let dot      = word_start(src, offset).checked_sub(1).filter(|&d| src.as_bytes()[d] == b'.')?;
        let receiver = src[..dot].strip_suffix("this")?;
        if receiver.ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            return None;
        }
        let node = tree.root_node().descendant_for_byte_range(dot, dot)?;
        Some(features::self_members(node, src))
    }
}

impl Language for Kotlin {
    fn id(&self) -> &'static str {
        "kotlin"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["kt", "kts"]
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
