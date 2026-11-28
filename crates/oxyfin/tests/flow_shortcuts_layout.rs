//! The shortcuts dialog keeps the design's width, padding and columns, puts the keycaps at the
//! end of their row, and stays inside a small window.

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use page::{LayoutPage, Shortcuts};
use slint::{ComponentHandle, LogicalSize};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { Shortcuts, ShortcutsOverlay } from "ui/components/shortcuts_overlay.slint";
        import { Theme } from "ui/theme.slint";

        export { Shortcuts }

        export component LayoutPage inherits Window {
            out property <length> width-max: Theme.shortcuts-width;
            out property <length> padding-x: Theme.shortcuts-padding-x;
            out property <length> column-gap: Theme.shortcuts-column-gap;
            out property <length> keycap-inset: Theme.border-thin + Theme.keycap-padding-x;
            out property <length> margin: Theme.space-xl;
            out property <length> border: Theme.border-thin;

            ShortcutsOverlay {
                width: 100%;
                height: 100%;
            }
        }
    }
}

fn dialog(page: &LayoutPage) -> ElementHandle {
    ElementHandle::find_by_accessible_label(page, "Keyboard shortcuts")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Groupbox))
        .unwrap_or_else(|| panic!("the dialog does not show"))
}

fn text(page: &LayoutPage, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(page, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Text))
        .unwrap_or_else(|| panic!("no text {label}"))
}

/// The keycap drawing `label`, whose edges come from the layout and not from font metrics.
fn keycap(page: &LayoutPage, label: &str) -> ElementHandle {
    let at = text(page, label).absolute_position();
    ElementHandle::find_by_element_type_name(page, "Keycap")
        .find(|keycap| {
            let (origin, size) = (keycap.absolute_position(), keycap.size());
            (origin.x..origin.x + size.width).contains(&at.x)
                && (origin.y..origin.y + size.height).contains(&at.y)
        })
        .unwrap_or_else(|| panic!("no keycap {label}"))
}

/// Checks that the keycap of `label` ends at `end` and insets its legend by the keycap padding.
fn assert_keycap_ends_at(page: &LayoutPage, label: &str, end: f32) {
    let keycap = keycap(page, label);
    let (origin, size) = (keycap.absolute_position(), keycap.size());
    assert!(
        (origin.x + size.width - end).abs() < 0.5,
        "the {label} keycap ends at {}, not {end}",
        origin.x + size.width
    );
    let legend = text(page, label).absolute_position().x;
    assert!(
        (legend - (origin.x + page.get_keycap_inset())).abs() < 0.5,
        "the {label} legend starts at {legend}, its keycap at {}",
        origin.x
    );
    assert!(size.width > 2. * page.get_keycap_inset());
}

/// Checks that the two columns share the dialog equally and end with their keycaps, whatever
/// the width of the labels in the fonts of the system.
fn assert_columns(page: &LayoutPage) {
    let shown = dialog(page);
    let (origin, size) = (shown.absolute_position(), shown.size());
    let inner = size.width - 2. * (page.get_border() + page.get_padding_x());
    let column = (inner - page.get_column_gap()) / 2.;
    let left = origin.x + page.get_border() + page.get_padding_x();

    let label = text(page, "Play or pause");
    assert!((label.absolute_position().x - left).abs() < 0.5);
    assert_keycap_ends_at(page, "Space", left + column);

    let everywhere = text(page, "EVERYWHERE");
    assert!(
        (everywhere.absolute_position().x - (left + column + page.get_column_gap())).abs() < 0.5
    );
    assert_keycap_ends_at(page, "?", left + inner);
    assert_keycap_ends_at(page, "N", left + column);
}

#[test]
fn dialog_follows_the_design_and_fits_the_window() {
    i_slint_backend_testing::init_no_event_loop();
    let page = LayoutPage::new().unwrap();
    page.window().set_size(LogicalSize::new(1280., 720.));
    page.show().unwrap();
    page.global::<Shortcuts>().invoke_show();

    let shown = dialog(&page);
    let (origin, size) = (shown.absolute_position(), shown.size());
    assert_eq!(size.width, page.get_width_max());
    assert!((origin.x - (1280. - size.width) / 2.).abs() < 0.5);
    assert!(origin.y >= page.get_margin() && origin.y + size.height <= 720. - page.get_margin());
    assert_columns(&page);

    page.window().set_size(LogicalSize::new(640., 400.));
    let shown = dialog(&page);
    let (origin, size) = (shown.absolute_position(), shown.size());
    assert_eq!(size.width, 640. - 2. * page.get_margin());
    assert_eq!(size.height, 400. - 2. * page.get_margin());
    assert!((origin.y - page.get_margin()).abs() < 0.5);
    assert_columns(&page);
}
