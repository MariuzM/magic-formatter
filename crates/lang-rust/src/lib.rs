pub mod format;

use std::sync::OnceLock;

use magic_core::highlight::{Highlighter, Precedence};
use magic_core::{FormatContext, Formatted, Language};
use tree_sitter::Node;

pub struct Rust {
    highlighter: OnceLock<Highlighter>,
}

pub static RUST: Rust = Rust {
    highlighter: OnceLock::new(),
};

const NON_LOCAL_PARENTS: &[&str] = &[
    "scoped_identifier",
    "scoped_type_identifier",
    "scoped_use_list",
    "use_as_clause",
    "use_list",
    "macro_invocation",
    "macro_definition",
    "function_item",
    "function_signature_item",
    "struct_item",
    "enum_item",
    "enum_variant",
    "union_item",
    "trait_item",
    "type_item",
    "mod_item",
    "const_item",
    "static_item",
    "attribute",
    "lifetime",
];

fn bindings<'t>(node: Node<'t>, src: &[u8], visible: usize, out: &mut Vec<(Node<'t>, usize)>) {
    match node.kind() {
        "identifier" => {
            if node.utf8_text(src).is_ok_and(|t| !t.starts_with(|c: char| c.is_uppercase())) {
                out.push((node, visible));
            }
        }
        "shorthand_field_identifier" => out.push((node, visible)),
        "scoped_identifier" | "field_identifier" | "type_identifier" | "scoped_type_identifier" => {}
        _ => {
            let mut cursor = node.walk();
            for (i, child) in node.children(&mut cursor).enumerate() {
                if !matches!(node.field_name_for_child(i as u32), Some("type" | "condition")) {
                    bindings(child, src, visible, out);
                }
            }
        }
    }
}

impl Language for Rust {
    fn id(&self) -> &'static str {
        "rust"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }

    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn highlighter(&self) -> &Highlighter {
        self.highlighter.get_or_init(|| Highlighter::new(&self.grammar(), tree_sitter_rust::HIGHLIGHTS_QUERY, Precedence::FirstWins, &[]))
    }

    fn identifier_kinds(&self) -> &'static [&'static str] {
        &["identifier", "field_identifier", "type_identifier", "shorthand_field_identifier"]
    }

    fn scope_kinds(&self) -> &'static [&'static str] {
        &[
            "block",
            "function_item",
            "closure_expression",
            "match_arm",
            "for_expression",
            "if_expression",
            "while_expression",
        ]
    }

    fn collect_definitions<'t>(&self, node: Node<'t>, src: &[u8], out: &mut Vec<(Node<'t>, usize)>) {
        let pattern = node.child_by_field_name("pattern");
        match node.kind() {
            "let_declaration" | "let_condition" => {
                if let Some(p) = pattern {
                    bindings(p, src, node.end_byte(), out);
                }
            }
            "parameter" | "for_expression" | "match_arm" => {
                if let Some(p) = pattern {
                    bindings(p, src, p.start_byte(), out);
                }
            }
            "closure_parameters" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor).filter(|c| c.kind() != "parameter") {
                    bindings(child, src, child.start_byte(), out);
                }
            }
            _ => {}
        }
    }

    fn resolves_locally(&self, node: Node) -> bool {
        matches!(node.kind(), "identifier" | "shorthand_field_identifier")
            && node.parent().is_none_or(|p| !NON_LOCAL_PARENTS.contains(&p.kind()))
    }

    fn format(&self, src: &str, ctx: &FormatContext) -> Result<Formatted, String> {
        let dir  = ctx.dir.map(|d| d.to_path_buf()).or_else(|| std::env::current_dir().ok()).unwrap_or_default();
        let text = format::format(src, &dir, ctx.tools.rustfmt.as_deref())?;
        match format::topcoat(&text, &dir, ctx.tools.topcoat.as_deref()) {
            Some(Ok(t)) => Ok(Formatted { text: t, warning: None }),
            Some(Err(e)) => Ok(Formatted {
                text,
                warning: Some(format!("topcoat fmt: {e}")),
            }),
            None => Ok(Formatted { text, warning: None }),
        }
    }
}
