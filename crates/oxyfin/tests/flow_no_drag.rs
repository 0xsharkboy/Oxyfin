//! Holding the mouse button and dragging never scrolls, unlike the wheel.

mod common;

use std::path::Path;
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use page::{DragPage, PageCard};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { PosterRow } from "ui/components/poster_row.slint";
        import { PosterGrid } from "ui/components/poster_grid.slint";
        import { TextField } from "ui/components/text_field.slint";
        import { Scrubber } from "ui/components/player_controls.slint";
        import { CardData, Theme } from "ui/theme.slint";

        export { CardData as PageCard }

        export component DragPage inherits Window {
            in property <[CardData]> in-row;
            in property <[CardData]> cards;
            out property <length> viewport-y <=> grid.viewport-y;
            out property <float> committed: -1;
            in-out property <string> typed <=> field.text;
            callback opened(string);

            width: 1000px;
            height: 900px;

            VerticalLayout {
                HorizontalLayout {
                    padding: 8px;
                    spacing: 8px;

                    field := TextField {
                        horizontal-stretch: 1;
                        label: "Field";
                    }
                    Scrubber {
                        width: 200px;
                        label: "Slider";
                        maximum: 100;
                        committed(value) => { root.committed = value; }
                    }
                }
                PosterRow {
                    title: "Row";
                    items: root.in-row;
                    clicked(id) => { root.opened(id); }
                }
                grid := PosterGrid {
                    cards: root.cards;
                    open(card) => { root.opened(card.id); }
                }
            }
        }
    }
}

fn cards(prefix: &str, count: usize) -> ModelRc<PageCard> {
    ModelRc::new(VecModel::from(
        (0..count)
            .map(|index| PageCard {
                id: format!("{prefix}{index}").into(),
                title: format!("{prefix} {index}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    ))
}

fn drag_page() -> DragPage {
    i_slint_backend_testing::init_no_event_loop();
    let page = DragPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.set_in_row(cards("Row", 20));
    page.set_cards(cards("Grid", 60));
    page.window().set_size(slint::LogicalSize::new(1000., 900.));
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    page
}

fn find(page: &DragPage, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(page, label)
        .find(|element| element.size().width > 0.)
        .unwrap_or_else(|| panic!("no {label} element"))
}

/// Presses the left button at `from`, moves by `(dx, dy)` in steps and releases it there.
fn drag(page: &DragPage, from: LogicalPosition, dx: f32, dy: f32) {
    let button = PointerEventButton::Left;
    let window = page.window();
    window.dispatch_event(WindowEvent::PointerMoved { position: from });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    let mut position = from;
    for step in 1..=10 {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(16));
        let part = step as f32 / 10.;
        position = LogicalPosition::new(from.x + dx * part, from.y + dy * part);
        window.dispatch_event(WindowEvent::PointerMoved { position });
    }
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn inside(element: &ElementHandle, x: f32, y: f32) -> LogicalPosition {
    let position = element.absolute_position();
    LogicalPosition::new(position.x + x, position.y + y)
}

#[test]
fn dragging_does_not_scroll_a_row() {
    let page = drag_page();
    let card = find(&page, "Row 3");
    let before = card.absolute_position().x;
    drag(&page, inside(&card, 40., 40.), -200., 0.);
    let released = card.absolute_position().x;
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
    let settled = card.absolute_position().x;
    assert!(
        (released - before).abs() < 0.5 && (settled - before).abs() < 0.5,
        "dragged by -200, the row moved by {} then {}",
        released - before,
        settled - before
    );
}

#[test]
fn dragging_does_not_scroll_a_grid_and_the_wheel_still_does() {
    let page = drag_page();
    let card = find(&page, "Grid 1");
    drag(&page, inside(&card, 40., 120.), 0., -100.);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
    assert_eq!(page.get_viewport_y(), 0., "the drag scrolled the grid");
    page.window().dispatch_event(WindowEvent::PointerScrolled {
        position: inside(&card, 40., 120.),
        delta_x: 0.,
        delta_y: -120.,
    });
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
    assert!((page.get_viewport_y() + 120.).abs() < 1.);
}

#[test]
fn a_click_reaches_a_card_at_once() {
    let page = drag_page();
    let card = find(&page, "Grid 1");
    page.window().dispatch_event(WindowEvent::PointerMoved {
        position: inside(&card, 40., 40.),
    });
    page.window().dispatch_event(WindowEvent::PointerPressed {
        position: inside(&card, 40., 40.),
        button: PointerEventButton::Left,
    });
    let opened = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = std::rc::Rc::clone(&opened);
    page.on_opened(move |_| count.set(count.get() + 1));
    // A hand that moves a little while clicking.
    page.window().dispatch_event(WindowEvent::PointerMoved {
        position: inside(&card, 40., 52.),
    });
    page.window().dispatch_event(WindowEvent::PointerReleased {
        position: inside(&card, 40., 52.),
        button: PointerEventButton::Left,
    });
    assert_eq!(opened.get(), 1, "the click was taken for a drag");
}

#[test]
fn text_is_selected_and_a_slider_dragged_with_the_mouse() {
    let page = drag_page();
    page.set_typed("hello".into());
    let field = ElementHandle::find_by_accessible_label(&page, "Field")
        .find(|element| element.accessible_role() == Some(AccessibleRole::TextInput))
        .unwrap_or_else(|| panic!("no field"));
    drag(&page, inside(&field, 1., 10.), 200., 0.);
    let text = slint::SharedString::from(slint::platform::Key::Delete);
    page.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    page.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    assert_eq!(page.get_typed(), "", "the drag selected nothing");

    let slider = find(&page, "Slider");
    drag(&page, inside(&slider, 20., 5.), 80., 0.);
    assert!(
        (page.get_committed() - 50.).abs() < 1.,
        "committed {}",
        page.get_committed()
    );
}

fn slint_files(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|err| panic!("cannot list {}: {err}", dir.display()));
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_dir() {
            slint_files(&path, found);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "slint")
        {
            found.push(path);
        }
    }
}

#[test]
fn no_flickable_pans_with_the_mouse() {
    let mut files = Vec::new();
    slint_files(&common::manifest_dir().join("ui"), &mut files);
    let mut missing = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|err| panic!("cannot read {}: {err}", file.display()));
        for (line, _) in text.match_indices("Flickable {") {
            let next = text[line..].lines().nth(1).map_or("", str::trim);
            if next != "mouse-drag-pan-enabled: false;" {
                missing.push(format!("{}: {next}", file.display()));
            }
        }
    }
    assert!(missing.is_empty(), "{missing:#?}");
}
