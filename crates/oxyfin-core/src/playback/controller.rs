//! The playback controller: a task that plays one item at a time and keeps
//! the server informed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use oxyfin_api::{
    ItemId, MediaSourceInfo, MediaStreamType, PlaybackOptions, Session, SubtitleDeliveryMethod,
    Ticks,
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
use super::tracks::{Delivery, TrackMap, subtitle_format};
use super::types::{PlayMode, PlaybackCommand, PlaybackError, PlaybackEvent, PlaybackTimings};
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
    /// Set by [`PlaybackHandle::shutdown`]: no new item is loaded.
    closing: Arc<AtomicBool>,
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
    /// A controller is bound to `session` and consumes `events`, so it lives
    /// for one signed-in session. After a sign-out or an expiry, the binary
    /// creates a new player, event sink and controller, and therefore a new
    /// `VideoRenderer`.
    pub fn spawn(
        session: Session,
        player: Arc<dyn MediaPlayer>,
        events: PlayerEvents,
    ) -> (PlaybackHandle, UnboundedReceiver<PlaybackEvent>) {
        Self::spawn_with_timings(session, player, events, PlaybackTimings::default())
    }

    /// Same as [`PlaybackController::spawn`], with custom timings.
    pub fn spawn_with_timings(
        session: Session,
        player: Arc<dyn MediaPlayer>,
        events: PlayerEvents,
        timings: PlaybackTimings,
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
            pending: None,
            closing: closing.clone(),
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
            let countdown = self.countdown.as_ref().map(|countdown| countdown.deadline);
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

    /// Emits the segment offered at the current position when it changed.
    fn show_segment(&mut self) {
        let change = self
            .current
            .as_mut()
            .filter(|current| current.loaded)
            .and_then(Current::update_segment);
        if let Some(segment) = change {
            self.emit(PlaybackEvent::Segment(segment));
        }
    }

    /// Cancels the next episode countdown, if one runs.
    fn cancel_countdown(&mut self) {
        if self.countdown.take().is_some() {
            self.emit(PlaybackEvent::NextUpCancelled);
        }
    }

    fn start_countdown(&mut self, episode: Episode) {
        let (countdown, event) = Countdown::start(episode, self.timings.next_up_countdown_tick);
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
    fn seek(&mut self, target: SeekTarget) {
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
            return;
        }
        let Some(current) = self.current.as_mut() else {
            return;
        };
        current.seek_pending = true;
        if backward && current.ended {
            current.ended = false;
            self.report(ReportKind::Start);
            self.schedule_progress();
        }
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
            PlaybackCommand::Seek(target) => self.seek(target),
            PlaybackCommand::SetSpeed(speed) => {
                self.call(self.player.set_speed(speed));
            }
            PlaybackCommand::SetVolume(volume) => {
                self.call(self.player.set_volume(volume));
            }
            PlaybackCommand::SetMuted(muted) => {
                self.call(self.player.set_muted(muted));
            }
            PlaybackCommand::SelectAudio(index) => {
                self.forget_expected(TrackKind::Audio);
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
                self.call(self.player.select_subtitle(None));
            }
            PlaybackCommand::SelectSubtitle(Some(index)) => {
                self.forget_expected(TrackKind::Subtitle);
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
                    None => self.seek(SeekTarget::Absolute(Duration::ZERO)),
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
        }
    }

    /// A user pick replaces the default selection of that kind.
    fn forget_expected(&mut self, kind: TrackKind) {
        if let Some(current) = self.current.as_mut() {
            current.forget_expected(kind);
        }
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
            subtitle: current.subtitle,
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
        current.delivery = prepared.delivery;
        current.resume = prepared.resume;
        // mpv keeps `pause` across loads, so a paused reload reports no change.
        current.paused = prepared.resume.is_some_and(|resume| resume.paused);
        current.trickplay = prepared.trickplay;
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
            if let Ok(id) = self.player.add_external_subtitle(&url, title, language) {
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
        let default_audio = current.source.default_audio_stream_index;
        let audio = resume
            .map_or(default_audio, |resume| resume.audio.or(default_audio))
            .and_then(|index| map.mpv_audio(index));
        if let Some(audio) = audio
            && self.player.select_audio(Some(audio)).is_ok()
        {
            mark_selected(&mut tracks, TrackKind::Audio, Some(audio));
            current.expected_audio = Some(audio);
        }
        let subtitle = resume
            .map_or(current.source.default_subtitle_stream_index, |resume| {
                resume.subtitle
            })
            .filter(|index| *index >= 0)
            .and_then(|index| map.mpv_subtitle(index));
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
