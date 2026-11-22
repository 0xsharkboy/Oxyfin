//! Clicks on the video: one toggles pause with a brief indicator, two toggle
//! fullscreen without pausing, and one with a menu open only closes it.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::click_at;
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{PlaybackCommand, PlaybackEvent};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, LogicalSize};

type Sent = Rc<RefCell<Vec<PlaybackCommand>>>;

/// Longer than the double-click interval of Slint (500 ms).
const AFTER_DOUBLE_CLICK: Duration = Duration::from_millis(600);

const VIDEO: LogicalPosition = LogicalPosition::new(640., 300.);

#[allow(clippy::unwrap_used)]
fn player_window() -> (AppWindow, Sent) {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(1280., 720.));
    window.show().unwrap();
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
    player::enter(&window);
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
    (window, sent)
}

/// Lets `duration` pass in small steps, so that a timer started by another one runs too.
fn elapse(duration: Duration) {
    let step = Duration::from_millis(50);
    let mut left = duration;
    while !left.is_zero() {
        let next = left.min(step);
        i_slint_backend_testing::mock_elapsed_time(next);
        left -= next;
    }
}

fn pauses(sent: &Sent) -> Vec<bool> {
    sent.borrow()
        .iter()
        .filter_map(|command| match command {
            PlaybackCommand::SetPaused(paused) => Some(*paused),
            _ => None,
        })
        .collect()
}

fn indicator(window: &AppWindow, label: &str) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label).find(|element| {
        element.accessible_role() == Some(AccessibleRole::Image)
            && element.computed_opacity() > 0.
            && element.size().width > 0.
    })
}

fn press(window: &AppWindow, text: &str) {
    let text: slint::SharedString = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn a_click_on_the_video_toggles_pause_with_an_indicator() {
    let (window, sent) = player_window();
    click_at(&window, VIDEO);
    assert!(
        pauses(&sent).is_empty(),
        "waits for a possible double click"
    );
    elapse(AFTER_DOUBLE_CLICK);
    assert_eq!(pauses(&sent), [true]);

    let shown = indicator(&window, "Paused").unwrap_or_else(|| panic!("no pause indicator"));
    assert_eq!(shown.size().width, 88.);
    assert_eq!(shown.size().height, 88.);
    let position = shown.absolute_position();
    assert_eq!(position.x + 44., 640., "centered");
    assert_eq!(position.y + 44., 360., "centered");
    elapse(Duration::from_secs(1));
    assert!(indicator(&window, "Paused").is_none(), "it fades out");

    player::apply(&window, PlaybackEvent::Paused(true));
    click_at(&window, VIDEO);
    elapse(AFTER_DOUBLE_CLICK);
    assert_eq!(pauses(&sent), [true, false]);
    assert!(indicator(&window, "Playing").is_some());
}

#[test]
fn a_double_click_toggles_fullscreen_without_pausing() {
    let (window, sent) = player_window();
    click_at(&window, VIDEO);
    elapse(Duration::from_millis(100));
    click_at(&window, VIDEO);
    assert!(window.window().is_fullscreen());
    elapse(AFTER_DOUBLE_CLICK);
    assert!(pauses(&sent).is_empty(), "{:?}", sent.borrow());
    assert!(indicator(&window, "Paused").is_none());
}

#[test]
fn a_click_with_a_menu_open_only_closes_it() {
    let (window, sent) = player_window();
    press(&window, "i");
    assert!(window.get_player_stats_visible());
    click_at(&window, VIDEO);
    elapse(AFTER_DOUBLE_CLICK);
    assert!(!window.get_player_stats_visible());
    assert!(pauses(&sent).is_empty(), "{:?}", sent.borrow());
}

#[test]
fn clicks_on_the_bars_do_not_pause() {
    let (window, sent) = player_window();
    // Between the time and the volume, and right of the title.
    for point in [
        LogicalPosition::new(640., 700.),
        LogicalPosition::new(640., 30.),
    ] {
        click_at(&window, point);
        elapse(AFTER_DOUBLE_CLICK);
    }
    assert!(pauses(&sent).is_empty(), "{:?}", sent.borrow());
}
