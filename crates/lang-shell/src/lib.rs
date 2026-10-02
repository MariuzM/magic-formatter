pub mod format;

use std::sync::OnceLock;

use magic_core::highlight::{Highlighter, Precedence};
use magic_core::{Analysis, FormatContext, Formatted, Language};
use tree_sitter::Node;

pub struct Shell {
    highlighter: OnceLock<Highlighter>,
}

pub static SHELL: Shell = Shell {
    highlighter: OnceLock::new(),
};

impl Analysis for Shell {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_bash::LANGUAGE.into()
    }

    fn highlighter(&self) -> &Highlighter {
        self.highlighter
            .get_or_init(|| Highlighter::new(&self.grammar(), include_str!("../queries/highlights.scm"), Precedence::FirstWins, &[]))
    }

    fn identifier_kinds(&self) -> &'static [&'static str] {
        &["variable_name", "word"]
    }

    fn scope_kinds(&self) -> &'static [&'static str] {
        &[]
    }

    fn collect_definitions<'t>(&self, _node: Node<'t>, _src: &[u8], _out: &mut Vec<(Node<'t>, usize)>) {}

    fn resolves_locally(&self, _node: Node) -> bool {
        false
    }

    fn definition_kinds(&self, _node: Node) -> &'static [(&'static str, &'static str)] {
        &[("function_definition", "name"), ("variable_assignment", "name")]
    }

    fn mask(&self, src: &str) -> Option<String> {
        Some(format::mask_zsh(src))
    }
}

impl Language for Shell {
    fn id(&self) -> &'static str {
        "shellscript"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["sh", "bash", "zsh"]
    }

    fn analysis(&self) -> Option<&dyn Analysis> {
        Some(self)
    }

    fn format(&self, src: &str, ctx: &FormatContext) -> Result<Formatted, String> {
        let opts = format::Options {
            indent: ctx.indent.clone(),
        };
        Ok(Formatted {
            text:    format::format(src, &opts)?,
            warning: None,
        })
    }
}
