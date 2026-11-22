//! Every key the shortcuts dialog lists does what it says in the real window: each player key
//! sends a command or toggles fullscreen, and each key listed for everywhere has its effect.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use oxyfin::screens::player;
use oxyfin::{
    AppWindow, CardData, ContextMenu, HomeRow, HomeRowKind, Screen, ShortcutKey, Shortcuts,
    SkipKind,
};
use oxyfin_core::{Chapter, PlaybackEvent, SEEK_STEP};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model, ModelRc, SharedString, VecModel};

fn press(window: &AppWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// Presses `key` with its modifiers held.
fn chord(window: &AppWindow, key: &ShortcutKey) {
    let mut held: Vec<SharedString> = Vec::new();
    if key.control {
        held.push(Key::Control.into());
    }
    if key.shift {
        held.push(Key::Shift.into());
    }
    for modifier in &held {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: modifier.clone(),
        });
    }
    press(window, key.text.clone());
    for modifier in held.iter().rev() {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: modifier.clone(),
        });
    }
}

fn rows() -> ModelRc<HomeRow> {
    let cards: Vec<CardData> = (0..3)
        .map(|index| CardData {
            id: format!("id{index}").into(),
            title: format!("Movie {index}").into(),
            ..CardData::default()
        })
        .collect();
    ModelRc::new(VecModel::from(vec![HomeRow {
        title: "Recently Added".into(),
        kind: HomeRowKind::Poster,
        items: ModelRc::new(VecModel::from(cards)),
        ..HomeRow::default()
    }]))
}

fn chapters() -> Vec<Chapter> {
    [0, 60, 120]
        .into_iter()
        .map(|seconds| Chapter {
            start: Duration::from_secs(seconds),
            name: String::new(),
            image: None,
        })
        .collect()
}

/// Starts home again and tabs to the first card: true once Return on the focus opens it.
fn focus_first_card(window: &AppWindow, opened: &RefCell<Vec<String>>) -> bool {
    window.set_screen(Screen::Movie);
    window.set_screen(Screen::Home);
    for _ in 0..40 {
        press(window, Key::Tab);
        opened.borrow_mut().clear();
        press(window, Key::Return);
        if opened.borrow().last().map(String::as_str) == Some("id0") {
            return true;
        }
    }
    false
}

fn last_opened(window: &AppWindow, opened: &RefCell<Vec<String>>) -> Option<String> {
    opened.borrow_mut().clear();
    press(window, Key::Return);
    opened.borrow().last().cloned()
}

#[test]
fn every_listed_shortcut_has_an_effect() {
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
    let opened = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&opened);
    window.on_open_item(move |id| recorder.borrow_mut().push(id.to_string()));
    let backs = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&backs);
    window.on_back(move || *counter.borrow_mut() += 1);

    let shortcuts = window.global::<Shortcuts>();
    assert_eq!(SEEK_STEP, Duration::from_secs(10), "the list says 10 s");

    player::reset(&window);
    window.set_screen(Screen::Player);
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(180)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: true,
            has_next: true,
        },
    );
    player::apply(&window, PlaybackEvent::Chapters(chapters()));
    window.set_player_loading(false);
    window.set_player_skip_kind(SkipKind::Intro);
    window.set_player_position(90.0);
    let listed = shortcuts.get_player();
    assert!(listed.row_count() > 0);
    for shortcut in listed.iter() {
        for key in shortcut.keys.iter() {
            sent.borrow_mut().clear();
            let fullscreen = window.window().is_fullscreen();
            chord(&window, &key);
            assert!(
                !sent.borrow().is_empty() || window.window().is_fullscreen() != fullscreen,
                "player key {:?} of {} does nothing",
                key.text,
                shortcut.label
            );
        }
    }
    if window.window().is_fullscreen() {
        window.window().set_fullscreen(false);
    }

    window.set_home_rows(rows());
    window.set_home_settled(true);
    for shortcut in shortcuts.get_everywhere().iter() {
        for key in shortcut.keys.iter() {
            let text = key.text.clone();
            let what = format!("{text:?} of {}", shortcut.label);
            if text == SharedString::from(Key::F11) {
                window.set_screen(Screen::Home);
                chord(&window, &key);
                assert!(window.window().is_fullscreen(), "{what}");
                chord(&window, &key);
                assert!(!window.window().is_fullscreen(), "{what}");
            } else if text == SharedString::from(Key::Escape) {
                window.set_screen(Screen::Movie);
                *backs.borrow_mut() = 0;
                chord(&window, &key);
                assert_eq!(*backs.borrow(), 1, "{what}");
            } else if text == SharedString::from(Key::Tab)
                || text == SharedString::from(Key::Return)
            {
                assert!(focus_first_card(&window, &opened), "{what}");
            } else if text == SharedString::from(Key::RightArrow) {
                assert!(focus_first_card(&window, &opened), "{what}");
                chord(&window, &key);
                assert_eq!(
                    last_opened(&window, &opened).as_deref(),
                    Some("id1"),
                    "{what}"
                );
            } else if text == SharedString::from(Key::LeftArrow) {
                assert!(focus_first_card(&window, &opened), "{what}");
                press(&window, Key::RightArrow);
                chord(&window, &key);
                assert_eq!(
                    last_opened(&window, &opened).as_deref(),
                    Some("id0"),
                    "{what}"
                );
            } else if text == SharedString::from(Key::F10) && key.shift {
                assert!(focus_first_card(&window, &opened), "{what}");
                chord(&window, &key);
                let menu = window.global::<ContextMenu>();
                assert!(menu.get_open(), "{what}");
                assert_eq!(menu.get_id().as_str(), "id0", "{what}");
                press(&window, Key::Escape);
                assert!(!menu.get_open());
            } else if text == "?" {
                window.set_screen(Screen::Home);
                chord(&window, &key);
                assert!(shortcuts.get_open(), "{what}");
                press(&window, Key::Escape);
                assert!(!shortcuts.get_open());
            } else {
                panic!("no check for the listed key {what}");
            }
        }
    }
}
