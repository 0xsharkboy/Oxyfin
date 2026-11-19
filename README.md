# Oxyfin

Oxyfin is a native Jellyfin desktop client written in Rust, with no web rendering of any kind: the interface is built with Slint and playback goes through libmpv.

> [!WARNING]
> **Work in progress.** Oxyfin is in early development and not ready for everyday use. Expect missing features, bugs and breaking changes. There are no releases yet: it runs from source or through the Nix flake.

What works today: sign in (password or Quick Connect), home, movie and show libraries, search, movie and series pages, playback, settings, in English and French. See [ROADMAP.md](ROADMAP.md) for what comes next.

## Running with Nix

On Linux, the flake builds and runs the app without cloning the repository:

```sh
nix run github:0xsharkboy/Oxyfin
```

From a clone, `nix build` puts the app in `result/bin/oxyfin`, with its desktop entry, icon and metainfo under `result/share/`.

## Running as a Flatpak

The manifest in `flatpak/` builds the app as a Flatpak from a clone. It is not on Flathub yet; see [CONTRIBUTING.md](CONTRIBUTING.md#flatpak) to build and install it locally.

## Development

The Nix dev shell provides the Rust toolchain, libmpv and every system library the app needs:

```sh
nix develop
cargo run -p oxyfin
```

Building without Nix and the contribution rules are described in [CONTRIBUTING.md](CONTRIBUTING.md).

## Roadmap

Planned work and progress are tracked in [ROADMAP.md](ROADMAP.md).

## License

Oxyfin is licensed under the [GNU General Public License v3.0](LICENSE).
