use std::path::{Path, PathBuf};

use tree_sitter::{Node, Tree};

use crate::completion::grammar_keywords;
use crate::describe::{Call, Description};
use crate::highlight::{Highlighter, Span};
use crate::symbols::{SymbolInfo, SymbolKind};

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

    fn filenames(&self) -> &'static [&'static str] {
        &[]
    }

    fn analysis(&self) -> Option<&dyn Analysis> {
        None
    }

    fn format(&self, src: &str, ctx: &FormatContext) -> Result<Formatted, String>;
}

pub trait Analysis: Send + Sync {
    fn grammar(&self) -> tree_sitter::Language;

    fn highlighter(&self) -> &Highlighter;

    fn identifier_kinds(&self) -> &'static [&'static str];

    fn scope_kinds(&self) -> &'static [&'static str];

    fn collect_definitions<'t>(&self, node: Node<'t>, src: &[u8], out: &mut Vec<(Node<'t>, usize)>);

    fn resolves_locally(&self, node: Node) -> bool;

    fn definition_kinds(&self, _node: Node) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    fn implementation_kinds(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    fn import_definitions(
        &self,
        _tree: &Tree,
        _src: &str,
        _offset: usize,
        _file: &Path,
        _roots: &[PathBuf],
    ) -> Vec<(PathBuf, usize, usize)> {
        Vec::new()
    }

    fn is_definition(&self, _name: Node) -> bool {
        true
    }

    fn implementations(&self, _tree: &Tree, _src: &str, _name: &str) -> Option<Vec<(usize, usize)>> {
        None
    }

    fn import_fallback(&self, _tree: &Tree, _src: &str, _name: &str) -> Option<(usize, usize)> {
        None
    }

    fn language_server(&self) -> bool {
        false
    }

    fn fence(&self) -> &'static str {
        ""
    }

    fn symbol(&self, _node: Node, _src: &str) -> Option<SymbolInfo> {
        None
    }

    fn describe(&self, _node: Node, _src: &str) -> Option<Description> {
        None
    }

    fn call_at<'t>(&self, _node: Node<'t>, _offset: usize) -> Option<Call<'t>> {
        None
    }

    fn fold(&self, _node: Node) -> Option<(usize, usize)> {
        None
    }

    fn comment_kinds(&self) -> &'static [&'static str] {
        &["comment"]
    }

    fn import_kinds(&self) -> &'static [&'static str] {
        &[]
    }

    fn keywords(&self) -> Vec<String> {
        grammar_keywords(&self.grammar())
    }

    fn builtins(&self) -> &'static [&'static str] {
        &[]
    }

    fn syntax_diagnostics(&self) -> bool {
        false
    }

    fn refine(&self, _tree: &Tree, _src: &str, _spans: &mut [Span]) {}

    fn mask(&self, _src: &str) -> Option<String> {
        None
    }

    fn member_completions(
        &self,
        _tree: &Tree,
        _src: &str,
        _offset: usize,
        _file: &Path,
        _roots: &[PathBuf],
    ) -> Option<Vec<(String, SymbolKind)>> {
        None
    }
}
