pub mod format;

use std::sync::OnceLock;

use magic_core::highlight::{Highlighter, Precedence};
use magic_core::{Analysis, FormatContext, Formatted, Language};
use tree_sitter::Node;

pub struct Toml {
    highlighter: OnceLock<Highlighter>,
}

pub static TOML: Toml = Toml {
    highlighter: OnceLock::new(),
};

impl Analysis for Toml {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_toml_ng::LANGUAGE.into()
    }

    fn highlighter(&self) -> &Highlighter {
        self.highlighter
            .get_or_init(|| Highlighter::new(&self.grammar(), include_str!("../queries/highlights.scm"), Precedence::FirstWins, &[]))
    }

    fn identifier_kinds(&self) -> &'static [&'static str] {
        &["bare_key"]
    }

    fn scope_kinds(&self) -> &'static [&'static str] {
        &[]
    }

    fn collect_definitions<'t>(&self, _node: Node<'t>, _src: &[u8], _out: &mut Vec<(Node<'t>, usize)>) {}

    fn resolves_locally(&self, _node: Node) -> bool {
        false
    }
}

impl Language for Toml {
    fn id(&self) -> &'static str {
        "toml"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["toml"]
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
