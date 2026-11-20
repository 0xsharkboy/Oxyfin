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
- [ ] Remember audio and subtitle choices per series
- [ ] Next episode options (countdown, autoplay, automatic segment skipping)
- [ ] Subtitle search and download
- [x] Playback statistics
- [ ] "More like this" on detail pages
- [ ] Cast and crew pages
- [ ] Collections (box sets)
- [ ] Server discovery on the local network
- [ ] Authorize another device (Quick Connect code entry)
- [x] Fullscreen launch option
- [ ] Discord Rich Presence (opt-in, never sends the server address)
- [ ] AUR package

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
