//! The player track menu is wide enough for the details of its rows.

mod common;

use std::rc::Rc;
use std::time::Duration;

use common::click;
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen, TrackRow};
use oxyfin_core::PlaybackEvent;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, ModelRc, VecModel};

#[test]
fn a_long_subtitle_row_shows_its_whole_detail() {
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
    window.set_player_subtitles(ModelRc::new(VecModel::from(vec![
        TrackRow {
            index: -1,
            ..TrackRow::default()
        },
        TrackRow {
            index: 2,
            label: "English - SUBRIP - External".into(),
            detail: "eng".into(),
            external: true,
            selected: true,
            ..TrackRow::default()
        },
    ])));

    let open = ElementHandle::find_by_accessible_label(&window, "Audio and subtitles")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .unwrap();
    click(&window, &open);

    let details: Vec<_> = ElementHandle::find_by_element_id(&window, "MenuOption::detail-text")
        .map(|element| element.size().width)
        .collect();
    assert_eq!(details.len(), 1, "{details:?}");
    // "eng · External" at the caption size is wider than 70 px; elided, it shrinks to a few dots.
    assert!(details[0] >= 70., "the detail is elided: {details:?}");
}
