//! The wheel over the video changes the volume by the volume step and shows it.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{PlaybackCommand, PlaybackEvent};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, LogicalSize};

type Sent = Rc<RefCell<Vec<PlaybackCommand>>>;

const VIDEO: LogicalPosition = LogicalPosition::new(640., 300.);

/// What one notch of a mouse wheel scrolls in Slint.
const NOTCH: f32 = 60.;

#[allow(clippy::unwrap_used)]
fn player_window() -> (AppWindow, Sent) {
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
    player::apply(&window, PlaybackEvent::Volume(50.0));
    (window, sent)
}

fn scroll(window: &AppWindow, position: LogicalPosition, delta_y: f32) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.,
            delta_y,
        });
}

fn volumes(sent: &Sent) -> Vec<f64> {
    sent.borrow()
        .iter()
        .filter_map(|command| match command {
            PlaybackCommand::SetVolume(volume) => Some(*volume),
            _ => None,
        })
        .collect()
}

fn shown(window: &AppWindow, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, label)
        .any(|element| element.accessible_role() == Some(AccessibleRole::Text))
}

fn press(window: &AppWindow, text: &str) {
    let text: slint::SharedString = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn each_notch_steps_the_volume_and_shows_it() {
    let (window, sent) = player_window();
    scroll(&window, VIDEO, NOTCH);
    scroll(&window, VIDEO, NOTCH);
    assert_eq!(
        volumes(&sent),
        [55.0, 60.0],
        "the second step starts from the first"
    );
    assert!(shown(&window, "Volume: 60%"));

    scroll(&window, VIDEO, -NOTCH);
    assert_eq!(volumes(&sent), [55.0, 60.0, 55.0]);
}

#[test]
fn small_touchpad_moves_add_up_to_one_step() {
    let (window, sent) = player_window();
    for _ in 0..2 {
        scroll(&window, VIDEO, NOTCH / 3.);
    }
    assert!(volumes(&sent).is_empty(), "{:?}", sent.borrow());
    scroll(&window, VIDEO, NOTCH / 3.);
    assert_eq!(volumes(&sent), [55.0]);
}

#[test]
fn the_wheel_over_a_menu_or_the_bar_leaves_the_volume() {
    let (window, sent) = player_window();
    press(&window, "i");
    scroll(&window, VIDEO, NOTCH);
    press(&window, "i");
    scroll(&window, LogicalPosition::new(640., 700.), NOTCH);
    assert!(volumes(&sent).is_empty(), "{:?}", sent.borrow());
}
