# Oxyfin

Oxyfin is a native Jellyfin desktop client written in Rust, with no web rendering of any kind: the interface is built with Slint and playback goes through libmpv.

**Status:** early development. Nothing is usable yet.

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
