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

pub struct Swift {
    highlighter: OnceLock<Highlighter>,
}

pub static SWIFT: Swift = Swift {
    highlighter: OnceLock::new(),
};

const NON_LOCAL_PARENTS: &[&str] = &[
    "navigation_suffix",
    "value_argument_label",
    "function_declaration",
    "protocol_function_declaration",
    "class_declaration",
    "protocol_declaration",
    "enum_entry",
    "identifier",
    "typealias_declaration",
    "associatedtype_declaration",
    "statement_label",
];

fn pattern_bindings<'t>(node: Node<'t>, visible: Option<usize>, bare: bool, out: &mut Vec<(Node<'t>, usize)>) {
    let mut cursor = node.walk();
    for (i, child) in node.children(&mut cursor).enumerate() {
        let after_dot = child.prev_sibling().is_some_and(|p| p.kind() == ".");
        match (node.field_name_for_child(i as u32), child.kind()) {
            (Some("bound_identifier"), _) => out.push((child, visible.unwrap_or(child.start_byte()))),
            (_, "simple_identifier") if bare && !after_dot => out.push((child, visible.unwrap_or(child.start_byte()))),
            (_, "pattern") => pattern_bindings(child, visible, bare, out),
            _ => {}
        }
    }
}

fn condition_bindings<'t>(node: Node<'t>, out: &mut Vec<(Node<'t>, usize)>) {
    let mut cursor       = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    for (i, child) in children.iter().enumerate() {
        if node.field_name_for_child(i as u32) != Some("bound_identifier") {
            continue;
        }
        let visible = match children.get(i + 1) {
            Some(eq) if eq.kind() == "=" => children.get(i + 2).map_or(eq.end_byte(), |v| v.end_byte()),
            _ => child.end_byte(),
        };
        out.push((*child, visible));
    }
}

impl Analysis for Swift {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_swift::LANGUAGE.into()
    }

    fn highlighter(&self) -> &Highlighter {
        self.highlighter.get_or_init(|| {
            Highlighter::new(
                &self.grammar(),
                &format!("{}\n{}", tree_sitter_swift::HIGHLIGHTS_QUERY, include_str!("../queries/highlights.scm")),
                Precedence::LastWins,
                &[("constructor", "keyword")],
            )
        })
    }

    fn identifier_kinds(&self) -> &'static [&'static str] {
        &["simple_identifier", "type_identifier"]
    }

    fn scope_kinds(&self) -> &'static [&'static str] {
        &[
            "function_declaration",
            "init_declaration",
            "subscript_declaration",
            "lambda_literal",
            "statements",
            "for_statement",
            "if_statement",
            "while_statement",
            "repeat_while_statement",
            "switch_entry",
            "catch_block",
        ]
    }

    fn collect_definitions<'t>(&self, node: Node<'t>, _src: &[u8], out: &mut Vec<(Node<'t>, usize)>) {
        match node.kind() {
            "parameter" | "lambda_parameter" => {
                let mut cursor = node.walk();
                if let Some(name) = node.children_by_field_name("name", &mut cursor).find(|c| c.kind() == "simple_identifier") {
                    out.push((name, name.start_byte()));
                }
            }
            "if_statement" | "guard_statement" | "while_statement" => condition_bindings(node, out),
            "property_declaration" => {
                let mut cursor = node.walk();
                for pattern in node.children_by_field_name("name", &mut cursor).filter(|c| c.kind() == "pattern") {
                    pattern_bindings(pattern, Some(node.end_byte()), true, out);
                }
            }
            "pattern" => match node.parent().map(|p| p.kind()) {
                Some("pattern" | "property_declaration") => {}
                Some("for_statement") => pattern_bindings(node, None, true, out),
                _ => pattern_bindings(node, None, false, out),
            },
            _ => {}
        }
    }

    fn resolves_locally(&self, node: Node) -> bool {
        if node.kind() != "simple_identifier" {
            return false;
        }
        let Some(parent) = node.parent() else { return true };
        if NON_LOCAL_PARENTS.contains(&parent.kind()) {
            return false;
        }
        parent.child_by_field_name("external_name") != Some(node)
    }

    fn definition_kinds(&self, _node: Node) -> &'static [(&'static str, &'static str)] {
        &[
            ("function_declaration", "name"),
            ("protocol_function_declaration", "name"),
            ("class_declaration", "name"),
            ("protocol_declaration", "name"),
            ("typealias_declaration", "name"),
            ("associatedtype_declaration", "name"),
            ("property_declaration", "name"),
            ("protocol_property_declaration", "name"),
            ("enum_entry", "name"),
        ]
    }

    fn implementation_kinds(&self) -> &'static [(&'static str, &'static str)] {
        &[("class_declaration", "name"), ("function_declaration", "name")]
    }

    fn language_server(&self) -> bool {
        true
    }

    fn fence(&self) -> &'static str {
        "swift"
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
        &["comment", "multiline_comment"]
    }

    fn import_kinds(&self) -> &'static [&'static str] {
        &["import_declaration"]
    }

    fn builtins(&self) -> &'static [&'static str] {
        features::BUILTINS
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
        let receiver = src[..dot].strip_suffix("self")?;
        if receiver.ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            return None;
        }
        let node = tree.root_node().descendant_for_byte_range(dot, dot)?;
        Some(features::self_members(node, src))
    }
}

impl Language for Swift {
    fn id(&self) -> &'static str {
        "swift"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["swift"]
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
