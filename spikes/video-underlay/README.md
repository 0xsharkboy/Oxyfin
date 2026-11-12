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

Test files: `sample-1080p.mkv` (H.264 1080p59.94, 4.1 Mbit/s) and `sample-2160p.mkv` (VP9 2160p59.94, 21.6 Mbit/s).

| | Wayland | X11 (XWayland) |
|---|---|---|
| Hardware decoding | `vaapi` (direct) | `vaapi-copy` |
| 1080p, windowed and fullscreen | `vaapi`, 0 to 1 dropped frame per 2 s | 0 dropped frames at a fixed size |
| 4K, fullscreen (1920x1080) | 0 to 1 dropped frame per 10 s | not measured |
| 4K, windowed | about 1 dropped frame per second (844x992, 945x992) | about 22 dropped frames per second (1900x992) |
| Overlaid controls, resize, fullscreen, pause | correct | correct |

Limitations:

- 4K windowed drops frames on both backends (see the table); fullscreen on X11 was not measured.
- Frames drop while the window is being resized; this stops once the size is stable.
- X11: Slint's winit backend asks glutin for GLX first (`ApiPreference::FallbackEgl`), and mpv's direct VAAPI interop needs EGL. `oxyfin-player` needs a way to force EGL on X11.
- A scale factor other than 1 was not tested.

## Notes for `oxyfin-player`

- The prototype leaks `Mpv` as `&'static Mpv` because `RenderContext` borrows it and the notifier callback must be `'static`. The real player needs a proper owning structure that drops the render context before the `Mpv` instance, with the GL context current.
- The position slider must not be updated by the playback timer while it is dragged, nor right after a seek until mpv reaches the target, otherwise it snaps back.
- Load the media independently of the render context. The prototype calls `loadfile` in `RenderingSetup`, which runs again if Slint tears down and re-creates rendering (for example on hide and show).
- Report unreadable or non-video files. The prototype only checks that the file exists, so an unplayable file gives an empty window with no message.
- Drive the slider "scrubbing" state from the slider's pressed state rather than from the changed and released callbacks, when the widget allows it.
