use std::path::{Path, PathBuf};

use tree_sitter::Node;

use crate::highlight::Highlighter;

#[derive(Clone, Debug, Default)]
pub struct ToolPaths {
    pub rustfmt: Option<PathBuf>,
    pub topcoat: Option<PathBuf>,
}

pub struct FormatContext<'a> {
    pub dir:    Option<&'a Path>,
    pub indent: String,
    pub tools:  &'a ToolPaths,
}

pub struct Formatted {
    pub text:    String,
    pub warning: Option<String>,
}

pub trait Language: Send + Sync {
    fn id(&self) -> &'static str;

    fn extensions(&self) -> &'static [&'static str];

    fn grammar(&self) -> tree_sitter::Language;

    fn highlighter(&self) -> &Highlighter;

    fn identifier_kinds(&self) -> &'static [&'static str];

    fn scope_kinds(&self) -> &'static [&'static str];

    fn collect_definitions<'t>(&self, node: Node<'t>, src: &[u8], out: &mut Vec<(Node<'t>, usize)>);

    fn resolves_locally(&self, node: Node) -> bool;

    fn format(&self, src: &str, ctx: &FormatContext) -> Result<Formatted, String>;
}
