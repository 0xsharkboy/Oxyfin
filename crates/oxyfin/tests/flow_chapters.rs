//! Chapters: the seek bar gaps, the chapters menu and the mpv chapter keys.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::click;
use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementQuery};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{Chapter, PlaybackCommand, PlaybackEvent};
use oxyfin_player::SeekTarget;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model};

fn press(window: &AppWindow, key: Key) {
    let text = slint::SharedString::from(key);
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn chapters() -> Vec<Chapter> {
    [(0, "Opening"), (60, ""), (120, "Credits")]
        .into_iter()
        .map(|(seconds, name)| Chapter {
            start: Duration::from_secs(seconds),
            name: name.to_owned(),
            image: None,
        })
        .collect()
}

#[allow(clippy::unwrap_used)]
fn player_window() -> (AppWindow, Rc<RefCell<Vec<PlaybackCommand>>>) {
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

fn chapters_button(window: &AppWindow) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, "Chapters")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .filter(|element| element.size().width > 0.)
}

/// The chapter rows on screen. The query can report a row several times, so
/// copies at the same place are dropped.
fn rows(window: &AppWindow) -> Vec<ElementHandle> {
    ElementQuery::from_root(window)
        .match_descendants()
        .match_accessible_role(AccessibleRole::ListItem)
        .find_all()
        .into_iter()
        .fold(Vec::new(), |mut rows: Vec<ElementHandle>, row| {
            if !rows
                .iter()
                .any(|seen| seen.absolute_position() == row.absolute_position())
            {
                rows.push(row);
            }
            rows
        })
}

fn last_seek(sent: &RefCell<Vec<PlaybackCommand>>) -> Option<PlaybackCommand> {
    sent.borrow()
        .iter()
        .rev()
        .find(|command| matches!(command, PlaybackCommand::Seek(_)))
        .cloned()
}

fn at(window: &AppWindow, seconds: u64) {
    player::apply(
        window,
        PlaybackEvent::Position(Duration::from_secs(seconds)),
    );
    player::apply(window, PlaybackEvent::Seeked);
}

fn absolute(seconds: u64) -> Option<PlaybackCommand> {
    Some(PlaybackCommand::Seek(SeekTarget::Absolute(
        Duration::from_secs(seconds),
    )))
}

#[test]
fn chapters_button_hidden_without_chapters() {
    let (window, _) = player_window();
    assert!(chapters_button(&window).is_none());

    player::apply(&window, PlaybackEvent::Chapters(Vec::new()));
    assert!(chapters_button(&window).is_none());

    player::apply(&window, PlaybackEvent::Chapters(chapters()));
    let button = chapters_button(&window).expect("a chapters button");
    let tracks = ElementHandle::find_by_accessible_label(&window, "Audio and subtitles")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .unwrap();
    assert!(
        button.absolute_position().x < tracks.absolute_position().x,
        "the chapters button sits left of the tracks button"
    );
}

#[test]
fn chapters_menu_seeks_on_click() {
    let (window, sent) = player_window();
    player::apply(&window, PlaybackEvent::Chapters(chapters()));
    at(&window, 130);

    click(&window, &chapters_button(&window).unwrap());
    let options = rows(&window);
    assert_eq!(options.len(), 3);
    let labels: Vec<_> = options
        .iter()
        .map(|row| row.accessible_label().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        labels,
        ["Opening 0:00", "Chapter 2 1:00", "Credits 2:00"],
        "an unnamed chapter is numbered"
    );
    let selected: Vec<_> = options
        .iter()
        .map(|row| row.accessible_item_selected().unwrap_or_default())
        .collect();
    assert_eq!(selected, [false, false, true]);

    click(&window, &options[1]);
    assert_eq!(last_seek(&sent), absolute(60));
    assert!(rows(&window).is_empty(), "the menu closes after a pick");
}

#[test]
fn page_down_seeks_next_chapter() {
    let (window, sent) = player_window();
    player::apply(&window, PlaybackEvent::Chapters(chapters()));

    at(&window, 70);
    press(&window, Key::PageDown);
    assert_eq!(last_seek(&sent), absolute(120));

    at(&window, 70);
    press(&window, Key::PageUp);
    assert_eq!(last_seek(&sent), absolute(60));

    at(&window, 61);
    press(&window, Key::PageUp);
    assert_eq!(last_seek(&sent), absolute(0));

    let count = sent.borrow().len();
    at(&window, 150);
    press(&window, Key::PageDown);
    assert_eq!(sent.borrow().len(), count, "no chapter after the last one");
}

#[test]
fn page_down_twice_moves_two_chapters() {
    let (window, sent) = player_window();
    player::apply(&window, PlaybackEvent::Chapters(chapters()));
    at(&window, 10);
    press(&window, Key::PageDown);
    press(&window, Key::PageDown);
    assert_eq!(last_seek(&sent), absolute(120));
}

#[test]
fn seek_bar_has_one_gap_per_inner_chapter() {
    let (window, _) = player_window();
    let gaps = || ElementHandle::find_by_element_id(&window, "Scrubber::gap").collect::<Vec<_>>();
    assert!(gaps().is_empty());

    player::apply(&window, PlaybackEvent::Chapters(chapters()));
    let gaps = gaps();
    assert_eq!(gaps.len(), 2);
    let seek = ElementHandle::find_by_accessible_label(&window, "Seek")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Slider))
        .unwrap();
    let left = seek.absolute_position().x;
    let width = seek.size().width;
    for (gap, fraction) in gaps.iter().zip([1. / 3., 2. / 3.]) {
        assert_eq!(gap.size().width, 3.);
        let center = gap.absolute_position().x + gap.size().width / 2. - left;
        assert!(
            (center - width * fraction).abs() <= 2.,
            "gap at {center} of {width}, expected {fraction}"
        );
    }

    player::apply(
        &window,
        PlaybackEvent::Loading {
            item: "other".into(),
            title: oxyfin_core::PlaybackTitle {
                title: "Other".into(),
                series_name: None,
                episode_label: None,
                artwork: None,
            },
        },
    );
    assert!(
        ElementHandle::find_by_element_id(&window, "Scrubber::gap")
            .next()
            .is_none(),
        "a new item drops the chapters"
    );
}

#[test]
fn current_chapter_follows_the_position() {
    let (window, _) = player_window();
    player::apply(&window, PlaybackEvent::Chapters(chapters()));
    at(&window, 30);
    assert_eq!(window.get_player_current_chapter(), 0);
    at(&window, 90);
    assert_eq!(window.get_player_current_chapter(), 1);
}

fn hover_seek_bar(window: &AppWindow, ratio: f32) {
    let bar = ElementHandle::find_by_accessible_label(window, "Seek")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Slider))
        .unwrap_or_else(|| panic!("no seek bar"));
    let position = slint::LogicalPosition::new(
        bar.absolute_position().x + bar.size().width * ratio,
        bar.absolute_position().y + bar.size().height / 2.,
    );
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
}

fn text(window: &AppWindow, label: &str) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Text))
}

#[test]
fn seek_preview_names_the_chapter_above_the_time() {
    let (window, _) = player_window();
    player::bind_preview(&window, Rc::new(|_| {}));
    player::apply(&window, PlaybackEvent::Chapters(chapters()));

    hover_seek_bar(&window, 0.5);
    let name = text(&window, "Chapter 2").expect("the unnamed chapter is numbered");
    let time = text(&window, "1:30").expect("the hovered time");
    assert!(name.absolute_position().y < time.absolute_position().y);

    hover_seek_bar(&window, 0.8);
    assert!(text(&window, "Credits").is_some());
}

fn at_millis(window: &AppWindow, millis: u64) {
    player::apply(
        window,
        PlaybackEvent::Position(Duration::from_millis(millis)),
    );
    player::apply(window, PlaybackEvent::Seeked);
}

#[test]
fn page_up_near_the_end_of_the_grace_goes_to_the_previous_chapter() {
    let (window, sent) = player_window();
    player::apply(&window, PlaybackEvent::Chapters(chapters()));
    at_millis(&window, 62_900);
    press(&window, Key::PageUp);
    assert_eq!(last_seek(&sent), absolute(0));
    at_millis(&window, 63_100);
    press(&window, Key::PageUp);
    assert_eq!(last_seek(&sent), absolute(60));
}

#[test]
fn chapter_keys_seek_to_the_exact_start_of_the_menu_row() {
    let (window, sent) = player_window();
    let start = Duration::from_millis(61_337);
    let mut list = chapters();
    list[1].start = start;
    player::apply(&window, PlaybackEvent::Chapters(list));
    at(&window, 10);
    press(&window, Key::PageDown);
    let row = window.get_player_chapters().row_data(1).unwrap();
    assert_eq!(
        last_seek(&sent),
        Some(PlaybackCommand::Seek(SeekTarget::Absolute(
            Duration::from_secs_f32(row.start)
        )))
    );
    assert_eq!(window.get_player_current_chapter(), 1);
}

#[test]
fn marks_follow_a_later_duration() {
    let (window, _) = player_window();
    player::apply(&window, PlaybackEvent::Chapters(chapters()));
    let marks = |window: &AppWindow| {
        window
            .get_player_chapter_marks()
            .iter()
            .collect::<Vec<f32>>()
    };
    assert_eq!(marks(&window), [1. / 3., 2. / 3.]);
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(240)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: false,
            has_next: false,
        },
    );
    assert_eq!(marks(&window), [0.25, 0.5]);
}

#[test]
fn a_late_current_chapter_is_scrolled_into_the_menu() {
    let (window, _) = player_window();
    let long: Vec<_> = (0..30)
        .map(|index| Chapter {
            start: Duration::from_secs(index * 6),
            name: format!("Part {index}"),
            image: None,
        })
        .collect();
    player::apply(&window, PlaybackEvent::Chapters(long));
    at(&window, 27 * 6 + 1);
    assert_eq!(window.get_player_current_chapter(), 27);

    click(&window, &chapters_button(&window).unwrap());
    let menu = ElementHandle::find_by_accessible_label(&window, "Chapters")
        .find(|element| element.accessible_role() == Some(AccessibleRole::List))
        .unwrap();
    let (top, bottom) = (
        menu.absolute_position().y,
        menu.absolute_position().y + menu.size().height,
    );
    let current = rows(&window)
        .into_iter()
        .find(|row| row.accessible_item_selected() == Some(true))
        .unwrap_or_else(|| panic!("the current row is not shown"));
    let y = current.absolute_position().y;
    assert!(
        y >= top && y + current.size().height <= bottom,
        "row at {y}..{} outside the menu {top}..{bottom}",
        y + current.size().height
    );
    // Rows scrolled out of the menu are not reported at all.
    let first = rows(&window).into_iter().find(|row| {
        row.accessible_label()
            .is_some_and(|label| label.starts_with("Part 0 "))
    });
    assert!(
        first.is_none_or(|row| row.absolute_position().y < top),
        "the menu scrolled"
    );
}

#[test]
fn hovering_the_seek_bar_shows_a_marker_and_a_label_under_the_thumbnail() {
    let (window, _) = player_window();
    player::bind_preview(&window, Rc::new(|_| {}));
    player::apply(&window, PlaybackEvent::Chapters(chapters()));
    let markers = || ElementHandle::find_by_element_id(&window, "Scrubber::marker").count();
    assert_eq!(markers(), 0);

    hover_seek_bar(&window, 0.5);
    let marker: Vec<_> = ElementHandle::find_by_element_id(&window, "Scrubber::marker").collect();
    assert_eq!(marker.len(), 1);
    assert_eq!(marker[0].size().width, 2.);
    assert_eq!(marker[0].size().height, 12.);
    let seek = ElementHandle::find_by_accessible_label(&window, "Seek")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Slider))
        .unwrap();
    let center = marker[0].absolute_position().x + 1.;
    let expected = seek.absolute_position().x + seek.size().width * 0.5;
    assert!((center - expected).abs() <= 1., "{center} vs {expected}");

    let label = ElementHandle::find_by_element_id(&window, "SeekPreviewPopup::label")
        .next()
        .expect("a label pill");
    let name = text(&window, "Chapter 2").unwrap();
    let time = text(&window, "1:30").unwrap();
    for inside in [&name, &time] {
        let y = inside.absolute_position().y;
        assert!(y >= label.absolute_position().y);
        assert!(y + inside.size().height <= label.absolute_position().y + label.size().height);
    }
}
