# Roadmap

This file is the single source of truth for planned work. An item is checked only when it is implemented and tested.

## Milestone 0: Foundations

- [x] Workspace and Nix dev shell
- [x] CI on Linux, Windows and macOS
- [x] Video rendering prototype (Slint + mpv) validated (Wayland, including HiDPI; X11 in copy mode, see the Later section)

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

## Later

- [ ] Direct VAAPI on X11 (force EGL in the Slint winit backend)
- [x] Media keys and MPRIS (Linux)
- [ ] Media keys on Windows (SMTC) and macOS (MPNowPlayingInfoCenter)
- [ ] Trickplay thumbnails
- [x] Skip intro (media segments)
- [ ] Subtitle settings
- [ ] Transcoding and quality selection
- [ ] Multiple servers and users
- [ ] Windows and macOS packages
- [ ] Gamepad and remote navigation, TV mode
- [ ] Music, then other media types
- [ ] Offline downloads
- [ ] SyncPlay
