//! The playback controller: a task that plays one item at a time and keeps
//! the server informed.

use std::sync::Arc;
use std::time::Duration;

use oxyfin_api::{ItemId, MediaStreamType, PlaybackReport, Session, Ticks};
use oxyfin_player::{MediaSource, PlayerError, PlayerEvent, SeekTarget, Track, TrackKind};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::{Instant, sleep_until};

use super::current::{Current, mark_selected};
use super::media_player::{MediaPlayer, PlayerEvents};
use super::tracks::{TrackMap, subtitle_format};
use super::{AudioOption, SubtitleOption};
use crate::error::CoreError;
use crate::models::{Episode, ItemSummary, UserState};

/// Interval between two progress reports while playing.
pub const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
/// Step of the skip backward and skip forward buttons.
pub const SEEK_STEP: Duration = Duration::from_secs(10);
/// Length of the countdown before the next episode starts.
pub const NEXT_UP_COUNTDOWN: Duration = Duration::from_secs(10);
/// Longest wait for the last report when shutting down.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
/// Playback speeds offered to the user.
pub const SPEEDS: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

const NO_DIRECT_PLAY: &str = "the server offers no source that can be played directly";

/// Durations driving the controller's timers.
///
/// [`Default`] gives the production values; tests shorten them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackTimings {
    /// Interval between two progress reports while playing.
    pub progress_interval: Duration,
    /// Duration of one step of the next episode countdown.
    pub next_up_countdown_tick: Duration,
    /// Longest wait for the last report when shutting down.
    pub shutdown_timeout: Duration,
}

impl Default for PlaybackTimings {
    fn default() -> Self {
        Self {
            progress_interval: PROGRESS_INTERVAL,
            next_up_countdown_tick: Duration::from_secs(1),
            shutdown_timeout: SHUTDOWN_TIMEOUT,
        }
    }
}

/// Where playback of an item starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    /// Resume at the saved position, if any.
    Auto,
    /// Start at the beginning.
    FromStart,
}

/// A request sent to the playback controller.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaybackCommand {
    /// Play an item, replacing the current one.
    Play {
        /// Item to play.
        item: ItemId,
        /// Where playback starts.
        mode: PlayMode,
    },
    /// Pause or resume.
    SetPaused(bool),
    /// Seek in the current item.
    Seek(SeekTarget),
    /// Go to the previous item.
    Previous,
    /// Go to the next item.
    Next,
    /// Start the next episode without waiting for the countdown.
    PlayNextNow,
    /// Cancel the next episode countdown.
    CancelNextUp,
    /// Set the playback speed.
    SetSpeed(f64),
    /// Set the volume, in percent.
    SetVolume(f64),
    /// Mute or unmute.
    SetMuted(bool),
    /// Select the audio track with this Jellyfin stream index.
    SelectAudio(i32),
    /// Select the subtitle track with this Jellyfin stream index, or none.
    SelectSubtitle(Option<i32>),
    /// Stop playback.
    Stop,
}

/// What the UI shows while an item plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackTitle {
    /// Title of the item.
    pub title: String,
    /// Name of the series, for an episode.
    pub series_name: Option<String>,
    /// Label such as `S01E02`, for an episode.
    pub episode_label: Option<String>,
}

/// A change reported by the playback controller.
#[derive(Clone, Debug, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "events are moved through a channel, never stored in bulk"
)]
pub enum PlaybackEvent {
    /// An item is being loaded.
    Loading {
        /// Item being loaded.
        item: ItemId,
        /// Titles to display.
        title: PlaybackTitle,
    },
    /// The item is loaded and playing.
    Ready {
        /// Duration, when known.
        duration: Option<Duration>,
        /// Audio tracks.
        audio: Vec<AudioOption>,
        /// Subtitle tracks.
        subtitles: Vec<SubtitleOption>,
        /// Whether a previous item exists.
        has_previous: bool,
        /// Whether a next item exists.
        has_next: bool,
    },
    /// The position changed.
    Position(Duration),
    /// The paused state changed.
    Paused(bool),
    /// The playback speed changed.
    Speed(f64),
    /// The volume changed.
    Volume(f64),
    /// The muted state changed.
    Muted(bool),
    /// A seek requested by the user started.
    Seeking,
    /// A seek requested by the user finished.
    Seeked,
    /// The tracks or their selection changed.
    TracksChanged {
        /// Audio tracks.
        audio: Vec<AudioOption>,
        /// Subtitle tracks.
        subtitles: Vec<SubtitleOption>,
    },
    /// The item played to its end.
    Ended,
    /// The next episode starts in `remaining` seconds.
    NextUpCountdown {
        /// Episode about to start.
        episode: Episode,
        /// Seconds left.
        remaining: u32,
    },
    /// The next episode countdown was cancelled.
    NextUpCancelled,
    /// Playback stopped.
    Stopped,
    /// Playback failed, with a message to display.
    Error(String),
    /// The server no longer accepts the session; playback stopped.
    SessionExpired,
}

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

#[derive(Clone, Copy)]
enum Report {
    Start,
    Progress,
    Stopped,
}

/// A task that plays one item at a time: it loads the media, applies the
/// default tracks, relays the player events and reports playback to the
/// server.
pub struct PlaybackController {
    session: Session,
    player: Arc<dyn MediaPlayer>,
    events: UnboundedSender<PlaybackEvent>,
    timings: PlaybackTimings,
    current: Option<Current>,
    next_progress: Option<Instant>,
}

impl PlaybackController {
    /// Starts the controller on the current tokio runtime with the default
    /// [`PlaybackTimings`].
    ///
    /// Returns the handle that sends commands and the receiver of the
    /// controller's events. The task ends when every handle is dropped.
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
        let controller = Self {
            session,
            player,
            events: event_sender,
            timings,
            current: None,
            next_progress: None,
        };
        tokio::spawn(controller.run(command_receiver, events.0));
        (PlaybackHandle { commands }, event_receiver)
    }

    async fn run(
        mut self,
        mut commands: UnboundedReceiver<PlaybackCommand>,
        mut player_events: UnboundedReceiver<PlayerEvent>,
    ) {
        let mut player_open = true;
        loop {
            let deadline = self.next_progress;
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => self.on_command(command).await,
                    None => break,
                },
                event = player_events.recv(), if player_open => match event {
                    Some(event) => self.on_player_event(event).await,
                    None => player_open = false,
                },
                () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    self.report(Report::Progress).await;
                    self.schedule_progress();
                }
            }
        }
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
                self.emit(PlaybackEvent::Error(CoreError::from(error).to_string()));
                false
            }
        }
    }

    fn schedule_progress(&mut self) {
        self.next_progress = match &self.current {
            Some(current) if current.loaded && !current.paused => {
                Some(Instant::now() + self.timings.progress_interval)
            }
            _ => None,
        };
    }

    fn fail(&mut self, error: CoreError) {
        if error == CoreError::SessionExpired {
            self.expire();
        } else {
            self.emit(PlaybackEvent::Error(error.to_string()));
        }
    }

    fn expire(&mut self) {
        self.current = None;
        self.next_progress = None;
        let _ = self.player.stop();
        self.emit(PlaybackEvent::SessionExpired);
    }

    /// Sends a report for the current item; returns `false` when the session expired.
    async fn report(&mut self, kind: Report) -> bool {
        match &self.current {
            Some(current) => self.send_report(kind, current.report()).await,
            None => true,
        }
    }

    async fn send_report(&mut self, kind: Report, report: PlaybackReport) -> bool {
        let result = match kind {
            Report::Start => self.session.report_playback_start(&report).await,
            Report::Progress => self.session.report_playback_progress(&report).await,
            Report::Stopped => self.session.report_playback_stopped(&report).await,
        };
        match result.map_err(CoreError::from) {
            Err(CoreError::SessionExpired) => {
                self.expire();
                false
            }
            _ => true,
        }
    }

    async fn on_command(&mut self, command: PlaybackCommand) {
        match command {
            PlaybackCommand::Play { item, mode } => self.play(item, mode).await,
            PlaybackCommand::SetPaused(paused) => {
                self.call(self.player.set_paused(paused));
            }
            PlaybackCommand::Seek(target) => {
                if self.call(self.player.seek(target))
                    && let Some(current) = self.current.as_mut()
                {
                    current.seek_pending = true;
                }
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
            PlaybackCommand::Stop => self.stop().await,
            PlaybackCommand::Previous
            | PlaybackCommand::Next
            | PlaybackCommand::PlayNextNow
            | PlaybackCommand::CancelNextUp => {}
        }
    }

    fn map(&self) -> Option<&TrackMap> {
        self.current
            .as_ref()
            .and_then(|current| current.map.as_ref())
    }

    async fn stop(&mut self) {
        self.next_progress = None;
        let _ = self.player.stop();
        if let Some(current) = self.current.take()
            && !self.send_report(Report::Stopped, current.report()).await
        {
            return;
        }
        self.emit(PlaybackEvent::Stopped);
    }

    async fn play(&mut self, item: ItemId, mode: PlayMode) {
        self.next_progress = None;
        if let Some(current) = self.current.take()
            && !self.send_report(Report::Stopped, current.report()).await
        {
            return;
        }
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

        let info = match self.session.playback_info(&item).await {
            Ok(info) => info,
            Err(error) => return self.fail(error.into()),
        };
        let Some(source) = info
            .media_sources
            .into_iter()
            .find(|source| source.supports_direct_play)
        else {
            return self.emit(PlaybackEvent::Error(NO_DIRECT_PLAY.to_owned()));
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
        if let Err(error) = self.player.load(&media) {
            return self.emit(PlaybackEvent::Error(CoreError::from(error).to_string()));
        }
        self.current = Some(Current::new(
            item,
            source,
            info.play_session_id,
            start.unwrap_or_default(),
        ));
    }

    async fn on_player_event(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::FileLoaded { duration, tracks } => {
                self.on_file_loaded(duration, tracks).await;
            }
            PlayerEvent::Tracks(tracks) => self.on_tracks(&tracks).await,
            PlayerEvent::Position(position) => {
                if let Some(current) = self.current.as_mut() {
                    current.position = position;
                    self.emit(PlaybackEvent::Position(position));
                }
            }
            PlayerEvent::Paused(paused) => self.on_paused(paused).await,
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
                    self.report(Report::Progress).await;
                    self.schedule_progress();
                }
            }
            PlayerEvent::Ended => {}
            PlayerEvent::Unloaded(oxyfin_player::UnloadReason::Error(message)) => {
                let Some(current) = self.current.take() else {
                    return;
                };
                self.next_progress = None;
                if self.send_report(Report::Stopped, current.report()).await {
                    self.emit(PlaybackEvent::Error(message));
                }
            }
            PlayerEvent::Unloaded(_) => {}
        }
    }

    async fn on_file_loaded(&mut self, duration: Option<Duration>, mut tracks: Vec<Track>) {
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
        self.emit(PlaybackEvent::Ready {
            duration,
            audio,
            subtitles,
            has_previous: false,
            has_next: false,
        });
        self.schedule_progress();
        self.report(Report::Start).await;
    }

    async fn on_tracks(&mut self, tracks: &[Track]) {
        let Some(current) = self.current.as_mut().filter(|c| c.loaded) else {
            return;
        };
        let changed = current.update_tracks(tracks);
        if let Some((audio, subtitles)) = current.refresh_options() {
            self.emit(PlaybackEvent::TracksChanged { audio, subtitles });
        }
        if changed {
            self.report(Report::Progress).await;
            self.schedule_progress();
        }
    }

    async fn on_paused(&mut self, paused: bool) {
        self.emit(PlaybackEvent::Paused(paused));
        let Some(current) = self.current.as_mut() else {
            return;
        };
        if current.paused == paused {
            return;
        }
        current.paused = paused;
        if current.loaded {
            self.report(Report::Progress).await;
            self.schedule_progress();
        }
    }
}
