pub mod format;

use std::sync::OnceLock;

use magic_core::highlight::{Highlighter, Precedence};
use magic_core::{Analysis, FormatContext, Formatted, Language};
use tree_sitter::Node;

pub struct Docker {
    highlighter: OnceLock<Highlighter>,
}

pub static DOCKER: Docker = Docker {
    highlighter: OnceLock::new(),
};

impl Analysis for Docker {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_containerfile::LANGUAGE.into()
    }

    fn highlighter(&self) -> &Highlighter {
        self.highlighter
            .get_or_init(|| Highlighter::new(&self.grammar(), include_str!("../queries/highlights.scm"), Precedence::LastWins, &[]))
    }

    fn identifier_kinds(&self) -> &'static [&'static str] {
        &["variable", "unquoted_string", "image_name", "image_alias"]
    }

    fn scope_kinds(&self) -> &'static [&'static str] {
        &[]
    }

    fn collect_definitions<'t>(&self, _node: Node<'t>, _src: &[u8], _out: &mut Vec<(Node<'t>, usize)>) {}

    fn resolves_locally(&self, _node: Node) -> bool {
        false
    }

    fn definition_kinds(&self, _node: Node) -> &'static [(&'static str, &'static str)] {
        &[("arg_pair", "name"), ("env_pair", "name"), ("from_instruction", "as")]
    }
}

impl Language for Docker {
    fn id(&self) -> &'static str {
        "dockerfile"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["dockerfile", "Dockerfile", "containerfile", "Containerfile"]
    }

    fn filenames(&self) -> &'static [&'static str] {
        &["Dockerfile", "Containerfile"]
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
