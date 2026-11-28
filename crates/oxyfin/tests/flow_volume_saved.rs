//! The volume the user sets is saved for the next start, once it rests or playback stops.

mod common;

use std::sync::{Arc, Mutex};

use common::{pause, run, start_with, wait_until};
use oxyfin::app::VOLUME_SAVE_DELAY;
use oxyfin_core::{Config, Paths};
use slint::ComponentHandle;

fn saved(paths: &Arc<Mutex<Option<Paths>>>) -> Option<Option<u8>> {
    let paths = paths.lock().ok()?.clone()?;
    Config::load_or_create(&paths)
        .ok()
        .map(|config| config.volume)
}

#[test]
fn a_volume_change_is_saved_once_it_rests() {
    let shared = Arc::new(Mutex::new(None));
    let keep = Arc::clone(&shared);
    let flow = start_with(
        |mock| async move { mock },
        move |paths, _, _| {
            if let Ok(mut slot) = keep.lock() {
                *slot = Some(paths.clone());
            }
        },
    );
    let window = flow.window.clone_strong();
    run(&flow, async move {
        if saved(&shared) != Some(None) {
            return Err("a volume is saved before any change".into());
        }
        for volume in [30.0, 35.0, 42.4] {
            window.invoke_player_command_set_volume(volume);
        }
        if saved(&shared) != Some(None) {
            return Err(format!("saved while it moved: {:?}", saved(&shared)));
        }
        wait_until("the volume to be saved", || {
            saved(&shared) == Some(Some(42))
        })
        .await?;

        window.set_player_volume(42.0);
        window.invoke_player_command_step_volume(-1);
        pause(VOLUME_SAVE_DELAY / 10).await;
        if saved(&shared) != Some(Some(42)) {
            return Err("the step is saved at once".into());
        }
        window.invoke_player_command_stop();
        if saved(&shared) != Some(Some(37)) {
            return Err(format!("stop did not save the step: {:?}", saved(&shared)));
        }
        Ok(())
    });
}
