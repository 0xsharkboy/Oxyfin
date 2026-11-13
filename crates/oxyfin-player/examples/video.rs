//! Plays a video in a Slint window through [`oxyfin_player::VideoRenderer`].
//!
//! Usage: `cargo run -p oxyfin-player --release --example video -- <path-or-url>`.
//! Space toggles pause, F toggles fullscreen, H hides the window for one
//! second. Rendering statistics are printed to stderr every 2 s.

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    app::main()
}

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
mod app {
    use std::process::ExitCode;
    use std::rc::Rc;
    use std::time::Duration;

    use oxyfin_player::{
        MediaSource, Player, PlayerEvent, PlayerOptions, SeekTarget, UnloadReason, VideoRenderer,
    };
    use raw_window_handle::HasDisplayHandle;
    use slint::{CloseRequestResponse, ComponentHandle, GraphicsAPI, RenderingState};

    slint::slint! {
        import { Button, Slider } from "std-widgets.slint";

        export component MainWindow inherits Window {
            in-out property <bool> paused: false;
            in-out property <float> position: 0;
            in-out property <float> duration: 0;
            in-out property <string> stats: "";
            in-out property <bool> seek-pending: false;
            out property <bool> scrubbing: false;

            callback toggle-pause();
            callback seek(float);
            callback toggle-fullscreen();
            callback hide-briefly();

            title: "oxyfin-player";
            // Without a transparent background Slint paints over the video.
            background: transparent;
            preferred-width: 1280px;
            preferred-height: 720px;

            FocusScope {
                key-pressed(event) => {
                    if (event.text == " ") {
                        root.toggle-pause();
                        return accept;
                    }
                    if (event.text == "f" || event.text == "F") {
                        root.toggle-fullscreen();
                        return accept;
                    }
                    if (event.text == "h" || event.text == "H") {
                        root.hide-briefly();
                        return accept;
                    }
                    reject
                }

                VerticalLayout {
                    Rectangle { }

                    Rectangle {
                        height: 56px;
                        background: #000000;
                        opacity: 0.6;

                        HorizontalLayout {
                            padding: 8px;
                            spacing: 8px;

                            Button {
                                text: root.paused ? "Play" : "Pause";
                                clicked => { root.toggle-pause(); }
                            }

                            Slider {
                                minimum: 0;
                                maximum: max(root.duration, 1);
                                value <=> root.position;
                                changed(value) => { root.scrubbing = true; }
                                released(value) => {
                                    root.scrubbing = false;
                                    root.seek(value);
                                }
                            }

                            Button {
                                text: "Fullscreen";
                                clicked => { root.toggle-fullscreen(); }
                            }

                            Text {
                                text: root.stats;
                                color: #ffffff;
                                vertical-alignment: center;
                            }
                        }
                    }
                }
            }
        }
    }

    const STATS_PERIOD: Duration = Duration::from_secs(2);
    const HIDDEN_FOR: Duration = Duration::from_secs(1);

    pub fn main() -> ExitCode {
        let Some(location) = std::env::args_os().nth(1) else {
            eprintln!("usage: video <path-or-url>");
            return ExitCode::from(2);
        };
        let Some(location) = location.to_str().map(str::to_owned) else {
            eprintln!("the path or URL is not valid UTF-8");
            return ExitCode::from(2);
        };
        match run(&location) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        }
    }

    fn run(location: &str) -> Result<(), Box<dyn std::error::Error>> {
        slint::BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("femtovg".into())
            .require_opengl()
            .select()?;

        let window = MainWindow::new()?;
        let weak = window.as_weak();
        let player = Rc::new(Player::new(PlayerOptions::default(), move |event| {
            let _ = weak.upgrade_in_event_loop(move |window| on_player_event(&window, event));
        })?);

        connect_controls(&window, &player);
        let stats_timer = start_stats(&window, &player);
        set_rendering_notifier(&window, &player)?;
        window.window().on_close_requested(|| {
            let _ = slint::quit_event_loop();
            CloseRequestResponse::HideWindow
        });

        player.load(&MediaSource::new(location))?;
        window.show()?;
        // Hiding the window must not end the loop, so it runs until an explicit quit.
        let result = slint::run_event_loop_until_quit();
        // Tears rendering down while the GL context exists, before the player goes.
        let hidden = window.hide();
        drop(stats_timer);
        result?;
        hidden?;
        Ok(())
    }

    fn on_player_event(window: &MainWindow, event: PlayerEvent) {
        match event {
            PlayerEvent::FileLoaded { duration, .. } => {
                window.set_duration(duration.map_or(0.0, |d| d.as_secs_f32()));
            }
            PlayerEvent::Position(position) => {
                if !window.get_scrubbing() && !window.get_seek_pending() {
                    window.set_position(position.as_secs_f32());
                }
            }
            PlayerEvent::Seeked => window.set_seek_pending(false),
            PlayerEvent::Paused(paused) => window.set_paused(paused),
            PlayerEvent::Unloaded(UnloadReason::Error(err)) => {
                eprintln!("error: playback failed: {err}");
            }
            _ => {}
        }
    }

    fn connect_controls(window: &MainWindow, player: &Rc<Player>) {
        let weak = window.as_weak();
        let control = Rc::clone(player);
        window.on_toggle_pause(move || {
            let Some(window) = weak.upgrade() else { return };
            if let Err(err) = control.set_paused(!window.get_paused()) {
                eprintln!("error: pause: {err}");
            }
        });

        let weak = window.as_weak();
        let control = Rc::clone(player);
        window.on_seek(move |seconds| {
            let Some(window) = weak.upgrade() else { return };
            let Ok(target) = Duration::try_from_secs_f32(seconds) else {
                return;
            };
            window.set_seek_pending(true);
            if let Err(err) = control.seek(SeekTarget::Absolute(target)) {
                window.set_seek_pending(false);
                eprintln!("error: seek: {err}");
            }
        });

        let weak = window.as_weak();
        window.on_toggle_fullscreen(move || {
            if let Some(window) = weak.upgrade() {
                let window = window.window();
                window.set_fullscreen(!window.is_fullscreen());
            }
        });

        let weak = window.as_weak();
        window.on_hide_briefly(move || {
            let Some(window) = weak.upgrade() else { return };
            if let Err(err) = window.hide() {
                eprintln!("error: hide: {err}");
                return;
            }
            let weak = weak.clone();
            slint::Timer::single_shot(HIDDEN_FOR, move || {
                if let Some(window) = weak.upgrade()
                    && let Err(err) = window.show()
                {
                    eprintln!("error: show: {err}");
                }
            });
        });
    }

    fn start_stats(window: &MainWindow, player: &Rc<Player>) -> slint::Timer {
        let weak = window.as_weak();
        let player = Rc::clone(player);
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, STATS_PERIOD, move || {
            let Some(window) = weak.upgrade() else { return };
            let size = window.window().size();
            let line = format!(
                "{} window={}x{} scale={:.2}",
                player.stats(),
                size.width,
                size.height,
                window.window().scale_factor(),
            );
            eprintln!("{line}");
            window.set_stats(line.into());
        });
        timer
    }

    fn set_rendering_notifier(
        window: &MainWindow,
        player: &Rc<Player>,
    ) -> Result<(), slint::SetRenderingNotifierError> {
        let weak = window.as_weak();
        let player = Rc::clone(player);
        let mut renderer: Option<VideoRenderer> = None;
        window
            .window()
            .set_rendering_notifier(move |state, graphics_api| {
                let result = match state {
                    RenderingState::RenderingSetup => {
                        create_renderer(&player, graphics_api, &weak).map(|r| renderer = Some(r))
                    }
                    RenderingState::BeforeRendering => match (&renderer, weak.upgrade()) {
                        (Some(renderer), Some(window)) => {
                            let size = window.window().size();
                            renderer.render(size.width, size.height).map_err(Into::into)
                        }
                        _ => Ok(()),
                    },
                    RenderingState::AfterRendering => {
                        if let Some(renderer) = &renderer {
                            renderer.report_swap();
                        }
                        Ok(())
                    }
                    RenderingState::RenderingTeardown => {
                        renderer = None;
                        Ok(())
                    }
                    _ => Ok(()),
                };
                if let Err(err) = result {
                    eprintln!("error: {err}");
                    let _ = slint::quit_event_loop();
                }
            })
    }

    fn create_renderer(
        player: &Player,
        graphics_api: &GraphicsAPI<'_>,
        weak: &slint::Weak<MainWindow>,
    ) -> Result<VideoRenderer, Box<dyn std::error::Error>> {
        let GraphicsAPI::NativeOpenGL { get_proc_address } = graphics_api else {
            return Err("the Slint renderer does not use OpenGL".into());
        };
        let window = weak.upgrade().ok_or("the window is gone")?;
        let handle = window.window().window_handle();
        let display = handle.display_handle().ok();
        if display.is_none() {
            eprintln!("warning: no native display, direct hardware decoding is unavailable");
        }
        let mut renderer = VideoRenderer::new(player, *get_proc_address, display)?;
        let weak = weak.clone();
        renderer.set_update_callback(move || {
            let _ = weak.upgrade_in_event_loop(|window| window.window().request_redraw());
        });
        Ok(renderer)
    }
}
