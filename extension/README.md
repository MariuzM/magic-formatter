# Rustfmt Magic

A Rust formatter that runs rustfmt and adds user-controlled layout on top. You decide the shape of your code with small
gestures in the source; the formatter respects them on every save.

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

**First argument position** — for a multiline call, the first argument's position is the switch. First argument on its
own line: the call stays one-argument-per-line. Move the first argument up next to the `(`: the whole call collapses
back to a single line (if it fits the configured width).

**Single-line if** — an `if` written on one line stays on one line:

```rust
if RegisterClassExW(&window_class) == 0 { return 1; };
```

Write it multiline and it stays multiline. Works for `if ... else ...` chains too.

**`=` alignment** — consecutive single-line assignments and declarations at the same indent get their `=` aligned:

```rust
self.game_path = None;
self.state     = InstallState::default();
let adapter    = self.selected_adapter();
```

Only complete statements participate (the line must end its statement with `;`), so `if let ...` headers and multiline
expression openers are never dragged into a group.

Everything else is plain rustfmt, including all options from your `rustfmt.toml`.

## Requirements

- `rustfmt` installed via rustup, with the **nightly** toolchain available (`rustup toolchain install nightly`) — needed
  if your `rustfmt.toml` uses nightly-only options.
- rustfmt options are read from the normal `rustfmt.toml` locations (project directory, or
  `~/Library/Application Support/rustfmt/rustfmt.toml` on macOS).

## Setup

Set the extension as your Rust formatter in `settings.json`:

```json
"[rust]": {
  "editor.defaultFormatter": "mariuzm.rustfmt-magic",
  "editor.formatOnSave": true
}
```

Alternatively, skip the extension and point rust-analyzer straight at the binary:

```json
"rust-analyzer.rustfmt.overrideCommand": ["/path/to/rustfmt-magic"]
```

## Settings

- `rustfmtMagic.binaryPath` — absolute path to a locally built `rustfmt-magic` binary (defaults to the bundled platform
  binary).
- `rustfmtMagic.rustfmtPath` — absolute path to the `rustfmt` to delegate to (defaults to `~/.cargo/bin/rustfmt`, then
  `rustfmt` on PATH).
