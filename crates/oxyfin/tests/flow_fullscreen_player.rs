//! Leaving the player restores the fullscreen state the window had when the
//! player opened.

use std::rc::Rc;

use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize};

#[allow(clippy::unwrap_used)]
fn player_window(fullscreen: bool) -> AppWindow {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(1280., 720.));
    window.window().set_fullscreen(fullscreen);
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    player::bind(&window, Rc::new(|_| {}));
    player::reset(&window);
    player::enter(&window);
    window.set_screen(Screen::Player);
    window
}

fn press(window: &AppWindow, text: &str) {
    for event in [
        WindowEvent::KeyPressed { text: text.into() },
        WindowEvent::KeyReleased { text: text.into() },
    ] {
        window.window().dispatch_event(event);
    }
}

#[test]
fn leaving_the_player_drops_the_fullscreen_it_turned_on() {
    let window = player_window(false);
    press(&window, "f");
    assert!(window.window().is_fullscreen());
    player::leave(&window);
    assert!(!window.window().is_fullscreen());
}

#[test]
fn leaving_the_player_keeps_the_fullscreen_of_the_launch() {
    let window = player_window(true);
    player::leave(&window);
    assert!(window.window().is_fullscreen());

    player::enter(&window);
    press(&window, "f");
    assert!(!window.window().is_fullscreen());
    player::leave(&window);
    assert!(window.window().is_fullscreen());
}
