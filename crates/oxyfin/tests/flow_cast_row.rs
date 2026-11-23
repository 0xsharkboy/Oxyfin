//! Scrolls the cast and crew row like the other rows: with the arrows, the wheel and the keyboard.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use page::{CastPage, PagePerson};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { CastRow, PersonCardData, PersonJob } from "ui/components/person_card.slint";
        import { Theme } from "ui/theme.slint";

        export { PersonCardData as PagePerson }

        export component CastPage inherits Window {
            in property <[PersonCardData]> people;
            out property <length> row-padding: Theme.space-3xl;
            out property <length> stride: Theme.person-card-width + Theme.space-xl;
            out property <length> photo: Theme.person-photo-size;
            out property <length> arrow-zone: Theme.row-arrow-zone;
            out property <length> arrow-inset: Theme.space-lg;
            callback opened(string);

            width: 1000px;
            height: 400px;

            VerticalLayout {
                alignment: start;

                CastRow {
                    people: root.people;
                    clicked(id) => { root.opened(id); }
                }
            }
        }
    }
}

const WIDTH: f32 = 1000.;
const PEOPLE: usize = 20;
// Without an identifier, it opens nothing and takes no focus.
const UNKNOWN: usize = 2;

fn person(index: usize, name: &str) -> PagePerson {
    let id = if index == UNKNOWN {
        String::new()
    } else {
        format!("id{index}")
    };
    PagePerson {
        id: id.into(),
        name: name.into(),
        role: "Role".into(),
        initials: "P".into(),
        ..Default::default()
    }
}

fn cast_page() -> CastPage {
    i_slint_backend_testing::init_no_event_loop();
    let page = CastPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.set_people(ModelRc::new(VecModel::from(
        (0..PEOPLE)
            .map(|index| person(index, &format!("Person {index}")))
            .collect::<Vec<_>>(),
    )));
    page.window().set_size(slint::LogicalSize::new(WIDTH, 400.));
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    page
}

fn find(page: &CastPage, label: &str) -> ElementHandle {
    button(page, label).unwrap_or_else(|| panic!("no {label} button"))
}

fn button(page: &CastPage, label: &str) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(page, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
}

/// How far the row has scrolled, measured on any card still in sight.
fn scrolled(page: &CastPage) -> f32 {
    (0..PEOPLE)
        .find_map(|index| {
            button(page, &format!("Person {index}")).map(|card| {
                page.get_row_padding() + index as f32 * page.get_stride()
                    - card.absolute_position().x
            })
        })
        .unwrap_or_else(|| panic!("no card in sight"))
}

fn settle() {
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
}

fn hover(page: &CastPage, label: &str) {
    let position = find(page, label).absolute_position();
    page.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(position.x + 60., position.y + 60.),
    });
}

fn press(page: &CastPage, key: Key) {
    let text = slint::SharedString::from(key);
    page.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    page.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    settle();
}

fn opened(page: &CastPage) -> Rc<RefCell<Vec<String>>> {
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let record = Rc::clone(&opened);
    page.on_opened(move |id| record.borrow_mut().push(id.to_string()));
    opened
}

#[test]
fn the_arrows_show_on_hover_at_the_height_of_the_photos() {
    let page = cast_page();
    assert!(button(&page, "Scroll right").is_none());
    hover(&page, "Person 1");
    assert!(
        button(&page, "Scroll left").is_none(),
        "nothing lies on the left"
    );
    let arrow = find(&page, "Scroll right");
    let centre = arrow.absolute_position().y + arrow.size().height / 2.;
    let photos = find(&page, "Person 1").absolute_position().y + page.get_photo() / 2.;
    assert!(
        (centre - photos).abs() < 1.,
        "arrow at {centre}, photos at {photos}"
    );
    let right = arrow.absolute_position().x + arrow.size().width;
    assert!((right - (WIDTH - page.get_arrow_inset())).abs() < 0.5);
    let left = find(&page, "Person 0").absolute_position().x;
    assert!((left - page.get_row_padding()).abs() < 0.5);
}

#[test]
fn an_arrow_click_scrolls_one_page() {
    let page = cast_page();
    let opened = opened(&page);
    hover(&page, "Person 1");
    common::click(&page, &find(&page, "Scroll right"));
    settle();
    let expected = WIDTH - page.get_arrow_zone();
    assert!((scrolled(&page) - expected).abs() < 1.);
    assert!(button(&page, "Scroll left").is_some());
    assert!(opened.borrow().is_empty(), "the arrow opened a person");
}

#[test]
fn the_wheel_scrolls_the_row_sideways() {
    let page = cast_page();
    let position = find(&page, "Person 1").absolute_position();
    page.window().dispatch_event(WindowEvent::PointerScrolled {
        position: LogicalPosition::new(position.x + 10., position.y + 10.),
        delta_x: -300.,
        delta_y: 0.,
    });
    settle();
    assert!((scrolled(&page) - 300.).abs() < 1.);
}

#[test]
fn left_and_right_move_along_the_people_who_open_a_page() {
    let page = cast_page();
    let opened = opened(&page);
    common::click(&page, &find(&page, "Person 1"));
    settle();
    press(&page, Key::RightArrow);
    press(&page, Key::Return);
    press(&page, Key::LeftArrow);
    press(&page, Key::Return);
    assert_eq!(*opened.borrow(), ["id1", "id3", "id1"]);
    for _ in 0..8 {
        press(&page, Key::RightArrow);
    }
    press(&page, Key::Return);
    assert_eq!(opened.borrow().last().map(String::as_str), Some("id10"));
    let card = find(&page, "Person 10");
    let x = card.absolute_position().x;
    let zone = page.get_arrow_zone();
    assert!(
        x >= zone && x + card.size().width <= WIDTH - zone,
        "the focused card is at {x}"
    );
}
