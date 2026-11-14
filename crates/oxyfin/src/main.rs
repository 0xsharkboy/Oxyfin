//! Oxyfin, a native Jellyfin desktop client.

use std::process::ExitCode;
use std::time::Duration;

use oxyfin::{App, AppWindow};
use oxyfin_core::{Paths, TokenVault};
use slint::{BackendSelector, ComponentHandle};

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
    BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg".into())
        .require_opengl()
        .select()?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let paths = Paths::from_system()?;
    let vault = TokenVault::system(&paths);
    let window = AppWindow::new()?;
    let app = App::new(&window, runtime.handle().clone(), paths, vault)?;
    #[cfg(feature = "mpv")]
    oxyfin::video::install(&window, app.video_source(), runtime.handle().clone())?;
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
