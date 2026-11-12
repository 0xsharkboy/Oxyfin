# video-underlay

Throwaway prototype that checks whether mpv can render video underneath a Slint scene through the mpv OpenGL render API and the Slint rendering notifier, without copying frames through the CPU.

It lives outside the Cargo workspace and is kept only as a reference for the future `oxyfin-player` crate.

## Run

```sh
nix develop -c cargo run --release --manifest-path spikes/video-underlay/Cargo.toml -- <video-file>
```

Space toggles pause, F toggles fullscreen.
