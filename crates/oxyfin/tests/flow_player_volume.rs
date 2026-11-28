//! Places the volume of the player bar right before its buttons, with or without chapters.

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use page::VolumePage;
use slint::ComponentHandle;

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { PlayerControls } from "ui/components/player_controls.slint";

        export component VolumePage inherits Window {
            in property <bool> has-chapters;

            VerticalLayout {
                PlayerControls { width: 1280px; volume: 50; duration: 10; has-chapters: root.has-chapters; }
            }
        }
    }
}

fn find(page: &VolumePage, label: &str, role: AccessibleRole) -> ElementHandle {
    ElementHandle::find_by_accessible_label(page, label)
        .find(|element| element.accessible_role() == Some(role) && element.size().width > 0.)
        .unwrap_or_else(|| panic!("no {label} element"))
}

fn left(element: &ElementHandle) -> f32 {
    element.absolute_position().x
}

fn right(element: &ElementHandle) -> f32 {
    element.absolute_position().x + element.size().width
}

#[test]
fn the_volume_sits_right_before_the_buttons() {
    i_slint_backend_testing::init_no_event_loop();
    for has_chapters in [false, true] {
        let page = VolumePage::new().unwrap();
        page.set_has_chapters(has_chapters);
        page.window().set_size(slint::LogicalSize::new(1280., 200.));
        page.show().unwrap();

        let slider = find(&page, "Volume", AccessibleRole::Slider);
        let mute = find(&page, "Mute", AccessibleRole::Button);
        let next = if has_chapters {
            find(&page, "Chapters", AccessibleRole::Button)
        } else {
            find(&page, "Audio and subtitles", AccessibleRole::Button)
        };
        let settings = find(&page, "Playback settings", AccessibleRole::Button);
        let stats = find(&page, "Playback statistics", AccessibleRole::Button);
        let spacing = left(&settings) - right(&stats);

        let gap = left(&next) - right(&slider);
        assert!(
            (gap - spacing).abs() < 0.5,
            "chapters {has_chapters}: the volume ends {gap} px before the next button, not {spacing}"
        );
        let inner = left(&slider) - right(&mute);
        assert!(
            (inner - spacing).abs() < 0.5,
            "chapters {has_chapters}: the slider starts {inner} px after the mute button"
        );
        page.hide().unwrap();
    }
}
