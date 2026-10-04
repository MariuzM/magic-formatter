# Magic Formatter

A formatter with user-controlled layout, plus semantic highlighting and Find All References, from a single fast native
language server.

## Supported languages

|                                                                                                                    | Language   | Formatting                                                                                                 | Highlighting  |  References   |
| :----------------------------------------------------------------------------------------------------------------: | ---------- | ---------------------------------------------------------------------------------------------------------- | :-----------: | :-----------: |
|    <img src="https://raw.githubusercontent.com/github/explore/main/topics/rust/rust.png" width="40" alt="Rust">    | **Rust**   | rustfmt plus magic trailing comma, first argument position, method chains, single-line `if`, `=` alignment | rust-analyzer | rust-analyzer |
|  <img src="https://raw.githubusercontent.com/github/explore/main/topics/swift/swift.png" width="40" alt="Swift">   | **Swift**  | Built-in: the same rules, plus re-indenting and spacing                                                    |       ✓       |       ✓       |
|               <img src="https://avatars.githubusercontent.com/u/7966854?s=80" width="40" alt="TOML">               | **TOML**   | Built-in: magic trailing comma and first element position for arrays, `=` alignment, spacing               |       ✓       |       ✓       |
|  <img src="https://raw.githubusercontent.com/github/explore/main/topics/shell/shell.png" width="40" alt="Shell">   | **Shell**  | Built-in: re-indenting blocks and continuation lines, heredocs and strings untouched                       |       ✓       |       ✓       |
| <img src="https://raw.githubusercontent.com/github/explore/main/topics/python/python.png" width="40" alt="Python"> | **Python** | Built-in: magic trailing comma, first argument position, method chains, `=` alignment, re-indenting        |       ✓       |       ✓       |
| <img src="https://raw.githubusercontent.com/github/explore/main/topics/kotlin/kotlin.png" width="40" alt="Kotlin"> | **Kotlin** | Built-in: magic trailing comma, first argument position, method chains, `=` alignment, re-indenting        |       ✓       |       ✓       |
| <img src="https://raw.githubusercontent.com/github/explore/main/topics/docker/docker.png" width="40" alt="Docker"> | **Docker** | Built-in: re-indenting continuation lines, exec-form array spacing, heredocs and strings untouched         |       ✓       |       ✓       |

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
},
"[toml]": {
  "editor.defaultFormatter": "mariuzm.magic-formatter",
  "editor.formatOnSave": true
},
"[shellscript]": {
  "editor.defaultFormatter": "mariuzm.magic-formatter",
  "editor.formatOnSave": true
},
"[python]": {
  "editor.defaultFormatter": "mariuzm.magic-formatter",
  "editor.formatOnSave": true
},
"[kotlin]": {
  "editor.defaultFormatter": "mariuzm.magic-formatter",
  "editor.formatOnSave": true
},
"[dockerfile]": {
  "editor.defaultFormatter": "mariuzm.magic-formatter",
  "editor.formatOnSave": true
}
```

## Requirements

- **Rust**: the [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) extension
  for highlighting, completion, and navigation; Magic Formatter only formats Rust.
- **Rust**: `rustfmt` installed via rustup, with the nightly toolchain available (`rustup toolchain install nightly`) if
  your `rustfmt.toml` uses nightly-only options.
- **Swift, Kotlin, TOML, Shell, Python, Docker**: nothing; their formatters are built in.
- **Swift** (optional): Xcode or a Swift toolchain, whose `sourcekit-lsp` answers go to definition, hover, and
  completion for SDK symbols such as SwiftUI types. For Xcode projects, also run
  [`xcode-build-server`](https://github.com/SolaWing/xcode-build-server)
  `config -project <App>.xcodeproj -scheme <App>` once so types from other files resolve too.
