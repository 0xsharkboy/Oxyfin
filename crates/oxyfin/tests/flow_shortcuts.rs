//! The `?` key opens the keyboard shortcuts dialog from home and from the player; Escape and a
//! click outside close it, keys never reach the screen behind it, and `?` typed in a text field
//! stays text (sign-in, search, Quick Connect code).

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::{click, click_at};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, CardData, HomeRow, HomeRowKind, Screen, Shortcuts};
use oxyfin_core::PlaybackCommand;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, VecModel};

fn press(window: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn is_open(window: &AppWindow) -> bool {
    window.global::<Shortcuts>().get_open()
}

fn dialog(window: &AppWindow) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, "Keyboard shortcuts")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Groupbox))
}

fn input(window: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::TextInput))
        .unwrap_or_else(|| panic!("no text input labelled {label}"))
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

fn outside() -> LogicalPosition {
    LogicalPosition::new(8., 712.)
}

#[test]
fn question_mark_opens_the_shortcuts_and_escape_or_a_click_outside_closes_them() {
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
    let backs = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&backs);
    window.on_back(move || *counter.borrow_mut() += 1);

    window.set_home_rows(rows());
    window.set_home_settled(true);
    window.set_screen(Screen::Home);
    assert!(dialog(&window).is_none());

    press(&window, "?");
    assert!(is_open(&window));
    let shown = dialog(&window).expect("the dialog shows");
    let size = shown.size();
    assert!(size.width > 0. && size.height > 0.);
    press(&window, Key::Escape);
    assert!(!is_open(&window));
    assert!(dialog(&window).is_none());

    press(&window, "?");
    assert!(is_open(&window));
    click(&window, &dialog(&window).expect("the dialog shows"));
    assert!(is_open(&window), "a click inside keeps the dialog");
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(600));
    click_at(&window, outside());
    assert!(!is_open(&window));

    press(&window, "?");
    window.set_screen(Screen::Movie);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(16));
    assert!(!is_open(&window), "another screen closes the dialog");
    press(&window, "?");
    assert!(is_open(&window));
    press(&window, Key::Escape);
    assert!(!is_open(&window));
    assert_eq!(
        *backs.borrow(),
        0,
        "Escape closes the dialog without going back"
    );

    player::reset(&window);
    window.set_player_loading(false);
    window.set_screen(Screen::Player);
    press(&window, "?");
    assert!(is_open(&window));
    press(&window, " ");
    press(&window, "f");
    press(&window, Key::Escape);
    assert!(!is_open(&window));
    assert!(sent.borrow().is_empty(), "keys stay with the dialog");
    assert!(!window.window().is_fullscreen());
    press(&window, " ");
    assert_eq!(*sent.borrow(), vec![PlaybackCommand::SetPaused(true)]);
    sent.borrow_mut().clear();

    press(&window, "?");
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(600));
    click_at(&window, outside());
    assert!(!is_open(&window));
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(600));
    assert!(sent.borrow().is_empty(), "the click outside does not pause");
    assert!(!window.window().is_fullscreen());
    press(&window, " ");
    assert_eq!(
        *sent.borrow(),
        vec![PlaybackCommand::SetPaused(true)],
        "the player keeps the keys after a click outside"
    );

    window.set_screen(Screen::SignIn);
    let password = input(&window, "Password");
    click(&window, &password);
    press(&window, "?");
    assert_eq!(password.accessible_value().as_deref(), Some("?"));
    assert!(!is_open(&window));

    window.set_screen(Screen::Search);
    let search = input(&window, "Search titles");
    click(&window, &search);
    press(&window, "?");
    assert_eq!(search.accessible_value().as_deref(), Some("?"));
    assert!(!is_open(&window));

    // Tall enough for the Devices section, below the fold at 720 px.
    window.window().set_size(LogicalSize::new(1280., 2000.));
    window.set_screen(Screen::Settings);
    let code = input(&window, "Quick Connect code");
    click(&window, &code);
    press(&window, "?");
    assert_eq!(code.accessible_value().as_deref(), Some("?"));
    assert!(!is_open(&window));
}
