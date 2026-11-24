//! After a row of a menu chosen from the keyboard, the focus goes back to the element that had
//! it, never away from a control taken since, and the window stops watching for it.

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, CardData, HomeRow, HomeRowKind, Screen};
use page::{MenuPage, PageCard};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { PosterRow } from "ui/components/poster_row.slint";
        import { ContextMenu, MenuAction } from "ui/components/context_menu.slint";
        import { CardData } from "ui/theme.slint";

        export { CardData as PageCard, ContextMenu, MenuAction }

        export component MenuPage inherits Window {
            in property <[CardData]> first;
            in property <[CardData]> second;
            callback opened(string);

            width: 1000px;
            height: 900px;

            VerticalLayout {
                alignment: start;

                PosterRow {
                    title: "First";
                    items: root.first;
                    clicked(id) => { root.opened("first"); }
                }
                PosterRow {
                    title: "Second";
                    items: root.second;
                    clicked(id) => { root.opened("second"); }
                }
            }
        }
    }
}

fn cards(no_menu: bool) -> ModelRc<PageCard> {
    ModelRc::new(VecModel::from(vec![PageCard {
        id: "same".into(),
        title: "Twice".into(),
        no_menu,
        ..Default::default()
    }]))
}

fn press(window: &impl ComponentHandle, key: Key) {
    let text = slint::SharedString::from(key);
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn menu_page() -> (MenuPage, Rc<RefCell<Vec<String>>>) {
    i_slint_backend_testing::init_no_event_loop();
    let page = MenuPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.set_first(cards(false));
    page.set_second(cards(false));
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let record = Rc::clone(&opened);
    page.on_opened(move |row| record.borrow_mut().push(row.to_string()));
    (page, opened)
}

fn card(page: &MenuPage, nth: usize) -> ElementHandle {
    ElementHandle::find_by_accessible_label(page, "Twice")
        .nth(nth)
        .unwrap_or_else(|| panic!("no card {nth}"))
}

#[test]
fn only_the_card_that_had_the_focus_takes_it_back() {
    let (page, opened) = menu_page();
    common::click(&page, &card(&page, 0));
    press(&page, Key::Menu);
    let menu = page.global::<page::ContextMenu>();
    assert!(menu.get_open(), "the menu key opens the menu");
    menu.invoke_choose(page::MenuAction::AddFavorite);
    // The change builds both rows again.
    page.set_first(cards(false));
    page.set_second(cards(false));
    let _ = (card(&page, 0), card(&page, 1));
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(100));
    press(&page, Key::Return);
    assert_eq!(*opened.borrow(), ["first", "first"]);
}

#[test]
fn a_card_without_a_menu_opens_none() {
    let (page, _) = menu_page();
    page.set_first(cards(true));
    let shown = Rc::new(Cell::new(0));
    let count = Rc::clone(&shown);
    page.global::<page::ContextMenu>()
        .on_requested(move |_, _, _| count.set(count.get() + 1));
    let at = card(&page, 0).absolute_position();
    let position = LogicalPosition::new(at.x + 20., at.y + 20.);
    let right_click = || {
        for event in [
            WindowEvent::PointerMoved { position },
            WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Right,
            },
            WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Right,
            },
        ] {
            page.window().dispatch_event(event);
        }
    };
    right_click();
    common::click(&page, &card(&page, 0));
    press(&page, Key::Menu);
    assert_eq!(shown.get(), 0, "a collection has no menu");
    page.set_first(cards(false));
    let _ = card(&page, 0);
    right_click();
    assert_eq!(shown.get(), 1);
}

fn home_window() -> (AppWindow, Rc<Cell<u32>>) {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap_or_else(|err| panic!("cannot create the window: {err}"));
    window
        .show()
        .unwrap_or_else(|err| panic!("cannot show the window: {err}"));
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    window.set_screen(Screen::Home);
    // The screen change forgets what had the focus, on the next turn.
    elapse(1);
    let tabs = Rc::new(Cell::new(0));
    let count = Rc::clone(&tabs);
    window.on_open_tab(move |_| count.set(count.get() + 1));
    (window, tabs)
}

fn elapse(steps: u32) {
    for _ in 0..steps {
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(250));
    }
}

#[test]
fn a_top_bar_control_taken_since_keeps_the_focus() {
    let (window, tabs) = home_window();
    let menu = window.global::<oxyfin::ContextMenu>();
    // The element to give the focus back to is gone.
    menu.set_focus_id("gone".into());
    let tab = ElementHandle::find_by_accessible_label(&window, "Movies")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Tab))
        .unwrap_or_else(|| panic!("no Movies tab"));
    common::click(&window, &tab);
    assert_eq!(tabs.get(), 1);
    elapse(4);
    press(&window, Key::Return);
    assert_eq!(tabs.get(), 2, "the tab lost the focus");
}

#[test]
fn the_window_stops_watching_for_the_focus() {
    let (window, _) = home_window();
    let menu = window.global::<oxyfin::ContextMenu>();
    menu.set_focus_id("there".into());
    menu.set_focus_answer(menu.get_focus_check());
    for _ in 0..200 {
        // An element holding the focus answers each time.
        elapse(1);
        menu.set_focus_answer(menu.get_focus_check());
    }
    assert_eq!(menu.get_focus_id(), "", "still watched after 50 s");
}

fn home_rows() -> ModelRc<HomeRow> {
    ModelRc::new(VecModel::from(vec![HomeRow {
        title: "Recently Added".into(),
        kind: HomeRowKind::Poster,
        items: ModelRc::new(VecModel::from(vec![CardData {
            id: "there".into(),
            title: "There".into(),
            ..CardData::default()
        }])),
        ..HomeRow::default()
    }]))
}

/// A home screen whose card had a row of its menu chosen from the keyboard, with the number of
/// times the card was opened.
fn chosen_from_a_card() -> (AppWindow, Rc<Cell<u32>>) {
    let (window, _) = home_window();
    window.set_home_rows(home_rows());
    let opened = Rc::new(Cell::new(0));
    let count = Rc::clone(&opened);
    window.on_open_item(move |_| count.set(count.get() + 1));
    common::click(&window, &home_card(&window));
    assert_eq!(opened.replace(0), 1, "the card opens");
    press(&window, Key::Menu);
    let menu = window.global::<oxyfin::ContextMenu>();
    assert!(menu.get_open(), "the menu key opens the menu");
    menu.invoke_choose(oxyfin::MenuAction::AddFavorite);
    (window, opened)
}

fn home_card(window: &AppWindow) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, "There")
        .next()
        .unwrap_or_else(|| panic!("no card"))
}

/// Builds the rows again, as an answer of the server does.
fn rebuild(window: &AppWindow) {
    window.set_home_rows(home_rows());
    let _ = home_card(window);
}

#[test]
fn every_rebuild_gives_the_focus_back() {
    let (window, opened) = chosen_from_a_card();
    // The change reloads the rows, then their images or a second source build them again.
    for _ in 0..3 {
        rebuild(&window);
        elapse(2);
    }
    press(&window, Key::Return);
    assert_eq!(opened.get(), 1, "a rebuilt card lost the focus");
}

#[test]
fn the_watch_starts_to_end_once_the_server_answered() {
    let (window, opened) = chosen_from_a_card();
    let menu = window.global::<oxyfin::ContextMenu>();
    menu.invoke_change_sent();
    // A slow server: longer than the watch lasts.
    elapse(100);
    menu.invoke_change_answered();
    rebuild(&window);
    press(&window, Key::Return);
    assert_eq!(opened.get(), 1, "the answer came after the watch ended");
    elapse(100);
    assert_eq!(menu.get_focus_id(), "", "still watched after the answer");
}
