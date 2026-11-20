//! Box sets: the movie page links to its collection, whose page lists the movies in premiere
//! order with their count, years and watched count, and Escape goes back to the movie.

mod common;

use common::collection::{CAPTION, LINK, ROW, open_collection, press_escape, row_titles, start};
use common::{run, wait_role, wait_until, wait_visible};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;

#[test]
fn movie_shows_collection_link() {
    let (flow, _) = start(true);
    let window = flow.window.as_weak();
    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        open_collection(&window).await?;
        let watched = wait_role(&window, "Mark as watched", AccessibleRole::Button).await?;
        let heart = wait_role(&window, "Add to favorites", AccessibleRole::Button).await?;
        let middle = |element: &i_slint_backend_testing::ElementHandle| {
            element.absolute_position().y + element.size().height / 2.
        };
        if (middle(&watched) - middle(&heart)).abs() > 1.
            || heart.absolute_position().x < watched.absolute_position().x
        {
            return Err("the favorite button is not on the line of the buttons".into());
        }
        wait_visible(&window, "Northbound Collection").await?;
        wait_visible(&window, CAPTION).await?;
        let row = wait_role(&window, ROW, AccessibleRole::List).await?;
        let titles = row_titles(&window, &row);
        if titles != ["The Long Coast", "Test Movie (2020)", "The Surveyor"] {
            return Err(format!("unexpected collection order: {titles:?}"));
        }
        press_escape(&window);
        wait_until("the movie page again", || {
            window.get_screen() == Screen::Movie
        })
        .await?;
        wait_role(&window, LINK, AccessibleRole::Button).await?;
        Ok(())
    });
}
