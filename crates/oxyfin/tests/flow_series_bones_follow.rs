//! Once the header bones of a series showed, the episode bones take over at once; after a fast
//! header they still wait out the skeleton delay.

mod common;

use std::time::Duration;

use common::skeleton_shown;
use oxyfin::{AppWindow, Screen};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize};

fn elapse(milliseconds: u64) {
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(milliseconds));
}

#[test]
fn episode_bones_follow_the_header_bones_without_a_gap() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(1280., 720.));
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    window.set_screen(Screen::Series);

    window.set_series_loading(true);
    elapse(0);
    elapse(200);
    assert!(
        skeleton_shown(&window),
        "the header bones show after the delay"
    );
    window.set_series_loading(false);
    window.set_series_loaded(true);
    window.set_series_episodes_loading(true);
    elapse(0);
    assert!(skeleton_shown(&window), "the episode bones show at once");
    window.set_series_episodes_loading(false);
    elapse(0);
    assert!(!skeleton_shown(&window));

    window.set_series_episodes_loading(true);
    elapse(0);
    assert!(
        !skeleton_shown(&window),
        "another season waits out the delay"
    );
    elapse(200);
    assert!(skeleton_shown(&window));
    window.set_series_episodes_loading(false);
    elapse(0);

    window.set_series_loaded(false);
    window.set_series_loading(true);
    elapse(0);
    elapse(50);
    window.set_series_loading(false);
    window.set_series_loaded(true);
    window.set_series_episodes_loading(true);
    elapse(0);
    assert!(
        !skeleton_shown(&window),
        "after a fast header, the episodes wait out the delay"
    );
}
