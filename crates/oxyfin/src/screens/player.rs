//! Player screen: sends the user's commands and shows the controller's events.

use std::rc::Rc;
use std::time::Duration;

use oxyfin_core::{
    Chapter, PlaybackCommand, PlaybackError, PlaybackEvent, QUALITIES, Quality, SEEK_STEP, SPEEDS,
    SegmentKind, SkippableSegment, Trickplay, TrickplaySheet, TrickplayTile, next_chapter,
    previous_chapter, start_at,
};
use oxyfin_player::SeekTarget;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, SharedString, VecModel};

use crate::present::{
    StatWords, audio_rows, chapter_marks, chapter_rows, clock, next_up_title,
    playback_error_message, playback_title, stat_sections, subtitle_rows,
};
use crate::{
    AppWindow, ChapterRow, Message, NextUp, QualityRow, SeekPreview, SkipKind, StatsTitles,
    TrickplayGrid,
};

/// Volume change of one step of the volume keys, in percent.
pub const VOLUME_STEP: f64 = 5.0;

/// Change of one step of the delay keys and buttons, in milliseconds.
pub const DELAY_STEP_MS: i64 = 100;

/// Lead given to the position before a chapter jump, so that a seek landing
/// a little before a chapter start counts as inside that chapter.
const CHAPTER_SLACK: Duration = Duration::from_millis(250);

/// Wires the player controls of `window` to `send`.
///
/// `send` is the only way out of the screen: the playback controller in the
/// application, a recorder in tests. Fullscreen is window state and is
/// handled here.
pub fn bind(window: &AppWindow, send: Rc<dyn Fn(PlaybackCommand)>) {
    window.set_player_speeds(ModelRc::new(VecModel::from(
        SPEEDS.iter().map(|speed| *speed as f32).collect::<Vec<_>>(),
    )));
    window.set_player_qualities(ModelRc::new(VecModel::from(
        QUALITIES
            .iter()
            .map(|quality| quality_row(*quality))
            .collect::<Vec<_>>(),
    )));

    let command = |command: PlaybackCommand| {
        let send = Rc::clone(&send);
        move || send(command.clone())
    };
    window.on_player_command_stop(command(PlaybackCommand::Stop));
    window.on_player_command_previous(command(PlaybackCommand::Previous));
    window.on_player_command_next(command(PlaybackCommand::Next));
    window.on_player_command_play_next_now(command(PlaybackCommand::PlayNextNow));
    window.on_player_command_cancel_next_up(command(PlaybackCommand::CancelNextUp));
    window.on_player_command_stats(command(PlaybackCommand::RequestStats));

    let to = Rc::clone(&send);
    window.on_player_command_set_paused(move |paused| to(PlaybackCommand::SetPaused(paused)));
    let to = Rc::clone(&send);
    window.on_player_command_set_muted(move |muted| to(PlaybackCommand::SetMuted(muted)));
    let to = Rc::clone(&send);
    window.on_player_command_set_speed(move |speed| to(PlaybackCommand::SetSpeed(speed.into())));
    let to = Rc::clone(&send);
    window.on_player_command_set_volume(move |volume| {
        to(PlaybackCommand::SetVolume(
            f64::from(volume).clamp(0.0, 100.0),
        ))
    });
    let to = Rc::clone(&send);
    window.on_player_command_select_audio(move |index| to(PlaybackCommand::SelectAudio(index)));
    let to = Rc::clone(&send);
    window.on_player_command_select_subtitle(move |index| {
        to(PlaybackCommand::SelectSubtitle(subtitle_choice(index)))
    });

    let (to, weak) = (Rc::clone(&send), window.as_weak());
    window.on_player_command_seek(move |seconds| {
        let seconds = seconds.max(0.0);
        if let Some(window) = weak.upgrade() {
            window.set_player_position(seconds);
            window.set_player_elapsed(clock(Duration::from_secs_f32(seconds)));
            window.set_player_seek_pending(true);
            show_current_chapter(&window);
        }
        to(PlaybackCommand::Seek(SeekTarget::Absolute(
            Duration::from_secs_f32(seconds),
        )));
    });
    let (to, weak) = (Rc::clone(&send), window.as_weak());
    window.on_player_command_skip(move |direction| {
        if let Some(window) = weak.upgrade() {
            window.set_player_seek_pending(true);
        }
        to(PlaybackCommand::Seek(skip_target(direction)));
    });
    let (to, weak) = (Rc::clone(&send), window.as_weak());
    window.on_player_command_set_quality(move |index| {
        let Some(quality) = usize::try_from(index)
            .ok()
            .and_then(|index| QUALITIES.get(index))
        else {
            return;
        };
        if let Some(window) = weak.upgrade() {
            window.set_player_quality(index);
        }
        to(PlaybackCommand::SetQuality(*quality));
    });
    let (to, weak) = (Rc::clone(&send), window.as_weak());
    window.on_player_command_step_speed(move |direction| {
        let Some(window) = weak.upgrade() else { return };
        if let Some(speed) = step_speed(window.get_player_speed().into(), direction) {
            to(PlaybackCommand::SetSpeed(speed));
        }
    });
    let to = Rc::clone(&send);
    window
        .on_player_command_set_audio_delay(move |ms| to(PlaybackCommand::SetAudioDelay(ms.into())));
    let to = Rc::clone(&send);
    window.on_player_command_set_subtitle_delay(move |ms| {
        to(PlaybackCommand::SetSubtitleDelay(ms.into()))
    });
    // Relative steps: the core adds them up, even before the window shows the previous one.
    let to = Rc::clone(&send);
    window.on_player_command_step_audio_delay(move |direction| {
        to(PlaybackCommand::AdjustAudioDelay(delay_step(direction)))
    });
    let to = Rc::clone(&send);
    window.on_player_command_step_subtitle_delay(move |direction| {
        to(PlaybackCommand::AdjustSubtitleDelay(delay_step(direction)))
    });
    let weak = window.as_weak();
    window.on_player_command_step_chapter(move |direction| {
        let Some(window) = weak.upgrade() else { return };
        if let Some(start) = chapter_step(&window, direction) {
            window.invoke_player_command_seek(start);
        }
    });
    let (to, weak) = (send, window.as_weak());
    window.on_player_command_step_volume(move |direction| {
        let Some(window) = weak.upgrade() else { return };
        to(PlaybackCommand::SetVolume(step_volume(
            window.get_player_volume().into(),
            direction,
        )));
    });

    let weak = window.as_weak();
    window.on_player_cursor_hidden(move |x, y| {
        let weak = weak.clone();
        // Replayed after the callback returns, outside the timer that hid the controls.
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(window) = weak.upgrade() {
                window.window().dispatch_event(WindowEvent::PointerMoved {
                    position: LogicalPosition::new(x, y),
                });
            }
        });
    });

    let weak = window.as_weak();
    window.on_player_toggle_fullscreen(move || {
        if let Some(window) = weak.upgrade() {
            let fullscreen = !window.window().is_fullscreen();
            window.window().set_fullscreen(fullscreen);
        }
    });
}

/// Clears what a previous playback left on the player screen.
pub fn reset(window: &AppWindow) {
    window.set_player_title(Default::default());
    window.set_player_subtitle(Default::default());
    window.set_player_loading(true);
    window.set_player_has_error(false);
    window.set_player_paused(false);
    window.set_player_speed(1.0);
    window.set_player_has_next(false);
    window.set_player_audio(ModelRc::default());
    window.set_player_subtitles(ModelRc::default());
    window.set_player_audio_delay(0);
    window.set_player_subtitle_delay(0);
    window.set_player_next_up(NextUp::default());
    window.set_player_scrubbing(false);
    clear_chapters(window);
    window.set_player_stats(ModelRc::default());
    window.set_player_stats_visible(false);
    clear_preview(window);
    show_segment(window, None);
    show_progress(window, Duration::ZERO, None);
}

/// Shows `quality` as the chosen one in the player menu; a quality the menu
/// does not offer shows as the original one.
pub fn show_quality(window: &AppWindow, quality: Quality) {
    let index = QUALITIES
        .iter()
        .position(|offered| *offered == quality)
        .unwrap_or_default();
    window.set_player_quality(i32::try_from(index).unwrap_or_default());
}

fn quality_row(quality: Quality) -> QualityRow {
    match quality {
        Quality::Original => QualityRow::default(),
        Quality::Limited { bitrate, height } => QualityRow {
            height: to_int(height),
            kbps: i32::try_from(bitrate / 1000).unwrap_or(i32::MAX),
        },
    }
}

/// Opens the player screen: remembers whether the window is fullscreen, for [`leave`].
pub fn enter(window: &AppWindow) {
    window.set_player_fullscreen_before(window.window().is_fullscreen());
}

/// Leaves the player screen: the window gets back the fullscreen state it had when the
/// player opened, so fullscreen turned on in the player does not stay behind the other
/// screens, and a window launched fullscreen stays so.
pub fn leave(window: &AppWindow) {
    let before = window.get_player_fullscreen_before();
    if window.window().is_fullscreen() != before {
        window.window().set_fullscreen(before);
    }
}

/// Shows a playback event on the player screen. Returns the message to show
/// in the notice banner when the event calls for one.
pub fn apply(window: &AppWindow, event: PlaybackEvent) -> Option<Message> {
    match event {
        PlaybackEvent::Loading { title, .. } => {
            let (title, subtitle) = playback_title(&title);
            window.set_player_title(title);
            window.set_player_subtitle(subtitle);
            clear_preview(window);
            window.set_player_loading(true);
            window.set_player_has_error(false);
            window.set_player_next_up(NextUp::default());
            clear_chapters(window);
            window.set_player_stats(ModelRc::default());
            show_segment(window, None);
            show_progress(window, Duration::ZERO, None);
        }
        PlaybackEvent::Ready {
            duration,
            audio,
            subtitles,
            has_next,
            ..
        } => {
            window.set_player_loading(false);
            window.set_player_has_next(has_next);
            show_tracks(window, &audio, &subtitles);
            window.set_player_seek_pending(false);
            window.set_player_duration(duration.map_or(0.0, |duration| duration.as_secs_f32()));
            window.set_player_total(clock(duration.unwrap_or_default()));
            show_marks(window);
            // The statistics were cleared on loading: ask at once rather than at the next tick.
            if window.get_player_stats_visible() {
                window.invoke_player_command_stats();
            }
        }
        PlaybackEvent::Position(position) => {
            window.set_player_reported_position(position.as_secs_f32());
            if !window.get_player_scrubbing() && !window.get_player_seek_pending() {
                window.set_player_position(position.as_secs_f32());
                window.set_player_elapsed(clock(position));
                show_current_chapter(window);
            }
        }
        PlaybackEvent::Paused(paused) => window.set_player_paused(paused),
        PlaybackEvent::Speed(speed) => window.set_player_speed(speed as f32),
        PlaybackEvent::Volume(volume) => window.set_player_volume(volume as f32),
        PlaybackEvent::Muted(muted) => window.set_player_muted(muted),
        PlaybackEvent::SubtitleDelay(ms) => window.set_player_subtitle_delay(saturate(ms)),
        PlaybackEvent::AudioDelay(ms) => window.set_player_audio_delay(saturate(ms)),
        PlaybackEvent::Seeking => {}
        PlaybackEvent::Seeked => {
            window.set_player_seek_pending(false);
            if !window.get_player_scrubbing() {
                let position = window.get_player_reported_position();
                window.set_player_position(position);
                window.set_player_elapsed(clock(Duration::from_secs_f32(position.max(0.0))));
                show_current_chapter(window);
            }
        }
        PlaybackEvent::TracksChanged { audio, subtitles } => {
            show_tracks(window, &audio, &subtitles);
        }
        PlaybackEvent::NextUpCountdown { episode, remaining } => {
            let mut next = window.get_player_next_up();
            let id = SharedString::from(episode.id.as_str());
            if next.id != id {
                next = NextUp {
                    id,
                    title: next_up_title(&episode),
                    ..NextUp::default()
                };
            }
            next.visible = true;
            next.remaining = i32::try_from(remaining).unwrap_or(i32::MAX);
            window.set_player_next_up(next);
        }
        PlaybackEvent::NextUpCancelled => window.set_player_next_up(NextUp::default()),
        PlaybackEvent::Stopped => {
            window.set_player_next_up(NextUp::default());
            show_segment(window, None);
        }
        PlaybackEvent::Segment(segment) => show_segment(window, segment),
        PlaybackEvent::Trickplay(trickplay) => window.set_player_trickplay(grid(&trickplay)),
        PlaybackEvent::Chapters(chapters) => show_chapters(window, &chapters),
        PlaybackEvent::Stats(report) => {
            let words = Words(window.global::<StatsTitles>());
            window.set_player_stats(ModelRc::new(VecModel::from(stat_sections(&report, &words))));
        }
        PlaybackEvent::Ended | PlaybackEvent::SessionExpired => {}
        PlaybackEvent::Error(error @ PlaybackError::Command { .. }) => {
            log::warn!("playback command failed: {error}");
            window.set_player_seek_pending(false);
            return Some(playback_error_message(&error));
        }
        PlaybackEvent::Error(error) => {
            log::warn!("playback failed: {error}");
            window.set_player_loading(false);
            window.set_player_seek_pending(false);
            show_segment(window, None);
            window.set_player_error(playback_error_message(&error));
            window.set_player_has_error(true);
        }
    }
    None
}

/// The statistics words translated by the window.
struct Words<'a>(StatsTitles<'a>);

impl StatWords for Words<'_> {
    fn playback(&self) -> String {
        self.0.invoke_playback().into()
    }
    fn video(&self) -> String {
        self.0.invoke_video().into()
    }
    fn audio(&self) -> String {
        self.0.invoke_audio().into()
    }
    fn network(&self) -> String {
        self.0.invoke_network().into()
    }
    fn method(&self) -> String {
        self.0.invoke_method().into()
    }
    fn container(&self) -> String {
        self.0.invoke_container().into()
    }
    fn bitrate(&self) -> String {
        self.0.invoke_bitrate().into()
    }
    fn codec(&self) -> String {
        self.0.invoke_codec().into()
    }
    fn resolution(&self) -> String {
        self.0.invoke_resolution().into()
    }
    fn frame_rate(&self) -> String {
        self.0.invoke_frame_rate().into()
    }
    fn dropped_frames(&self) -> String {
        self.0.invoke_dropped_frames().into()
    }
    fn hdr(&self) -> String {
        self.0.invoke_hdr().into()
    }
    fn output(&self) -> String {
        self.0.invoke_output().into()
    }
    fn buffer(&self) -> String {
        self.0.invoke_buffer().into()
    }
    fn speed(&self) -> String {
        self.0.invoke_speed().into()
    }
    fn direct_play(&self) -> String {
        self.0.invoke_direct_play().into()
    }
    fn direct_stream(&self) -> String {
        self.0.invoke_direct_stream().into()
    }
    fn transcode(&self) -> String {
        self.0.invoke_transcode().into()
    }
    fn software(&self) -> String {
        self.0.invoke_software().into()
    }
    fn hardware(&self, api: &str) -> String {
        self.0.invoke_hardware(api.into()).into()
    }
    fn dropped(&self, decoder: &str, output: &str) -> String {
        self.0.invoke_dropped(decoder.into(), output.into()).into()
    }
    fn ahead(&self, seconds: &str) -> String {
        self.0.invoke_ahead(seconds.into()).into()
    }
}

/// Wires the seek bar preview of `window`: `request` is asked for each
/// trickplay sheet the preview needs and answers with [`show_sheet`], at
/// once or later. It is the image loader in the application, a recorder in
/// tests.
pub fn bind_preview(window: &AppWindow, request: Rc<dyn Fn(TrickplaySheet)>) {
    let weak = window.as_weak();
    window.on_player_preview_requested(move |seconds| {
        if let Some(window) = weak.upgrade() {
            preview(&window, seconds, &*request);
        }
    });
}

/// Shows the hovered time and its thumbnail, asking for its sheet when the
/// preview shows another one.
fn preview(window: &AppWindow, seconds: f32, request: &dyn Fn(TrickplaySheet)) {
    let time = Duration::from_secs_f32(seconds.max(0.0));
    let mut preview = window.get_player_preview();
    preview.time = clock(time);
    let current = with_starts(window, |starts| start_at(starts, time));
    preview.chapter = current
        .and_then(|index| window.get_player_chapters().row_data(index))
        .map(|row| row.title)
        .unwrap_or_default();
    preview.chapter_number =
        current.map_or(0, |index| i32::try_from(index + 1).unwrap_or(i32::MAX));
    let Some(trickplay) = trickplay(&window.get_player_trickplay()) else {
        window.set_player_preview(preview);
        return;
    };
    let tile = trickplay.tile_at(time);
    preview.x = to_int(tile.x);
    preview.y = to_int(tile.y);
    preview.width = to_int(tile.width);
    preview.height = to_int(tile.height);
    let key = sheet_key(&tile.sheet);
    let wanted = preview.sheet != key;
    if wanted {
        preview.sheet = key;
        preview.has_image = false;
        preview.image = slint::Image::default();
    }
    // Set first: the answer may come before `request` returns.
    window.set_player_preview(preview);
    if wanted {
        request(tile.sheet);
    }
}

/// Shows `image`, the decoded `sheet`, if the preview still wants it.
pub fn show_sheet(window: &AppWindow, sheet: &TrickplaySheet, image: slint::Image) {
    let mut preview = window.get_player_preview();
    if preview.sheet == sheet_key(sheet) {
        preview.image = image;
        preview.has_image = true;
        window.set_player_preview(preview);
    }
}

fn clear_preview(window: &AppWindow) {
    window.set_player_trickplay(TrickplayGrid::default());
    window.set_player_preview(SeekPreview::default());
}

fn sheet_key(sheet: &TrickplaySheet) -> SharedString {
    let (interval, columns, rows) = sheet.layout;
    format!(
        "{}/{}/{}/{interval}/{columns}x{rows}/{}",
        sheet.item, sheet.media_source, sheet.resolution, sheet.index
    )
    .into()
}

fn to_int(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The window's copy of the trickplay of the playing item.
fn grid(trickplay: &Trickplay) -> TrickplayGrid {
    TrickplayGrid {
        item: trickplay.item.as_str().into(),
        source: trickplay.media_source.as_str().into(),
        resolution: to_int(trickplay.resolution),
        width: to_int(trickplay.width),
        height: to_int(trickplay.height),
        columns: to_int(trickplay.columns),
        rows: to_int(trickplay.rows),
        count: to_int(trickplay.count),
        interval_ms: i32::try_from(trickplay.interval.as_millis()).unwrap_or(i32::MAX),
    }
}

/// The trickplay of the playing item, back from the window; `None` without any.
fn trickplay(grid: &TrickplayGrid) -> Option<Trickplay> {
    let positive = |value: i32| u32::try_from(value).ok().filter(|value| *value > 0);
    Some(Trickplay {
        item: grid.item.as_str().into(),
        media_source: grid.source.as_str().into(),
        resolution: positive(grid.resolution)?,
        width: positive(grid.width)?,
        height: positive(grid.height)?,
        columns: positive(grid.columns)?,
        rows: positive(grid.rows)?,
        count: positive(grid.count)?,
        interval: Duration::from_millis(u64::from(positive(grid.interval_ms)?)),
    })
}

fn show_progress(window: &AppWindow, position: Duration, duration: Option<Duration>) {
    window.set_player_seek_pending(false);
    window.set_player_reported_position(position.as_secs_f32());
    window.set_player_position(position.as_secs_f32());
    window.set_player_elapsed(clock(position));
    window.set_player_duration(duration.map_or(0.0, |duration| duration.as_secs_f32()));
    window.set_player_total(clock(duration.unwrap_or_default()));
}

/// Offers to skip `segment`, or hides the offer.
fn show_segment(window: &AppWindow, segment: Option<SkippableSegment>) {
    window.set_player_skip_kind(segment.map_or(SkipKind::None, |segment| skip_kind(segment.kind)));
    window.set_player_skip_end(segment.map_or(0.0, |segment| segment.end.as_secs_f32()));
}

/// The chapter rows of the window, with the chapter starts kept beside them
/// so that following the position needs no copy of the rows.
struct ChapterModel {
    rows: VecModel<ChapterRow>,
    starts: Vec<Duration>,
}

impl Model for ChapterModel {
    type Data = ChapterRow;

    fn row_count(&self) -> usize {
        self.rows.row_count()
    }

    fn row_data(&self, row: usize) -> Option<ChapterRow> {
        self.rows.row_data(row)
    }

    fn set_row_data(&self, row: usize, data: ChapterRow) {
        self.rows.set_row_data(row, data);
    }

    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        self.rows.model_tracker()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Calls `f` with the chapter starts of the window, empty without chapters.
fn with_starts<T>(window: &AppWindow, f: impl FnOnce(&[Duration]) -> T) -> T {
    let model = window.get_player_chapters();
    let starts = model
        .as_any()
        .downcast_ref::<ChapterModel>()
        .map_or(&[][..], |chapters| chapters.starts.as_slice());
    f(starts)
}

/// Shows the chapters of the playing item: the menu rows, the seek bar cuts
/// and the chapter playing.
fn show_chapters(window: &AppWindow, chapters: &[Chapter]) {
    let rows = chapter_rows(chapters);
    // Starts as the menu seeks to them, so that a jump lands in its chapter.
    let starts = rows
        .iter()
        .map(|row| Duration::from_secs_f32(row.start.max(0.0)))
        .collect();
    window.set_player_chapters(ModelRc::new(ChapterModel {
        rows: VecModel::from(rows),
        starts,
    }));
    show_marks(window);
    show_current_chapter(window);
}

/// Cuts the seek bar at the chapter starts, for the current duration.
fn show_marks(window: &AppWindow) {
    let duration = window.get_player_duration();
    let duration = (duration > 0.0).then(|| Duration::from_secs_f32(duration));
    let marks = with_starts(window, |starts| chapter_marks(starts, duration));
    window.set_player_chapter_marks(ModelRc::new(VecModel::from(marks)));
}

/// Trickplay thumbnails standing for the chapters the server has no image
/// of: the tile at each such chapter start, with the chapter index. Empty
/// when the item has no trickplay.
///
/// Relies on the core sending [`PlaybackEvent::Trickplay`] before
/// [`PlaybackEvent::Ready`] and [`PlaybackEvent::Chapters`] (see the playback
/// controller), so the window holds the trickplay when the chapters come.
pub fn chapter_tiles(window: &AppWindow, chapters: &[Chapter]) -> Vec<(usize, TrickplayTile)> {
    fallback_tiles(trickplay(&window.get_player_trickplay()).as_ref(), chapters)
}

fn fallback_tiles(
    trickplay: Option<&Trickplay>,
    chapters: &[Chapter],
) -> Vec<(usize, TrickplayTile)> {
    let Some(trickplay) = trickplay else {
        return Vec::new();
    };
    chapters
        .iter()
        .enumerate()
        .filter(|(_, chapter)| chapter.image.is_none())
        .map(|(index, chapter)| (index, trickplay.tile_at(chapter.start)))
        .collect()
}

fn clear_chapters(window: &AppWindow) {
    window.set_player_chapters(ModelRc::default());
    window.set_player_chapter_marks(ModelRc::default());
    window.set_player_current_chapter(-1);
}

fn shown_position(window: &AppWindow) -> Duration {
    Duration::from_secs_f32(window.get_player_position().max(0.0))
}

fn show_current_chapter(window: &AppWindow) {
    let position = shown_position(window) + CHAPTER_SLACK;
    let current = with_starts(window, |starts| start_at(starts, position))
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or(-1);
    window.set_player_current_chapter(current);
}

/// Start, in seconds, of the chapter the chapter keys go to from the shown
/// position: the next one, or back like mpv for a negative `direction`.
fn chapter_step(window: &AppWindow, direction: i32) -> Option<f32> {
    let position = shown_position(window);
    let index = with_starts(window, |starts| {
        if direction < 0 {
            previous_chapter(starts, position)
        } else {
            next_chapter(starts, position + CHAPTER_SLACK)
        }
    })?;
    window
        .get_player_chapters()
        .row_data(index)
        .map(|row| row.start)
}

fn skip_kind(kind: SegmentKind) -> SkipKind {
    match kind {
        SegmentKind::Intro => SkipKind::Intro,
        SegmentKind::Recap => SkipKind::Recap,
        SegmentKind::Preview => SkipKind::Preview,
        SegmentKind::Credits => SkipKind::Credits,
    }
}

fn show_tracks(
    window: &AppWindow,
    audio: &[oxyfin_core::AudioOption],
    subtitles: &[oxyfin_core::SubtitleOption],
) {
    window.set_player_audio(ModelRc::new(VecModel::from(audio_rows(audio))));
    window.set_player_subtitles(ModelRc::new(VecModel::from(subtitle_rows(subtitles))));
}

/// Seek of the skip buttons and arrow keys: backward for a negative direction.
fn skip_target(direction: i32) -> SeekTarget {
    if direction < 0 {
        SeekTarget::Backward(SEEK_STEP)
    } else {
        SeekTarget::Forward(SEEK_STEP)
    }
}

/// The speed of [`SPEEDS`] after `current` in `direction`, if any.
fn step_speed(current: f64, direction: i32) -> Option<f64> {
    const SAME: f64 = 1e-3;
    if direction < 0 {
        SPEEDS
            .iter()
            .rev()
            .copied()
            .find(|speed| *speed < current - SAME)
    } else {
        SPEEDS.iter().copied().find(|speed| *speed > current + SAME)
    }
}

/// The volume one step from `current` in `direction`, within 0 to 100.
fn step_volume(current: f64, direction: i32) -> f64 {
    let step = if direction < 0 {
        -VOLUME_STEP
    } else {
        VOLUME_STEP
    };
    (current + step).clamp(0.0, 100.0)
}

/// One delay step in `direction`, in milliseconds: earlier for a negative direction.
fn delay_step(direction: i32) -> i64 {
    if direction < 0 {
        -DELAY_STEP_MS
    } else {
        DELAY_STEP_MS
    }
}

/// A delay for the window, which keeps it in an `int`.
fn saturate(ms: i64) -> i32 {
    i32::try_from(ms).unwrap_or(if ms < 0 { i32::MIN } else { i32::MAX })
}

/// The subtitle track of a menu row: the "Off" row has a negative index.
fn subtitle_choice(index: i32) -> Option<i32> {
    (index >= 0).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_go_one_step_either_way() {
        assert_eq!(skip_target(1), SeekTarget::Forward(SEEK_STEP));
        assert_eq!(skip_target(-1), SeekTarget::Backward(SEEK_STEP));
    }

    #[test]
    fn speed_steps_through_the_offered_speeds() {
        assert_eq!(step_speed(1.0, 1), Some(1.25));
        assert_eq!(step_speed(1.0, -1), Some(0.75));
        assert_eq!(step_speed(2.0, 1), None);
        assert_eq!(step_speed(0.5, -1), None);
    }

    #[test]
    fn speed_steps_from_an_unlisted_speed_to_its_neighbours() {
        assert_eq!(step_speed(1.1, 1), Some(1.25));
        assert_eq!(step_speed(1.1, -1), Some(1.0));
        assert_eq!(step_speed(3.0, -1), Some(2.0));
    }

    #[test]
    fn volume_steps_stay_within_bounds() {
        assert_eq!(step_volume(50.0, 1), 55.0);
        assert_eq!(step_volume(50.0, -1), 45.0);
        assert_eq!(step_volume(98.0, 1), 100.0);
        assert_eq!(step_volume(3.0, -1), 0.0);
    }

    #[test]
    fn delay_steps_are_a_tenth_of_a_second() {
        assert_eq!(delay_step(1), 100);
        assert_eq!(delay_step(-1), -100);
        assert_eq!(saturate(i64::from(i32::MAX) + 100), i32::MAX);
        assert_eq!(saturate(i64::MIN), i32::MIN);
        assert_eq!(saturate(-300), -300);
    }

    #[test]
    fn trickplay_survives_the_trip_through_the_window() {
        let trickplay = Trickplay {
            item: "af282b1c408f374beeee50519e9157a4".into(),
            media_source: "af282b1c408f374beeee50519e9157a4".into(),
            resolution: 320,
            width: 320,
            height: 134,
            columns: 10,
            rows: 10,
            count: 742,
            interval: Duration::from_millis(10_000),
        };
        assert_eq!(super::trickplay(&grid(&trickplay)), Some(trickplay));
        assert_eq!(super::trickplay(&TrickplayGrid::default()), None);
    }

    #[test]
    fn chapters_without_an_image_fall_back_on_their_trickplay_tile() {
        let trickplay = Trickplay {
            item: "a".into(),
            media_source: "a".into(),
            resolution: 320,
            width: 320,
            height: 180,
            columns: 10,
            rows: 10,
            count: 500,
            interval: Duration::from_secs(10),
        };
        let chapter = |seconds, image| Chapter {
            start: Duration::from_secs(seconds),
            name: String::new(),
            image,
        };
        let tagged = oxyfin_core::ImageRef {
            item_id: "a".into(),
            image_type: oxyfin_api::ImageType::Chapter,
            tag: "t".into(),
            index: Some(1),
        };
        let chapters = [
            chapter(0, None),
            chapter(60, Some(tagged)),
            chapter(1010, None),
        ];
        let tiles = fallback_tiles(Some(&trickplay), &chapters);
        assert_eq!(
            tiles,
            [
                (0, trickplay.tile_at(Duration::ZERO)),
                (2, trickplay.tile_at(Duration::from_secs(1010))),
            ]
        );
        assert_eq!(tiles[1].1.sheet.index, 1);
        assert!(fallback_tiles(None, &chapters).is_empty());
    }

    #[test]
    fn off_row_turns_subtitles_off() {
        assert_eq!(subtitle_choice(-1), None);
        assert_eq!(subtitle_choice(0), Some(0));
        assert_eq!(subtitle_choice(3), Some(3));
    }
}
