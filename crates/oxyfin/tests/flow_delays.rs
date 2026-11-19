//! Audio and subtitle delays: the track menu rows and the mpv shortcuts send
//! their commands, the menu and the on-screen feedback show the values.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::click;
use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementQuery};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{PlaybackCommand, PlaybackEvent};
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

fn with_control(window: &AppWindow, text: &str) {
    let control = slint::SharedString::from(Key::Control);
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: control.clone(),
    });
    press(window, text);
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: control });
}

fn find(window: &AppWindow, label: &str, role: AccessibleRole) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {role:?} labelled {label}"))
}

fn texts(window: &AppWindow) -> Vec<String> {
    ElementQuery::from_root(window)
        .match_descendants()
        .match_accessible_role(AccessibleRole::Text)
        .find_all()
        .into_iter()
        .filter_map(|element| element.accessible_label().map(|label| label.to_string()))
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

#[test]
fn shortcuts_step_the_delays_like_mpv_and_show_them() {
    let (window, sent) = player_window();
    player::apply(&window, PlaybackEvent::SubtitleDelay(200));

    press(&window, "z");
    press(&window, "Z");
    press(&window, "x");
    with_control(&window, "+");
    with_control(&window, "=");
    with_control(&window, "-");
    assert_eq!(
        *sent.borrow(),
        vec![
            PlaybackCommand::SetSubtitleDelay(100),
            PlaybackCommand::SetSubtitleDelay(300),
            PlaybackCommand::SetSubtitleDelay(300),
            PlaybackCommand::SetAudioDelay(100),
            PlaybackCommand::SetAudioDelay(100),
            PlaybackCommand::SetAudioDelay(-100),
        ]
    );
    assert!(!window.window().is_fullscreen());

    player::apply(&window, PlaybackEvent::AudioDelay(-100));
    assert!(
        texts(&window)
            .iter()
            .any(|text| text == "Audio delay: -0.1 s"),
        "{:?}",
        texts(&window)
    );
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1600));
    assert!(
        !texts(&window).iter().any(|text| text.contains("delay:")),
        "{:?}",
        texts(&window)
    );

    press(&window, "z");
    player::apply(&window, PlaybackEvent::SubtitleDelay(1300));
    assert!(
        texts(&window)
            .iter()
            .any(|text| text == "Subtitle delay: +1.3 s"),
        "{:?}",
        texts(&window)
    );
}

#[test]
fn the_track_menu_shows_and_changes_the_delays() {
    let (window, sent) = player_window();
    player::apply(&window, PlaybackEvent::AudioDelay(-1200));

    click(
        &window,
        &find(&window, "Audio and subtitles", AccessibleRole::Button),
    );
    let audio = find(&window, "Audio delay", AccessibleRole::Text);
    assert_eq!(audio.accessible_value().as_deref(), Some("-1.2 s"));
    let subtitle = find(&window, "Subtitle delay", AccessibleRole::Text);
    assert_eq!(subtitle.accessible_value().as_deref(), Some("+0.0 s"));

    let reset = find(&window, "Reset the subtitle delay", AccessibleRole::Button);
    assert_eq!(reset.accessible_enabled(), Some(false));
    click(&window, &reset);
    click(
        &window,
        &find(
            &window,
            "Increase the subtitle delay",
            AccessibleRole::Button,
        ),
    );
    click(
        &window,
        &find(&window, "Decrease the audio delay", AccessibleRole::Button),
    );
    click(
        &window,
        &find(&window, "Reset the audio delay", AccessibleRole::Button),
    );
    assert_eq!(
        *sent.borrow(),
        vec![
            PlaybackCommand::SetSubtitleDelay(100),
            PlaybackCommand::SetAudioDelay(-1300),
            PlaybackCommand::SetAudioDelay(0),
        ]
    );

    // The rows fit in the panel: nothing pokes out of its right edge.
    let panel = find(&window, "Audio and subtitles", AccessibleRole::List);
    let right = panel.absolute_position().x + panel.size().width;
    for label in [
        "Reset the audio delay",
        "Reset the subtitle delay",
        "Increase the audio delay",
    ] {
        let button = find(&window, label, AccessibleRole::Button);
        assert!(
            button.absolute_position().x + button.size().width <= right,
            "{label} overflows the menu"
        );
    }

    // The buttons leave the focus to the player: its shortcuts still work.
    sent.borrow_mut().clear();
    press(&window, " ");
    assert_eq!(*sent.borrow(), vec![PlaybackCommand::SetPaused(true)]);
}

#[test]
fn a_new_playback_starts_without_delays() {
    let (window, _sent) = player_window();
    player::apply(&window, PlaybackEvent::SubtitleDelay(500));
    player::apply(&window, PlaybackEvent::AudioDelay(300));
    player::reset(&window);
    assert_eq!(window.get_player_subtitle_delay(), 0);
    assert_eq!(window.get_player_audio_delay(), 0);
}
