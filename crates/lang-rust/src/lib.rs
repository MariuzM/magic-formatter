pub mod format;

use magic_core::{FormatContext, Formatted, Language};

pub struct Rust;

pub static RUST: Rust = Rust;

impl Language for Rust {
    fn id(&self) -> &'static str {
        "rust"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
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
