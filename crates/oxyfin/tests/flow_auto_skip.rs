//! The notice of an automatic skip offers to undo it and leaves after a
//! moment; the next episode toast waits for the user in ask mode.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::{click, exists};
use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementQuery};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen, TrackRow};
use oxyfin_core::{
    Episode, PlaybackCommand, PlaybackEvent, SegmentKind, SkippableSegment, UserState,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, VecModel};

const WIDTH: f32 = 1280.;
const HEIGHT: f32 = 720.;

type Sent = Rc<RefCell<Vec<PlaybackCommand>>>;

fn visible(window: &AppWindow, label: &str, role: AccessibleRole) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label).find(|element| {
        let size = element.size();
        element.accessible_role() == Some(role) && size.width > 0. && size.height > 0.
    })
}

fn button(window: &AppWindow, label: &str) -> ElementHandle {
    visible(window, label, AccessibleRole::Button)
        .unwrap_or_else(|| panic!("no visible button labelled {label}"))
}

fn notice(window: &AppWindow, label: &str) -> Option<ElementHandle> {
    visible(window, label, AccessibleRole::Groupbox)
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

fn episode() -> Episode {
    Episode {
        id: "50d3822a34079d70485f1bf36c274c35".into(),
        series_id: "9ea196e7ccc3ae17d2c3f75c3ab34406".into(),
        series_name: Some("Series".into()),
        season_id: None,
        title: "The Return".into(),
        season_number: Some(1),
        number: Some(2),
        label: Some("S01E02".into()),
        overview: None,
        thumb: None,
        duration: None,
        user: UserState::default(),
    }
}

/// A window on the player screen, playing an episode with a next one.
fn playing() -> (AppWindow, Sent) {
    i_slint_backend_testing::init_no_event_loop();
    let window =
        AppWindow::new().unwrap_or_else(|error| panic!("cannot create the window: {error}"));
    window.window().set_size(LogicalSize::new(WIDTH, HEIGHT));
    window
        .show()
        .unwrap_or_else(|error| panic!("cannot show the window: {error}"));
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let sent: Sent = Rc::new(RefCell::new(Vec::new()));
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
    player::apply(&window, PlaybackEvent::Position(Duration::from_secs(30)));
    (window, sent)
}

/// Shows that `kind` was skipped, then lets the window react as its event loop would at once.
fn skip(window: &AppWindow, kind: SegmentKind) {
    player::apply(window, skipped(kind));
    i_slint_backend_testing::mock_elapsed_time(Duration::ZERO);
}

fn skipped(kind: SegmentKind) -> PlaybackEvent {
    PlaybackEvent::SegmentSkipped {
        kind,
        from: Duration::from_secs(20),
    }
}

#[test]
fn skip_notice_undo() {
    let (window, sent) = playing();
    assert!(notice(&window, "Intro skipped").is_none());

    skip(&window, SegmentKind::Intro);
    player::apply(&window, PlaybackEvent::Segment(None));
    let pill = notice(&window, "Intro skipped").expect("no intro notice");
    assert_bottom_right(&pill);
    let undo = button(&window, "Undo");
    let (pill_at, undo_at) = (pill.absolute_position(), undo.absolute_position());
    assert!(
        undo_at.x >= pill_at.x && undo_at.x + undo.size().width <= pill_at.x + pill.size().width,
        "Undo is not inside the notice"
    );
    let pause = button(&window, "Pause");
    assert!(
        pill_at.y + pill.size().height <= pause.absolute_position().y,
        "the notice overlaps the controls"
    );

    click(&window, &undo);
    assert_eq!(*sent.borrow(), vec![PlaybackCommand::UndoSkip]);
    assert!(notice(&window, "Intro skipped").is_none());
    sent.borrow_mut().clear();

    skip(&window, SegmentKind::Recap);
    notice(&window, "Recap skipped").expect("no recap notice");
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(4_000));
    assert!(notice(&window, "Recap skipped").is_some());
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1_500));
    assert!(
        notice(&window, "Recap skipped").is_none(),
        "the notice stays past 5 s"
    );

    skip(&window, SegmentKind::Preview);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(3_000));
    skip(&window, SegmentKind::Recap);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(3_000));
    assert!(
        notice(&window, "Recap skipped").is_some(),
        "a new skip shows for its own 5 s"
    );

    player::apply(
        &window,
        PlaybackEvent::Segment(Some(SkippableSegment {
            kind: SegmentKind::Intro,
            start: Duration::from_secs(30),
            end: Duration::from_secs(90),
        })),
    );
    let pill = notice(&window, "Recap skipped").expect("no recap notice");
    let skip = button(&window, "Skip intro");
    assert_bottom_right(&skip);
    assert_bottom_right(&pill);
    assert!(
        pill.absolute_position().y + pill.size().height <= skip.absolute_position().y,
        "the notice overlaps the skip button"
    );
    assert!(sent.borrow().is_empty(), "{:?}", sent.borrow());

    player::apply(&window, PlaybackEvent::Stopped);
    assert!(notice(&window, "Recap skipped").is_none());
}

#[test]
fn toast_ask_mode_has_no_countdown() {
    let (window, sent) = playing();
    player::apply(&window, PlaybackEvent::Ended);
    player::apply(
        &window,
        PlaybackEvent::NextUpReady {
            episode: Box::new(episode()),
        },
    );
    let toast = notice(&window, "Next episode").expect("no toast");
    assert_bottom_right(&toast);
    assert!(exists(&window, "Up next"));
    assert!(exists(&window, "S01E02 \u{b7} The Return"));
    assert!(
        visible(&window, "Countdown", AccessibleRole::ProgressIndicator).is_none(),
        "ask mode shows a countdown line"
    );
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(30));
    assert!(sent.borrow().is_empty(), "{:?}", sent.borrow());
    notice(&window, "Next episode").expect("the toast left by itself");

    click(&window, &button(&window, "Close"));
    assert_eq!(*sent.borrow(), vec![PlaybackCommand::CancelNextUp]);
    sent.borrow_mut().clear();
    click(&window, &button(&window, "Play now"));
    assert_eq!(*sent.borrow(), vec![PlaybackCommand::PlayNextNow]);

    player::apply(
        &window,
        PlaybackEvent::NextUpCountdown {
            episode: Box::new(episode()),
            remaining: 7,
            total: 10,
        },
    );
    assert!(exists(&window, "Next episode in 7 s"));
    assert!(!exists(&window, "Up next"));
    button(&window, "Cancel");
    let line = visible(&window, "Countdown", AccessibleRole::ProgressIndicator)
        .expect("no countdown line");
    assert_eq!(line.accessible_value().as_deref(), Some("7"));
    assert_eq!(line.accessible_value_maximum(), Some(10.));
    let toast = notice(&window, "Next episode").expect("no toast");
    assert_eq!(line.absolute_position(), toast.absolute_position());
    assert_eq!(line.size(), toast.size());
}

fn countdown(remaining: u32) -> PlaybackEvent {
    PlaybackEvent::NextUpCountdown {
        episode: Box::new(episode()),
        remaining,
        total: 10,
    }
}

/// Width of the countdown fill against the width of its line.
fn fill_ratio(window: &AppWindow) -> f32 {
    let line = visible(window, "Countdown", AccessibleRole::ProgressIndicator)
        .unwrap_or_else(|| panic!("no countdown line"));
    let fill = ElementHandle::find_by_element_id(window, "NextEpisodeToast::fill")
        .next()
        .unwrap_or_else(|| panic!("no countdown fill"));
    fill.size().width / line.size().width
}

#[test]
fn countdown_line_fills_by_the_last_step() {
    let (window, _sent) = playing();
    player::apply(&window, PlaybackEvent::Ended);
    player::apply(&window, countdown(10));
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1_000));
    assert!(
        (fill_ratio(&window) - 0.1).abs() < 0.01,
        "{}",
        fill_ratio(&window)
    );

    player::apply(&window, countdown(5));
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1_000));
    assert!(
        (fill_ratio(&window) - 0.6).abs() < 0.01,
        "{}",
        fill_ratio(&window)
    );

    player::apply(&window, countdown(1));
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(500));
    let halfway = fill_ratio(&window);
    assert!(halfway > 0.6 && halfway < 1., "{halfway}");
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(500));
    assert!(
        (fill_ratio(&window) - 1.).abs() < 0.01,
        "{}",
        fill_ratio(&window)
    );
}

fn press_at(window: &AppWindow, position: LogicalPosition) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    for event in [
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        window.window().dispatch_event(event);
    }
}

#[test]
fn skip_notice_stays_hidden_under_an_open_menu() {
    let (window, sent) = playing();
    window.set_player_subtitles(ModelRc::new(VecModel::from(vec![
        TrackRow {
            index: -1,
            ..TrackRow::default()
        },
        TrackRow {
            index: 2,
            label: "English".into(),
            ..TrackRow::default()
        },
    ])));
    skip(&window, SegmentKind::Recap);
    let pill = notice(&window, "Recap skipped").expect("no recap notice");
    let (pill_at, pill_size) = (pill.absolute_position(), pill.size());
    click(&window, &button(&window, "Undo"));
    assert!(notice(&window, "Recap skipped").is_none());
    sent.borrow_mut().clear();

    click(&window, &button(&window, "Playback settings"));
    skip(&window, SegmentKind::Intro);
    assert!(
        notice(&window, "Intro skipped").is_none(),
        "the notice shows over the menu"
    );
    // A menu option drawn where the notice was.
    let option = ElementQuery::from_root(&window)
        .match_descendants()
        .match_accessible_role(AccessibleRole::ListItem)
        .find_all()
        .into_iter()
        .find(|option| {
            let (at, size) = (option.absolute_position(), option.size());
            at.x < pill_at.x + pill_size.width
                && at.x + size.width > pill_at.x
                && at.y < pill_at.y + pill_size.height
                && at.y + size.height > pill_at.y
        })
        .unwrap_or_else(|| panic!("no menu option where the notice was"));
    let (at, size) = (option.absolute_position(), option.size());
    // The middle of where the option and the notice overlap.
    let (left, right) = (
        at.x.max(pill_at.x),
        (at.x + size.width).min(pill_at.x + pill_size.width),
    );
    let (top, bottom) = (
        at.y.max(pill_at.y),
        (at.y + size.height).min(pill_at.y + pill_size.height),
    );
    let spot = LogicalPosition::new((left + right) / 2., (top + bottom) / 2.);
    press_at(&window, spot);
    assert!(
        matches!(
            sent.borrow()[..],
            [PlaybackCommand::SetSpeed(_) | PlaybackCommand::SetQuality(_)]
        ),
        "{:?}",
        sent.borrow()
    );
}
