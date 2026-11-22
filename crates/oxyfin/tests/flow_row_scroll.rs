//! Scrolls poster and landscape rows sideways with the arrows, the wheel and the keyboard.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use page::{HomeRowsPage, PageCard, RowPage};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { PosterRow } from "ui/components/poster_row.slint";
        import { HomeScreen, HomeRowKind } from "ui/screens/home.slint";
        import { CardData, Theme } from "ui/theme.slint";

        export { CardData as PageCard }

        export component RowPage inherits Window {
            in property <[CardData]> long;
            in property <[CardData]> short;
            out property <length> row-padding: Theme.space-3xl;
            out property <length> card-width: Theme.poster-width;
            out property <length> stride: Theme.poster-width + Theme.space-xl;
            out property <length> arrow-zone: Theme.row-arrow-zone;
            out property <length> arrow-size: Theme.row-arrow-size;
            out property <length> arrow-inset: Theme.space-lg;
            callback opened(string);

            width: 1000px;
            height: 500px;

            Flickable {
                content-width: self.width;
                content-height: rows.preferred-height;

                rows := VerticalLayout {
                    alignment: start;
                    spacing: Theme.space-2xl;

                    PosterRow {
                        title: "Long";
                        items: root.long;
                        clicked(id) => { root.opened(id); }
                    }
                    PosterRow {
                        title: "Short";
                        items: root.short;
                        clicked(id) => { root.opened(id); }
                    }
                }
            }
        }

        export component HomeRowsPage inherits Window {
            in property <[CardData]> cards;

            width: 1000px;
            height: 700px;

            HomeScreen {
                settled: true;
                rows: [{ title: "Continue", kind: HomeRowKind.landscape, items: root.cards }];
            }
        }
    }
}

const WIDTH: f32 = 1000.;
const CARDS: usize = 20;

fn card(prefix: &str, index: usize) -> PageCard {
    PageCard {
        id: format!("id{index}").into(),
        title: format!("{prefix} {index}").into(),
        subtitle: "2024".into(),
        progress: 0.0,
        watched: false,
        image: Default::default(),
        has_image: false,
    }
}

fn cards(prefix: &str, count: usize) -> ModelRc<PageCard> {
    ModelRc::new(VecModel::from(
        (0..count).map(|i| card(prefix, i)).collect::<Vec<_>>(),
    ))
}

fn row_page() -> RowPage {
    i_slint_backend_testing::init_no_event_loop();
    let page = RowPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.set_long(cards("Long", CARDS));
    page.set_short(cards("Short", 2));
    page.window().set_size(slint::LogicalSize::new(WIDTH, 500.));
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    page
}

fn find(page: &impl ComponentHandle, label: &str, role: AccessibleRole) -> ElementHandle {
    ElementHandle::find_by_accessible_label(page, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {label} element"))
}

/// Left edge of a card, or `None` once it has scrolled out of sight.
fn x_of(page: &impl ComponentHandle, label: &str) -> Option<f32> {
    ElementHandle::find_by_accessible_label(page, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .map(|element| element.absolute_position().x)
}

/// How far the long row has scrolled, measured on any card still in sight.
fn scrolled(page: &RowPage) -> f32 {
    (0..CARDS)
        .find_map(|i| {
            x_of(page, &format!("Long {i}"))
                .map(|x| page.get_row_padding() + i as f32 * page.get_stride() - x)
        })
        .unwrap_or_else(|| panic!("no card of the long row in sight"))
}

/// A card of the long row shown in full between the two arrow zones.
fn clear_card(page: &RowPage) -> String {
    let zone = page.get_arrow_zone();
    (0..CARDS)
        .map(|i| format!("Long {i}"))
        .find(|label| {
            x_of(page, label)
                .is_some_and(|x| x >= zone && x + page.get_card_width() <= WIDTH - zone)
        })
        .unwrap_or_else(|| panic!("no card clear of the arrows"))
}

/// Arrows of the row labelled `row`, by label.
fn arrows(page: &impl ComponentHandle, row: &str) -> Vec<String> {
    let row = find(page, row, AccessibleRole::List);
    let (top, bottom) = (
        row.absolute_position().y,
        row.absolute_position().y + row.size().height,
    );
    let mut found = Vec::new();
    for label in ["Scroll left", "Scroll right"] {
        for element in ElementHandle::find_by_accessible_label(page, label) {
            let y = element.absolute_position().y;
            let size = element.size();
            if element.accessible_role() == Some(AccessibleRole::Button)
                && size.width > 0.
                && y >= top
                && y < bottom
            {
                found.push(label.to_string());
            }
        }
    }
    found
}

fn hover(page: &impl ComponentHandle, label: &str) {
    let element = find(page, label, AccessibleRole::Button);
    let position = element.absolute_position();
    let size = element.size();
    page.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(position.x + size.width / 2., position.y + size.height / 2.),
    });
}

fn settle() {
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
}

fn press(page: &impl ComponentHandle, key: Key) {
    let text = slint::SharedString::from(key);
    page.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    page.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    settle();
}

fn wheel(page: &impl ComponentHandle, over: &str, delta_x: f32, delta_y: f32) {
    let element = find(page, over, AccessibleRole::Button);
    let position = element.absolute_position();
    page.window().dispatch_event(WindowEvent::PointerScrolled {
        position: LogicalPosition::new(position.x + 10., position.y + 10.),
        delta_x,
        delta_y,
    });
    settle();
}

fn click_arrow(page: &impl ComponentHandle, label: &str) {
    common::click(page, &find(page, label, AccessibleRole::Button));
    settle();
}

#[test]
fn arrows_show_on_hover_only_on_the_side_that_can_scroll() {
    let page = row_page();
    assert!(arrows(&page, "Long").is_empty(), "no arrow before hovering");

    hover(&page, "Long 0");
    assert_eq!(arrows(&page, "Long"), ["Scroll right"]);
    let arrow = find(&page, "Scroll right", AccessibleRole::Button);
    assert_eq!(arrow.size().width, page.get_arrow_size());
    let inset = page.get_arrow_inset();
    assert!(
        (arrow.absolute_position().x + arrow.size().width - (WIDTH - inset)).abs() < 0.5,
        "the right arrow sits {inset} px from the window edge, ends at {}",
        arrow.absolute_position().x + arrow.size().width
    );

    hover(&page, "Short 0");
    assert!(arrows(&page, "Long").is_empty(), "arrows hide on leave");
    assert!(
        arrows(&page, "Short").is_empty(),
        "a row that fits has no arrow"
    );
}

#[test]
fn an_arrow_click_scrolls_one_page_until_the_end() {
    let page = row_page();
    let opened = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&opened);
    page.on_opened(move |id| recorder.borrow_mut().push(id.to_string()));
    let page_width = WIDTH - page.get_arrow_zone();
    assert_eq!(scrolled(&page), 0.);

    hover(&page, "Long 0");
    click_arrow(&page, "Scroll right");
    assert_eq!(scrolled(&page), page_width);
    assert!(opened.borrow().is_empty(), "an arrow click opens nothing");
    hover(&page, &clear_card(&page));
    assert_eq!(arrows(&page, "Long"), ["Scroll left", "Scroll right"]);

    for _ in 0..10 {
        if !arrows(&page, "Long").contains(&"Scroll right".to_string()) {
            break;
        }
        click_arrow(&page, "Scroll right");
    }
    let last = format!("Long {}", CARDS - 1);
    let end = x_of(&page, &last).unwrap() + page.get_card_width();
    assert!(
        (end - (WIDTH - page.get_row_padding())).abs() < 0.5,
        "the last card stops at the right padding, ends at {end}"
    );
    hover(&page, &clear_card(&page));
    assert_eq!(arrows(&page, "Long"), ["Scroll left"]);

    let at_end = scrolled(&page);
    click_arrow(&page, "Scroll left");
    assert_eq!(scrolled(&page), at_end - page_width);

    common::click(
        &page,
        &find(&page, &clear_card(&page), AccessibleRole::Button),
    );
    assert_eq!(opened.borrow().len(), 1, "cards stay clickable");
}

#[test]
fn the_wheel_scrolls_a_row_sideways() {
    let page = row_page();
    let short_y = find(&page, "Short 0", AccessibleRole::Button)
        .absolute_position()
        .y;

    wheel(&page, "Long 1", -100., 0.);
    let after_horizontal = scrolled(&page);
    assert!(
        after_horizontal > 0.,
        "a horizontal wheel scrolls the row, by {after_horizontal}"
    );

    if !cfg!(target_os = "macos") {
        page.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Shift.into(),
        });
        wheel(&page, &clear_card(&page), 0., -100.);
        page.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Shift.into(),
        });
        assert!(
            scrolled(&page) > after_horizontal,
            "Shift and the wheel scroll the row"
        );
    }

    let before = scrolled(&page);
    wheel(&page, &clear_card(&page), 0., -100.);
    assert_eq!(scrolled(&page), before, "a plain wheel leaves the row");
    assert!(
        find(&page, "Short 0", AccessibleRole::Button)
            .absolute_position()
            .y
            < short_y,
        "a plain wheel scrolls the page"
    );
}

#[test]
fn left_and_right_move_the_focus_and_keep_it_visible() {
    let page = row_page();
    let opened = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&opened);
    page.on_opened(move |id| recorder.borrow_mut().push(id.to_string()));

    common::click(&page, &find(&page, "Long 0", AccessibleRole::Button));
    for _ in 0..8 {
        press(&page, Key::RightArrow);
    }
    let x = x_of(&page, "Long 8").unwrap_or(f32::NAN);
    assert!(
        x >= 0. && x + page.get_card_width() <= WIDTH,
        "the focused card stays visible, at {x}"
    );
    press(&page, Key::Return);

    press(&page, Key::LeftArrow);
    press(&page, Key::LeftArrow);
    press(&page, Key::Return);

    common::click(&page, &find(&page, "Long 5", AccessibleRole::Button));
    press(&page, Key::RightArrow);
    press(&page, Key::Return);

    for _ in 0..CARDS + 2 {
        press(&page, Key::RightArrow);
    }
    press(&page, Key::Return);
    assert_eq!(
        *opened.borrow(),
        ["id0", "id8", "id6", "id5", "id6", "id19"]
    );
}

#[test]
fn the_landscape_row_of_the_home_screen_has_arrows() {
    i_slint_backend_testing::init_no_event_loop();
    let page = HomeRowsPage::new().unwrap();
    page.set_cards(cards("Episode", 10));
    page.window().set_size(slint::LogicalSize::new(WIDTH, 700.));
    page.show().unwrap();

    assert!(x_of(&page, "Episode 4").is_none());
    let start = x_of(&page, "Episode 1").unwrap();
    hover(&page, "Episode 1");
    assert_eq!(arrows(&page, "Continue"), ["Scroll right"]);
    click_arrow(&page, "Scroll right");
    assert!(x_of(&page, "Episode 1").is_none_or(|x| x < start));
    assert!(
        x_of(&page, "Episode 4").is_some(),
        "the next page comes in sight"
    );
}

#[test]
fn fast_arrow_clicks_scroll_a_full_page_each() {
    let page = row_page();
    let page_width = WIDTH - page.get_arrow_zone();
    hover(&page, "Long 0");
    let arrow = find(&page, "Scroll right", AccessibleRole::Button);
    common::click(&page, &arrow);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(50));
    common::click(&page, &find(&page, "Scroll right", AccessibleRole::Button));
    settle();
    assert_eq!(scrolled(&page), 2. * page_width);
}

#[test]
fn the_focused_card_stays_clear_of_the_faded_edges() {
    let page = row_page();
    common::click(&page, &find(&page, "Long 0", AccessibleRole::Button));
    let zone = page.get_arrow_zone();
    for index in 1..12 {
        press(&page, Key::RightArrow);
        let x = x_of(&page, &format!("Long {index}")).unwrap_or(f32::NAN);
        assert!(
            x + page.get_card_width() <= WIDTH - zone + 0.5,
            "Long {index} ends at {}, under the right edge",
            x + page.get_card_width()
        );
    }
    for index in (2..11).rev() {
        press(&page, Key::LeftArrow);
        let x = x_of(&page, &format!("Long {index}")).unwrap_or(f32::NAN);
        assert!(
            x >= zone - 0.5,
            "Long {index} starts at {x}, under the left edge"
        );
    }
}
