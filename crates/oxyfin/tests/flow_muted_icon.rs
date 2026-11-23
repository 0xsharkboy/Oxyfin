//! A muted player shows a crossed-out speaker, on its button and in the volume feedback.

use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::PlaybackEvent;
use page::MutePage;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, LogicalSize};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { PlayerControls } from "ui/components/player_controls.slint";
        import { Icons } from "ui/icons.slint";

        export component MutePage inherits Window {
            in property <bool> muted <=> controls.muted;
            out property <bool> crossed: controls.volume-icon == Icons.volume-muted;
            out property <bool> plain: controls.volume-icon == Icons.volume;
            callback toggled <=> controls.toggle-mute;

            width: 1280px;
            height: 200px;

            controls := PlayerControls { }
        }
    }
}

fn labelled(window: &impl ComponentHandle, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, label)
        .any(|element| element.accessible_role() == Some(AccessibleRole::Button))
}

#[test]
fn the_button_shows_a_crossed_out_speaker_while_muted() {
    i_slint_backend_testing::init_no_event_loop();
    let page = MutePage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    assert!(page.get_plain() && !page.get_crossed());
    assert!(labelled(&page, "Mute"));
    page.set_muted(true);
    assert!(page.get_crossed() && !page.get_plain());
    assert!(labelled(&page, "Unmute"));
}

#[test]
fn the_volume_feedback_shows_the_speaker_of_the_button() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap_or_else(|err| panic!("cannot create the window: {err}"));
    window.window().set_size(LogicalSize::new(1280., 720.));
    window
        .show()
        .unwrap_or_else(|err| panic!("cannot show the window: {err}"));
    player::bind(&window, Rc::new(|_| {}));
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
    let icon = |id: &str| {
        ElementHandle::find_by_element_id(&window, id)
            .next()
            .is_some()
    };
    let position = LogicalPosition::new(640., 300.);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.,
            delta_y: 60.,
        });
    assert!(
        icon("PlayerScreen::osd-icon"),
        "no speaker beside the volume"
    );
}
