//! Instantiates every base component and drives a button through its accessible action.

use std::cell::Cell;
use std::rc::Rc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Message, Messages};
use page::ComponentsPage;
use slint::ComponentHandle;

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { Button, ButtonStyle } from "ui/components/button.slint";
        import { IconButton } from "ui/components/icon_button.slint";
        import { NavItem } from "ui/components/nav_item.slint";
        import { TextField } from "ui/components/text_field.slint";
        import { Chip } from "ui/components/chip.slint";
        import { ProgressBar } from "ui/components/progress_bar.slint";
        import { PosterCard } from "ui/components/poster_card.slint";
        import { LandscapeCard } from "ui/components/landscape_card.slint";
        import { CardStatus } from "ui/components/card.slint";
        import { TopBar } from "ui/components/top_bar.slint";
        import { Banner } from "ui/components/banner.slint";
        import { Icons } from "ui/icons.slint";

        export component ComponentsPage inherits Window {
            callback sign-in-clicked();

            VerticalLayout {
                TopBar { }
                Banner { title: "Title"; meta: "2024"; }
                HorizontalLayout {
                    Button { label: "Sign in"; clicked => { root.sign-in-clicked(); } }
                    Button { label: "Secondary"; style: ButtonStyle.secondary; }
                    Button { label: "Ghost"; style: ButtonStyle.ghost; icon: Icons.play; }
                    IconButton { icon: Icons.search; label: "Search"; }
                    NavItem { label: "Home"; active: true; }
                    Chip { label: "Sci-Fi"; selected: true; }
                }
                TextField { label: "Server address"; }
                ProgressBar { value: 0.5; }
                HorizontalLayout {
                    PosterCard { title: "Movie"; subtitle: "2024"; status: CardStatus.watched; }
                    LandscapeCard { title: "Series"; subtitle: "S02E05"; status: CardStatus.in-progress; progress: 0.75; }
                }
            }
        }
    }
}

#[test]
fn button_runs_its_accessible_action_and_messages_are_translated() {
    i_slint_backend_testing::init_no_event_loop();

    let page = ComponentsPage::new().unwrap();
    let clicks = Rc::new(Cell::new(0));
    let counter = Rc::clone(&clicks);
    page.on_sign_in_clicked(move || counter.set(counter.get() + 1));

    let buttons: Vec<_> = ElementHandle::find_by_accessible_label(&page, "Sign in")
        .filter(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .collect();
    assert_eq!(buttons.len(), 1);
    buttons[0].invoke_accessible_default_action();
    assert_eq!(clicks.get(), 1);

    let window = AppWindow::new().unwrap();
    let text = window
        .global::<Messages>()
        .invoke_message_text(Message::SessionExpired, "".into());
    assert!(!text.is_empty());
}
