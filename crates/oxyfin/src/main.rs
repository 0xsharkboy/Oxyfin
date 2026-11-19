//! Oxyfin, a native Jellyfin desktop client.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use oxyfin::launch;
use oxyfin::locale::select_language;
use oxyfin::{App, AppWindow};
use oxyfin_core::{MemoryStore, Paths, TokenVault};
use slint::platform::WindowEvent;
use slint::{BackendSelector, ComponentHandle, LogicalSize, PhysicalSize};

/// How long the exit waits for the sessions still shutting down.
const EXIT_GRACE: Duration = Duration::from_secs(3);

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("oxyfin failed to start: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
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

    let paths = Paths::from_system()?;
    let vault = if launch::enabled(std::env::var(launch::MEMORY_TOKENS_VAR).ok().as_deref()) {
        log::info!("tokens are kept in memory only");
        TokenVault::new(
            Arc::new(MemoryStore::default()),
            Arc::new(MemoryStore::default()),
        )
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
    app.start_mpris();
    window.show()?;
    slint::run_event_loop_until_quit()?;
    if let Err(error) = window.hide() {
        log::warn!("cannot hide the window: {error}");
    }

    let pending = app.take_pending_ends();
    runtime.block_on(async {
        let deadline = tokio::time::Instant::now() + EXIT_GRACE;
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
