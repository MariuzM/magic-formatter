# Magic Formatter

A formatter for Rust and Swift with user-controlled layout. You decide the shape of your code with small gestures in the source; the formatter respects them on every save. It also provides semantic highlighting and Find All References for both languages, from a single fast native language server.

## The rules

**Magic trailing comma** — a single-line call with a trailing comma expands to one argument per line:

```rust
let x = create(a, b, c,);
```

becomes

```rust
let x = create(
    a,
    b,
    c,
);
```

**First argument position** — for a multiline call, the first argument's position is the switch. First argument on its own line: the call stays one-argument-per-line. Move the first argument up next to the `(`: the whole call collapses back to a single line (if it fits the configured width).

**Single-line if** — an `if` written on one line stays on one line:

```rust
if RegisterClassExW(&window_class) == 0 { return 1; };
```

Write it multiline and it stays multiline. Works for `if ... else ...` chains too.

**Method chains** — put the first chained suffix on a new line and every suffix stays on its own line:

```rust
let port = std::env::var("PORT")
    .ok()
    .and_then(|p| p.parse::<u16>().ok())
    .unwrap_or(4002);
```

**`=` alignment** — consecutive single-line assignments and declarations at the same indent get their `=` aligned:

```rust
self.game_path = None;
self.state     = InstallState::default();
let adapter    = self.selected_adapter();
```

Only complete statements participate (the line must end its statement with `;`), so `if let ...` headers and multiline expression openers are never dragged into a group.

Everything else is plain rustfmt, including all options from your `rustfmt.toml`.

## Swift

Swift is formatted by a built-in formatter, so no Swift toolchain or `swift-format` is needed. The same rules apply: magic trailing comma, first argument position, method chains, single-line `if`, and `=` alignment. On top of that it re-indents code (including `switch`/`case`, `#if` blocks, and SwiftUI modifier chains) and normalizes spacing, while keeping your line breaks. Lines the parser does not understand are left exactly as written.

## Highlighting and references

Semantic highlighting and Find All References work for both languages. References resolve local variables by scope and everything else by name across the workspace. Both features default to `auto`, which turns them off for a language when its full language server is installed (rust-analyzer for Rust, the Swift extension for Swift).

## Requirements

- Rust only: `rustfmt` installed via rustup, with the **nightly** toolchain available (`rustup toolchain install nightly`) — needed if your `rustfmt.toml` uses nightly-only options.
- rustfmt options are read from the normal `rustfmt.toml` locations (project directory, or `~/Library/Application Support/rustfmt/rustfmt.toml` on macOS).

## Setup

Set the extension as your formatter in `settings.json`:

```json
"[rust]": {
  "editor.defaultFormatter": "mariuzm.magic-formatter",
  "editor.formatOnSave": true
},
"[swift]": {
  "editor.defaultFormatter": "mariuzm.magic-formatter",
  "editor.formatOnSave": true
}
```

Alternatively, skip the extension and point rust-analyzer straight at the binary:

```json
"rust-analyzer.rustfmt.overrideCommand": ["/path/to/magic-formatter"]
```

## Settings

- `magicFormatter.binaryPath` — absolute path to a locally built `magic-formatter` binary (defaults to the bundled platform binary).
- `magicFormatter.rustfmtPath` — absolute path to the `rustfmt` to delegate to (defaults to `~/.cargo/bin/rustfmt`, then `rustfmt` on PATH).
- `magicFormatter.rust.semanticHighlighting`, `magicFormatter.rust.references`, `magicFormatter.swift.semanticHighlighting`, `magicFormatter.swift.references` — `auto` (default), `on`, or `off`.

## Why

rustfmt applies `--config` values passed on the command line nondeterministically (observed on stable 1.9.0 and nightly 1.10.0), and offers no way to keep a call expanded or an `if` on one line. This tool works around the first and adds the second, while letting rustfmt do everything it is good at.
