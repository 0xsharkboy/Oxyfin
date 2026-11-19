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

`nix build` builds the package the flake ships (`packages.default`, run with `nix run`): the release binary in `result/bin/oxyfin` and the files of `data/` under `result/share/`. The package does not run the tests, which CI runs with cargo.

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
- Logging goes through the `log` facade. No `println!` outside the binary's entry point.
- `unsafe` is only allowed in `oxyfin-player`, around libmpv. Every `unsafe` block has a `// SAFETY:` comment explaining why it is sound.
- Prototypes under `spikes/` are exempt from the `unsafe`, `println!` and test rules.
- Every user-facing string goes through Slint translations (`@tr(...)`). After changing one, regenerate the catalog with `find crates/oxyfin/ui -name "*.slint" | sort | xargs slint-tr-extractor --package-name oxyfin -o crates/oxyfin/po/oxyfin.pot` (the extractor is in the Nix dev shell).
- Translations are bundled into the binary from `crates/oxyfin/po/<lang>/LC_MESSAGES/oxyfin.po`. After regenerating the catalog, update each language with `msgmerge --update crates/oxyfin/po/<lang>/LC_MESSAGES/oxyfin.po crates/oxyfin/po/oxyfin.pot` (`msgmerge` comes from gettext) and translate the new and fuzzy entries; the `po_complete` tests fail while a French entry is missing, empty or still flagged fuzzy (Slint drops fuzzy entries).
- To add a language, copy the catalog to `crates/oxyfin/po/<lang>/LC_MESSAGES/oxyfin.po` (`<lang>` is the two-letter code, such as `de`), set `Language` and `Plural-Forms` in its header, translate every entry, and add a completeness test like the French one. Keep `Oxyfin`, `Jellyfin` and `Quick Connect` untranslated and the `{}` and `{n}` placeholders unchanged. At startup the app picks the language from `LANGUAGE`, `LC_ALL`, `LC_MESSAGES` and `LANG`, then from the system locale, and falls back to English.
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
- `oxyfin-api` is tested against a mock HTTP server in CI. Opt-in tests against a real server are described below.
- A new feature comes with tests. A bug fix comes with a test that fails without the fix.

### Testing against a real Jellyfin server

`scripts/jellyfin-test-server.sh` starts a disposable Jellyfin server in Docker, with generated media (a movie and a two-season show). It needs Docker, `ffmpeg`, `curl` and `jq`; when `ffmpeg` is not on the `PATH`, `up` re-runs itself under `nix develop`. The library scan wait defaults to 600 s (`OXYFIN_TEST_SCAN_TIMEOUT`). Pick a version tag (`10.10.7`, `10.11.11` or `12.4`):

```sh
eval "$(scripts/jellyfin-test-server.sh up 12.4)"   # exports the OXYFIN_TEST_* variables
scripts/jellyfin-test-server.sh down                # removes the container
```

The live tests are `#[ignore]`d, so `cargo test --workspace` skips them. There are two tiers:

```sh
# Read-only: no writes
cargo test -p oxyfin-api --test live_read -- --ignored

# Writes to the server: disposable Docker server only
cargo test -p oxyfin-api --test live_write -- --ignored --test-threads=1
```

With a variable missing, the tests fail and name it instead of passing silently.

`live_write` marks items played, toggles favorites, reports playback progress and signs out. It refuses to run unless `OXYFIN_TEST_DISPOSABLE=1`, which only the script sets. Never set it by hand and never point `live_write` at a personal server: it would alter your watch history and resume positions.

### Headless app

`scripts/headless-app.sh` runs Oxyfin without a window, with Slint's embedded MCP server, so the interface can be driven and captured from the shell (mainly by coding agents checking the design against the reference). It builds into `target/headless` with `slint/mcp` and the software renderer, then runs the app with throwaway config, data and cache directories and tokens kept in memory, so nothing is left behind. It prints `OXYFIN_MCP_URL=...` once the server answers and stops the app when it is stopped.

```sh
scripts/headless-app.sh --size 900x700 --scale 2 --lang fr &   # options: --size, --scale, --lang, --port, --no-build
scripts/mcp-call.sh take_screenshot '{"windowHandle":{"index":"1","generation":"1"}}' /tmp/shot.png
scripts/mcp-call.sh set_element_value "{\"elementHandle\":$(scripts/mcp-call.sh --find 'Server address' TextInput),\"value\":\"http://127.0.0.1:18096\"}"
scripts/mcp-call.sh click_element "{\"elementHandle\":$(scripts/mcp-call.sh --find 'Sign in' Button),\"action\":\"SingleClick\",\"button\":\"Left\"}"
scripts/mcp-call.sh dispatch_key_event '{"windowHandle":{"index":"1","generation":"1"},"text":"\u001b"}'   # Escape (eventType defaults to PressAndRelease)
```

`scripts/mcp-call.sh TOOL [JSON_ARGS] [OUT.png]` calls any tool of the server (`list_windows`, `get_element_tree`, `click_element`, `scroll_element`, ...) and writes the image of an answer to `OUT.png`; `--find LABEL [ROLE]` prints the handle of the first element with that accessible label. Limits: the build has no mpv, so video does not show; the software renderer draws text slightly differently from the app's femtovg renderer; the first build takes several minutes. The app needs a server typed in, like a user would: the disposable server above, or your own.

## Desktop files

`data/` holds the Linux desktop entry, the scalable icon and the AppStream metainfo. Each file is named after the application ID (`oxyfin::APP_ID`, which the window also advertises to the compositor), and a test checks that they agree. After editing them, validate them:

```sh
nix shell nixpkgs#desktop-file-utils -c desktop-file-validate data/*.desktop
nix shell nixpkgs#appstream -c appstreamcli validate --no-net data/*.metainfo.xml
```

## Flatpak

`flatpak/` holds the Flatpak manifest, `io.github._0xsharkboy.Oxyfin.yml`, on the freedesktop 25.08 runtime. It builds libmpv and the libraries the runtime lacks (libplacebo, libass, uchardet), uses the runtime's FFmpeg with its codecs-extra extension, and builds the app from the working tree (without `target/`) with the crates listed in `flatpak/cargo-sources.json`, since the build has no network. Build, install and run it for your user, keeping the build directories outside the repository:

```sh
nix shell nixpkgs#flatpak-builder nixpkgs#appstream -c flatpak-builder --user --install --force-clean \
  --install-deps-from=flathub --state-dir=/tmp/oxyfin-flatpak/state \
  /tmp/oxyfin-flatpak/build flatpak/io.github._0xsharkboy.Oxyfin.yml
flatpak run io.github._0xsharkboy.Oxyfin
flatpak uninstall --user --delete-data io.github._0xsharkboy.Oxyfin
```

`flatpak-builder` runs `appstreamcli` from the host to compose the metainfo. Without Nix, install `flatpak-builder` and `appstream` from your distribution or use `flatpak run org.flatpak.Builder` with the same arguments. Lint the manifest with `flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest flatpak/io.github._0xsharkboy.Oxyfin.yml`.

After any change to `Cargo.lock`, regenerate `flatpak/cargo-sources.json` with [`flatpak-cargo-generator.py`](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo) and commit it with the lock file:

```sh
nix shell --impure --expr 'with import (builtins.getFlake "nixpkgs") {}; python3.withPackages (p: [p.aiohttp p.tomlkit])' \
  -c python3 flatpak-cargo-generator.py Cargo.lock -o flatpak/cargo-sources.json
```

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
