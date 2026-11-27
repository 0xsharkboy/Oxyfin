//! An item that cannot be played offers to try again, and a file the server converts instead is
//! told in the notice.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::click;
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Message, Screen};
use oxyfin_core::{PlaybackCommand, PlaybackError, PlaybackEvent, Quality};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize};

fn find(window: &AppWindow, label: &str, role: AccessibleRole) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label).find(|element| {
        let size = element.size();
        element.accessible_role() == Some(role) && size.width > 0. && size.height > 0.
    })
}

#[test]
fn a_failed_item_is_tried_again() {
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
    player::show_quality(&window, Quality::Original);
    window.set_screen(Screen::Player);

    assert_eq!(
        player::apply(&window, PlaybackEvent::Converting),
        Some(Message::Converting)
    );
    assert!(find(&window, "Retry", AccessibleRole::Button).is_none());

    player::apply(
        &window,
        PlaybackEvent::Error(PlaybackError::LoadFailed {
            detail: String::new(),
        }),
    );
    let text = find(
        &window,
        "This item could not be loaded.",
        AccessibleRole::Text,
    )
    .expect("no error");
    let retry = find(&window, "Retry", AccessibleRole::Button).expect("no retry button");
    let (text_at, retry_at) = (text.absolute_position(), retry.absolute_position());
    assert!(
        retry_at.y >= text_at.y + text.size().height,
        "the button overlaps the error"
    );
    let middle = retry_at.x + retry.size().width / 2.;
    assert!((middle - 640.).abs() < 1., "{retry_at:?}");
    let before = sent.borrow().len();
    click(&window, &retry);
    assert_eq!(sent.borrow()[before..], [PlaybackCommand::Retry]);

    // The qualities stay within reach of an error, the chosen one included.
    let gear = find(&window, "Playback settings", AccessibleRole::Button).expect("no settings");
    click(&window, &gear);
    let original = find(&window, "Original", AccessibleRole::ListItem).expect("no quality");
    click(&window, &original);
    assert_eq!(
        sent.borrow().last(),
        Some(&PlaybackCommand::SetQuality(Quality::Original))
    );
}
