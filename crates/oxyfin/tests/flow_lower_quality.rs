//! A stream that keeps stopping to load offers a lower quality in the notice; the quality
//! changes only when the user takes it.
#![cfg(feature = "mpv")]

mod common;

use std::rc::Rc;

use common::{click, run, start, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Message;
use oxyfin::screens::player;
use oxyfin_core::{PlaybackEvent, QUALITIES, Quality};
use slint::ComponentHandle;

#[test]
fn a_lower_quality_is_offered_in_the_notice() {
    let flow = start(|mock| async { mock });
    let window = flow.window.as_weak();
    let app = Rc::clone(&flow.app);

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        player::show_quality(&window, Quality::Original);
        app.show_playback(PlaybackEvent::LowerQualityAdvised(QUALITIES[3]));
        let notice = window.get_notice();
        if !notice.visible || notice.message != Message::SlowStream {
            return Err(format!("no offer: {notice:?}"));
        }
        if window.get_player_quality() != 0 {
            return Err("the quality changed by itself".into());
        }
        let take = wait_role(&window, "Lower quality", AccessibleRole::Button).await?;
        click(&window, &take);
        wait_until("the notice to leave", || !window.get_notice().visible).await?;
        if window.get_player_quality() != 3 {
            return Err(format!("quality {}", window.get_player_quality()));
        }
        Ok(())
    });
}
