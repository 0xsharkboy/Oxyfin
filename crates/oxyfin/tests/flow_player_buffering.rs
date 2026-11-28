//! The player shows its spinner only once it has waited for data a little while.

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementQuery};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{PlaybackEvent, Quality};
use slint::{ComponentHandle, LogicalSize};

#[allow(missing_docs)]
mod theme {
    slint::slint! {
        import { Theme } from "ui/theme.slint";

        export component Delays inherits Window {
            out property <duration> buffering: Theme.buffering-delay;
        }
    }
}

fn spinner_shown(window: &AppWindow) -> bool {
    ElementQuery::from_root(window)
        .match_accessible_role(AccessibleRole::ProgressIndicator)
        .find_all()
        .into_iter()
        .any(|element| element.size().width > 0. && element.size().height > 0.)
}

/// Lets `time` pass frame by frame, as the event loop does.
fn advance(time: Duration) {
    let frame = Duration::from_millis(16);
    let mut passed = Duration::ZERO;
    while passed < time {
        i_slint_backend_testing::mock_elapsed_time(frame);
        passed += frame;
    }
}

#[test]
fn the_spinner_waits_before_showing() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(1280., 720.));
    window.show().unwrap();
    player::bind(&window, std::rc::Rc::new(|_| {}));
    player::reset(&window);
    player::show_quality(&window, Quality::Original);
    window.set_screen(Screen::Player);
    window.set_player_loading(false);
    assert!(!spinner_shown(&window));

    player::apply(&window, PlaybackEvent::Buffering(true));
    let delay = Duration::from_millis(theme::Delays::new().unwrap().get_buffering().unsigned_abs());
    advance(delay - Duration::from_millis(50));
    assert!(!spinner_shown(&window), "a short wait shows nothing");
    advance(Duration::from_millis(100));
    assert!(spinner_shown(&window));

    player::apply(&window, PlaybackEvent::Buffering(false));
    assert!(!spinner_shown(&window));
    advance(Duration::from_millis(16));
    player::apply(&window, PlaybackEvent::Buffering(true));
    assert!(!spinner_shown(&window), "each wait has its own delay");
    advance(Duration::from_millis(16));
    assert!(!spinner_shown(&window), "each wait has its own delay");
    advance(delay);
    assert!(spinner_shown(&window));
    player::apply(
        &window,
        PlaybackEvent::Loading {
            item: "next".into(),
            title: oxyfin_core::PlaybackTitle {
                title: "Next".into(),
                series_name: None,
                episode_label: None,
                year: None,
                artwork: None,
            },
        },
    );
    window.set_player_loading(false);
    assert!(!spinner_shown(&window), "a new item does not wait yet");
}
