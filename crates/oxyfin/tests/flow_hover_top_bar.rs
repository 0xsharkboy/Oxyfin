//! A card scrolled under the top bar shows no hover card: the bar keeps the pointer.

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use page::{HoverCard, HoverDetails, HoverPage, PageCard};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { HomeScreen, HomeRow, HomeRowKind } from "ui/screens/home.slint";
        import { HoverCard, HoverCardOverlay, HoverDetails } from "ui/components/hover_card.slint";
        import { CardData } from "ui/theme.slint";

        export { CardData as PageCard, HoverCard, HoverDetails }

        export component HoverPage inherits Window {
            in property <[CardData]> cards;

            width: 1000px;
            height: 600px;

            HomeScreen {
                settled: true;
                rows: [
                    { title: "First", kind: HomeRowKind.poster, items: root.cards },
                    { title: "Second", kind: HomeRowKind.poster, items: root.cards },
                    { title: "Third", kind: HomeRowKind.poster, items: root.cards },
                ];
            }

            HoverCardOverlay { }
        }
    }
}

fn card(index: usize) -> PageCard {
    PageCard {
        id: format!("id{index}").into(),
        title: format!("Poster {index}").into(),
        subtitle: "2024".into(),
        progress: 0.0,
        watched: false,
        image: Default::default(),
        has_image: false,
        no_menu: false,
    }
}

fn shown_card(page: &HoverPage) -> bool {
    ElementHandle::find_by_accessible_label(page, "Details")
        .any(|element| element.accessible_role() == Some(AccessibleRole::Groupbox))
}

fn rest_at(page: &HoverPage, x: f32, y: f32) {
    page.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(x, y),
    });
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(450));
}

#[test]
fn a_card_under_the_top_bar_shows_nothing() {
    i_slint_backend_testing::init_no_event_loop();
    let page = HoverPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.set_cards(ModelRc::new(VecModel::from(
        (0..4).map(card).collect::<Vec<_>>(),
    )));
    let weak = page.as_weak();
    page.global::<HoverCard>()
        .on_requested(move |id, _progress| {
            if let Some(page) = weak.upgrade() {
                page.global::<HoverCard>().set_details(HoverDetails {
                    id,
                    title: "Details".into(),
                    ..HoverDetails::default()
                });
            }
        });
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));

    let bar = ElementHandle::find_by_accessible_label(&page, "Navigation")
        .find(|element| element.accessible_role() == Some(AccessibleRole::TabList))
        .unwrap_or_else(|| panic!("no top bar"));
    let bar_bottom = bar.absolute_position().y + bar.size().height;
    let x = 60.;
    page.window().dispatch_event(WindowEvent::PointerScrolled {
        position: LogicalPosition::new(500., 400.),
        delta_x: 0.,
        delta_y: -150.,
    });
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(500));
    let under_bar = ElementHandle::find_by_accessible_label(&page, "Poster 0").any(|poster| {
        let at = poster.absolute_position();
        poster.accessible_role() == Some(AccessibleRole::Button)
            && at.x <= x
            && x < at.x + poster.size().width
            && at.y < bar_bottom - 4.
            && bar_bottom - 4. < at.y + poster.size().height
    });
    assert!(under_bar, "a poster lies under the bar at x {x}");

    rest_at(&page, x, bar_bottom - 4.);
    assert!(
        !shown_card(&page),
        "the bar keeps the pointer from the poster"
    );

    rest_at(&page, x, bar_bottom + 40.);
    assert!(shown_card(&page), "below the bar the poster shows its card");

    let top = |page: &HoverPage| {
        ElementHandle::find_by_accessible_label(page, "Second")
            .find(|element| element.accessible_role() == Some(AccessibleRole::List))
            .map(|row| row.absolute_position().y)
            .unwrap_or_else(|| panic!("no second row"))
    };
    let before = top(&page);
    page.window().dispatch_event(WindowEvent::PointerScrolled {
        position: LogicalPosition::new(500., bar_bottom - 4.),
        delta_x: 0.,
        delta_y: -50.,
    });
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(500));
    assert!(
        top(&page) < before,
        "the wheel over the bar still scrolls the page"
    );
}
