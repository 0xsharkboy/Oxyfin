//! MPRIS calls drive the player through the commands of its controls.
#![cfg(target_os = "linux")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use oxyfin::mpris::{Request, perform};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::PlaybackCommand;
use oxyfin_player::SeekTarget;
use slint::ComponentHandle;

#[test]
fn mpris_calls_send_the_player_commands() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.show().unwrap();

    let sent = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&sent);
    player::bind(
        &window,
        Rc::new(move |command| recorder.borrow_mut().push(command)),
    );
    let closed = Rc::new(Cell::new(false));
    let flag = Rc::clone(&closed);
    window.on_close_player(move || flag.set(true));
    player::reset(&window);

    window.set_screen(Screen::Home);
    perform(&window, Request::PlayPause);
    assert!(sent.borrow().is_empty(), "ignored away from the player");

    window.set_screen(Screen::Player);
    window.set_player_loading(false);
    window.set_player_duration(90.0);
    window.set_player_position(30.0);

    perform(&window, Request::PlayPause);
    perform(&window, Request::Play);
    window.set_player_paused(true);
    perform(&window, Request::Pause);
    perform(&window, Request::Play);
    perform(&window, Request::Next);
    window.set_player_has_next(true);
    perform(&window, Request::Next);
    perform(&window, Request::Previous);
    perform(&window, Request::SeekBy(-40.0));
    perform(&window, Request::SeekTo(200.0));
    perform(&window, Request::SetRate(3.0));
    perform(&window, Request::SetVolume(0.5));
    assert!(!closed.get());
    perform(&window, Request::Stop);
    assert!(closed.get());

    assert_eq!(
        *sent.borrow(),
        vec![
            PlaybackCommand::SetPaused(true),
            PlaybackCommand::SetPaused(false),
            PlaybackCommand::Next,
            PlaybackCommand::Previous,
            PlaybackCommand::Seek(SeekTarget::Absolute(Duration::ZERO)),
            PlaybackCommand::Seek(SeekTarget::Absolute(Duration::from_secs(90))),
            PlaybackCommand::SetSpeed(2.0),
            PlaybackCommand::SetVolume(50.0),
            PlaybackCommand::Stop,
        ]
    );
}
