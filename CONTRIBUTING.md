# Contributing to Oxyfin

These are the rules every change to Oxyfin follows, whether it is written by a human or an AI agent. The CI enforces most of them; the rest is checked in review.

## Language

Everything in the repository is written in English: code, identifiers, comments, documentation, commit messages, branch names, pull requests and issues. No exceptions.

## Getting started

### With Nix (recommended)

```sh
nix develop
```

The dev shell provides the Rust toolchain, libmpv and every system library the app needs, and activates the git hooks automatically.

### Without Nix

Install:

- the stable Rust toolchain with `rustfmt` and `clippy` (via [rustup](https://rustup.rs));
- `pkg-config` and the libmpv development package (`libmpv-dev`, `mpv-devel` or equivalent);
- on Linux, the Wayland, X11, xkbcommon, fontconfig and OpenGL development packages.

Then activate the git hooks once:

```sh
git config core.hooksPath .githooks
```

## Workflow

1. Branch from an up-to-date `main`. Name the branch `<type>/<short-description>`, for example `feat/quick-connect` or `fix/resume-position`.
2. Commit in small, self-contained steps (see below).
3. Rebase on `main` when it moves. Never merge `main` into your branch.
4. Open a pull request against `main`. It is merged once CI is green, using **rebase and merge**: every commit of the branch lands on `main` as is.
5. Because commits are kept, clean up history before merging (`git rebase -i`, `fixup!` commits), never after.

Nobody pushes directly to `main`.

## Commit messages

Commits follow [Conventional Commits](https://www.conventionalcommits.org):

```
<type>(<scope>): <summary>

<body: why the change is needed, when it is not obvious>
```

- **type**: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`, `ci`, `chore`, `revert`.
- **scope** (required): `api`, `player`, `core`, `ui`, `nix`, `ci`, `docs`, `deps`, `repo`, `spike`. New scopes are added here first.
- **summary**: imperative mood, lowercase, no trailing period, 72 characters at most.
- Breaking changes use `!` after the scope and a `BREAKING CHANGE:` footer.
- Each commit builds and passes the tests on its own.
- Never use the em dash character (U+2014), here or anywhere else in the repository.
- Attribution trailers such as `Co-authored-by` are up to each contributor and never required.

## Git hooks

The hooks in `.githooks/` run the fast checks locally. CI runs the same checks, so skipping the hooks only delays the failure.

| Hook | Checks |
|---|---|
| `pre-commit` | `cargo fmt --check`, no em dash in staged files, no staged file under `docs/superpowers/` or `.superpowers/` |
| `commit-msg` | Conventional Commits format, allowed scope, summary length, no em dash |
| `pre-push` | `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` |

## Code standards

- `rustfmt` formatting and zero `clippy` warnings.
- No `unwrap()` or `expect()` outside tests.
- Libraries (`oxyfin-api`, `oxyfin-player`, `oxyfin-core`) return typed errors built with `thiserror`. Only the `oxyfin` binary uses `anyhow`.
- Logging goes through `tracing`. No `println!` outside the binary's entry point.
- `unsafe` is only allowed in `oxyfin-player`, around libmpv. Every `unsafe` block has a `// SAFETY:` comment explaining why it is sound.
- Prototypes under `spikes/` are exempt from the `unsafe`, `println!` and test rules.
- Every user-facing string goes through Slint translations (`@tr(...)`).
- The UI follows the [reference design](https://www.figma.com/design/KjhDn8ypMS0CP6UQNP5Lbv). Colors, spacing and radii come from the `Theme` global, never from literal values.

## Documentation and comments

- Every public item is documented with rustdoc (`///`), and every crate and module starts with a `//!` summary. The `missing_docs` lint is an error.
- Inline `//` comments are rare and short. They explain a non-obvious *why*, never *what* the code does.

```rust
// Good: explains a constraint the code cannot show.
// Jellyfin expects ticks of 100 ns, not milliseconds.
let ticks = position.as_nanos() / 100;

// Bad: restates the code.
// Divide the position by 100 to get ticks.
let ticks = position.as_nanos() / 100;
```

## Tests

- Unit tests live next to the code they test (`#[cfg(test)] mod tests`).
- `oxyfin-api` is tested against a mock HTTP server, never against a real Jellyfin instance.
- A new feature comes with tests. A bug fix comes with a test that fails without the fix.

## Dependencies

- Prefer the standard library and the dependencies already in use.
- A new dependency is justified in the commit body.
- Licenses must be compatible with GPL-3.0. `cargo deny check` enforces this and rejects crates with known security advisories.

## Roadmap

[`ROADMAP.md`](ROADMAP.md) is the single source of truth for planned work. A feature is done when it is implemented, tested and checked in the roadmap, in the same pull request.

## Releases

- Versions follow [SemVer](https://semver.org) and stay in `0.x` until the first milestone is complete.
- Each release is a `vX.Y.Z` tag on `main`.
- `CHANGELOG.md` is generated from commit messages with `git-cliff`.

## Before opening a pull request

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Commits follow the message format and each one builds on its own.
- [ ] Public items are documented, comments are minimal.
- [ ] `ROADMAP.md` is updated if a roadmap item is completed.
