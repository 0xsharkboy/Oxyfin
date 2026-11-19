//! A relative seek made while paused moves the seek bar and the elapsed time
//! to where the player landed, though no later position arrives.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::click;
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{PlaybackCommand, PlaybackEvent, SEEK_STEP};
use oxyfin_player::SeekTarget;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize};

fn button(window: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| {
            let size = element.size();
            element.accessible_role() == Some(AccessibleRole::Button)
                && size.width > 0.
                && size.height > 0.
        })
        .unwrap_or_else(|| panic!("no visible button labelled {label}"))
}

/// A paused player at 0:25 of a 3 minute item, and the commands it sends.
fn paused_at_25s() -> (AppWindow, Rc<RefCell<Vec<PlaybackCommand>>>) {
    i_slint_backend_testing::init_no_event_loop();
    let window =
        AppWindow::new().unwrap_or_else(|error| panic!("cannot create the window: {error}"));
    window.window().set_size(LogicalSize::new(1280., 720.));
    window
        .show()
        .unwrap_or_else(|error| panic!("cannot show the window: {error}"));
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));

    let sent = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&sent);
    player::bind(
        &window,
        Rc::new(move |command| recorder.borrow_mut().push(command)),
    );
    player::reset(&window);
    window.set_screen(Screen::Player);
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(180)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: false,
            has_next: false,
        },
    );
    player::apply(&window, PlaybackEvent::Position(Duration::from_secs(25)));
    player::apply(&window, PlaybackEvent::Paused(true));
    (window, sent)
}

#[test]
fn paused_skip_moves_the_seek_bar() {
    let (window, sent) = paused_at_25s();

    click(&window, &button(&window, "Forward 10 seconds"));
    assert_eq!(
        *sent.borrow(),
        vec![PlaybackCommand::Seek(SeekTarget::Forward(SEEK_STEP))]
    );

    player::apply(&window, PlaybackEvent::Position(Duration::from_secs(35)));
    player::apply(&window, PlaybackEvent::Seeked);
    assert_eq!(window.get_player_position(), 35.0);
    assert_eq!(window.get_player_elapsed(), "0:35");
}

#[test]
fn two_paused_skips_end_on_the_second_target() {
    let (window, sent) = paused_at_25s();

    click(&window, &button(&window, "Forward 10 seconds"));
    click(&window, &button(&window, "Forward 10 seconds"));
    assert_eq!(sent.borrow().len(), 2);

    player::apply(&window, PlaybackEvent::Position(Duration::from_secs(35)));
    player::apply(&window, PlaybackEvent::Seeked);
    player::apply(&window, PlaybackEvent::Position(Duration::from_secs(45)));
    player::apply(&window, PlaybackEvent::Seeked);
    assert_eq!(window.get_player_position(), 45.0);
    assert_eq!(window.get_player_elapsed(), "0:45");
}
