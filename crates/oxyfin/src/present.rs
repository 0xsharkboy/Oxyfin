//! Conversions from core models and errors to what the screens display.

use std::time::Duration;

use oxyfin_core::{
    AudioOption, CoreError, Episode, ImageSize, ItemKind, ItemSummary, PlaybackError,
    PlaybackTitle, SubtitleOption,
};
use slint::SharedString;

use crate::state::Screen;
use crate::{CardData, Message, TrackRow};

/// Message and detail to show for a core error.
///
/// The detail is only filled for [`Message::UnsupportedServer`], as
/// `"<found> / <minimum>"`. Expired sessions go through the state machine
/// before reaching this function.
pub fn core_error_message(error: &CoreError) -> (Message, SharedString) {
    let message = match error {
        CoreError::SessionExpired => Message::SessionExpired,
        CoreError::Network(_) => Message::ServerUnreachable,
        CoreError::UnsupportedServer { found, minimum } => {
            return (
                Message::UnsupportedServer,
                format!("{found} / {minimum}").into(),
            );
        }
        CoreError::InvalidServerUrl(_) => Message::InvalidServerUrl,
        CoreError::InvalidCredentials => Message::InvalidCredentials,
        CoreError::QuickConnectDisabled => Message::QuickConnectDisabled,
        CoreError::QuickConnectExpired => Message::QuickConnectExpired,
        CoreError::Server { .. } => Message::ServerError,
        CoreError::Decode(_) => Message::UnexpectedResponse,
        CoreError::Storage(_) => Message::StorageError,
        CoreError::Player(_) => Message::CommandFailed,
    };
    (message, SharedString::default())
}

/// Message to show for a playback error. The player detail is never rendered.
pub fn playback_error_message(error: &PlaybackError) -> Message {
    match error {
        PlaybackError::NoDirectPlaySource => Message::NoDirectPlay,
        PlaybackError::NothingToPlay => Message::NothingToPlay,
        PlaybackError::LoadFailed { .. } => Message::LoadFailed,
        PlaybackError::Network => Message::ServerUnreachable,
        PlaybackError::Server { .. } => Message::ServerError,
        PlaybackError::Command { .. } => Message::CommandFailed,
    }
}

/// Formats a duration as `m:ss`, or `h:mm:ss` from one hour.
pub fn clock(duration: Duration) -> SharedString {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}").into()
    } else {
        format!("{minutes}:{seconds:02}").into()
    }
}

/// Pixel size to request for an image shown at a logical size, rounded up.
pub fn image_size(logical_width: f32, logical_height: f32, scale: f32) -> ImageSize {
    let pixels = |logical: f32| (logical * scale).ceil().max(1.0) as u32;
    ImageSize {
        max_width: pixels(logical_width),
        max_height: pixels(logical_height),
    }
}

/// Detail page an item opens: movies and videos their own, series their own, episodes their series.
pub fn detail_target(item: &ItemSummary) -> Option<Screen> {
    match item.kind {
        ItemKind::Movie | ItemKind::Video => Some(Screen::Movie(item.id.as_str().to_owned())),
        ItemKind::Series => Some(Screen::Series(item.id.as_str().to_owned())),
        ItemKind::Episode => item
            .series_id
            .as_ref()
            .map(|series| Screen::Series(series.as_str().to_owned())),
    }
}

/// Card for an item. The image is loaded later.
pub fn card(item: &ItemSummary) -> CardData {
    let subtitle = match (item.kind, &item.series_name, &item.episode_label) {
        (ItemKind::Episode, Some(series), Some(label)) => format!("{series} \u{b7} {label}"),
        (ItemKind::Episode, None, Some(label)) => label.clone(),
        _ => item.year.map(|year| year.to_string()).unwrap_or_default(),
    };
    CardData {
        id: item.id.as_str().into(),
        title: item.title.as_str().into(),
        subtitle: subtitle.into(),
        progress: item.user.progress.unwrap_or(0.0),
        watched: item.user.played,
        image: Default::default(),
        has_image: false,
    }
}

fn channel_layout(channels: u32) -> String {
    match channels {
        1 => "1.0".into(),
        2 => "2.0".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n}ch"),
    }
}

fn join_detail(parts: impl IntoIterator<Item = String>) -> SharedString {
    parts
        .into_iter()
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
        .into()
}

/// Rows of the audio menu.
pub fn audio_rows(options: &[AudioOption]) -> Vec<TrackRow> {
    options
        .iter()
        .map(|option| TrackRow {
            index: option.index,
            label: option.label.as_str().into(),
            detail: join_detail(
                option
                    .language
                    .clone()
                    .into_iter()
                    .chain(option.channels.map(channel_layout)),
            ),
            forced: false,
            external: false,
            selected: option.selected,
        })
        .collect()
}

/// Rows of the subtitle menu. The first row, with index `-1` and an empty
/// label, switches subtitles off: the menu renders its translated label.
pub fn subtitle_rows(options: &[SubtitleOption]) -> Vec<TrackRow> {
    let off = TrackRow {
        index: -1,
        label: SharedString::default(),
        detail: SharedString::default(),
        forced: false,
        external: false,
        selected: options.iter().all(|option| !option.selected),
    };
    std::iter::once(off)
        .chain(options.iter().map(|option| TrackRow {
            index: option.index,
            label: option.label.as_str().into(),
            detail: option.language.as_deref().unwrap_or_default().into(),
            forced: option.forced,
            external: option.external,
            selected: option.selected,
        }))
        .collect()
}

/// Title and subtitle of the player: the series and `S01E02 · episode title`
/// for an episode, the title alone otherwise.
pub fn playback_title(title: &PlaybackTitle) -> (SharedString, SharedString) {
    match &title.series_name {
        Some(series) => {
            let subtitle = join_detail(
                title
                    .episode_label
                    .clone()
                    .into_iter()
                    .chain(std::iter::once(title.title.clone())),
            );
            (series.as_str().into(), subtitle)
        }
        None => (title.title.as_str().into(), SharedString::default()),
    }
}

/// Title of the next episode toast: `S01E02 · episode title`, or the title alone.
pub fn next_up_title(episode: &Episode) -> SharedString {
    join_detail(
        episode
            .label
            .clone()
            .into_iter()
            .chain(std::iter::once(episode.title.clone())),
    )
}

/// Name this device registers with the server.
pub fn device_name(hostname: Option<&str>, device_id: &str) -> String {
    match hostname.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => name.to_owned(),
        None => format!("Oxyfin {}", device_id.chars().take(6).collect::<String>()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyfin_core::UserState;

    fn summary(kind: ItemKind) -> ItemSummary {
        ItemSummary {
            id: "abc".into(),
            kind,
            title: "Title".into(),
            year: Some(1999),
            series_id: None,
            series_name: None,
            episode_label: None,
            poster: None,
            backdrop: None,
            thumb: None,
            duration: None,
            user: UserState::default(),
        }
    }

    #[test]
    fn every_core_error_has_a_message() {
        let cases = [
            (CoreError::SessionExpired, Message::SessionExpired),
            (CoreError::Network("x".into()), Message::ServerUnreachable),
            (
                CoreError::InvalidServerUrl("x".into()),
                Message::InvalidServerUrl,
            ),
            (CoreError::InvalidCredentials, Message::InvalidCredentials),
            (
                CoreError::QuickConnectDisabled,
                Message::QuickConnectDisabled,
            ),
            (CoreError::QuickConnectExpired, Message::QuickConnectExpired),
            (CoreError::Server { status: 500 }, Message::ServerError),
            (CoreError::Decode("x".into()), Message::UnexpectedResponse),
            (CoreError::Storage("x".into()), Message::StorageError),
            (CoreError::Player("x".into()), Message::CommandFailed),
        ];
        for (error, expected) in cases {
            let (message, detail) = core_error_message(&error);
            assert_eq!(message, expected);
            assert!(detail.is_empty(), "{error:?} leaked a detail");
        }
    }

    #[test]
    fn unsupported_server_detail_has_both_versions() {
        let error = CoreError::UnsupportedServer {
            found: "10.8.0".into(),
            minimum: "10.10.0".into(),
        };
        let (message, detail) = core_error_message(&error);
        assert_eq!(message, Message::UnsupportedServer);
        assert_eq!(detail, "10.8.0 / 10.10.0");
    }

    #[test]
    fn every_playback_error_has_a_message() {
        let cases = [
            (PlaybackError::NoDirectPlaySource, Message::NoDirectPlay),
            (PlaybackError::NothingToPlay, Message::NothingToPlay),
            (
                PlaybackError::LoadFailed { detail: "x".into() },
                Message::LoadFailed,
            ),
            (PlaybackError::Network, Message::ServerUnreachable),
            (PlaybackError::Server { status: 500 }, Message::ServerError),
            (
                PlaybackError::Command { detail: "x".into() },
                Message::CommandFailed,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(playback_error_message(&error), expected);
        }
    }

    fn episode(label: Option<&str>) -> Episode {
        Episode {
            id: "e2".into(),
            series_id: "s".into(),
            series_name: Some("Series".into()),
            season_id: None,
            title: "The Return".into(),
            season_number: Some(2),
            number: Some(6),
            label: label.map(str::to_owned),
            overview: None,
            thumb: None,
            duration: None,
            user: UserState::default(),
        }
    }

    #[test]
    fn next_up_title_leads_with_the_episode_label() {
        assert_eq!(
            next_up_title(&episode(Some("S02E06"))),
            "S02E06 \u{b7} The Return"
        );
        assert_eq!(next_up_title(&episode(None)), "The Return");
    }

    #[test]
    fn clock_formats_minutes_and_hours() {
        assert_eq!(clock(Duration::from_secs(5)), "0:05");
        assert_eq!(clock(Duration::from_secs(123)), "2:03");
        assert_eq!(clock(Duration::from_secs(3723)), "1:02:03");
    }

    #[test]
    fn image_size_rounds_up_with_a_minimum_of_one() {
        assert_eq!(
            image_size(160.0, 240.0, 2.0),
            ImageSize {
                max_width: 320,
                max_height: 480
            }
        );
        assert_eq!(
            image_size(160.0, 240.0, 1.25),
            ImageSize {
                max_width: 200,
                max_height: 300
            }
        );
        assert_eq!(
            image_size(0.0, 0.2, 1.0),
            ImageSize {
                max_width: 1,
                max_height: 1
            }
        );
    }

    #[test]
    fn card_for_a_movie_shows_the_year() {
        let mut item = summary(ItemKind::Movie);
        item.user.played = true;
        item.user.progress = Some(0.4);
        let card = card(&item);
        assert_eq!(card.id, "abc");
        assert_eq!(card.title, "Title");
        assert_eq!(card.subtitle, "1999");
        assert_eq!(card.progress, 0.4);
        assert!(card.watched);
        assert!(!card.has_image);
    }

    #[test]
    fn card_without_year_or_progress_is_empty() {
        let mut item = summary(ItemKind::Movie);
        item.year = None;
        let card = card(&item);
        assert_eq!(card.subtitle, "");
        assert_eq!(card.progress, 0.0);
    }

    #[test]
    fn card_for_an_episode_shows_series_and_label() {
        let mut item = summary(ItemKind::Episode);
        item.series_name = Some("Show".into());
        item.episode_label = Some("S01E02".into());
        assert_eq!(card(&item).subtitle, "Show \u{b7} S01E02");
        item.series_name = None;
        assert_eq!(card(&item).subtitle, "S01E02");
    }

    #[test]
    fn detail_target_follows_the_kind() {
        assert_eq!(
            detail_target(&summary(ItemKind::Movie)),
            Some(Screen::Movie("abc".into()))
        );
        assert_eq!(
            detail_target(&summary(ItemKind::Video)),
            Some(Screen::Movie("abc".into()))
        );
        assert_eq!(
            detail_target(&summary(ItemKind::Series)),
            Some(Screen::Series("abc".into()))
        );
        let mut episode = summary(ItemKind::Episode);
        assert_eq!(detail_target(&episode), None);
        episode.series_id = Some("show".into());
        assert_eq!(detail_target(&episode), Some(Screen::Series("show".into())));
    }

    fn audio(index: i32, language: Option<&str>, channels: Option<u32>) -> AudioOption {
        AudioOption {
            index,
            label: format!("Track {index}"),
            language: language.map(Into::into),
            channels,
            selected: index == 1,
        }
    }

    #[test]
    fn audio_rows_describe_language_and_channels() {
        let rows = audio_rows(&[
            audio(1, Some("fra"), Some(6)),
            audio(2, None, Some(2)),
            audio(3, Some("eng"), None),
            audio(4, None, Some(3)),
            audio(5, None, None),
        ]);
        let details: Vec<_> = rows.iter().map(|row| row.detail.to_string()).collect();
        assert_eq!(details, ["fra \u{b7} 5.1", "2.0", "eng", "3ch", ""]);
        assert!(rows[0].selected && !rows[1].selected);
        assert_eq!(rows[1].index, 2);
    }

    #[test]
    fn channel_layouts_are_numeric() {
        let layouts: Vec<_> = [1, 2, 6, 8].map(channel_layout).into();
        assert_eq!(layouts, ["1.0", "2.0", "5.1", "7.1"]);
    }

    fn subtitle(index: i32, selected: bool) -> SubtitleOption {
        SubtitleOption {
            index,
            label: format!("Sub {index}"),
            language: Some("eng".into()),
            forced: index == 3,
            external: index == 4,
            selected,
        }
    }

    #[test]
    fn subtitle_rows_start_with_an_off_row() {
        let rows = subtitle_rows(&[subtitle(3, false), subtitle(4, false)]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].index, -1);
        assert!(rows[0].label.is_empty());
        assert!(rows[0].selected);
        assert!(rows[1].forced && !rows[1].external);
        assert!(rows[2].external && !rows[2].forced);
        assert_eq!(rows[1].detail, "eng");
    }

    #[test]
    fn off_row_is_not_selected_when_a_subtitle_is() {
        let rows = subtitle_rows(&[subtitle(3, true)]);
        assert!(!rows[0].selected);
        assert!(rows[1].selected);
        assert_eq!(subtitle_rows(&[]).len(), 1);
    }

    #[test]
    fn device_name_uses_the_hostname_or_the_id() {
        assert_eq!(device_name(Some("  laptop \n"), "abcdef123"), "laptop");
        assert_eq!(device_name(Some("  "), "abcdef123"), "Oxyfin abcdef");
        assert_eq!(device_name(None, "abc"), "Oxyfin abc");
    }

    fn playing(series: Option<&str>, label: Option<&str>) -> PlaybackTitle {
        PlaybackTitle {
            title: "Pilot".into(),
            series_name: series.map(Into::into),
            episode_label: label.map(Into::into),
        }
    }

    #[test]
    fn episode_title_shows_series_then_label_and_episode() {
        let (title, subtitle) = playback_title(&playing(Some("Show"), Some("S01E01")));
        assert_eq!(title, "Show");
        assert_eq!(subtitle, "S01E01 \u{b7} Pilot");
        let (_, subtitle) = playback_title(&playing(Some("Show"), None));
        assert_eq!(subtitle, "Pilot");
    }

    #[test]
    fn movie_title_stands_alone() {
        let (title, subtitle) = playback_title(&playing(None, None));
        assert_eq!(title, "Pilot");
        assert!(subtitle.is_empty());
    }
}
