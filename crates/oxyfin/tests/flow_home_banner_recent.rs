//! The home banner shows a recent movie, to play, when nothing is started.

mod common;

use common::home_banner::{EMPTY, RECENT_MOVIE, mount_banner};
use common::{exists, run, save_session, start_with, wait_role, wait_until, wait_visible};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{BannerKind, Screen};
use slint::ComponentHandle;

#[test]
fn the_banner_shows_a_recent_movie_when_nothing_is_started() {
    let flow = start_with(|mock| mount_banner(mock, EMPTY, RECENT_MOVIE), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_visible(&window, "RECENTLY ADDED").await?;
        wait_visible(&window, "Movie").await?;
        wait_visible(&window, "2020 \u{b7} 1 h 40 min").await?;
        wait_role(&window, "Play", AccessibleRole::Button).await?;
        let banner = window.get_home_banner();
        if banner.resuming || banner.kind != BannerKind::Movie {
            return Err("the recent movie is shown as one to resume".into());
        }
        if exists(&window, "CONTINUE WATCHING") || exists(&window, "Resume") {
            return Err("the banner offers to resume a movie never started".into());
        }
        Ok(())
    });
}
