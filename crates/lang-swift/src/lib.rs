pub mod format;

use std::sync::OnceLock;

use magic_core::highlight::{Highlighter, Precedence};
use magic_core::{FormatContext, Formatted, Language};
use tree_sitter::Node;

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

impl Language for Swift {
    fn id(&self) -> &'static str {
        "swift"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["swift"]
    }

    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_swift::LANGUAGE.into()
    }

    fn highlighter(&self) -> &Highlighter {
        self.highlighter.get_or_init(|| {
            Highlighter::new(
                &self.grammar(),
                tree_sitter_swift::HIGHLIGHTS_QUERY,
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
