//! The collection page plays its first unwatched movie and opens the movies of its row.

mod common;

use std::time::Duration;

use common::collection::{OVERVIEW, ROW, open_collection, scroll, start};
use common::{click, pause, run, wait_role, wait_until, wait_visible};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::Screen;
use slint::ComponentHandle;

#[test]
fn collection_play_first_unwatched() {
    let (flow, _) = start(true);
    let window = flow.window.as_weak();
    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        open_collection(&window).await?;
        let play = wait_role(&window, "Play Test Movie (2020)", AccessibleRole::Button).await?;
        let row = wait_role(&window, ROW, AccessibleRole::List).await?;
        if row.absolute_position().y < play.absolute_position().y + play.size().height {
            return Err("the row is not below the header".into());
        }
        wait_role(&window, "Mark as watched", AccessibleRole::Button).await?;
        wait_role(&window, "Add to favorites", AccessibleRole::Button).await?;
        #[cfg(not(feature = "mpv"))]
        {
            click(&window, &play);
            wait_until("the playback notice", || {
                window.get_notice().visible
                    && window.get_notice().message == oxyfin::Message::PlaybackUnavailable
            })
            .await?;
        }
        let bottom = window.window().size().height as f32 / window.window().scale_factor();
        let mut card = None;
        for _ in 0..5 {
            card = ElementHandle::find_by_element_type_name(&window, "PosterCard").find(|card| {
                card.accessible_label().as_deref() == Some("Test Movie (2020)")
                    && card.absolute_position().y + card.size().height / 2. <= bottom
            });
            if card.is_some() {
                break;
            }
            scroll(&window, -400.);
            pause(Duration::from_millis(50)).await;
        }
        let card = card.ok_or("the card of the movie is not in the window")?;
        click(&window, &card);
        wait_until("the movie page", || window.get_screen() == Screen::Movie).await?;
        wait_visible(&window, OVERVIEW).await?;
        Ok(())
    });
}
