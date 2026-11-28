//! Opens the context menu of a poster with a right click or Shift+F10, next to the pointer or
//! the card, keeps it inside the window, and closes it with Escape, a click outside or a row.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use page::{ContextMenu, HoverCard, HoverDetails, MenuAction, MenuEntry, MenuPage, PageCard};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, SharedString, VecModel};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { PosterRow } from "ui/components/poster_row.slint";
        import { HoverCard, HoverCardOverlay, HoverDetails } from "ui/components/hover_card.slint";
        import { ContextMenu, ContextMenuOverlay, MenuAction, MenuEntry } from "ui/components/context_menu.slint";
        import { CardData, Theme } from "ui/theme.slint";

        export { CardData as PageCard, ContextMenu, HoverCard, HoverDetails, MenuAction, MenuEntry }

        export component MenuPage inherits Window {
            in property <[CardData]> cards;
            out property <length> menu-width: Theme.context-menu-width;
            out property <length> row-height: 2 * Theme.space-sm + Theme.body-line-height;
            out property <length> margin: Theme.space-sm;
            out property <length> keyboard-offset: Theme.space-lg;
            callback opened(string);

            preferred-width: 1000px;
            preferred-height: 600px;
            forward-focus: keys;

            // As the app window does: the open menu takes the keys before the focused card.
            keys := FocusScope {
                capture-key-pressed(event) => {
                    if ContextMenu.open {
                        ContextMenu.key(event);
                        return accept;
                    }
                    reject
                }

                VerticalLayout {
                    alignment: start;
                    padding-top: Theme.space-lg;

                    PosterRow {
                        title: "Row";
                        items: root.cards;
                        clicked(id) => { root.opened(id); }
                    }
                }
            }

            HoverCardOverlay { }
            ContextMenuOverlay { }
        }
    }
}

const WIDTH: f32 = 1000.;
const HEIGHT: f32 = 600.;

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

fn entry(action: MenuAction, detail: &str, divider_before: bool) -> MenuEntry {
    MenuEntry {
        action,
        detail: detail.into(),
        divider_before,
    }
}

/// What the page records: the menus asked for and the rows chosen, with the opening numbers.
#[derive(Default)]
struct Log {
    asked: Vec<(i32, String)>,
    chosen: Vec<(i32, MenuAction)>,
    opened: Vec<String>,
}

/// A page answering every menu request at once with the rows of a started episode.
fn menu_page() -> (MenuPage, Rc<RefCell<Log>>) {
    i_slint_backend_testing::init_no_event_loop();
    let page = MenuPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.set_cards(ModelRc::new(VecModel::from(
        (0..20).map(card).collect::<Vec<_>>(),
    )));
    page.window()
        .set_size(slint::LogicalSize::new(WIDTH, HEIGHT));
    let log = Rc::new(RefCell::new(Log::default()));
    let weak = page.as_weak();
    let asked = Rc::clone(&log);
    page.global::<ContextMenu>()
        .on_requested(move |serial, id, _| {
            asked.borrow_mut().asked.push((serial, id.to_string()));
            let Some(page) = weak.upgrade() else { return };
            let menu = page.global::<ContextMenu>();
            menu.set_title(SharedString::from(format!("Title {id}")));
            menu.set_subtitle("S02E05 \u{b7} The Pact".into());
            menu.set_entries(ModelRc::new(VecModel::from(vec![
                entry(MenuAction::Resume, "18:42", true),
                entry(MenuAction::PlayFromStart, "", false),
                entry(MenuAction::MarkWatched, "", true),
                entry(MenuAction::GoToSeason, "2", true),
            ])));
            menu.set_ready_serial(serial);
        });
    let hovered = page.as_weak();
    page.global::<HoverCard>().on_requested(move |id, _| {
        if let Some(page) = hovered.upgrade() {
            page.global::<HoverCard>().set_details(HoverDetails {
                id: id.clone(),
                title: format!("Hover {id}").into(),
                ..HoverDetails::default()
            });
        }
    });
    let chosen = Rc::clone(&log);
    page.global::<ContextMenu>()
        .on_chosen(move |serial, action| chosen.borrow_mut().chosen.push((serial, action)));
    let opened = Rc::clone(&log);
    page.on_opened(move |id| opened.borrow_mut().opened.push(id.to_string()));
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    (page, log)
}

fn find(page: &MenuPage, label: &str, role: AccessibleRole) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(page, label).find(|element| {
        let size = element.size();
        element.accessible_role() == Some(role) && size.width > 0. && size.height > 0.
    })
}

fn poster(page: &MenuPage, index: usize) -> ElementHandle {
    find(page, &format!("Poster {index}"), AccessibleRole::Button)
        .unwrap_or_else(|| panic!("no poster {index}"))
}

fn menu(page: &MenuPage, index: usize) -> Option<ElementHandle> {
    find(page, &format!("Title id{index}"), AccessibleRole::List)
}

fn row(page: &MenuPage, label: &str) -> ElementHandle {
    find(page, label, AccessibleRole::Button).unwrap_or_else(|| panic!("no row {label}"))
}

fn press(page: &MenuPage, button: PointerEventButton, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed { position, button },
        WindowEvent::PointerReleased { position, button },
    ] {
        page.window().dispatch_event(event);
    }
}

fn right_click(page: &MenuPage, index: usize, dx: f32, dy: f32) -> (f32, f32) {
    let at = poster(page, index).absolute_position();
    let (x, y) = (at.x + dx, at.y + dy);
    press(page, PointerEventButton::Right, x, y);
    (x, y)
}

fn key(page: &MenuPage, text: SharedString) {
    page.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    page.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn special(page: &MenuPage, which: Key) {
    key(page, which.into());
}

fn shift_f10(page: &MenuPage) {
    let shift: SharedString = Key::Shift.into();
    page.window().dispatch_event(WindowEvent::KeyPressed {
        text: shift.clone(),
    });
    special(page, Key::F10);
    page.window()
        .dispatch_event(WindowEvent::KeyReleased { text: shift });
}

fn is_open(page: &MenuPage) -> bool {
    page.global::<ContextMenu>().get_open()
}

#[test]
fn a_right_click_opens_the_menu_at_the_pointer() {
    let (page, log) = menu_page();
    let (x, y) = right_click(&page, 1, 30., 40.);

    assert_eq!(log.borrow().asked, [(1, "id1".to_owned())]);
    let panel = menu(&page, 1).unwrap_or_else(|| panic!("no menu"));
    let at = panel.absolute_position();
    assert!((at.x - x).abs() < 0.5 && (at.y - y).abs() < 0.5, "{at:?}");
    assert!((panel.size().width - page.get_menu_width()).abs() < 0.5);
    for label in [
        "Resume from 18:42",
        "Play from the beginning",
        "Mark as watched",
        "Go to season 2",
    ] {
        let height = row(&page, label).size().height;
        assert!(
            (height - page.get_row_height()).abs() < 0.5,
            "{label} is {height} px high"
        );
    }
    assert!(
        page.global::<HoverCard>().get_suppressed(),
        "no hover card while the menu is open"
    );
}

#[test]
fn the_menu_stays_inside_the_window() {
    let (page, _) = menu_page();
    page.window().set_size(slint::LogicalSize::new(WIDTH, 300.));
    let at = poster(&page, 5).absolute_position();
    let (x, y) = (WIDTH - 20., at.y + 200.);
    press(&page, PointerEventButton::Right, x, y);

    let panel = menu(&page, 5).unwrap_or_else(|| panic!("no menu"));
    let (position, size) = (panel.absolute_position(), panel.size());
    assert!(
        (position.x + size.width - x).abs() < 0.5,
        "it opens left of the pointer near the right edge"
    );
    assert!(
        position.y + size.height <= 300. - page.get_margin() + 0.5,
        "it ends {} px above the bottom edge",
        page.get_margin()
    );
    assert!(position.y >= page.get_margin() - 0.5);
}

#[test]
fn escape_closes_the_menu() {
    let (page, log) = menu_page();
    right_click(&page, 2, 30., 40.);
    assert!(menu(&page, 2).is_some());

    special(&page, Key::Escape);
    assert!(!is_open(&page));
    assert!(menu(&page, 2).is_none());
    assert!(!page.global::<HoverCard>().get_suppressed());
    assert!(log.borrow().chosen.is_empty());
}

#[test]
fn a_click_outside_closes_the_menu_only() {
    let (page, log) = menu_page();
    right_click(&page, 1, 30., 40.);
    let other = poster(&page, 4).absolute_position();
    press(
        &page,
        PointerEventButton::Left,
        other.x + 20.,
        other.y + 20.,
    );

    assert!(!is_open(&page));
    assert!(
        log.borrow().opened.is_empty(),
        "the click closing the menu does not open the card below"
    );
    assert!(log.borrow().chosen.is_empty());
}

#[test]
fn clicking_a_row_chooses_it_and_closes_the_menu() {
    let (page, log) = menu_page();
    right_click(&page, 1, 30., 40.);
    let target = row(&page, "Mark as watched");
    let at = target.absolute_position();
    press(&page, PointerEventButton::Left, at.x + 40., at.y + 10.);

    assert_eq!(log.borrow().chosen, [(1, MenuAction::MarkWatched)]);
    assert!(!is_open(&page));
    assert!(log.borrow().opened.is_empty());
}

#[test]
fn shift_f10_opens_the_menu_of_the_focused_card() {
    let (page, log) = menu_page();
    let card = poster(&page, 2);
    let at = card.absolute_position();
    // A Return on the focused card opens it: the card has the focus.
    press(&page, PointerEventButton::Left, at.x + 20., at.y + 20.);
    assert_eq!(log.borrow().opened, ["id2"]);

    shift_f10(&page);
    assert_eq!(log.borrow().asked, [(1, "id2".to_owned())]);
    let panel = menu(&page, 2).unwrap_or_else(|| panic!("no menu"));
    let offset = page.get_keyboard_offset();
    let position = panel.absolute_position();
    assert!(
        (position.x - at.x - offset).abs() < 0.5 && (position.y - at.y - offset).abs() < 0.5,
        "the menu opens over the card: {position:?}"
    );

    special(&page, Key::DownArrow);
    special(&page, Key::Return);
    assert_eq!(
        log.borrow().chosen,
        [(1, MenuAction::PlayFromStart)],
        "the first row is picked when opened with the keyboard, then the arrows move"
    );
    assert!(!is_open(&page));

    special(&page, Key::Return);
    assert_eq!(
        log.borrow().opened,
        ["id2", "id2"],
        "the card has the focus back"
    );
}

#[test]
fn the_menu_key_opens_and_closes_the_menu() {
    let (page, log) = menu_page();
    let at = poster(&page, 3).absolute_position();
    press(&page, PointerEventButton::Left, at.x + 20., at.y + 20.);
    special(&page, Key::Menu);
    assert!(menu(&page, 3).is_some());
    special(&page, Key::Menu);
    assert!(!is_open(&page));
    assert!(log.borrow().chosen.is_empty());
}

fn hover_card_shown(page: &MenuPage, index: usize) -> bool {
    find(page, &format!("Hover id{index}"), AccessibleRole::Groupbox).is_some()
}

fn move_to(page: &MenuPage, x: f32, y: f32) {
    page.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(x, y),
    });
}

fn elapse(ms: u64) {
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(ms));
}

#[test]
fn the_hover_card_waits_for_the_pointer_to_move_after_the_menu() {
    let (page, _) = menu_page();
    let (x, y) = right_click(&page, 1, 30., 40.);
    special(&page, Key::Escape);
    // Slint sends the pointer again where it rests once the overlay is gone.
    move_to(&page, x, y);
    elapse(1000);
    assert!(
        !hover_card_shown(&page, 1),
        "the hover card comes back under a resting pointer"
    );

    move_to(&page, x + 6., y);
    elapse(1000);
    assert!(hover_card_shown(&page, 1), "a move shows it again");
}
