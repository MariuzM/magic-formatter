pub mod completion;
pub mod definitions;
pub mod describe;
pub mod diagnostics;
pub mod folding;
pub mod highlight;
pub mod index;
pub mod language;
pub mod locals;
pub mod references;
pub mod symbols;
pub mod text;

pub use language::{Analysis, FormatContext, Formatted, Language, ToolPaths};
