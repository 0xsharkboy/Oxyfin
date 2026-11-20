//! Fullscreen in the player: leaving the player drops only the fullscreen the
//! player turned on, Escape leaves that fullscreen after closing a menu or
//! the statistics, and F11 toggles fullscreen on every screen.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::click;
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::PlaybackCommand;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, SharedString};

type Sent = Rc<RefCell<Vec<PlaybackCommand>>>;

#[allow(clippy::unwrap_used)]
fn window(fullscreen: bool) -> AppWindow {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(1280., 900.));
    window.window().set_fullscreen(fullscreen);
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    window
}

fn player_window(fullscreen: bool) -> (AppWindow, Sent) {
    let window = window(fullscreen);
    let sent = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&sent);
    player::bind(
        &window,
        Rc::new(move |command| recorder.borrow_mut().push(command)),
    );
    player::reset(&window);
    player::enter(&window);
    window.set_screen(Screen::Player);
    (window, sent)
}

fn press(window: &AppWindow, text: impl Into<SharedString>) {
    let text = text.into();
    for event in [
        WindowEvent::KeyPressed { text: text.clone() },
        WindowEvent::KeyReleased { text: text.clone() },
    ] {
        window.window().dispatch_event(event);
    }
}

fn stopped(sent: &Sent) -> bool {
    sent.borrow().contains(&PlaybackCommand::Stop)
}

fn shows_stats(window: &AppWindow) -> bool {
    window.get_player_stats_visible()
}

fn find(window: &AppWindow, label: &str, role: AccessibleRole) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
}

#[test]
fn leaving_the_player_drops_the_fullscreen_it_turned_on() {
    let (window, _sent) = player_window(false);
    press(&window, "f");
    assert!(window.window().is_fullscreen());
    player::leave(&window);
    assert!(!window.window().is_fullscreen());
}

#[test]
fn leaving_the_player_keeps_the_fullscreen_of_the_launch() {
    let (window, _sent) = player_window(true);
    player::leave(&window);
    assert!(window.window().is_fullscreen());

    player::enter(&window);
    press(&window, "f");
    assert!(!window.window().is_fullscreen());
    player::leave(&window);
    assert!(
        !window.window().is_fullscreen(),
        "the fullscreen left in the player stays left"
    );

    player::enter(&window);
    press(&window, "f");
    assert!(window.window().is_fullscreen());
    player::leave(&window);
    assert!(
        !window.window().is_fullscreen(),
        "the player turned this one on"
    );
}

#[test]
fn escape_in_a_fullscreen_launch_closes_the_stats_then_goes_back() {
    let (window, sent) = player_window(true);
    press(&window, "i");
    assert!(shows_stats(&window));

    press(&window, Key::Escape);
    assert!(!shows_stats(&window));
    assert!(window.window().is_fullscreen());
    assert!(!stopped(&sent));

    press(&window, Key::Escape);
    assert!(stopped(&sent), "back to the previous screen");
    assert!(window.window().is_fullscreen());
}

#[test]
fn escape_closes_a_menu_and_the_stats_before_the_fullscreen_of_the_player() {
    let (window, sent) = player_window(false);
    press(&window, "f");
    press(&window, "i");
    let gear = find(&window, "Playback settings", AccessibleRole::Button)
        .unwrap_or_else(|| panic!("no settings button"));
    click(&window, &gear);
    assert!(find(&window, "Playback statistics", AccessibleRole::ListItem).is_some());

    press(&window, Key::Escape);
    assert!(
        find(&window, "Playback statistics", AccessibleRole::ListItem).is_none(),
        "the menu closes first"
    );
    assert!(shows_stats(&window));
    assert!(window.window().is_fullscreen());

    press(&window, Key::Escape);
    assert!(!shows_stats(&window));
    assert!(window.window().is_fullscreen());

    press(&window, Key::Escape);
    assert!(!window.window().is_fullscreen());
    assert!(!stopped(&sent));

    press(&window, Key::Escape);
    assert!(stopped(&sent));
}

#[test]
fn escape_never_turns_fullscreen_on() {
    let (window, sent) = player_window(false);
    press(&window, "f");
    // The desktop left fullscreen on its own.
    window.window().set_fullscreen(false);
    press(&window, Key::Escape);
    assert!(!window.window().is_fullscreen());
    assert!(stopped(&sent));
}

#[test]
fn f11_toggles_fullscreen_once_in_the_player() {
    let (window, _sent) = player_window(false);
    press(&window, Key::F11);
    assert!(window.window().is_fullscreen());
    player::leave(&window);
    assert!(
        !window.window().is_fullscreen(),
        "F11 in the player is the player's fullscreen"
    );

    player::enter(&window);
    press(&window, Key::F11);
    press(&window, Key::F11);
    assert!(!window.window().is_fullscreen());
}

#[test]
fn f11_toggles_fullscreen_on_every_screen() {
    let window = window(true);
    player::bind(&window, Rc::new(|_| {}));
    window.set_screen(Screen::Home);
    press(&window, Key::F11);
    assert!(!window.window().is_fullscreen(), "nothing focused yet");
    press(&window, Key::F11);
    assert!(window.window().is_fullscreen());

    window.set_screen(Screen::Settings);
    press(&window, Key::F11);
    assert!(!window.window().is_fullscreen(), "after a screen change");

    window.set_screen(Screen::SignIn);
    let password = find(&window, "Password", AccessibleRole::TextInput)
        .unwrap_or_else(|| panic!("no password field"));
    click(&window, &password);
    press(&window, Key::F11);
    assert!(window.window().is_fullscreen(), "from a text field");
    assert_eq!(password.accessible_value().as_deref(), Some(""));

    // The focused player shortcuts go away with the player screen.
    window.set_screen(Screen::Player);
    press(&window, "m");
    window.set_screen(Screen::Home);
    press(&window, Key::F11);
    assert!(!window.window().is_fullscreen(), "after the player");
}
