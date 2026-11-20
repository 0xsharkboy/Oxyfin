//! Playback statistics: the I key and the playback settings menu toggle the
//! panel, which asks the core for statistics every second while it shows.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::click;
use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementQuery};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen, StatRow};
use oxyfin_core::{PlayMethod, PlaybackCommand, PlaybackEvent, PlaybackStats, StatsReport};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, Model};

const TITLE: &str = "Playback statistics";

fn press(window: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn texts(window: &AppWindow) -> Vec<String> {
    ElementQuery::from_root(window)
        .match_descendants()
        .match_accessible_role(AccessibleRole::Text)
        .find_all()
        .into_iter()
        .filter_map(|element| element.accessible_label().map(|label| label.to_string()))
        .collect()
}

fn shows_stats(window: &AppWindow) -> bool {
    texts(window).iter().any(|text| text == TITLE)
}

fn requests(sent: &RefCell<Vec<PlaybackCommand>>) -> usize {
    sent.borrow()
        .iter()
        .filter(|command| **command == PlaybackCommand::RequestStats)
        .count()
}

fn full_report() -> StatsReport {
    StatsReport {
        method: PlayMethod::DirectPlay,
        transcode_reasons: Vec::new(),
        player: PlaybackStats {
            file_format: Some("mkv".into()),
            video_codec: Some("hevc Main 10".into()),
            hwdec: Some("vaapi".into()),
            source_size: Some((3840, 2160)),
            output_size: Some((2560, 1440)),
            fps: Some(23.976),
            dropped_decoder: Some(0),
            dropped_output: Some(2),
            hdr: Some("HDR10".into()),
            audio_codec: Some("eac3".into()),
            audio_channels: Some("5.1".into()),
            audio_output: Some("PipeWire · 48 kHz stereo".into()),
            video_bitrate: Some(11_760_000),
            audio_bitrate: Some(640_000),
            cache_seconds: Some(38.4),
            cache_speed: Some(5_762_500),
        },
    }
}

/// Every row of the panel, as `(section, label, value, warn, good)`.
fn rows(window: &AppWindow) -> Vec<(String, StatRow)> {
    let sections = window.get_player_stats();
    sections
        .iter()
        .flat_map(|section| {
            let title = section.title.to_string();
            section
                .rows
                .iter()
                .map(move |row| (title.clone(), row))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn row(window: &AppWindow, label: &str) -> Option<StatRow> {
    rows(window)
        .into_iter()
        .map(|(_, row)| row)
        .find(|row| row.label == label)
}

#[allow(clippy::unwrap_used)]
fn player_window() -> (AppWindow, Rc<RefCell<Vec<PlaybackCommand>>>) {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(1280., 720.));
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&sent);
    player::bind(
        &window,
        Rc::new(move |command| recorder.borrow_mut().push(command)),
    );
    player::reset(&window);
    window.set_screen(Screen::Player);
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(180)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: false,
            has_next: false,
        },
    );
    (window, sent)
}

#[test]
fn key_i_toggles_stats() {
    let (window, sent) = player_window();
    assert!(!shows_stats(&window));

    press(&window, "i");
    assert!(shows_stats(&window), "{:?}", texts(&window));
    assert_eq!(requests(&sent), 1, "asked at once");
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1100));
    assert_eq!(requests(&sent), 2, "asked again every second");

    player::apply(&window, PlaybackEvent::Stats(Box::new(full_report())));
    let shown = texts(&window);
    for text in ["PLAYBACK", "Direct play", "Matroska (MKV)", "38 s ahead"] {
        assert!(shown.iter().any(|t| t == text), "{text} in {shown:?}");
    }

    press(&window, "I");
    assert!(!shows_stats(&window), "{:?}", texts(&window));
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(3000));
    assert_eq!(requests(&sent), 2, "no request once closed");
}

#[test]
fn stats_are_asked_at_once_when_a_new_item_is_ready() {
    let (window, sent) = player_window();
    press(&window, "i");
    player::apply(&window, PlaybackEvent::Stats(Box::new(full_report())));
    assert_eq!(requests(&sent), 1);

    player::apply(
        &window,
        PlaybackEvent::Loading {
            item: "next".into(),
            title: oxyfin_core::PlaybackTitle {
                title: "Next".into(),
                series_name: None,
                episode_label: None,
                artwork: None,
            },
        },
    );
    assert!(window.get_player_stats().row_count() == 0, "cleared");
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(60)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: true,
            has_next: false,
        },
    );
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(10));
    assert_eq!(requests(&sent), 2, "asked on ready, not a second later");

    press(&window, "i");
    player::apply(
        &window,
        PlaybackEvent::Loading {
            item: "other".into(),
            title: oxyfin_core::PlaybackTitle {
                title: "Other".into(),
                series_name: None,
                episode_label: None,
                artwork: None,
            },
        },
    );
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(60)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: true,
            has_next: false,
        },
    );
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(10));
    assert_eq!(requests(&sent), 2, "not asked while hidden");

    press(&window, "i");
    assert!(shows_stats(&window));
    player::reset(&window);
    assert!(!shows_stats(&window), "a new playback starts without them");
}

#[test]
fn escape_closes_stats_before_leaving() {
    let (window, sent) = player_window();
    press(&window, "i");
    assert!(shows_stats(&window));
    press(&window, Key::Escape);
    assert!(!shows_stats(&window));
    assert!(!sent.borrow().contains(&PlaybackCommand::Stop));
    assert_eq!(window.get_screen(), Screen::Player);
}

#[test]
fn missing_values_are_omitted() {
    let (window, _sent) = player_window();
    press(&window, "i");
    let mut report = full_report();
    report.player.hdr = None;
    report.player.cache_speed = None;
    player::apply(&window, PlaybackEvent::Stats(Box::new(report)));
    assert!(row(&window, "HDR").is_none());
    assert!(row(&window, "Speed").is_none());
    assert!(!texts(&window).iter().any(|text| text == "HDR"));

    // Before the first video packets, a total would only be the audio bitrate.
    let mut report = full_report();
    report.player.video_bitrate = None;
    report.player.dropped_decoder = None;
    player::apply(&window, PlaybackEvent::Stats(Box::new(report)));
    assert!(row(&window, "Bitrate").is_none());
    assert!(row(&window, "Dropped frames").is_none());

    let mut report = full_report();
    report.player.video_codec = None;
    report.player.video_bitrate = None;
    player::apply(&window, PlaybackEvent::Stats(Box::new(report)));
    assert_eq!(
        row(&window, "Bitrate").map(|row| row.value.to_string()),
        Some("640 kbps".to_owned()),
        "audio only"
    );

    player::apply(
        &window,
        PlaybackEvent::Stats(Box::new(StatsReport {
            method: PlayMethod::DirectPlay,
            transcode_reasons: Vec::new(),
            player: PlaybackStats::default(),
        })),
    );
    let titles: Vec<String> = window
        .get_player_stats()
        .iter()
        .map(|section| section.title.to_string())
        .collect();
    assert_eq!(titles, vec!["Playback".to_owned()], "empty sections hide");
}

#[test]
fn dropped_frames_warn() {
    let (window, _sent) = player_window();
    press(&window, "i");
    player::apply(&window, PlaybackEvent::Stats(Box::new(full_report())));
    let dropped = row(&window, "Dropped frames").unwrap_or_else(|| panic!("no dropped row"));
    assert_eq!(dropped.value, "0 decoder · 2 output");
    assert!(dropped.warn);

    let mut report = full_report();
    report.player.dropped_output = Some(0);
    player::apply(&window, PlaybackEvent::Stats(Box::new(report)));
    let dropped = row(&window, "Dropped frames").unwrap_or_else(|| panic!("no dropped row"));
    assert!(!dropped.warn);
}

#[test]
fn values_follow_the_design() {
    let (window, _sent) = player_window();
    player::apply(&window, PlaybackEvent::Stats(Box::new(full_report())));
    let all: Vec<(String, String, String)> = rows(&window)
        .into_iter()
        .map(|(section, row)| (section, row.label.to_string(), row.value.to_string()))
        .collect();
    let expected = [
        ("Playback", "Method", "Direct play"),
        ("Playback", "Container", "Matroska (MKV)"),
        ("Playback", "Bitrate", "12.4 Mbps"),
        ("Video", "Codec", "HEVC Main 10 (hardware: vaapi)"),
        ("Video", "Resolution", "3840 × 2160 → 2560 × 1440"),
        ("Video", "Frame rate", "23.976 fps"),
        ("Video", "Dropped frames", "0 decoder · 2 output"),
        ("Video", "HDR", "HDR10"),
        ("Audio", "Codec", "E-AC3 5.1 · 640 kbps"),
        ("Audio", "Output", "PipeWire · 48 kHz stereo"),
        ("Network", "Buffer", "38 s ahead"),
        ("Network", "Speed", "46.1 Mbps"),
    ]
    .map(|(a, b, c)| (a.to_owned(), b.to_owned(), c.to_owned()));
    assert_eq!(all, expected.to_vec());
    assert!(row(&window, "Method").is_some_and(|row| row.good));

    let mut report = full_report();
    report.method = PlayMethod::Transcode;
    report.transcode_reasons = vec!["VideoCodecNotSupported".into()];
    report.player.hwdec = None;
    report.player.output_size = Some((3840, 2160));
    player::apply(&window, PlaybackEvent::Stats(Box::new(report)));
    let method = row(&window, "Method").unwrap_or_else(|| panic!("no method row"));
    assert_eq!(method.value, "Transcode (VideoCodecNotSupported)");
    assert!(!method.good);
    assert_eq!(
        row(&window, "Resolution").map(|row| row.value.to_string()),
        Some("3840 × 2160".to_owned())
    );
    assert_eq!(
        rows(&window)
            .into_iter()
            .find(|(section, row)| section == "Video" && row.label == "Codec")
            .map(|(_, row)| row.value.to_string()),
        Some("HEVC Main 10 (software)".to_owned())
    );
}

#[test]
fn the_settings_menu_toggles_stats() {
    let (window, sent) = player_window();
    // Tall enough for the whole menu, which scrolls in a smaller window.
    window.window().set_size(LogicalSize::new(1280., 900.));
    let gear = ElementHandle::find_by_accessible_label(&window, "Playback settings")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no settings button"));
    click(&window, &gear);
    let entry = ElementHandle::find_by_accessible_label(&window, TITLE)
        .find(|element| element.accessible_role() == Some(AccessibleRole::ListItem))
        .unwrap_or_else(|| panic!("no statistics entry"));
    click(&window, &entry);
    assert!(shows_stats(&window), "{:?}", texts(&window));
    assert_eq!(requests(&sent), 1);
}

#[test]
fn the_panel_fits_a_wrapped_value() {
    let (window, _sent) = player_window();
    press(&window, "i");
    let mut report = full_report();
    report.method = PlayMethod::Transcode;
    report.transcode_reasons = vec![
        "ContainerBitrateExceedsLimit".into(),
        "VideoCodecNotSupported".into(),
    ];
    player::apply(&window, PlaybackEvent::Stats(Box::new(report)));

    let panel = ElementHandle::find_by_accessible_label(&window, TITLE)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Groupbox))
        .unwrap_or_else(|| panic!("no panel"));
    let last = ElementHandle::find_by_accessible_label(&window, "46.1 Mbps")
        .next()
        .unwrap_or_else(|| panic!("no speed value"));
    let method = ElementHandle::find_by_accessible_label(
        &window,
        "Transcode (ContainerBitrateExceedsLimit, VideoCodecNotSupported)",
    )
    .next()
    .unwrap_or_else(|| panic!("no method value"));
    let panel_bottom = panel.absolute_position().y + panel.size().height;
    let last_bottom = last.absolute_position().y + last.size().height;
    assert!(
        method.size().height > 20.0,
        "the method wraps: {:?}",
        method.size()
    );
    assert!(
        last_bottom <= panel_bottom - 8.0,
        "last row ends at {last_bottom}, panel at {panel_bottom}"
    );
}
