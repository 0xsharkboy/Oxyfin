# video-underlay

Throwaway prototype that checks whether mpv can render video underneath a Slint scene through the mpv OpenGL render API and the Slint rendering notifier, without copying frames through the CPU.

It lives outside the Cargo workspace and is kept only as a reference for the future `oxyfin-player` crate.

## Run

```sh
nix develop -c cargo run --release --manifest-path spikes/video-underlay/Cargo.toml -- <video-file>
```

Space toggles pause, F toggles fullscreen.

## How it works

- mpv is created with `vo=libmpv`, `hwdec=auto-safe` and `keep-open=yes`.
- The mpv OpenGL render context is created in `RenderingState::RenderingSetup`. Slint's `get_proc_address` only lives during that notifier call, so a small trampoline forwards to it; mpv resolves every GL symbol inside `mpv_render_context_create`, and the loader is never used afterwards.
- The native Wayland or X11 display is passed to mpv at creation. Without it, mpv falls back to `vaapi-copy`.
- `BeforeRendering` acknowledges updates and always redraws the last frame into the current framebuffer at the window's physical size; `AfterRendering` reports the swap; `RenderingTeardown` frees the render context while the GL context is current.
- A metrics line (`hwdec`, video size, dropped and delayed frames, fps, window size, scale factor) is printed every 2 s and shown in the control bar.

## Results (AMD Barcelo, Mesa, COSMIC, NixOS)

Verdict: **GO**, with the limitations below.

| | Wayland | X11 (XWayland) |
|---|---|---|
| Hardware decoding | `vaapi` (direct) | `vaapi-copy` |
| 1080p60 H.264 | smooth | smooth |
| 4K60 VP9, fullscreen | 0 to 1 dropped frame per 10 s | about 22 dropped frames per second |
| Overlaid controls, resize, fullscreen, pause | correct | correct |

Limitations:

- Wayland, windowed: about one dropped frame per second in 4K60, none in fullscreen.
- Frames drop while the window is being resized; this stops once the size is stable.
- X11: Slint's winit backend asks glutin for GLX first (`ApiPreference::FallbackEgl`), and mpv's direct VAAPI interop needs EGL. `oxyfin-player` needs a way to force EGL on X11.
- A scale factor other than 1 was not tested.

## Notes for `oxyfin-player`

- The prototype leaks `Mpv` as `&'static Mpv` because `RenderContext` borrows it and the notifier callback must be `'static`. The real player needs a proper owning structure that drops the render context before the `Mpv` instance, with the GL context current.
- The position slider must not be updated by the playback timer while it is dragged, nor right after a seek until mpv reaches the target, otherwise it snaps back.
