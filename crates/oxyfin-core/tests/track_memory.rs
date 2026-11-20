//! Tests of the per series memory of audio and subtitle choices.

use oxyfin_api::{ItemId, MediaStream, MediaStreamType};
use oxyfin_core::{
    AudioPick, Paths, SubtitlePick, TRACK_MEMORY_LIMIT, TrackChoice, TrackMemory, best_audio,
    best_subtitle,
};

fn audio(index: i32, language: &str, channels: u32) -> MediaStream {
    MediaStream {
        index,
        r#type: MediaStreamType::Audio,
        language: Some(language.to_owned()),
        channels: Some(channels),
        ..MediaStream::default()
    }
}

fn subtitle(index: i32, language: &str, forced: bool) -> MediaStream {
    MediaStream {
        index,
        r#type: MediaStreamType::Subtitle,
        language: Some(language.to_owned()),
        is_forced: forced,
        ..MediaStream::default()
    }
}

fn french(forced: bool) -> SubtitlePick {
    SubtitlePick::Track {
        language: Some("fre".to_owned()),
        forced,
        hearing_impaired: false,
        external: false,
    }
}

#[test]
fn best_audio_matches_language_then_channels() {
    let streams = [audio(1, "eng", 6), audio(2, "fre", 2), audio(3, "fre", 6)];
    let pick = AudioPick {
        language: Some("fre".to_owned()),
        channels: Some(6),
    };
    assert_eq!(best_audio(&streams, &pick), Some(3));
    let stereo = AudioPick {
        channels: Some(2),
        ..pick.clone()
    };
    assert_eq!(best_audio(&streams, &stereo), Some(2));
    let other_layout = AudioPick {
        channels: Some(8),
        ..pick
    };
    assert_eq!(
        best_audio(&streams, &other_layout),
        Some(3),
        "the closest layout"
    );
}

#[test]
fn best_audio_accepts_both_iso_codes() {
    let streams = [audio(1, "eng", 2), audio(2, "fra", 2)];
    let pick = AudioPick {
        language: Some("fre".to_owned()),
        channels: Some(2),
    };
    assert_eq!(best_audio(&streams, &pick), Some(2));
}

#[test]
fn best_audio_no_match_is_none() {
    let streams = [audio(1, "eng", 2), audio(2, "jpn", 2)];
    let pick = AudioPick {
        language: Some("fre".to_owned()),
        channels: Some(2),
    };
    assert_eq!(best_audio(&streams, &pick), None);
    let unknown = AudioPick {
        language: None,
        channels: Some(2),
    };
    assert_eq!(best_audio(&streams, &unknown), None);
}

#[test]
fn best_audio_avoids_commentary() {
    let mut commentary = audio(1, "eng", 2);
    commentary.title = Some("Director's Commentary".to_owned());
    let streams = [commentary, audio(2, "eng", 6)];
    let pick = AudioPick {
        language: Some("eng".to_owned()),
        channels: Some(2),
    };
    assert_eq!(best_audio(&streams, &pick), Some(2));
}

#[test]
fn best_subtitle_off() {
    let streams = [subtitle(3, "fre", false)];
    assert_eq!(best_subtitle(&streams, &SubtitlePick::Off), Some(None));
    assert_eq!(best_subtitle(&streams, &SubtitlePick::Default), None);
}

#[test]
fn best_subtitle_prefers_same_flags() {
    let streams = [
        subtitle(2, "eng", false),
        subtitle(3, "fre", true),
        subtitle(4, "fre", false),
    ];
    assert_eq!(best_subtitle(&streams, &french(false)), Some(Some(4)));
    assert_eq!(best_subtitle(&streams, &french(true)), Some(Some(3)));

    let mut sdh = subtitle(5, "fre", false);
    sdh.is_hearing_impaired = true;
    let streams = [subtitle(4, "fre", false), sdh];
    let pick = SubtitlePick::Track {
        language: Some("fre".to_owned()),
        forced: false,
        hearing_impaired: true,
        external: false,
    };
    assert_eq!(best_subtitle(&streams, &pick), Some(Some(5)));
    assert_eq!(best_subtitle(&streams, &french(false)), Some(Some(4)));
}

#[test]
fn best_subtitle_without_the_language_is_none() {
    let streams = [subtitle(2, "eng", false)];
    assert_eq!(best_subtitle(&streams, &french(false)), None);
}

#[test]
fn best_subtitle_ignores_commentary_title_when_alternative() {
    let mut commentary = subtitle(2, "fre", false);
    commentary.title = Some("Commentary".to_owned());
    let streams = [commentary.clone(), subtitle(3, "fre", false)];
    assert_eq!(best_subtitle(&streams, &french(false)), Some(Some(3)));
    assert_eq!(best_subtitle(&[commentary], &french(false)), Some(Some(2)));
}

#[test]
fn choice_describes_the_streams_picked() {
    let mut sdh = subtitle(5, "fre", false);
    sdh.is_hearing_impaired = true;
    sdh.is_external = true;
    assert_eq!(
        SubtitlePick::of(&sdh),
        Some(SubtitlePick::Track {
            language: Some("fre".to_owned()),
            forced: false,
            hearing_impaired: true,
            external: true,
        })
    );
    assert_eq!(
        AudioPick::of(&audio(1, "eng", 6)),
        Some(AudioPick {
            language: Some("eng".to_owned()),
            channels: Some(6),
        })
    );
}

#[test]
fn streams_without_a_language_describe_no_pick() {
    for language in [None, Some("und"), Some(" UND "), Some("")] {
        let mut sound = audio(1, "eng", 2);
        sound.language = language.map(str::to_owned);
        let mut text = subtitle(2, "eng", false);
        text.language = language.map(str::to_owned);
        assert_eq!(AudioPick::of(&sound), None, "{language:?}");
        assert_eq!(SubtitlePick::of(&text), None, "{language:?}");
    }
}

fn series(n: usize) -> ItemId {
    ItemId::from(format!("{n:032x}").as_str())
}

fn choice(language: &str) -> TrackChoice {
    TrackChoice {
        audio: Some(AudioPick {
            language: Some(language.to_owned()),
            channels: Some(2),
        }),
        subtitle: SubtitlePick::Off,
    }
}

#[test]
fn memory_survives_reload() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let mut memory = TrackMemory::load(&paths);
    assert_eq!(memory.get(&series(1)), None);
    memory.set(&series(1), choice("fre")).unwrap();
    memory.set(&series(2), choice("jpn")).unwrap();
    memory.set(&series(1), choice("eng")).unwrap();
    assert!(paths.data_dir.join("track-memory.json").is_file());

    let reloaded = TrackMemory::load(&paths);
    assert_eq!(reloaded.get(&series(1)), Some(choice("eng")));
    assert_eq!(reloaded.get(&series(2)), Some(choice("jpn")));
}

#[test]
fn unreadable_memory_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    std::fs::write(paths.data_dir.join("track-memory.json"), "not json").unwrap();
    let mut memory = TrackMemory::load(&paths);
    assert_eq!(memory.get(&series(1)), None);
    memory.set(&series(1), choice("fre")).unwrap();
    assert_eq!(
        TrackMemory::load(&paths).get(&series(1)),
        Some(choice("fre"))
    );
}

#[test]
fn memory_bounded_to_500() {
    assert_eq!(TRACK_MEMORY_LIMIT, 500);
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let mut memory = TrackMemory::load(&paths);
    for n in 0..TRACK_MEMORY_LIMIT {
        memory.set(&series(n), choice("fre")).unwrap();
    }
    memory.set(&series(0), choice("eng")).unwrap();
    memory
        .set(&series(TRACK_MEMORY_LIMIT), choice("jpn"))
        .unwrap();

    let reloaded = TrackMemory::load(&paths);
    assert_eq!(reloaded.get(&series(1)), None, "the oldest is dropped");
    assert_eq!(reloaded.get(&series(0)), Some(choice("eng")));
    assert_eq!(reloaded.get(&series(2)), Some(choice("fre")));
    assert_eq!(
        reloaded.get(&series(TRACK_MEMORY_LIMIT)),
        Some(choice("jpn"))
    );
}
