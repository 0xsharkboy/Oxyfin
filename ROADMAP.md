# Roadmap

This file is the single source of truth for planned work. An item is checked only when it is implemented and tested.

## Milestone 0: Foundations

- [x] Workspace and Nix dev shell
- [x] CI on Linux, Windows and macOS
- [x] Video rendering prototype (Slint + mpv) validated (Wayland, including HiDPI; X11 in copy mode, see Milestone 3)

## Milestone 1: Core experience (movies and shows)

- [x] Sign in (password and Quick Connect, session remembered)
- [x] Home (featured banner, Continue Watching, Next Up, Recently Added)
- [x] Libraries (grid, sort, filters)
- [x] Search
- [x] Movie and series details (seasons, episodes)
- [x] Player (direct play, controls, audio and subtitle menu, mpv shortcuts, resume and progress sync, next episode)
- [x] Settings (account, sign out, version)
- [x] English UI with translation support
- [x] Linux packages (Flatpak, Nix)

## Milestone 2: Everyday comfort

- [x] Media keys and MPRIS (Linux)
- [x] Trickplay thumbnails
- [x] Skip intro (media segments)
- [x] Subtitle settings
- [x] Transcoding and quality selection
- [x] Keep the screen awake during playback (Linux)
- [x] Subtitle and audio delay
- [x] Chapters (seek bar markers and menu)
- [x] Remember audio and subtitle choices per series
- [x] Next episode options (countdown, autoplay, automatic segment skipping)
- [x] Subtitle search and download
- [x] Playback statistics
- [x] "More like this" on detail pages
- [x] Cast and crew pages
- [x] Collections (box sets)
- [x] Server discovery on the local network
- [x] Authorize another device (Quick Connect code entry)
- [x] Fullscreen launch option
- [x] Discord Rich Presence (opt-in, never sends the server address)
- [x] AUR package (`oxyfin-git`)

## Milestone 3: Robustness and reach

- [ ] Multiple servers and users
- [ ] Remote control from Jellyfin apps
- [ ] Windows and macOS packages
- [ ] Media keys on Windows (SMTC) and macOS (MPNowPlayingInfoCenter)
- [ ] Direct VAAPI on X11 (force EGL in the Slint winit backend)

## Milestone 4: New uses

- [ ] Gamepad and remote navigation, TV mode
- [ ] Music, then other media types
- [ ] Offline downloads
- [ ] SyncPlay

## Known limits

- External image subtitles (`.sup`, `.idx`/`.sub` files beside the video) do not show; image subtitles inside the video file show when the file plays as is, not in a conversion.
- A video stream mpv cannot decode while its audio plays is not detected: the sound plays over a black picture and no conversion is offered. Choosing a quality below the bitrate of the file makes the server convert it.
