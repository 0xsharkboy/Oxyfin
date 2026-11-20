//! A box set found by the search opens its collection page.

mod common;

use common::collection::{press, start};
use common::{click, run, wait_role, wait_until, wait_visible};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;

#[test]
fn search_opens_collection() {
    let (flow, _) = start(true);
    let window = flow.window.as_weak();
    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let magnifier = wait_role(&window, "Search", AccessibleRole::Button).await?;
        click(&window, &magnifier);
        wait_role(&window, "Search titles", AccessibleRole::TextInput).await?;
        for character in "north".chars() {
            press(&window, character.to_string());
        }
        let card = wait_visible(&window, "Northbound Collection").await?;
        click(&window, &card);
        wait_until("the collection page", || {
            window.get_screen() == Screen::Collection
        })
        .await?;
        wait_role(&window, "Play Test Movie (2020)", AccessibleRole::Button).await?;
        Ok(())
    });
}
