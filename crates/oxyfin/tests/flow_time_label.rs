//! A click on the time of the player bar switches between the total
//! duration and the time left.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

mod common;

use common::click;
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::PlaybackEvent;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize};

fn time(window: &AppWindow) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, "Time display")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no time button"))
}

fn shown(window: &AppWindow) -> String {
    time(window)
        .accessible_value()
        .map(|value| value.to_string())
        .unwrap_or_default()
}

#[test]
#[allow(clippy::unwrap_used)]
fn the_time_switches_to_the_time_left_and_back() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(1280., 720.));
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    player::bind(&window, Rc::new(|_| {}));
    player::reset(&window);
    window.set_screen(Screen::Player);
    let changes = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&changes);
    window.on_player_time_remaining_changed(move |remaining| recorder.borrow_mut().push(remaining));
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_millis(1_725_900)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: false,
            has_next: false,
        },
    );
    player::apply(
        &window,
        PlaybackEvent::Position(Duration::from_millis(1_122_400)),
    );
    assert_eq!(shown(&window), "18:42 / 28:45");

    click(&window, &time(&window));
    assert_eq!(shown(&window), "18:42 / \u{2212}10:03");
    assert!(window.get_player_time_remaining());
    player::apply(
        &window,
        PlaybackEvent::Position(Duration::from_millis(1_130_000)),
    );
    assert_eq!(
        shown(&window),
        "18:50 / \u{2212}9:55",
        "it follows the position"
    );

    click(&window, &time(&window));
    assert_eq!(shown(&window), "18:50 / 28:45");
    assert_eq!(*changes.borrow(), [true, false]);
}
