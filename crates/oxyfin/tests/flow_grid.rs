//! Drives the poster grid and the dropdown with real pointer events.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use page::{GridPage, PageCard};
use slint::{ComponentHandle, ModelRc, VecModel};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { PosterGrid } from "ui/components/poster_grid.slint";
        import { Dropdown } from "ui/components/dropdown.slint";
        import { CardData } from "ui/theme.slint";

        export { CardData as PageCard }

        export component GridPage inherits Window {
            in property <[CardData]> cards;
            in property <[string]> options;
            in-out property <int> selected: 0;
            in-out property <length> viewport-y <=> grid.viewport-y;
            callback opened(string);
            callback chosen(int);
            callback near-end();

            width: 1300px;
            height: 1400px;

            VerticalLayout {
                HorizontalLayout {
                    z: 10;
                    Dropdown {
                        label: "Sort";
                        options: root.options;
                        selected: root.selected;
                        chosen(index) => {
                            root.selected = index;
                            root.chosen(index);
                        }
                    }
                }
                grid := PosterGrid {
                    cards: root.cards;
                    open(card) => { root.opened(card.id); }
                    near-end => { root.near-end(); }
                }
            }
        }
    }
}

fn card(index: usize) -> PageCard {
    PageCard {
        id: format!("id{index}").into(),
        title: format!("Title {index}").into(),
        subtitle: "2024".into(),
        progress: 0.0,
        watched: index.is_multiple_of(3),
        image: Default::default(),
        has_image: false,
    }
}

fn find(page: &GridPage, label: &str, role: AccessibleRole) -> ElementHandle {
    let element = ElementHandle::find_by_accessible_label(page, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("no {label} element"));
    let size = element.size();
    assert!(
        size.width > 0. && size.height > 0.,
        "{label} must have a size"
    );
    element
}

#[test]
fn grid_lays_out_rows_and_opens_on_the_first_click() {
    i_slint_backend_testing::init_no_event_loop();
    let page = GridPage::new().unwrap();
    page.set_cards(ModelRc::new(VecModel::from(
        (0..60).map(card).collect::<Vec<_>>(),
    )));
    page.set_options(ModelRc::new(VecModel::from(vec![
        "Date added".into(),
        "Name".into(),
    ])));
    page.window()
        .set_size(slint::LogicalSize::new(1300., 1400.));
    page.show().unwrap();

    let positions: Vec<_> = (0..20)
        .map(|i| {
            let element = find(&page, &format!("Title {i}"), AccessibleRole::Button);
            (element.absolute_position(), element.size())
        })
        .collect();
    let columns = positions
        .iter()
        .take_while(|(position, _)| position.y == positions[0].0.y)
        .count();
    assert!(columns > 1 && columns < 20, "got {columns} columns");
    let (card_width, gap, margin) = (
        positions[0].1.width,
        positions[columns].0.y - positions[0].0.y - positions[0].1.height,
        positions[0].0.x,
    );
    let last = positions[columns - 1];
    assert!(
        (last.0.x + last.1.width - (1300. - margin)).abs() < 0.5,
        "the last column must end at the right margin, ends at {}",
        last.0.x + last.1.width
    );
    // Same formula as poster_grid.slint; at 1300px it differs from one without the margins.
    assert_eq!(
        columns,
        ((1300. - 2. * margin + gap) / (card_width + gap)).floor() as usize
    );
    assert_ne!(
        columns,
        ((1300. + gap) / (card_width + gap)).floor() as usize
    );
    for i in 1..columns {
        assert!(positions[i].0.x > positions[i - 1].0.x);
    }
    assert!(positions[columns].0.y > positions[0].0.y);
    assert_eq!(positions[columns].0.x, positions[0].0.x);

    let opened = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&opened);
    page.on_opened(move |id| recorder.borrow_mut().push(id.to_string()));
    let seventh = find(&page, "Title 7", AccessibleRole::Button);
    common::click(&page, &seventh);
    assert_eq!(*opened.borrow(), ["id7"]);

    let chosen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&chosen);
    page.on_chosen(move |index| recorder.borrow_mut().push(index));
    assert!(
        ElementHandle::find_by_accessible_label(&page, "Name")
            .next()
            .is_none(),
        "the list starts closed"
    );
    let dropdown = find(&page, "Sort: Date added", AccessibleRole::Button);
    common::click(&page, &dropdown);
    let option = ElementHandle::find_by_accessible_label(&page, "Name")
        .next()
        .expect("the list opens");
    common::click(&page, &option);
    assert_eq!(*chosen.borrow(), [1]);
    assert!(
        ElementHandle::find_by_accessible_label(&page, "Name")
            .next()
            .is_none(),
        "choosing closes the list"
    );
    find(&page, "Sort: Name", AccessibleRole::Button);

    common::click(&page, &find(&page, "Sort: Name", AccessibleRole::Button));
    assert!(
        ElementHandle::find_by_accessible_label(&page, "Date added")
            .next()
            .is_some()
    );
    let title = find(&page, "Title 0", AccessibleRole::Button);
    common::click(&page, &title);
    assert!(
        ElementHandle::find_by_accessible_label(&page, "Date added")
            .next()
            .is_none(),
        "a click outside closes the list"
    );
    assert_eq!(*opened.borrow(), ["id7"], "the dismiss click opens no card");

    common::click(&page, &find(&page, "Sort: Name", AccessibleRole::Button));
    assert!(
        ElementHandle::find_by_accessible_label(&page, "Date added")
            .next()
            .is_some()
    );
    page.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
    page.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased {
            text: slint::platform::Key::Escape.into(),
        });
    assert!(
        ElementHandle::find_by_accessible_label(&page, "Date added")
            .next()
            .is_none(),
        "Escape closes the list"
    );

    let ends = Rc::new(std::cell::Cell::new(0));
    let counter = Rc::clone(&ends);
    page.on_near_end(move || counter.set(counter.get() + 1));
    page.window().set_size(slint::LogicalSize::new(1300., 700.));
    slint::platform::update_timers_and_animations();
    assert_eq!(ends.get(), 0, "far from the end of the content");
    page.set_viewport_y(-2000.);
    slint::platform::update_timers_and_animations();
    assert!(ends.get() > 0, "less than one view height is left");
}
