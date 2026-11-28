//! The choice between the total duration and the time left is saved and
//! comes back at the next start.

mod common;

use std::sync::{Arc, Mutex};

use common::{click, run, start_with, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::Screen;
use oxyfin_core::{Config, Paths};
use slint::{ComponentHandle, LogicalSize};

fn saved(paths: &Arc<Mutex<Option<Paths>>>) -> Option<bool> {
    let paths = paths.lock().ok()?.clone()?;
    Config::load_or_create(&paths)
        .ok()
        .map(|config| config.time_remaining)
}

#[test]
fn the_time_display_is_saved_and_restored() {
    let shared = Arc::new(Mutex::new(None));
    let keep = Arc::clone(&shared);
    let flow = start_with(
        |mock| async move { mock },
        move |paths, _, _| {
            let mut config = Config::load_or_create(paths).unwrap_or_else(|err| panic!("{err}"));
            config.time_remaining = true;
            config.save(paths).unwrap_or_else(|err| panic!("{err}"));
            if let Ok(mut slot) = keep.lock() {
                *slot = Some(paths.clone());
            }
        },
    );
    flow.window.window().set_size(LogicalSize::new(1280., 720.));
    let window = flow.window.clone_strong();
    run(&flow, async move {
        if !window.get_player_time_remaining() {
            return Err("the saved choice is not shown".into());
        }
        window.set_screen(Screen::Player);
        let time = ElementHandle::find_by_accessible_label(&window, "Time display")
            .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
            .ok_or("no time button")?;
        click(&window, &time);
        wait_until("the choice to be saved", || saved(&shared) == Some(false)).await
    });
}
