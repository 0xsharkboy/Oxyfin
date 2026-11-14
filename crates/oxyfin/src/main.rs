//! Oxyfin, a native Jellyfin desktop client.

use std::process::ExitCode;

use oxyfin::AppWindow;
use slint::{BackendSelector, CloseRequestResponse, ComponentHandle};

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

    let _runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let window = AppWindow::new()?;
    window.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        CloseRequestResponse::HideWindow
    });
    window.show()?;
    slint::run_event_loop_until_quit()?;
    Ok(())
}
