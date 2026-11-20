//! The skip button follows the offered segment, stays up while the controls
//! hide, and seeks to the segment end on a click or the S key.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::{click, exists};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, NextUp, Screen};
use oxyfin_core::{PlaybackCommand, PlaybackEvent, SegmentKind, SkippableSegment};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize};

const WIDTH: f32 = 1280.;
const HEIGHT: f32 = 720.;

fn press(window: &AppWindow, text: &str) {
    let text: slint::SharedString = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

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

fn segment(kind: SegmentKind, start: u64, end: u64) -> PlaybackEvent {
    PlaybackEvent::Segment(Some(SkippableSegment {
        kind,
        start: Duration::from_secs(start),
        end: Duration::from_secs(end),
    }))
}

/// Asserts `element` lies inside the window, in its bottom right quarter.
fn assert_bottom_right(element: &ElementHandle) {
    let position = element.absolute_position();
    let size = element.size();
    assert!(
        position.x > WIDTH / 2. && position.y > HEIGHT / 2.,
        "{position:?}"
    );
    assert!(position.x + size.width <= WIDTH, "{position:?} {size:?}");
    assert!(position.y + size.height <= HEIGHT, "{position:?} {size:?}");
}

#[test]
fn skip_button_seeks_to_the_end_of_the_segment() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(WIDTH, HEIGHT));
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
    window.set_screen(Screen::Player);
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(1500)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: true,
            has_next: true,
        },
    );
    player::apply(&window, PlaybackEvent::Position(Duration::from_secs(10)));
    assert!(!exists(&window, "Skip intro"));

    player::apply(&window, PlaybackEvent::Position(Duration::from_secs(30)));
    player::apply(&window, segment(SegmentKind::Intro, 20, 90));
    let skip = button(&window, "Skip intro");
    assert_bottom_right(&skip);
    let pause = button(&window, "Pause");
    assert!(
        skip.absolute_position().y + skip.size().height <= pause.absolute_position().y,
        "the skip button overlaps the controls"
    );

    let above_controls = skip.absolute_position().y;
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(3100));
    let skip = button(&window, "Skip intro");
    assert!(
        skip.absolute_position().y > above_controls,
        "the button should move down as the controls hide"
    );
    assert_bottom_right(&skip);

    click(&window, &skip);
    assert_eq!(*sent.borrow(), vec![PlaybackCommand::SkipSegment]);
    assert_eq!(window.get_player_position(), 90.0);
    sent.borrow_mut().clear();

    player::apply(&window, PlaybackEvent::Seeked);
    player::apply(&window, PlaybackEvent::Position(Duration::from_secs(90)));
    player::apply(&window, PlaybackEvent::Segment(None));
    assert!(!exists(&window, "Skip intro"));
    press(&window, "s");
    assert!(
        sent.borrow().is_empty(),
        "S skips nothing without a segment"
    );

    player::apply(&window, segment(SegmentKind::Credits, 1400, 1500));
    button(&window, "Skip credits");
    press(&window, "s");
    assert_eq!(*sent.borrow(), vec![PlaybackCommand::SkipSegment]);
    sent.borrow_mut().clear();

    window.set_player_next_up(NextUp {
        visible: true,
        title: "S01E02".into(),
        remaining: 5,
        ..NextUp::default()
    });
    assert!(
        !exists(&window, "Skip credits"),
        "the toast takes the place"
    );
    window.set_player_next_up(NextUp::default());
    button(&window, "Skip credits");

    player::apply(&window, PlaybackEvent::Stopped);
    assert!(!exists(&window, "Skip credits"));

    for (kind, label) in [
        (SegmentKind::Recap, "Skip recap"),
        (SegmentKind::Preview, "Skip preview"),
    ] {
        player::apply(&window, segment(kind, 0, 60));
        button(&window, label);
    }
    player::apply(
        &window,
        PlaybackEvent::Loading {
            item: "0000000000000000000000000000000b".into(),
            title: oxyfin_core::PlaybackTitle {
                title: "Other".into(),
                series_name: None,
                episode_label: None,
                year: None,
                artwork: None,
            },
        },
    );
    assert!(!exists(&window, "Skip preview"));

    if let Err(error) = slint::select_bundled_translation("fr") {
        panic!("cannot select French: {error}");
    }
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(1500)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: true,
            has_next: false,
        },
    );
    player::apply(&window, segment(SegmentKind::Credits, 1400, 1500));
    assert_bottom_right(&button(&window, "Passer le générique"));
}
