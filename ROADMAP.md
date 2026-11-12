# Roadmap

This file is the single source of truth for planned work. An item is checked only when it is implemented and tested.

## Milestone 0: Foundations

- [x] Workspace and Nix dev shell
- [x] CI on Linux, Windows and macOS
- [x] Video rendering prototype (Slint + mpv) validated (Wayland; HiDPI untested, X11 in copy mode: see spikes/video-underlay/README.md)

## Milestone 1: Core experience (movies and shows)

- [ ] Sign in (password and Quick Connect, session remembered)
- [ ] Home (featured banner, Continue Watching, Next Up, Recently Added)
- [ ] Libraries (grid, sort, filters)
- [ ] Search
- [ ] Movie and series details (seasons, episodes)
- [ ] Player (direct play, controls, audio and subtitle menu, mpv shortcuts, resume and progress sync, next episode)
- [ ] Settings (account, sign out, version)
- [ ] English UI with translation support
- [ ] Linux packages (Flatpak, Nix)

## Later

- [ ] Media keys and MPRIS
- [ ] Trickplay thumbnails
- [ ] Skip intro (media segments)
- [ ] Subtitle settings
- [ ] Transcoding and quality selection
- [ ] Multiple servers and users
- [ ] Windows and macOS packages
- [ ] Gamepad and remote navigation, TV mode
- [ ] Music, then other media types
- [ ] Offline downloads
- [ ] SyncPlay
