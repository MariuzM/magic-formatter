mod document;
mod server;
mod workspace;

use std::io::{self, Read};
use std::path::Path;
use std::process::exit;

use magic_core::{FormatContext, Language, ToolPaths};

pub static LANGUAGES: [&'static dyn Language; 4] = [&lang_rust::RUST, &lang_swift::SWIFT, &lang_toml::TOML, &lang_shell::SHELL];

pub fn language_by_id(id: &str) -> Option<&'static dyn Language> {
    LANGUAGES.iter().copied().find(|l| l.id() == id)
}

pub fn language_for_path(path: &Path) -> Option<&'static dyn Language> {
    let ext = path.extension()?.to_str()?;
    LANGUAGES.iter().copied().find(|l| l.extensions().contains(&ext))
}

fn format_stdin(id: &str, indent: &str) {
    let Some(lang) = language_by_id(id) else {
        eprintln!("magic-formatter: unknown language `{id}`");
        exit(2);
    };
    let indent = match indent {
        "tab" => "\t".to_string(),
        n => match n.parse::<usize>() {
            Ok(n) => " ".repeat(n),
            Err(_) => {
                eprintln!("magic-formatter: invalid indent `{n}`");
                exit(2);
            }
        },
    };
    let mut src = String::new();
    if io::stdin().read_to_string(&mut src).is_err() {
        exit(1);
    }
    let dir   = std::env::current_dir().ok();
    let tools = ToolPaths::default();
    let ctx   = FormatContext {
        dir: dir.as_deref(),
        indent,
        tools: &tools,
    };
    match lang.format(&src, &ctx) {
        Ok(f) => {
            if let Some(w) = f.warning {
                eprintln!("magic-formatter: {w}");
            }
            print!("{}", f.text);
        }
        Err(e) => {
            eprint!("{e}");
            exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--lsp", ..] => server::run(),
        ["--version"] => println!("{}", env!("CARGO_PKG_VERSION")),
        [] => format_stdin("rust", "4"),
        ["--lang", id] => format_stdin(id, "4"),
        ["--lang", id, "--indent", indent] => format_stdin(id, indent),
        _ => {
            eprintln!("usage: magic-formatter [--lsp | --lang <rust|swift|toml|shellscript> [--indent <n|tab>] | --version]");
            exit(2);
        }
    }
}
