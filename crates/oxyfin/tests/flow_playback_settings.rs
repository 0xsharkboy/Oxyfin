//! Playback settings: the next episode and segment rows save the choices for
//! the next playback.

mod common;

use std::time::Duration;

use common::playback_settings::{WIDTH, checked, chip, open_settings, saved};
use common::{click, wait_until};
use oxyfin_core::PlaybackPrefs;

/// Width of the settings column.
const COLUMN: f32 = 640.;

#[test]
fn settings_playback_rows_save() {
    open_settings(async |window, paths| {
        if !checked(window, "Automatically", 0)
            || !checked(window, "10 s", 0)
            || !checked(window, "Show a skip button", 0)
            || !checked(window, "Show a skip button", 1)
            || !checked(window, "Remember per series", 0)
        {
            return Err("the playback rows do not show the defaults".into());
        }
        let right = (WIDTH + COLUMN) / 2.;
        for (label, nth) in [("Skip automatically", 1), ("Remember per series", 0)] {
            let chip = chip(window, label, nth)?;
            let end = chip.absolute_position().x + chip.size().width;
            if end > right {
                return Err(format!("{label} ends at {end}, past the column at {right}"));
            }
        }

        click(window, &chip(window, "30 s", 0)?);
        wait_until("the countdown saved", || {
            saved(paths).is_some_and(|prefs| prefs.countdown == Duration::from_secs(30))
        })
        .await?;
        click(window, &chip(window, "Skip automatically", 0)?);
        wait_until("the intros saved", || {
            saved(paths).is_some_and(|prefs| prefs.skip_intros && !prefs.skip_recaps)
        })
        .await?;
        click(window, &chip(window, "Skip automatically", 1)?);
        wait_until("the recaps saved", || {
            saved(paths).is_some_and(|prefs| prefs.skip_intros && prefs.skip_recaps)
        })
        .await?;
        click(window, &chip(window, "Server defaults", 0)?);
        wait_until("the tracks choice saved", || {
            saved(paths).is_some_and(|prefs| !prefs.remember_tracks)
        })
        .await?;
        click(window, &chip(window, "Ask", 0)?);
        wait_until("the ask mode saved", || {
            saved(paths).is_some_and(|prefs| !prefs.autoplay_next)
        })
        .await?;
        wait_until("the chips follow", || {
            checked(window, "Ask", 0)
                && checked(window, "30 s", 0)
                && checked(window, "Skip automatically", 0)
                && checked(window, "Skip automatically", 1)
                && checked(window, "Server defaults", 0)
        })
        .await?;
        let expected = PlaybackPrefs {
            autoplay_next: false,
            countdown: Duration::from_secs(30),
            skip_intros: true,
            skip_recaps: true,
            remember_tracks: false,
        };
        match saved(paths) {
            Some(prefs) if prefs == expected => Ok(()),
            other => Err(format!("saved {other:?}")),
        }
    });
}
