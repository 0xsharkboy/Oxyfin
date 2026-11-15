//! Player shortcuts send their commands; typing in the sign-in form sends none.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::click;
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, NextUp, Screen};
use oxyfin_core::{PlaybackCommand, SEEK_STEP};
use oxyfin_player::SeekTarget;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize};

fn press(window: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn find(window: &AppWindow, label: &str, role: AccessibleRole) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {role:?} labelled {label}"))
}

#[test]
fn player_shortcuts_send_commands_and_sign_in_typing_does_not() {
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
    find(&window, "Player", AccessibleRole::Groupbox);

    press(&window, " ");
    press(&window, Key::RightArrow);
    press(&window, "m");
    assert!(!window.window().is_fullscreen());
    press(&window, "f");
    assert!(window.window().is_fullscreen());
    assert_eq!(
        *sent.borrow(),
        vec![
            PlaybackCommand::SetPaused(true),
            PlaybackCommand::Seek(SeekTarget::Forward(SEEK_STEP)),
            PlaybackCommand::SetMuted(true),
        ]
    );

    press(&window, Key::Escape);
    assert!(!window.window().is_fullscreen());
    sent.borrow_mut().clear();

    window.set_player_loading(false);
    click(&window, &find(&window, "Pause", AccessibleRole::Button));
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(3100));
    press(&window, " ");
    window.set_player_next_up(NextUp {
        visible: true,
        title: "S01E02".into(),
        remaining: 5,
        ..NextUp::default()
    });
    click(&window, &find(&window, "Cancel", AccessibleRole::Button));
    window.set_player_next_up(NextUp::default());
    press(&window, " ");
    assert_eq!(
        *sent.borrow(),
        vec![
            PlaybackCommand::SetPaused(true),
            PlaybackCommand::SetPaused(true),
            PlaybackCommand::CancelNextUp,
            PlaybackCommand::SetPaused(true),
        ]
    );
    sent.borrow_mut().clear();

    window.set_screen(Screen::SignIn);
    let password = find(&window, "Password", AccessibleRole::TextInput);
    for _ in 0..3 {
        press(&window, Key::Tab);
    }
    press(&window, "f");
    press(&window, " ");

    assert_eq!(password.accessible_value().as_deref(), Some("f "));
    assert!(sent.borrow().is_empty());
    assert!(!window.window().is_fullscreen());
}
