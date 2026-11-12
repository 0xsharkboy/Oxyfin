# AGENTS.md

Instructions for AI coding agents working on Oxyfin. Human contributors follow [CONTRIBUTING.md](CONTRIBUTING.md); everything in it applies to agents too. This file adds the rules agents most often get wrong.

## Project

Oxyfin is a native Jellyfin desktop client written in Rust, with a Slint UI and libmpv for playback. No web rendering of any kind.

| Path | Role |
|---|---|
| `crates/oxyfin-api` | Typed Jellyfin HTTP client. No UI code. |
| `crates/oxyfin-player` | Safe libmpv wrapper and OpenGL video rendering. The only crate allowed to use `unsafe`. |
| `crates/oxyfin-core` | Application logic: session, use cases, progress sync, caching. |
| `crates/oxyfin` | Binary: Slint UI wired to `oxyfin-core`. |
| `spikes/` | Throwaway prototypes, outside the workspace. Reference only. |
| `ROADMAP.md` | Planned work and progress. |

The UI never talks to HTTP or mpv directly: it goes through `oxyfin-core`.

## Commands

```sh
nix develop                                        # toolchain, libmpv, hooks
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p oxyfin
```

Run fmt, clippy and tests before every commit. Do not commit if any of them fails.

## Rules

1. **English only**, in every file, identifier, comment, commit message and branch name, regardless of the language the user writes in.
2. **Never use the em dash character (U+2014)**, anywhere. Use a colon, comma, parentheses or a plain hyphen.
3. **Comments are minimal.** Document public items with rustdoc (`///`, `//!`). Write an inline `//` comment only to explain a non-obvious reason, in one short line. Never narrate what the code does, never leave notes addressed to the user or the reviewer, never leave commented-out code.
4. **Never commit `docs/superpowers/` or `.superpowers/`.** They hold working documents and stay local.
5. **Never push, open, merge or close a pull request** unless the user explicitly asks for it in the current conversation.
6. **Commits follow CONTRIBUTING.md**: Conventional Commits with a scope, one self-contained change per commit, each commit building and passing tests. Do not add `Co-authored-by` or any other attribution trailer unless the user asks for it.
7. **No `unwrap()` or `expect()` outside tests**, no `unsafe` outside `oxyfin-player`, no `println!` outside the binary's entry point.
8. **Do not add dependencies casually.** Prefer what is already in `Cargo.toml`. If a new crate is needed, say why in the commit body.
9. **Follow the reference design** (https://www.figma.com/design/KjhDn8ypMS0CP6UQNP5Lbv). Use the `Theme` global for colors, spacing and radii, and `@tr(...)` for every user-facing string.
10. **Keep `ROADMAP.md` honest.** Check an item only when it is implemented and tested.

## Comment examples

```rust
// Acceptable: a constraint the code cannot express.
// mpv resolves all GL symbols during context creation, so the loader is not kept.

// Not acceptable:
// Create the render context
// TODO: maybe refactor this later?
// Changed this to fix the bug you mentioned
```
