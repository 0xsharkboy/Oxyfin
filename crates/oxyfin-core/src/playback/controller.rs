//! The playback controller: a task that plays one item at a time and keeps
//! the server informed.

use std::sync::Arc;
use std::time::Duration;

use oxyfin_api::{ItemId, MediaStreamType, Session, Ticks};
use oxyfin_player::{
    MediaSource, PlayerError, PlayerEvent, SeekTarget, Track, TrackKind, UnloadReason,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::{Instant, sleep_until, timeout_at};

use super::countdown::Countdown;
use super::current::{Current, Siblings, mark_selected};
use super::media_player::{MediaPlayer, PlayerEvents};
use super::queue::neighbours;
use super::reporter::{ReportKind, Reporter};
use super::tracks::{TrackMap, subtitle_format};
use super::types::{
    PlayMode, PlaybackCommand, PlaybackError, PlaybackEvent, PlaybackTimings, PlaybackTitle,
};
use crate::catalog::Catalog;
use crate::error::CoreError;
use crate::models::{Episode, ItemSummary, UserState};

/// Sends commands to a running [`PlaybackController`].
#[derive(Clone, Debug)]
pub struct PlaybackHandle {
    commands: UnboundedSender<PlaybackCommand>,
}

impl PlaybackHandle {
    /// Sends a command; ignored once the controller has ended.
    pub fn send(&self, command: PlaybackCommand) {
        let _ = self.commands.send(command);
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
}

impl PlaybackController {
    /// Starts the controller on the current tokio runtime with the default
    /// [`PlaybackTimings`].
    ///
    /// Returns the handle that sends commands and the receiver of the
    /// controller's events. The task ends when every handle is dropped,
    /// after the queued reports are sent.
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
        let (commands, command_receiver) = unbounded_channel();
        let (event_sender, event_receiver) = unbounded_channel();
        let (expired_sender, expired) = unbounded_channel();
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
        };
        tokio::spawn(controller.run(command_receiver, expired));
        (PlaybackHandle { commands }, event_receiver)
    }

    async fn run(
        mut self,
        mut commands: UnboundedReceiver<PlaybackCommand>,
        mut expired: UnboundedReceiver<()>,
    ) {
        loop {
            let deadline = self.next_progress;
            let countdown = self.countdown.as_ref().map(|countdown| countdown.deadline);
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => self.on_command(command).await,
                    None => break,
                },
                event = self.player_events.recv(), if self.player_open => match event {
                    Some(event) => self.on_player_event(event),
                    None => self.player_open = false,
                },
                Some(()) = expired.recv() => self.expire(),
                () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    self.report(ReportKind::Progress);
                    self.schedule_progress();
                }
                () = sleep_until(countdown.unwrap_or_else(Instant::now)), if countdown.is_some() => {
                    self.tick_countdown().await;
                }
            }
        }
        let _ = self.reporter.close().await;
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

    async fn tick_countdown(&mut self) {
        let Some(countdown) = self.countdown.as_mut() else {
            return;
        };
        if let Some(event) = countdown.step() {
            return self.emit(event);
        }
        if let Some(countdown) = self.countdown.take() {
            self.play(countdown.into_episode().id, PlayMode::Auto).await;
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

    async fn on_command(&mut self, command: PlaybackCommand) {
        match command {
            PlaybackCommand::Play { item, mode } => self.play(item, mode).await,
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
                let track = self.map().and_then(|map| map.mpv_audio(index));
                if let Some(track) = track {
                    self.call(self.player.select_audio(Some(track)));
                }
            }
            PlaybackCommand::SelectSubtitle(None) => {
                self.call(self.player.select_subtitle(None));
            }
            PlaybackCommand::SelectSubtitle(Some(index)) => {
                let track = self.map().and_then(|map| map.mpv_subtitle(index));
                if let Some(track) = track {
                    self.call(self.player.select_subtitle(Some(track)));
                }
            }
            PlaybackCommand::Stop => {
                self.cancel_countdown();
                self.release(true);
                self.emit(PlaybackEvent::Stopped);
            }
            PlaybackCommand::Previous => {
                let Some(siblings) = self.siblings() else {
                    return;
                };
                match siblings.previous.as_ref().map(|episode| episode.id.clone()) {
                    Some(previous) => self.play(previous, PlayMode::Auto).await,
                    None => self.seek(SeekTarget::Absolute(Duration::ZERO)),
                }
            }
            PlaybackCommand::Next => {
                let next = self
                    .siblings()
                    .and_then(|siblings| siblings.next.as_ref())
                    .map(|episode| episode.id.clone());
                if let Some(next) = next {
                    self.play(next, PlayMode::Auto).await;
                }
            }
            PlaybackCommand::PlayNextNow => {
                if let Some(countdown) = self.countdown.take() {
                    self.play(countdown.into_episode().id, PlayMode::Auto).await;
                }
            }
            PlaybackCommand::CancelNextUp => self.cancel_countdown(),
        }
    }

    fn map(&self) -> Option<&TrackMap> {
        self.current
            .as_ref()
            .and_then(|current| current.map.as_ref())
    }

    async fn play(&mut self, item: ItemId, mode: PlayMode) {
        self.cancel_countdown();
        self.release(true);
        let dto = match self.session.item(&item).await {
            Ok(dto) => dto,
            Err(error) => return self.fail(error.into()),
        };
        let title = match ItemSummary::from_dto(&dto) {
            Some(summary) => PlaybackTitle {
                title: summary.title,
                series_name: summary.series_name,
                episode_label: summary.episode_label,
            },
            None => PlaybackTitle {
                title: dto.name.clone().unwrap_or_default(),
                series_name: None,
                episode_label: None,
            },
        };
        let saved = UserState::from_dto(dto.user_data.as_ref()).position;
        self.emit(PlaybackEvent::Loading {
            item: item.clone(),
            title,
        });

        let siblings = match Episode::from_dto(&dto) {
            None => Siblings::default(),
            Some(episode) => {
                let catalog = Catalog::new(self.session.clone());
                match catalog.all_episodes(&episode.series_id).await {
                    Ok(list) => {
                        let (previous, next) = neighbours(&list, &item);
                        Siblings { previous, next }
                    }
                    Err(CoreError::SessionExpired) => return self.expire(),
                    Err(_) => Siblings::default(),
                }
            }
        };
        let info = match self.session.playback_info(&item).await {
            Ok(info) => info,
            Err(error) => return self.fail(error.into()),
        };
        let Some(source) = info
            .media_sources
            .into_iter()
            .find(|source| source.supports_direct_play)
        else {
            return self.emit(PlaybackEvent::Error(PlaybackError::NoDirectPlaySource));
        };
        let stream = self.session.stream_source(&item, &source.id);
        let mut media = MediaSource::new(stream.url.as_str())
            .with_header("Authorization", stream.authorization);
        let start = match mode {
            PlayMode::Auto if !saved.is_zero() => Some(saved),
            _ => None,
        };
        if let Some(start) = start {
            media = media.with_start(start);
        }
        self.wait_for_unload().await;
        if let Err(error) = self.player.load(&media) {
            return self.emit(PlaybackEvent::Error(PlaybackError::load(error)));
        }
        self.current = Some(Current::new(
            item,
            source,
            info.play_session_id,
            start.unwrap_or_default(),
            siblings,
        ));
    }

    /// Drops the stale events of the stopped media until its unload, waiting
    /// at most the unload grace.
    async fn wait_for_unload(&mut self) {
        let deadline = Instant::now() + self.timings.unload_grace;
        while self.awaiting_unload && self.player_open {
            match timeout_at(deadline, self.player_events.recv()).await {
                Ok(Some(event)) => self.on_player_event(event),
                Ok(None) => self.player_open = false,
                Err(_) => break,
            }
        }
        self.awaiting_unload = false;
    }

    fn on_player_event(&mut self, event: PlayerEvent) {
        let global = matches!(
            event,
            PlayerEvent::Volume(_) | PlayerEvent::Muted(_) | PlayerEvent::Speed(_)
        );
        if self.awaiting_unload && !global {
            if matches!(event, PlayerEvent::Unloaded(_)) {
                self.awaiting_unload = false;
            }
            return;
        }
        match event {
            PlayerEvent::FileLoaded { duration, tracks } => self.on_file_loaded(duration, tracks),
            PlayerEvent::Tracks(tracks) => self.on_tracks(&tracks),
            PlayerEvent::Position(position) => {
                if let Some(current) = self.current.as_mut() {
                    current.position = position;
                    self.emit(PlaybackEvent::Position(position));
                }
            }
            PlayerEvent::Paused(paused) => self.on_paused(paused),
            PlayerEvent::Speed(speed) => self.emit(PlaybackEvent::Speed(speed)),
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
        self.emit(PlaybackEvent::Ended);
        if let Some(next) = next {
            self.start_countdown(next);
        }
    }

    fn on_file_loaded(&mut self, duration: Option<Duration>, mut tracks: Vec<Track>) {
        let Some(current) = self.current.as_mut().filter(|c| !c.loaded) else {
            return;
        };
        let externals = current
            .source
            .media_streams
            .iter()
            .filter(|s| s.is_external && s.r#type == MediaStreamType::Subtitle);
        for stream in externals {
            let url = self
                .session
                .subtitle_url(
                    &current.item,
                    &current.source.id,
                    stream.index,
                    subtitle_format(stream.codec.as_deref()),
                )
                .to_string();
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
        );
        let audio = current
            .source
            .default_audio_stream_index
            .and_then(|index| map.mpv_audio(index));
        if let Some(audio) = audio
            && self.player.select_audio(Some(audio)).is_ok()
        {
            mark_selected(&mut tracks, TrackKind::Audio, Some(audio));
        }
        let subtitle = current
            .source
            .default_subtitle_stream_index
            .filter(|index| *index >= 0)
            .and_then(|index| map.mpv_subtitle(index));
        if self.player.select_subtitle(subtitle).is_ok() {
            mark_selected(&mut tracks, TrackKind::Subtitle, subtitle);
        }
        let _ = self.player.set_speed(1.0);

        current.update_tracks(&tracks);
        let (audio, subtitles) = current.refresh_options().unwrap_or_default();
        current.loaded = true;
        let duration = duration.or_else(|| current.source.run_time_ticks.map(Ticks::to_duration));
        current.duration = duration;
        let has_next = current.siblings.next.is_some();
        self.emit(PlaybackEvent::Ready {
            duration,
            audio,
            subtitles,
            has_previous: true,
            has_next,
        });
        self.schedule_progress();
        self.report(ReportKind::Start);
    }

    fn on_tracks(&mut self, tracks: &[Track]) {
        let Some(current) = self.current.as_mut().filter(|c| c.loaded) else {
            return;
        };
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
