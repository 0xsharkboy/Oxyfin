//! The home banner shows the episode to resume, with its series, and opens the series.

mod common;

use common::home_banner::{EPISODE_TO_RESUME, mount_banner};
use common::{click, exists, run, save_session, start_with, wait_role, wait_until, wait_visible};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;

#[test]
fn the_banner_shows_the_episode_to_resume_and_opens_its_series() {
    let flow = start_with(
        |mock| mount_banner(mock, EPISODE_TO_RESUME, "[]"),
        save_session,
    );
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_visible(&window, "CONTINUE WATCHING").await?;
        wait_visible(&window, "Northern Saga").await?;
        wait_visible(&window, "Series").await?;
        wait_visible(&window, "S01E02 \u{b7} Sword \u{b7} 24 min").await?;
        wait_visible(&window, "12 min left").await?;
        wait_visible(&window, "A raid on the coast.").await?;
        let progress = wait_role(&window, "12 min left", AccessibleRole::ProgressIndicator).await?;
        if progress.size().width != 240. {
            return Err(format!(
                "the progress bar is {} wide",
                progress.size().width
            ));
        }
        wait_role(&window, "Resume S01E02", AccessibleRole::Button).await?;
        if exists(&window, "Play") || exists(&window, "RECENTLY ADDED") {
            return Err("the banner still shows a recent item".into());
        }

        let more = wait_role(&window, "More info", AccessibleRole::Button).await?;
        click(&window, &more);
        wait_until("the series", || window.get_screen() == Screen::Series).await?;
        Ok(())
    });
}
