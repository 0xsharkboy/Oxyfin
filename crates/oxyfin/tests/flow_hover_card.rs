//! Shows the details of a poster after the pointer rests on it, beside it, and only then.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use page::{HoverCard, HoverDetails, HoverKind, HoverPage, PageCard};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, ModelRc, SharedString, VecModel};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { PosterRow } from "ui/components/poster_row.slint";
        import { HoverCard, HoverCardOverlay, HoverDetails, HoverKind } from "ui/components/hover_card.slint";
        import { CardData, Theme } from "ui/theme.slint";

        export { CardData as PageCard, HoverCard, HoverDetails, HoverKind }

        export component HoverPage inherits Window {
            in property <[CardData]> cards;
            out property <length> gap: Theme.space-md;
            out property <length> drop: Theme.space-sm;
            out property <length> card-width: Theme.hover-card-width;
            out property <length> margin: Theme.space-lg;
            callback opened(string);

            width: 1000px;
            height: 600px;

            VerticalLayout {
                alignment: start;
                padding-top: Theme.space-lg;

                PosterRow {
                    title: "Row";
                    items: root.cards;
                    clicked(id) => { root.opened(id); }
                }
            }

            HoverCardOverlay { }
        }
    }
}

const WIDTH: f32 = 1000.;
const CARDS: usize = 20;

fn card(index: usize) -> PageCard {
    PageCard {
        id: format!("id{index}").into(),
        title: format!("Poster {index}").into(),
        subtitle: "2024".into(),
        progress: if index == 1 { 0.25 } else { 0.0 },
        watched: false,
        image: Default::default(),
        has_image: false,
    }
}

/// Everything a hover card can show, for `id`.
fn full(id: &str) -> HoverDetails {
    HoverDetails {
        id: id.into(),
        title: format!("Details {id}").into(),
        kind: HoverKind::Movie,
        episode_label: SharedString::default(),
        episode_title: SharedString::default(),
        year: 2002,
        minutes: 94,
        seasons: 0,
        rating: "8.4".into(),
        genres: ModelRc::new(VecModel::from(vec![
            SharedString::from("Horror"),
            SharedString::from("Thriller"),
        ])),
        overview: "A crew explores a house.".into(),
        progress: 0.25,
        remaining_minutes: 62,
    }
}

/// Only the title and the kind, as for an item the server says little about.
fn bare(id: &str) -> HoverDetails {
    HoverDetails {
        id: id.into(),
        title: format!("Details {id}").into(),
        kind: HoverKind::Series,
        ..HoverDetails::default()
    }
}

/// A page answering every request with `answer`, and the ids it was asked for.
fn hover_page(answer: fn(&str) -> HoverDetails) -> (HoverPage, Rc<RefCell<Vec<String>>>) {
    i_slint_backend_testing::init_no_event_loop();
    let page = HoverPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.set_cards(ModelRc::new(VecModel::from(
        (0..CARDS).map(card).collect::<Vec<_>>(),
    )));
    page.window().set_size(slint::LogicalSize::new(WIDTH, 600.));
    let asked = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&asked);
    let weak = page.as_weak();
    page.global::<HoverCard>()
        .on_requested(move |id, _progress| {
            log.borrow_mut().push(id.to_string());
            if let Some(page) = weak.upgrade() {
                page.global::<HoverCard>().set_details(answer(&id));
            }
        });
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    (page, asked)
}

fn find(page: &HoverPage, label: &str, role: AccessibleRole) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(page, label).find(|element| {
        let size = element.size();
        element.accessible_role() == Some(role) && size.width > 0. && size.height > 0.
    })
}

fn poster(page: &HoverPage, index: usize) -> ElementHandle {
    find(page, &format!("Poster {index}"), AccessibleRole::Button)
        .unwrap_or_else(|| panic!("no poster {index}"))
}

/// The hover card shown for `id`, if any.
fn hover_card(page: &HoverPage, id: &str) -> Option<ElementHandle> {
    find(page, &format!("Details {id}"), AccessibleRole::Groupbox)
}

fn any_card(page: &HoverPage) -> bool {
    (0..CARDS).any(|i| hover_card(page, &format!("id{i}")).is_some())
}

fn move_to(page: &HoverPage, x: f32, y: f32) {
    page.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(x, y),
    });
}

fn leave(page: &HoverPage) {
    move_to(page, WIDTH / 2., 590.);
}

fn hover(page: &HoverPage, index: usize) {
    let element = poster(page, index);
    let position = element.absolute_position();
    move_to(page, position.x + 20., position.y + 20.);
}

fn elapse(ms: u64) {
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
}

/// Labels of the texts shown inside `card`.
fn texts(card: &ElementHandle) -> Vec<String> {
    card.query_descendants()
        .match_accessible_role(AccessibleRole::Text)
        .find_all()
        .into_iter()
        .filter(|element| element.size().width > 0.)
        .filter_map(|element| element.accessible_label().map(|label| label.to_string()))
        .collect()
}

fn progress(card: &ElementHandle) -> Option<String> {
    card.query_descendants()
        .match_accessible_role(AccessibleRole::ProgressIndicator)
        .find_first()
        .and_then(|element| element.accessible_label())
        .map(|label| label.to_string())
}

#[test]
fn the_card_shows_after_the_delay_beside_the_poster() {
    let (page, asked) = hover_page(full);
    hover(&page, 1);
    elapse(300);
    assert!(
        hover_card(&page, "id1").is_none(),
        "nothing before the delay"
    );
    assert!(asked.borrow().is_empty(), "nothing asked before the delay");

    elapse(150);
    let card = hover_card(&page, "id1").unwrap_or_else(|| panic!("no hover card"));
    assert_eq!(asked.borrow().as_slice(), ["id1"]);
    let poster = poster(&page, 1);
    let at = poster.absolute_position();
    let expected_x = at.x + poster.size().width + page.get_gap();
    assert!(
        (card.absolute_position().x - expected_x).abs() < 0.5,
        "the card starts {} px right of the poster, at {}",
        page.get_gap(),
        card.absolute_position().x
    );
    assert!(
        (card.absolute_position().y - (at.y + page.get_drop())).abs() < 0.5,
        "the card starts {} px below the artwork top, at {}",
        page.get_drop(),
        card.absolute_position().y
    );
    assert_eq!(card.size().width, page.get_card_width());

    let shown = texts(&card);
    for text in [
        "Details id1",
        "Horror",
        "Thriller",
        "A crew explores a house.",
    ] {
        assert!(shown.iter().any(|t| t == text), "{text} shows in {shown:?}");
    }
    assert!(
        shown
            .iter()
            .any(|t| t.contains("2002") && t.contains("1h 34m")),
        "year and duration show in {shown:?}"
    );
    assert!(shown.iter().any(|t| t.contains("8.4")), "the rating shows");
    assert!(shown.iter().any(|t| t == "Movie"), "the kind shows");
    assert_eq!(progress(&card).as_deref(), Some("1h 2m left"));

    leave(&page);
    assert!(
        hover_card(&page, "id1").is_none(),
        "the card hides on leave"
    );
}

#[test]
fn the_card_flips_to_the_left_near_the_right_edge() {
    let (page, _) = hover_page(full);
    let room = |index: usize| {
        let poster = poster(&page, index);
        poster.absolute_position().x + poster.size().width + page.get_gap() + page.get_card_width()
            <= WIDTH - page.get_margin()
    };
    assert!(room(0), "the first poster has room on its right");
    let near_edge = (0..CARDS)
        .find(|&index| !room(index))
        .unwrap_or_else(|| panic!("no poster near the edge"));
    let poster = poster(&page, near_edge);
    hover(&page, near_edge);
    elapse(450);
    let card =
        hover_card(&page, &format!("id{near_edge}")).unwrap_or_else(|| panic!("no hover card"));
    let expected_x = poster.absolute_position().x - page.get_gap() - page.get_card_width();
    assert!(
        (card.absolute_position().x - expected_x).abs() < 0.5,
        "the card ends {} px left of the poster, starts at {}",
        page.get_gap(),
        card.absolute_position().x
    );
}

#[test]
fn the_card_waits_for_the_row_to_stop_scrolling() {
    let (page, asked) = hover_page(full);
    hover(&page, 1);
    elapse(450);
    assert!(hover_card(&page, "id1").is_some());

    let at = poster(&page, 1).absolute_position();
    let scroll = |page: &HoverPage| {
        page.window().dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(at.x + 20., at.y + 20.),
            delta_x: -40.,
            delta_y: 0.,
        });
    };
    scroll(&page);
    elapse(50);
    assert!(!any_card(&page), "the card hides on scroll");
    for _ in 0..5 {
        elapse(250);
        scroll(&page);
    }
    elapse(250);
    assert!(!any_card(&page), "no card while the row keeps scrolling");
    assert_eq!(asked.borrow().len(), 1, "nothing asked while scrolling");

    elapse(600);
    let pointer_x = at.x + 20.;
    let under = (0..CARDS)
        .find(|&i| {
            find(&page, &format!("Poster {i}"), AccessibleRole::Button).is_some_and(|p| {
                let x = p.absolute_position().x;
                x <= pointer_x && pointer_x < x + p.size().width
            })
        })
        .unwrap_or_else(|| panic!("no poster under the pointer"));
    assert_ne!(under, 1, "the row moved another poster under the pointer");
    let card = hover_card(&page, &format!("id{under}"))
        .unwrap_or_else(|| panic!("no card for poster {under}, now under the pointer"));
    let poster = poster(&page, under);
    let expected_x = poster.absolute_position().x + poster.size().width + page.get_gap();
    assert!(
        (card.absolute_position().x - expected_x).abs() < 0.5,
        "the card sits beside the poster now under the pointer, at {}",
        card.absolute_position().x
    );
}

#[test]
fn a_click_hides_the_card_until_the_pointer_comes_back() {
    let (page, _) = hover_page(full);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&opened);
    page.on_opened(move |id| recorder.borrow_mut().push(id.to_string()));
    hover(&page, 1);
    elapse(450);
    assert!(hover_card(&page, "id1").is_some());

    common::click(&page, &poster(&page, 1));
    assert_eq!(opened.borrow().as_slice(), ["id1"]);
    elapse(1000);
    assert!(hover_card(&page, "id1").is_none(), "a click hides the card");

    leave(&page);
    hover(&page, 1);
    elapse(450);
    assert!(
        hover_card(&page, "id1").is_some(),
        "back on the poster, it shows again"
    );
}

#[test]
fn a_suppressed_card_stays_hidden() {
    let (page, _) = hover_page(full);
    hover(&page, 1);
    elapse(450);
    assert!(hover_card(&page, "id1").is_some());

    page.global::<HoverCard>().set_suppressed(true);
    elapse(16);
    assert!(
        hover_card(&page, "id1").is_none(),
        "a context menu hides it"
    );
    hover(&page, 2);
    elapse(450);
    assert!(hover_card(&page, "id2").is_none(), "nor does another show");

    page.global::<HoverCard>().set_suppressed(false);
    leave(&page);
    hover(&page, 2);
    elapse(450);
    assert!(hover_card(&page, "id2").is_some());
}

#[test]
fn missing_details_are_left_out() {
    let (page, _) = hover_page(bare);
    hover(&page, 1);
    elapse(450);
    let card = hover_card(&page, "id1").unwrap_or_else(|| panic!("no hover card"));
    assert_eq!(
        texts(&card),
        ["Details id1", "Series"],
        "only the title and the kind show"
    );
    assert_eq!(progress(&card), None, "no progress for an item not started");
}
