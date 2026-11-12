//! Throwaway prototype: mpv renders video underneath a Slint scene.

use std::path::PathBuf;
use std::process::ExitCode;

slint::include_modules!();

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: video-underlay <video-file>");
        return ExitCode::from(2);
    };
    if !path.is_file() {
        eprintln!("file not found: {}", path.display());
        return ExitCode::from(2);
    }

    match run(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg".into())
        .require_opengl()
        .select()?;

    let window = MainWindow::new()?;

    let weak = window.as_weak();
    window.on_toggle_fullscreen(move || {
        if let Some(window) = weak.upgrade() {
            let w = window.window();
            w.set_fullscreen(!w.is_fullscreen());
        }
    });

    window.run()?;
    Ok(())
}
