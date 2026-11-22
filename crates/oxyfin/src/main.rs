//! Oxyfin, a native Jellyfin desktop client.

use std::process::ExitCode;
use std::time::Duration;

use oxyfin::launch;
use oxyfin::locale::select_language;
use oxyfin::{App, AppWindow};
use oxyfin_core::{Paths, TokenVault};
use slint::platform::WindowEvent;
use slint::{BackendSelector, ComponentHandle, LogicalSize, PhysicalSize};

/// How long the exit waits for the sessions still shutting down.
const EXIT_GRACE: Duration = Duration::from_secs(3);
/// How long the exit waits for the Discord activity to clear.
const DISCORD_GRACE: Duration = Duration::from_secs(1);

#[allow(clippy::print_stdout)]
fn main() -> ExitCode {
    if launch::version_requested(std::env::args_os().skip(1)) {
        println!("{}", launch::version_line());
        return ExitCode::SUCCESS;
    }
    let paths = Paths::from_system();
    oxyfin::logs::init(paths.as_ref().ok().map(oxyfin::logs::log_dir).as_deref());
    log::info!("{} on {}", launch::version_line(), oxyfin::logs::system());
    match paths.map_err(Into::into).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("oxyfin failed to start: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(paths: Paths) -> Result<(), Box<dyn std::error::Error>> {
    // An explicit SLINT_BACKEND (headless agent runs) picks its own backend and renderer.
    if std::env::var("SLINT_BACKEND").map_or(true, |value| value.trim().is_empty()) {
        BackendSelector::new()
            .backend_name("winit".into())
            .renderer_name("femtovg".into())
            .require_opengl()
            .select()?;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let vault = if launch::enabled(std::env::var(launch::MEMORY_TOKENS_VAR).ok().as_deref()) {
        log::info!("tokens are kept in memory only");
        TokenVault::memory()
    } else {
        TokenVault::system(&paths)
    };
    let window = AppWindow::new()?;
    // Needs the platform set up by AppWindow::new(); it applies when the window is first shown.
    slint::set_xdg_app_id(oxyfin::APP_ID)?;
    let scale = std::env::var(launch::SCALE_FACTOR_VAR)
        .ok()
        .and_then(|value| {
            let scale = launch::scale_factor(&value);
            if scale.is_none() {
                log::warn!("ignoring {}={value:?}", launch::SCALE_FACTOR_VAR);
            }
            scale
        });
    if let Some(scale_factor) = scale {
        window
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor });
    }
    if let Ok(value) = std::env::var(launch::WINDOW_SIZE_VAR) {
        match (launch::window_size(&value), scale) {
            // The headless backend keeps its physical size when the scale changes.
            (Some((width, height)), Some(scale)) => window.window().set_size(PhysicalSize::new(
                (width as f32 * scale).round() as u32,
                (height as f32 * scale).round() as u32,
            )),
            (Some((width, height)), None) => window
                .window()
                .set_size(LogicalSize::new(width as f32, height as f32)),
            (None, _) => log::warn!("ignoring {}={value:?}", launch::WINDOW_SIZE_VAR),
        }
    }
    select_language();
    let app = App::new(&window, runtime.handle().clone(), paths, vault)?;
    #[cfg(feature = "mpv")]
    {
        let weak = std::rc::Rc::downgrade(&app);
        oxyfin::video::install(
            &window,
            app.video_source(),
            runtime.handle().clone(),
            move |id| {
                let weak = weak.clone();
                // Leaves the rendering notifier before touching the window.
                slint::Timer::single_shot(Duration::ZERO, move || {
                    if let Some(app) = weak.upgrade() {
                        app.video_failed(id);
                    }
                });
            },
        )?;
    }
    #[cfg(all(target_os = "linux", feature = "mpv"))]
    {
        app.start_mpris();
        app.start_inhibitor();
    }
    app.start_discord();
    let fullscreen =
        launch::fullscreen_requested(std::env::args_os().skip(1)) || app.start_fullscreen();
    window.window().set_fullscreen(fullscreen);
    window.show()?;
    if fullscreen {
        // Asked again once the event loop runs: the first configure of a Wayland window can undo it.
        let weak = window.as_weak();
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(window) = weak.upgrade() {
                window.window().set_fullscreen(true);
            }
        });
    }
    slint::run_event_loop_until_quit()?;
    if let Err(error) = window.hide() {
        log::warn!("cannot hide the window: {error}");
    }

    if let Some(discord) = app.take_discord() {
        discord.shutdown(DISCORD_GRACE);
    }
    let pending = app.take_pending_ends();
    runtime.block_on(async {
        let deadline = tokio::time::Instant::now() + EXIT_GRACE;
        #[cfg(all(target_os = "linux", feature = "mpv"))]
        if let Some(inhibitor) = app.take_inhibitor()
            && tokio::time::timeout_at(deadline, inhibitor.release())
                .await
                .is_err()
        {
            log::warn!("the screen inhibition was not released before exit");
        }
        for task in pending {
            if tokio::time::timeout_at(deadline, task).await.is_err() {
                log::warn!("a session did not finish shutting down before exit");
                break;
            }
        }
    });
    drop(app);
    runtime.shutdown_timeout(Duration::from_millis(500));
    Ok(())
}
