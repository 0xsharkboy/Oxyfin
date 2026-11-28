//! The offer of a lower quality leaves with the item it was made for, and when the user picks a
//! quality: a stale offer never applies its quality.
#![cfg(feature = "mpv")]

mod common;

use std::rc::Rc;

use common::{run, start};
use oxyfin::Message;
use oxyfin::screens::player;
use oxyfin_core::{PlaybackEvent, PlaybackTitle, QUALITIES, Quality};
use slint::ComponentHandle;

fn offered(window: &oxyfin::AppWindow) -> bool {
    let notice = window.get_notice();
    notice.visible && notice.message == Message::SlowStream
}

#[test]
fn a_stale_offer_of_a_lower_quality_leaves() {
    let flow = start(|mock| async { mock });
    let window = flow.window.as_weak();
    let app = Rc::clone(&flow.app);

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        player::show_quality(&window, Quality::Original);
        app.show_playback(PlaybackEvent::LowerQualityAdvised(QUALITIES[3]));
        if !offered(&window) {
            return Err("no offer".into());
        }
        window.invoke_player_command_set_quality(5);
        if offered(&window) {
            return Err("the offer stays after the user picked a quality".into());
        }

        app.show_playback(PlaybackEvent::LowerQualityAdvised(QUALITIES[3]));
        if !offered(&window) {
            return Err("no second offer".into());
        }
        app.show_playback(PlaybackEvent::Loading {
            item: "0000000000000000000000000000000b".into(),
            title: PlaybackTitle {
                title: "Other".into(),
                series_name: None,
                episode_label: None,
                year: None,
                artwork: None,
            },
        });
        if offered(&window) {
            return Err("the offer stays on another item".into());
        }
        if window.get_player_quality() != 5 {
            return Err(format!("quality {}", window.get_player_quality()));
        }
        Ok(())
    });
}
