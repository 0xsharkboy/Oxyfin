//! The quality choices of the player settings menu send their command and
//! show which one is chosen.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::click;
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{PlaybackCommand, QUALITIES, Quality};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize};

fn find(window: &AppWindow, label: &str, role: AccessibleRole) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
}

fn option(window: &AppWindow, label: &str) -> ElementHandle {
    find(window, label, AccessibleRole::ListItem)
        .unwrap_or_else(|| panic!("no menu option labelled {label}"))
}

fn selected(window: &AppWindow, label: &str) -> bool {
    option(window, label)
        .accessible_item_selected()
        .unwrap_or(false)
}

fn open_menu(window: &AppWindow) {
    let gear = find(window, "Playback settings", AccessibleRole::Button)
        .unwrap_or_else(|| panic!("no playback settings button"));
    click(window, &gear);
}

#[test]
fn choosing_a_quality_sends_it_and_shows_it_chosen() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(1280., 900.));
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
    player::show_quality(&window, Quality::Original);
    window.set_player_loading(false);
    window.set_screen(Screen::Player);

    open_menu(&window);
    assert!(selected(&window, "Original"));
    assert!(!selected(&window, "720p 4 Mbps"));
    assert!(find(&window, "480p 720 kbps", AccessibleRole::ListItem).is_some());
    click(&window, &option(&window, "720p 4 Mbps"));

    assert_eq!(
        *sent.borrow(),
        vec![PlaybackCommand::SetQuality(Quality::Limited {
            bitrate: 4_000_000,
            height: 720,
        })]
    );
    assert!(
        find(&window, "720p 4 Mbps", AccessibleRole::ListItem).is_none(),
        "the menu stays open"
    );

    open_menu(&window);
    assert!(selected(&window, "720p 4 Mbps"));
    assert!(!selected(&window, "Original"));
    click(&window, &option(&window, "Original"));
    assert_eq!(
        sent.borrow().last(),
        Some(&PlaybackCommand::SetQuality(Quality::Original))
    );

    player::show_quality(&window, QUALITIES[4]);
    open_menu(&window);
    assert!(selected(&window, "480p 1.5 Mbps"));
    assert!(!selected(&window, "Original"));
}
