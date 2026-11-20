//! The playback controller: a task that plays one item at a time and keeps
//! the server informed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use oxyfin_api::{
    ItemId, MediaSourceInfo, MediaStream, MediaStreamType, PlaybackOptions, Session,
    SubtitleDeliveryMethod, Ticks,
};
use oxyfin_player::{
    PlayerError, PlayerEvent, SeekTarget, SubtitleAppearance, Track, TrackKind, UnloadReason,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::time::{Instant, sleep_until};

use super::countdown::Countdown;
use super::current::{Current, Siblings, mark_selected};
use super::loader::{LoadFailure, LoadMessage, LoadRequest, LoadStage, Loader, Prepared, Resume};
use super::media_player::{MediaPlayer, PlayerEvents};
use super::quality::Quality;
use super::reporter::{ReportKind, Reporter};
use super::segments::SkipMemory;
use super::track_memory::{
    AudioPick, SubtitlePick, TrackChoice, TrackMemory, best_audio, best_subtitle,
};
use super::tracks::{Delivery, TrackMap, subtitle_format};
use super::types::{
    PlayMode, PlaybackCommand, PlaybackError, PlaybackEvent, PlaybackPrefs, PlaybackTimings,
    StatsReport,
};
use crate::error::CoreError;
use crate::models::Episode;

#[derive(Debug)]
enum Message {
    Command(PlaybackCommand),
    Shutdown(oneshot::Sender<()>),
}

/// Sends commands to a running [`PlaybackController`].
#[derive(Clone, Debug)]
pub struct PlaybackHandle {
    messages: UnboundedSender<Message>,
    shutdown_timeout: Duration,
    closing: Arc<AtomicBool>,
}

impl PlaybackHandle {
    /// Sends a command; ignored once the controller has ended.
    pub fn send(&self, command: PlaybackCommand) {
        let _ = self.messages.send(Message::Command(command));
    }

    /// Ends the controller cleanly: reports the stop of the current item,
    /// stops the player and waits for the queued reports to be sent.
    ///
    /// Returns after at most [`PlaybackTimings::shutdown_timeout`], even when
    /// the controller is busy or the server is slow. The controller then
    /// finishes on its own in the runtime, sending what it still can; an item
    /// it was loading is never loaded. Other handles stop working.
    pub async fn shutdown(self) {
        self.closing.store(true, Ordering::SeqCst);
        let (done, finished) = oneshot::channel();
        if self.messages.send(Message::Shutdown(done)).is_err() {
            return;
        }
        let _ = tokio::time::timeout(self.shutdown_timeout, finished).await;
    }
}

/// A task that plays one item at a time: it loads the media, applies the
/// default tracks, relays the player events and reports playback to the
/// server.
pub struct PlaybackController {
    session: Session,
    player: Arc<dyn MediaPlayer>,
    player_events: UnboundedReceiver<PlayerEvent>,
    player_open: bool,
    events: UnboundedSender<PlaybackEvent>,
    reporter: Reporter,
    timings: PlaybackTimings,
    current: Option<Current>,
    /// The player was told to stop a media and has not reported its unload:
    /// its events are stale, except the global volume, mute and speed.
    awaiting_unload: bool,
    /// `SessionExpired` was emitted; it is emitted once per controller.
    expired: bool,
    next_progress: Option<Instant>,
    countdown: Option<Countdown>,
    loader: Loader,
    /// A fetched item waiting for the unload of the previous media, until
    /// the deadline.
    pending: Option<(Prepared, Instant)>,
    /// An item is being fetched or waits to load: there is no current item yet.
    loading: bool,
    /// The latest `Previous` or `Next` sent while `loading`, applied once the
    /// item is ready.
    navigation: Option<PlaybackCommand>,
    /// How subtitles look, once the binary has said so.
    subtitles: Option<SubtitleAppearance>,
    /// Highest streaming quality, for every load.
    quality: Quality,
    /// The load in progress, while `loading`.
    request: Option<LoadRequest>,
    /// Playback speed, as the player last reported it.
    speed: f64,
    /// Subtitle delay of the current item, in milliseconds.
    subtitle_delay: i64,
    /// Audio delay of the current item, in milliseconds.
    audio_delay: i64,
    /// Set by [`PlaybackHandle::shutdown`]: no new item is loaded.
    closing: Arc<AtomicBool>,
    /// What the user chose for the end of an episode and for segments.
    prefs: PlaybackPrefs,
    /// Segments of the current item already skipped without asking.
    skips: SkipMemory,
    /// The tracks picked in each series.
    memory: TrackMemory,
}

impl PlaybackController {
    /// Starts the controller on the current tokio runtime with the default
    /// [`PlaybackTimings`].
    ///
    /// Returns the handle that sends commands and the receiver of the
    /// controller's events. The task ends on [`PlaybackHandle::shutdown`], or
    /// when every handle is dropped: it then does the same work (stop report,
    /// player stopped, queued reports sent) with no one waiting for it.
    ///
    /// The runtime must be multi-threaded: adding an external subtitle makes
    /// mpv download it on the controller's thread.
    ///
    /// `memory` holds the tracks picked in each series: the controller applies
    /// them to the next episodes and saves the user's new picks.
    ///
    /// A controller is bound to `session` and consumes `events`, so it lives
    /// for one signed-in session. After a sign-out or an expiry, the binary
    /// creates a new player, event sink and controller, and therefore a new
    /// `VideoRenderer`.
    pub fn spawn(
        session: Session,
        player: Arc<dyn MediaPlayer>,
        events: PlayerEvents,
        memory: TrackMemory,
    ) -> (PlaybackHandle, UnboundedReceiver<PlaybackEvent>) {
        Self::spawn_with_timings(session, player, events, PlaybackTimings::default(), memory)
    }

    /// Same as [`PlaybackController::spawn`], with custom timings.
    pub fn spawn_with_timings(
        session: Session,
        player: Arc<dyn MediaPlayer>,
        events: PlayerEvents,
        timings: PlaybackTimings,
        memory: TrackMemory,
    ) -> (PlaybackHandle, UnboundedReceiver<PlaybackEvent>) {
        let (messages, message_receiver) = unbounded_channel();
        let (event_sender, event_receiver) = unbounded_channel();
        let (expired_sender, expired) = unbounded_channel();
        let closing = Arc::new(AtomicBool::new(false));
        let (loader, loads) = Loader::new();
        let controller = Self {
            reporter: Reporter::spawn(session.clone(), expired_sender),
            session,
            player,
            player_events: events.0,
            player_open: true,
            events: event_sender,
            timings,
            current: None,
            awaiting_unload: false,
            expired: false,
            next_progress: None,
            countdown: None,
            loader,
            loading: false,
            navigation: None,
            subtitles: None,
            quality: Quality::Original,
            request: None,
            speed: 1.0,
            subtitle_delay: 0,
            audio_delay: 0,
            pending: None,
            closing: closing.clone(),
            prefs: PlaybackPrefs::default(),
            skips: SkipMemory::default(),
            memory,
        };
        tokio::spawn(controller.run(message_receiver, expired, loads));
        (
            PlaybackHandle {
                messages,
                shutdown_timeout: timings.shutdown_timeout,
                closing,
            },
            event_receiver,
        )
    }

    async fn run(
        mut self,
        mut messages: UnboundedReceiver<Message>,
        mut expired: UnboundedReceiver<()>,
        mut loads: UnboundedReceiver<LoadMessage>,
    ) {
        loop {
            let deadline = self.next_progress;
            let countdown = self
                .countdown
                .as_ref()
                .and_then(|countdown| countdown.deadline);
            let grace = self.pending.as_ref().map(|(_, deadline)| *deadline);
            tokio::select! {
                message = messages.recv() => match message {
                    Some(Message::Command(command)) => self.on_command(command),
                    Some(Message::Shutdown(done)) => {
                        self.finish().await;
                        let _ = done.send(());
                        return;
                    }
                    None => break,
                },
                event = self.player_events.recv(), if self.player_open => match event {
                    Some(event) => self.on_player_event(event),
                    None => {
                        self.player_open = false;
                        self.load_pending();
                    }
                },
                Some(message) = loads.recv() => self.on_load(message),
                Some(()) = expired.recv() => self.expire(),
                () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    self.report(ReportKind::Progress);
                    self.schedule_progress();
                }
                () = sleep_until(countdown.unwrap_or_else(Instant::now)), if countdown.is_some() => {
                    self.tick_countdown();
                }
                () = sleep_until(grace.unwrap_or_else(Instant::now)), if grace.is_some() => {
                    self.awaiting_unload = false;
                    self.load_pending();
                }
            }
        }
        self.finish().await;
    }

    /// Reports the stop of the current item, stops the player and waits
    /// until every queued report is sent.
    async fn finish(mut self) {
        self.countdown = None;
        self.cancel_load();
        self.release(true);
        let _ = self.reporter.close().await;
    }

    fn closing(&self) -> bool {
        self.closing.load(Ordering::SeqCst)
    }

    fn emit(&self, event: PlaybackEvent) {
        let _ = self.events.send(event);
    }

    /// Handles the result of a player command; returns whether it succeeded.
    fn call(&self, result: Result<(), PlayerError>) -> bool {
        match result {
            Ok(()) => true,
            Err(PlayerError::NoMediaLoaded) => false,
            Err(error) => {
                self.emit(PlaybackEvent::Error(PlaybackError::command(&error)));
                false
            }
        }
    }

    fn schedule_progress(&mut self) {
        self.next_progress = match &self.current {
            Some(current) if current.reporting() && !current.paused => {
                Some(Instant::now() + self.timings.progress_interval)
            }
            _ => None,
        };
    }

    fn fail(&mut self, error: CoreError) {
        if error == CoreError::SessionExpired {
            self.expire();
        } else {
            self.emit(PlaybackEvent::Error(PlaybackError::request(error)));
        }
    }

    fn expire(&mut self) {
        self.cancel_load();
        self.release(false);
        if !self.expired {
            self.expired = true;
            self.emit(PlaybackEvent::SessionExpired);
        }
    }

    /// Queues a report for the current item while its server session is open.
    fn report(&self, kind: ReportKind) {
        if let Some(current) = self.current.as_ref().filter(|c| c.reporting()) {
            self.reporter.send(kind, current.report());
        }
    }

    /// Stops the player and forgets the current item, reporting its stop
    /// when `report` is set and its server session is open.
    fn release(&mut self, report: bool) {
        self.next_progress = None;
        let Some(current) = self.current.take() else {
            return;
        };
        let _ = self.player.stop();
        self.awaiting_unload = true;
        if report && current.reporting() {
            self.reporter.send(ReportKind::Stopped, current.report());
        }
        if report {
            self.stop_transcoding(&current);
        }
    }

    /// Asks the server to end the transcode of `current`, if it has one.
    fn stop_transcoding(&self, current: &Current) {
        // Without a play session id, the server would end every transcode of the device.
        if let (Delivery::Transcoded { .. }, Some(id)) =
            (current.delivery, &current.play_session_id)
        {
            self.reporter.stop_transcoding(id.clone());
        }
    }

    /// Emits the segment offered at the current position when it changed,
    /// or skips it when the user asked so and it was not skipped yet.
    fn show_segment(&mut self) {
        let Some(current) = self.current.as_mut().filter(|current| current.loaded) else {
            return;
        };
        let Some(segment) = current.update_segment() else {
            return;
        };
        let from = current.position;
        let skip =
            segment.filter(|entered| self.prefs.skips(entered.kind) && self.skips.fresh(entered));
        if let Some(entered) = skip
            && self.seek(SeekTarget::Absolute(entered.end))
        {
            self.skips.skipped(&entered, from);
            return self.emit(PlaybackEvent::SegmentSkipped {
                kind: entered.kind,
                from,
            });
        }
        self.emit(PlaybackEvent::Segment(segment));
    }

    /// Cancels the next episode countdown, if one runs.
    fn cancel_countdown(&mut self) {
        if self.countdown.take().is_some() {
            self.emit(PlaybackEvent::NextUpCancelled);
        }
    }

    fn start_countdown(&mut self, episode: Episode) {
        let (countdown, event) = if self.prefs.autoplay_next {
            Countdown::start(
                episode,
                self.prefs.countdown,
                self.timings.next_up_countdown_tick,
            )
        } else {
            Countdown::wait(episode)
        };
        self.countdown = Some(countdown);
        self.emit(event);
    }

    fn tick_countdown(&mut self) {
        let Some(countdown) = self.countdown.as_mut() else {
            return;
        };
        if let Some(event) = countdown.step() {
            return self.emit(event);
        }
        if let Some(countdown) = self.countdown.take() {
            self.play(countdown.into_episode().id, PlayMode::Auto);
        }
    }

    /// Seeks. A backward seek cancels the next episode countdown, and after a
    /// real end it resumes playback, so the server session starts again.
    /// Returns whether the player took the seek.
    fn seek(&mut self, target: SeekTarget) -> bool {
        let backward = match target {
            SeekTarget::Backward(_) => true,
            SeekTarget::Forward(_) => false,
            SeekTarget::Absolute(position) => self
                .current
                .as_ref()
                .is_some_and(|current| position < current.position),
        };
        if backward {
            self.cancel_countdown();
        }
        if !self.call(self.player.seek(target)) {
            return false;
        }
        let Some(current) = self.current.as_mut() else {
            return true;
        };
        current.seek_pending = true;
        if backward && current.ended {
            current.ended = false;
            self.report(ReportKind::Start);
            self.schedule_progress();
        }
        true
    }

    fn siblings(&self) -> Option<&Siblings> {
        self.current.as_ref().map(|current| &current.siblings)
    }

    fn on_command(&mut self, command: PlaybackCommand) {
        match command {
            PlaybackCommand::Play { item, mode } => self.play(item, mode),
            PlaybackCommand::SetPaused(paused) => {
                self.call(self.player.set_paused(paused));
            }
            PlaybackCommand::Seek(target) => {
                self.seek(target);
            }
            PlaybackCommand::SetSpeed(speed) => {
                self.call(self.player.set_speed(speed));
            }
            PlaybackCommand::SetVolume(volume) => {
                self.call(self.player.set_volume(volume));
            }
            PlaybackCommand::SetMuted(muted) => {
                self.call(self.player.set_muted(muted));
            }
            PlaybackCommand::SetSubtitleDelay(millis) => {
                if self.call(self.player.set_subtitle_delay(millis)) {
                    self.subtitle_delay = millis;
                    self.emit(PlaybackEvent::SubtitleDelay(millis));
                }
            }
            PlaybackCommand::AdjustSubtitleDelay(step) => {
                let millis = self.subtitle_delay.saturating_add(step);
                self.on_command(PlaybackCommand::SetSubtitleDelay(millis));
            }
            PlaybackCommand::AdjustAudioDelay(step) => {
                let millis = self.audio_delay.saturating_add(step);
                self.on_command(PlaybackCommand::SetAudioDelay(millis));
            }
            PlaybackCommand::SetAudioDelay(millis) => {
                if self.call(self.player.set_audio_delay(millis)) {
                    self.audio_delay = millis;
                    self.emit(PlaybackEvent::AudioDelay(millis));
                }
            }
            PlaybackCommand::SelectAudio(index) => {
                self.forget_expected(TrackKind::Audio);
                self.remember_audio(index);
                let track = self.map().and_then(|map| map.mpv_audio(index));
                if let Some(track) = track {
                    self.call(self.player.select_audio(Some(track)));
                } else if self.transcoding() {
                    self.reload(Some(index));
                }
            }
            PlaybackCommand::SetQuality(quality) => self.set_quality(quality),
            PlaybackCommand::SelectSubtitle(None) => {
                self.forget_expected(TrackKind::Subtitle);
                self.remember_subtitle(None);
                self.call(self.player.select_subtitle(None));
            }
            PlaybackCommand::SelectSubtitle(Some(index)) => {
                self.forget_expected(TrackKind::Subtitle);
                self.remember_subtitle(Some(index));
                let track = self.map().and_then(|map| map.mpv_subtitle(index));
                if let Some(track) = track {
                    self.call(self.player.select_subtitle(Some(track)));
                }
            }
            PlaybackCommand::SetSubtitleStyle(style) => {
                let appearance = style.appearance();
                self.subtitles = Some(appearance);
                self.call(self.player.set_subtitle_appearance(&appearance));
            }
            PlaybackCommand::Stop => {
                self.cancel_countdown();
                self.cancel_load();
                self.release(true);
                self.emit(PlaybackEvent::Stopped);
            }
            PlaybackCommand::Previous | PlaybackCommand::Next if self.loading => {
                self.navigation = Some(command);
            }
            PlaybackCommand::Previous => {
                let Some(siblings) = self.siblings() else {
                    return;
                };
                match siblings.previous.as_ref().map(|episode| episode.id.clone()) {
                    Some(previous) => self.play(previous, PlayMode::Auto),
                    None => {
                        self.seek(SeekTarget::Absolute(Duration::ZERO));
                    }
                }
            }
            PlaybackCommand::Next => {
                let next = self
                    .siblings()
                    .and_then(|siblings| siblings.next.as_ref())
                    .map(|episode| episode.id.clone());
                if let Some(next) = next {
                    self.play(next, PlayMode::Auto);
                }
            }
            PlaybackCommand::PlayNextNow => {
                if let Some(countdown) = self.countdown.take() {
                    self.play(countdown.into_episode().id, PlayMode::Auto);
                }
            }
            PlaybackCommand::CancelNextUp => self.cancel_countdown(),
            PlaybackCommand::SetPrefs(prefs) => self.prefs = prefs,
            PlaybackCommand::UndoSkip => {
                if let Some(from) = self.skips.undo() {
                    self.seek(SeekTarget::Absolute(from));
                }
            }
            PlaybackCommand::RequestStats => self.report_stats(),
        }
    }

    fn report_stats(&self) {
        let Some(current) = self.current.as_ref().filter(|current| current.loaded) else {
            return;
        };
        self.emit(PlaybackEvent::Stats(Box::new(StatsReport {
            method: current.play_method(),
            transcode_reasons: current.source.transcode_reasons(),
            player: self.player.stats(),
        })));
    }

    /// A user pick replaces the default selection of that kind.
    fn forget_expected(&mut self, kind: TrackKind) {
        if let Some(current) = self.current.as_mut() {
            current.forget_expected(kind);
        }
    }

    fn stream(&self, index: i32, kind: MediaStreamType) -> Option<&MediaStream> {
        self.current
            .as_ref()?
            .source
            .media_streams
            .iter()
            .find(|stream| stream.index == index && stream.r#type == kind)
    }

    fn remember_audio(&mut self, index: i32) {
        if let Some(pick) = self
            .stream(index, MediaStreamType::Audio)
            .map(AudioPick::of)
        {
            self.remember(|choice| choice.audio = Some(pick));
        }
    }

    fn remember_subtitle(&mut self, index: Option<i32>) {
        let pick = match index {
            None => Some(SubtitlePick::Off),
            Some(index) => self
                .stream(index, MediaStreamType::Subtitle)
                .map(SubtitlePick::of),
        };
        if let Some(pick) = pick {
            self.remember(|choice| choice.subtitle = pick);
        }
    }

    /// Saves a user pick for the next episodes of the current series.
    fn remember(&mut self, change: impl FnOnce(&mut TrackChoice)) {
        if !self.prefs.remember_tracks {
            return;
        }
        let Some(series) = self
            .current
            .as_ref()
            .and_then(|current| current.series.clone())
        else {
            return;
        };
        let mut choice = self.memory.get(&series).unwrap_or_default();
        change(&mut choice);
        if let Err(error) = self.memory.set(&series, choice) {
            log::warn!("cannot save the track choice: {error}");
        }
    }

    /// The new request for a transcode made with another audio stream than
    /// the one remembered for its series.
    fn remembered_transcode(&self, prepared: &Prepared) -> Option<LoadRequest> {
        let Delivery::Transcoded { audio } = prepared.delivery else {
            return None;
        };
        let request = self.request.as_ref().filter(|request| {
            request.resume.is_none() && request.options.audio_stream_index.is_none()
        })?;
        let (wanted, _) = remembered(
            &self.memory,
            self.prefs,
            prepared.series.as_ref(),
            &prepared.source.media_streams,
        );
        let wanted = wanted.filter(|wanted| Some(*wanted) != audio)?;
        let mut request = request.clone();
        if request.item != prepared.item {
            request.item = prepared.item.clone();
            request.mode = PlayMode::Auto;
        }
        request.options.audio_stream_index = Some(wanted);
        Some(request)
    }

    fn map(&self) -> Option<&TrackMap> {
        self.current
            .as_ref()
            .and_then(|current| current.map.as_ref())
    }

    /// Stops the current item and starts fetching `item`; the controller
    /// keeps running meanwhile and loads it once fetched.
    fn play(&mut self, item: ItemId, mode: PlayMode) {
        self.start_load(LoadRequest {
            item,
            mode,
            options: PlaybackOptions {
                max_streaming_bitrate: self.quality.max_bitrate(),
                max_height: self.quality.max_height(),
                ..PlaybackOptions::default()
            },
            resume: None,
        });
    }

    fn start_load(&mut self, request: LoadRequest) {
        self.cancel_countdown();
        self.release(true);
        self.navigation = None;
        self.loading = true;
        self.pending = None;
        self.request = Some(request.clone());
        self.loader.start(self.session.clone(), request);
    }

    /// Whether the current item plays a server transcode.
    fn transcoding(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|current| matches!(current.delivery, Delivery::Transcoded { .. }))
    }

    /// Loads the current item again at its position, with its tracks, pause
    /// and speed; `audio` replaces its audio stream.
    fn reload(&mut self, audio: Option<i32>) {
        let Some(current) = self.current.as_ref() else {
            return;
        };
        let resume = Resume {
            position: current.position,
            audio: audio.or(current.audio),
            subtitle: current.pending_subtitle.or(current.subtitle),
            paused: current.paused,
            speed: self.speed,
        };
        let request = LoadRequest {
            item: current.item.clone(),
            mode: PlayMode::FromStart,
            options: PlaybackOptions {
                max_streaming_bitrate: self.quality.max_bitrate(),
                audio_stream_index: resume.audio,
                subtitle_stream_index: Some(text_subtitle(&current.source, resume.subtitle)),
                max_height: self.quality.max_height(),
            },
            resume: Some(resume),
        };
        self.start_load(request);
    }

    /// Applies a new quality: the item being loaded starts again, the item
    /// playing loads again unless it reached its end.
    fn set_quality(&mut self, quality: Quality) {
        if quality == self.quality {
            return;
        }
        self.quality = quality;
        if self.loading {
            if let Some(mut request) = self.request.clone() {
                request.options.max_streaming_bitrate = quality.max_bitrate();
                request.options.max_height = quality.max_height();
                self.start_load(request);
            }
        } else if self.current.as_ref().is_some_and(|current| !current.ended) {
            self.reload(None);
        }
    }

    /// Abandons the item being fetched or waiting to load, if any.
    fn cancel_load(&mut self) {
        self.loader.cancel();
        self.pending = None;
        self.loading = false;
        self.request = None;
        self.navigation = None;
    }

    /// The load ended without an item: a navigation waiting for it is dropped.
    fn load_failed(&mut self) {
        self.loading = false;
        self.request = None;
        self.navigation = None;
    }

    fn on_load(&mut self, message: LoadMessage) {
        // `shutdown` raises the flag before its message reaches the queue.
        if !self.loader.is_current(&message) || self.closing() {
            return;
        }
        match message.stage {
            LoadStage::Loading { item, title } => self.emit(PlaybackEvent::Loading { item, title }),
            LoadStage::Segments(segments) => {
                if let Some((prepared, _)) = self.pending.as_mut() {
                    prepared.segments = segments;
                } else if let Some(current) = self.current.as_mut() {
                    current.segments = segments;
                    self.show_segment();
                }
            }
            LoadStage::Done(outcome) => match *outcome {
                Ok(prepared) => {
                    if let Some(request) = self.remembered_transcode(&prepared) {
                        return self.start_load(request);
                    }
                    let deadline = Instant::now() + self.timings.unload_grace;
                    self.pending = Some((prepared, deadline));
                    self.load_pending();
                }
                Err(LoadFailure::Request(error)) => {
                    self.load_failed();
                    self.fail(error);
                }
                Err(LoadFailure::NoPlayableSource) => {
                    self.load_failed();
                    self.emit(PlaybackEvent::Error(PlaybackError::NoPlayableSource));
                }
                Err(LoadFailure::NothingToPlay) => {
                    self.load_failed();
                    self.emit(PlaybackEvent::Error(PlaybackError::NothingToPlay));
                }
            },
        }
    }

    /// Puts both delays back to zero for a new item; mpv would keep them.
    fn reset_delays(&mut self) {
        if self.subtitle_delay != 0 && self.call(self.player.set_subtitle_delay(0)) {
            self.subtitle_delay = 0;
            self.emit(PlaybackEvent::SubtitleDelay(0));
        }
        if self.audio_delay != 0 && self.call(self.player.set_audio_delay(0)) {
            self.audio_delay = 0;
            self.emit(PlaybackEvent::AudioDelay(0));
        }
    }

    /// Loads the fetched item, once the previous media has unloaded or its
    /// grace has passed.
    fn load_pending(&mut self) {
        if self.closing() {
            self.pending = None;
            return;
        }
        if self.awaiting_unload && self.player_open {
            return;
        }
        let Some((prepared, _)) = self.pending.take() else {
            return;
        };
        // Best effort: the style is already set unless something reset it.
        if let Some(appearance) = &self.subtitles
            && let Err(error) = self.player.set_subtitle_appearance(appearance)
        {
            log::warn!("cannot set the subtitle style: {error}");
        }
        if prepared.resume.is_none() {
            self.reset_delays();
        }
        self.skips.start(&prepared.item, prepared.resume.is_some());
        self.awaiting_unload = false;
        if let Err(error) = self.player.load(&prepared.media) {
            self.load_failed();
            return self.emit(PlaybackEvent::Error(PlaybackError::load(error)));
        }
        let mut current = Current::new(
            prepared.item,
            prepared.source,
            prepared.play_session_id,
            prepared.start.unwrap_or_default(),
            prepared.siblings,
        );
        current.series = prepared.series;
        current.delivery = prepared.delivery;
        current.resume = prepared.resume;
        // mpv keeps `pause` across loads, so a paused reload reports no change.
        current.paused = prepared.resume.is_some_and(|resume| resume.paused);
        current.trickplay = prepared.trickplay;
        current.chapters = prepared.chapters;
        current.segments = prepared.segments;
        self.current = Some(current);
        self.loading = false;
        self.request = None;
    }

    fn on_player_event(&mut self, event: PlayerEvent) {
        let global = matches!(
            event,
            PlayerEvent::Volume(_) | PlayerEvent::Muted(_) | PlayerEvent::Speed(_)
        );
        if self.awaiting_unload && !global {
            if matches!(event, PlayerEvent::Unloaded(_)) {
                self.awaiting_unload = false;
                self.load_pending();
            }
            return;
        }
        match event {
            PlayerEvent::FileLoaded { duration, tracks } => self.on_file_loaded(duration, tracks),
            PlayerEvent::Tracks(tracks) => self.on_tracks(&tracks),
            PlayerEvent::Position(position) => {
                if let Some(current) = self.current.as_mut() {
                    current.position = position;
                    current.position_moved(position);
                    self.emit(PlaybackEvent::Position(position));
                    self.show_segment();
                }
            }
            PlayerEvent::Paused(paused) => self.on_paused(paused),
            PlayerEvent::Speed(speed) => {
                self.speed = speed;
                self.emit(PlaybackEvent::Speed(speed));
            }
            PlayerEvent::Volume(volume) => self.emit(PlaybackEvent::Volume(volume)),
            PlayerEvent::Muted(muted) => self.emit(PlaybackEvent::Muted(muted)),
            PlayerEvent::Seeking => {
                if self.current.as_ref().is_some_and(|c| c.seek_pending) {
                    self.emit(PlaybackEvent::Seeking);
                }
            }
            PlayerEvent::Seeked => {
                if let Some(current) = self.current.as_mut().filter(|c| c.seek_pending) {
                    current.seek_pending = false;
                    self.emit(PlaybackEvent::Seeked);
                    self.report(ReportKind::Progress);
                    self.schedule_progress();
                }
            }
            PlayerEvent::Ended => self.on_ended(),
            PlayerEvent::Unloaded(UnloadReason::Error(message)) => {
                let Some(current) = self.current.take() else {
                    return;
                };
                self.next_progress = None;
                if current.reporting() {
                    self.reporter.send(ReportKind::Stopped, current.report());
                }
                self.stop_transcoding(&current);
                self.navigation = None;
                self.emit(PlaybackEvent::Error(PlaybackError::load(message)));
            }
            PlayerEvent::Unloaded(_) => {}
        }
    }

    /// Handles the end of the media, once per load: a real end reports the
    /// stop and starts the next episode countdown; an early one is a stop.
    fn on_ended(&mut self) {
        let Some(current) = self.current.as_mut().filter(|c| c.reporting()) else {
            return;
        };
        if !current.at_real_end() {
            self.release(true);
            return self.emit(PlaybackEvent::Stopped);
        }
        self.reporter.send(ReportKind::Stopped, current.report());
        current.ended = true;
        let next = current.siblings.next.clone();
        self.next_progress = None;
        self.show_segment();
        self.emit(PlaybackEvent::Ended);
        if let Some(next) = next {
            self.start_countdown(next);
        }
    }

    fn on_file_loaded(&mut self, duration: Option<Duration>, mut tracks: Vec<Track>) {
        let Some(current) = self.current.as_mut().filter(|c| !c.loaded) else {
            return;
        };
        let transcoded = matches!(current.delivery, Delivery::Transcoded { .. });
        let subtitles = current
            .source
            .media_streams
            .iter()
            .filter(|s| s.r#type == MediaStreamType::Subtitle);
        for stream in subtitles {
            let url = if transcoded {
                // A transcode carries no subtitle: the server serves them as files.
                let delivered = stream.delivery_method == Some(SubtitleDeliveryMethod::External);
                match stream.delivery_url.as_deref().filter(|_| delivered) {
                    Some(url) => match self.session.server_url(url) {
                        Some(url) => url.to_string(),
                        None => continue,
                    },
                    None => continue,
                }
            } else if stream.is_external {
                self.session
                    .subtitle_url(
                        &current.item,
                        &current.source.id,
                        stream.index,
                        subtitle_format(stream.codec.as_deref()),
                    )
                    .to_string()
            } else {
                continue;
            };
            let title = stream.display_title.as_deref().or(stream.title.as_deref());
            let language = stream.language.as_deref();
            if transcoded {
                // The server may first extract it from the source file, which takes minutes.
                if let Err(error) = self
                    .player
                    .add_external_subtitle_later(&url, title, language)
                {
                    log::warn!("cannot add the subtitle file: {error}");
                }
            } else if let Ok(id) = self.player.add_external_subtitle(&url, title, language) {
                current.externals.push((stream.index, id));
                tracks.push(Track {
                    id,
                    kind: TrackKind::Subtitle,
                    language: stream.language.clone(),
                    title: title.map(str::to_owned),
                    codec: stream.codec.clone(),
                    channels: None,
                    is_default: false,
                    is_forced: stream.is_forced,
                    is_external: true,
                    is_selected: false,
                    stream_index: None,
                    external_url: Some(url.clone()),
                });
            }
            current.external_urls.push((stream.index, url));
        }

        let map = TrackMap::new(
            &current.source.media_streams,
            &tracks,
            &current.externals,
            &current.external_urls,
            current.delivery,
        );
        let resume = current.resume.take();
        let (remembered_audio, remembered_subtitle) = match resume {
            Some(_) => (None, None),
            None => remembered(
                &self.memory,
                self.prefs,
                current.series.as_ref(),
                &current.source.media_streams,
            ),
        };
        let default_audio = current.source.default_audio_stream_index;
        let audio = resume
            .map_or(remembered_audio, |resume| resume.audio)
            .and_then(|index| map.mpv_audio(index))
            .or_else(|| default_audio.and_then(|index| map.mpv_audio(index)));
        if let Some(audio) = audio
            && self.player.select_audio(Some(audio)).is_ok()
        {
            mark_selected(&mut tracks, TrackKind::Audio, Some(audio));
            current.expected_audio = Some(audio);
        }
        let default_subtitle = current.source.default_subtitle_stream_index;
        let deliverable = |index: i32| {
            map.mpv_subtitle(index).is_some()
                || current
                    .external_urls
                    .iter()
                    .any(|(added, _)| *added == index)
        };
        let wanted = match (resume, remembered_subtitle) {
            (Some(resume), _) => resume.subtitle,
            (None, Some(Some(index))) if !deliverable(index) => default_subtitle,
            (None, Some(remembered)) => remembered,
            (None, None) => default_subtitle,
        }
        .filter(|index| *index >= 0);
        let subtitle = wanted.and_then(|index| map.mpv_subtitle(index));
        current.pending_subtitle = wanted.filter(|index| {
            transcoded
                && subtitle.is_none()
                && current
                    .external_urls
                    .iter()
                    .any(|(added, _)| added == index)
        });
        if self.player.select_subtitle(subtitle).is_ok() {
            mark_selected(&mut tracks, TrackKind::Subtitle, subtitle);
            current.expected_subtitle = Some(subtitle);
        }
        let _ = self
            .player
            .set_speed(resume.map_or(1.0, |resume| resume.speed));
        // mpv keeps `pause` across loads without reporting it.
        match self
            .player
            .set_paused(resume.is_some_and(|resume| resume.paused))
        {
            Ok(()) | Err(PlayerError::NoMediaLoaded) => {}
            Err(error) => {
                let _ = self
                    .events
                    .send(PlaybackEvent::Error(PlaybackError::command(&error)));
            }
        }

        current.update_tracks(&tracks);
        let (audio, subtitles) = current.refresh_options().unwrap_or_default();
        current.loaded = true;
        let duration = duration.or_else(|| current.source.run_time_ticks.map(Ticks::to_duration));
        current.duration = duration;
        let has_next = current.siblings.next.is_some();
        let trickplay = current.trickplay.clone();
        let chapters = current.chapters.clone();
        if let Some(trickplay) = trickplay {
            self.emit(PlaybackEvent::Trickplay(trickplay));
        }
        self.emit(PlaybackEvent::Ready {
            duration,
            audio,
            subtitles,
            has_previous: true,
            has_next,
        });
        self.emit(PlaybackEvent::Chapters(chapters));
        if let Some(resume) = resume {
            self.emit(PlaybackEvent::Paused(resume.paused));
        }
        self.schedule_progress();
        self.report(ReportKind::Start);
        self.show_segment();
        if let Some(command) = self.navigation.take() {
            self.on_command(command);
        }
    }

    fn on_tracks(&mut self, tracks: &[Track]) {
        let Some(current) = self.current.as_mut().filter(|c| c.loaded) else {
            return;
        };
        if let Some(index) = current.pending_subtitle {
            let url = current
                .external_urls
                .iter()
                .find(|(added, _)| *added == index)
                .map(|(_, url)| url.as_str());
            let arrived = tracks
                .iter()
                .find(|track| {
                    track.kind == TrackKind::Subtitle
                        && url.is_some()
                        && track.external_url.as_deref() == url
                })
                .map(|track| track.id);
            if let Some(id) = arrived {
                current.pending_subtitle = None;
                if self.player.select_subtitle(Some(id)).is_ok() {
                    current.expected_subtitle = Some(Some(id));
                }
            }
        }
        if !current.selection_settled(tracks) {
            return;
        }
        let changed = current.update_tracks(tracks);
        if let Some((audio, subtitles)) = current.refresh_options() {
            self.emit(PlaybackEvent::TracksChanged { audio, subtitles });
        }
        if changed {
            self.report(ReportKind::Progress);
            self.schedule_progress();
        }
    }

    fn on_paused(&mut self, paused: bool) {
        self.emit(PlaybackEvent::Paused(paused));
        let Some(current) = self.current.as_mut() else {
            return;
        };
        if current.paused == paused {
            return;
        }
        current.paused = paused;
        if current.reporting() {
            self.report(ReportKind::Progress);
            self.schedule_progress();
        }
    }
}

/// The subtitle stream to ask of the server: `subtitle` when the server can
/// deliver it as a file, `-1` otherwise, so that a transcode never has
/// subtitles burned into its picture.
fn text_subtitle(source: &MediaSourceInfo, subtitle: Option<i32>) -> i32 {
    subtitle
        .filter(|index| {
            source.media_streams.iter().any(|stream| {
                stream.index == *index
                    && stream.r#type == MediaStreamType::Subtitle
                    && (stream.is_text_subtitle_stream || stream.is_external)
            })
        })
        .unwrap_or(-1)
}

/// The audio and subtitle streams of `streams` matching the choice remembered
/// for `series`, as [`best_audio`] and [`best_subtitle`] give them; nothing
/// when remembering is off or the item belongs to no series.
fn remembered(
    memory: &TrackMemory,
    prefs: PlaybackPrefs,
    series: Option<&ItemId>,
    streams: &[MediaStream],
) -> (Option<i32>, Option<Option<i32>>) {
    let choice = series
        .filter(|_| prefs.remember_tracks)
        .and_then(|series| memory.get(series));
    let Some(choice) = choice else {
        return (None, None);
    };
    let audio = choice
        .audio
        .as_ref()
        .and_then(|pick| best_audio(streams, pick));
    (audio, best_subtitle(streams, &choice.subtitle))
}
