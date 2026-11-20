//! MPRIS service: shows the playback to the desktop on the D-Bus session bus,
//! so media keys, desktop media widgets and `playerctl` can control it.
//!
//! The service runs on the Tokio runtime and mirrors the playback events the
//! app forwards with [`Mpris::update`]. Calls from the bus come back as
//! [`Request`]s, which the app performs on the Slint thread with [`perform`],
//! through the same callbacks as the player controls. The bus name is owned
//! only while the player screen shows an item.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use oxyfin_core::{PlaybackEvent, PlaybackTitle, SPEEDS};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zbus::connection::Builder;
use zbus::names::InterfaceName;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, Value};
use zbus::{Connection, interface};

use crate::{AppWindow, Screen};

/// Well-known name of the service on the session bus.
pub const BUS_NAME: &str = "org.mpris.MediaPlayer2.oxyfin";
/// Object path of the MPRIS interfaces.
pub const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
/// Interface of the player properties.
pub const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
/// Track id meaning that no track is loaded.
pub const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";
/// Prefix of the track ids, followed by the item id.
const TRACK_PREFIX: &str = "/io/github/_0xsharkboy/Oxyfin/item/";

/// A call from the bus, performed on the Slint thread with [`perform`].
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Resume playback.
    Play,
    /// Pause playback.
    Pause,
    /// Toggle pause.
    PlayPause,
    /// Stop playback and leave the player.
    Stop,
    /// Play the next episode.
    Next,
    /// Play the previous episode, or restart the item.
    Previous,
    /// Seek by this offset, backward when negative.
    SeekBy(f64),
    /// Seek to this position, in seconds.
    SeekTo(f64),
    /// Set the playback speed.
    SetRate(f64),
    /// Set the volume, from 0 to 1.
    SetVolume(f64),
}

/// A change of the playback the service shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// The player screen was opened or closed.
    Active(bool),
    /// An event of the playback controller.
    Event(PlaybackEvent),
    /// URL of the artwork of the item being played.
    Artwork(Option<String>),
}

/// The item shown to the desktop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    /// Jellyfin item id.
    pub item: String,
    /// Titles of the item.
    pub title: Option<PlaybackTitle>,
    /// Artwork URL.
    pub art: Option<String>,
}

/// What the service shows, updated from [`Update`]s.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerState {
    /// Whether the player screen is shown.
    pub active: bool,
    /// The item being played.
    pub track: Option<Track>,
    /// Whether the item is loaded and playing or paused.
    pub ready: bool,
    /// Whether playback is paused.
    pub paused: bool,
    /// Playback position.
    pub position: Duration,
    /// Duration of the item, when known.
    pub duration: Option<Duration>,
    /// Whether a previous episode, or a restart, is possible.
    pub has_previous: bool,
    /// Whether a next episode exists.
    pub has_next: bool,
    /// Playback speed.
    pub rate: f64,
    /// Volume, in percent.
    pub volume: f64,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            active: false,
            track: None,
            ready: false,
            paused: false,
            position: Duration::ZERO,
            duration: None,
            has_previous: false,
            has_next: false,
            rate: 1.0,
            volume: 100.0,
        }
    }
}

/// A property of the player interface that signals its changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    /// `PlaybackStatus`.
    PlaybackStatus,
    /// `Metadata`.
    Metadata,
    /// `Rate`.
    Rate,
    /// `Volume`.
    Volume,
    /// `CanGoNext`.
    CanGoNext,
    /// `CanGoPrevious`.
    CanGoPrevious,
    /// `CanPlay`.
    CanPlay,
    /// `CanPause`.
    CanPause,
    /// `CanSeek`.
    CanSeek,
}

impl Property {
    /// Every property, in a stable order.
    pub const ALL: [Property; 9] = [
        Property::PlaybackStatus,
        Property::Metadata,
        Property::Rate,
        Property::Volume,
        Property::CanGoNext,
        Property::CanGoPrevious,
        Property::CanPlay,
        Property::CanPause,
        Property::CanSeek,
    ];

    /// Name of the property on the bus.
    pub fn name(self) -> &'static str {
        match self {
            Property::PlaybackStatus => "PlaybackStatus",
            Property::Metadata => "Metadata",
            Property::Rate => "Rate",
            Property::Volume => "Volume",
            Property::CanGoNext => "CanGoNext",
            Property::CanGoPrevious => "CanGoPrevious",
            Property::CanPlay => "CanPlay",
            Property::CanPause => "CanPause",
            Property::CanSeek => "CanSeek",
        }
    }

    fn value(self, state: &PlayerState) -> Value<'static> {
        match self {
            Property::PlaybackStatus => Value::from(state.status()),
            Property::Metadata => Value::from(state.metadata()),
            Property::Rate => Value::from(state.rate),
            Property::Volume => Value::from(state.volume / 100.0),
            Property::CanGoNext => Value::from(state.can_go_next()),
            Property::CanGoPrevious => Value::from(state.can_go_previous()),
            Property::CanPlay => Value::from(state.can_play()),
            Property::CanPause => Value::from(state.can_play()),
            Property::CanSeek => Value::from(state.can_seek()),
        }
    }
}

impl PlayerState {
    /// Applies `update`; returns whether it was a finished seek, which the
    /// `Seeked` signal reports.
    pub fn apply(&mut self, update: Update) -> bool {
        match update {
            Update::Active(active) => {
                self.active = active;
                if !active {
                    *self = Self {
                        rate: self.rate,
                        volume: self.volume,
                        ..Self::default()
                    };
                }
            }
            Update::Artwork(art) => {
                if let Some(track) = self.track.as_mut() {
                    track.art = art;
                }
            }
            Update::Event(event) => return self.apply_event(event),
        }
        false
    }

    fn apply_event(&mut self, event: PlaybackEvent) -> bool {
        match event {
            PlaybackEvent::Loading { item, title } => {
                self.track = Some(Track {
                    item: item.to_string(),
                    title: Some(title),
                    art: None,
                });
                self.ready = false;
                self.paused = false;
                self.position = Duration::ZERO;
                self.duration = None;
                self.has_previous = false;
                self.has_next = false;
            }
            PlaybackEvent::Ready {
                duration,
                has_previous,
                has_next,
                ..
            } => {
                self.ready = true;
                self.duration = duration;
                self.has_previous = has_previous;
                self.has_next = has_next;
            }
            PlaybackEvent::Position(position) => self.position = position,
            PlaybackEvent::Paused(paused) => self.paused = paused,
            PlaybackEvent::Speed(speed) => self.rate = speed,
            PlaybackEvent::Volume(volume) => self.volume = volume,
            PlaybackEvent::Seeked => return self.ready,
            PlaybackEvent::Stopped | PlaybackEvent::SessionExpired => {
                self.track = None;
                self.ready = false;
            }
            PlaybackEvent::Error(oxyfin_core::PlaybackError::Command { .. }) => {}
            PlaybackEvent::Error(_) => self.ready = false,
            PlaybackEvent::Muted(_)
            | PlaybackEvent::SubtitleDelay(_)
            | PlaybackEvent::AudioDelay(_)
            | PlaybackEvent::Seeking
            | PlaybackEvent::Segment(_)
            | PlaybackEvent::Trickplay(_)
            | PlaybackEvent::Chapters(_)
            | PlaybackEvent::Ended
            | PlaybackEvent::TracksChanged { .. }
            | PlaybackEvent::NextUpCountdown { .. }
            | PlaybackEvent::NextUpReady { .. }
            | PlaybackEvent::SegmentSkipped { .. }
            | PlaybackEvent::NextUpCancelled
            | PlaybackEvent::Stats(_) => {}
        }
        false
    }

    /// Whether the service should own its bus name: the player screen shows an item.
    pub fn present(&self) -> bool {
        self.active && self.track.is_some()
    }

    /// `PlaybackStatus`: `Playing`, `Paused` or `Stopped`.
    pub fn status(&self) -> &'static str {
        match (&self.track, self.paused) {
            (None, _) => "Stopped",
            (Some(_), true) => "Paused",
            (Some(_), false) => "Playing",
        }
    }

    /// `mpris:trackid` of the item, or [`NO_TRACK`].
    pub fn track_id(&self) -> ObjectPath<'static> {
        self.track
            .as_ref()
            .and_then(|track| ObjectPath::try_from(track_path(&track.item)).ok())
            .unwrap_or_else(|| ObjectPath::from_static_str_unchecked(NO_TRACK))
    }

    /// `Metadata`: track id, titles, length and artwork.
    pub fn metadata(&self) -> HashMap<String, Value<'static>> {
        let mut metadata = HashMap::new();
        metadata.insert("mpris:trackid".to_owned(), Value::from(self.track_id()));
        let Some(track) = &self.track else {
            return metadata;
        };
        if let Some(title) = &track.title {
            let name = match &title.episode_label {
                Some(label) if title.series_name.is_some() => {
                    format!("{label} \u{b7} {}", title.title)
                }
                _ => title.title.clone(),
            };
            metadata.insert("xesam:title".to_owned(), Value::from(name));
            if let Some(series) = &title.series_name {
                metadata.insert("xesam:album".to_owned(), Value::from(series.clone()));
                metadata.insert("xesam:artist".to_owned(), Value::from(vec![series.clone()]));
            }
        }
        if let Some(length) = self.duration.and_then(micros) {
            metadata.insert("mpris:length".to_owned(), Value::from(length));
        }
        if let Some(art) = &track.art {
            metadata.insert("mpris:artUrl".to_owned(), Value::from(art.clone()));
        }
        metadata
    }

    /// `Position`, in microseconds.
    pub fn position_micros(&self) -> i64 {
        micros(self.position).unwrap_or(0)
    }

    fn can_play(&self) -> bool {
        self.track.is_some()
    }

    fn can_seek(&self) -> bool {
        self.ready && self.duration.is_some()
    }

    fn can_go_next(&self) -> bool {
        self.ready && self.has_next
    }

    fn can_go_previous(&self) -> bool {
        self.ready && self.has_previous
    }
}

/// The properties whose value differs between `before` and `after`.
pub fn changed(before: &PlayerState, after: &PlayerState) -> Vec<Property> {
    Property::ALL
        .into_iter()
        .filter(|property| property.value(before) != property.value(after))
        .collect()
}

/// Track id of an item: an object path, whose elements only allow `[A-Za-z0-9_]`.
pub fn track_path(item: &str) -> String {
    let element: String = item
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let element = if element.is_empty() {
        "_".to_owned()
    } else {
        element
    };
    format!("{TRACK_PREFIX}{element}")
}

fn micros(duration: Duration) -> Option<i64> {
    i64::try_from(duration.as_micros()).ok()
}

/// Performs a call from the bus with the player controls of `window`; ignored
/// when the player screen is not shown.
pub fn perform(window: &AppWindow, request: Request) {
    if window.get_screen() != Screen::Player {
        return;
    }
    let paused = window.get_player_paused();
    let duration = f64::from(window.get_player_duration());
    let seek = |seconds: f64| {
        if duration > 0.0 {
            window.invoke_player_command_seek(seconds.clamp(0.0, duration) as f32);
        }
    };
    match request {
        Request::Play if paused => window.invoke_player_command_set_paused(false),
        Request::Pause if !paused => window.invoke_player_command_set_paused(true),
        Request::Play | Request::Pause => {}
        Request::PlayPause => window.invoke_player_command_set_paused(!paused),
        Request::Stop => {
            window.invoke_player_command_stop();
            window.invoke_close_player();
        }
        Request::Next if window.get_player_has_next() => window.invoke_player_command_next(),
        Request::Next => {}
        Request::Previous => window.invoke_player_command_previous(),
        Request::SeekBy(offset) => seek(f64::from(window.get_player_position()) + offset),
        Request::SeekTo(position) => seek(position),
        Request::SetRate(rate) => {
            let (min, max) = rate_range();
            window.invoke_player_command_set_speed(rate.clamp(min, max) as f32);
        }
        Request::SetVolume(volume) => {
            window.invoke_player_command_set_volume((volume * 100.0).clamp(0.0, 100.0) as f32);
        }
    }
}

fn rate_range() -> (f64, f64) {
    (SPEEDS[0], SPEEDS[SPEEDS.len() - 1])
}

/// The bus the service connects to.
#[derive(Clone, Debug)]
pub enum Bus {
    /// The user's session bus.
    Session,
    /// The bus at this D-Bus address, for tests.
    Address(String),
}

/// Handle on the MPRIS service running in the background.
pub struct Mpris {
    updates: UnboundedSender<Update>,
}

impl Mpris {
    /// Starts the service on `bus` in the background, on `runtime`. Returns the
    /// handle and the calls from the bus. Failing to reach the bus only logs a
    /// warning: the app runs without the service.
    pub fn start(runtime: &Handle, bus: Bus) -> (Self, UnboundedReceiver<Request>) {
        let (updates, incoming) = unbounded_channel();
        let (requests, received) = unbounded_channel();
        runtime.spawn(async move {
            if let Err(error) = serve(bus, requests, incoming).await {
                log::warn!("media keys and MPRIS are unavailable: {error}");
            }
        });
        (Self { updates }, received)
    }

    /// Shows `update` to the desktop.
    pub fn update(&self, update: Update) {
        let _ = self.updates.send(update);
    }
}

type Shared = Arc<Mutex<PlayerState>>;

fn lock(state: &Shared) -> std::sync::MutexGuard<'_, PlayerState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn serve(
    bus: Bus,
    requests: UnboundedSender<Request>,
    mut updates: UnboundedReceiver<Update>,
) -> zbus::Result<()> {
    let state = Shared::default();
    let builder = match bus {
        Bus::Session => Builder::session()?,
        Bus::Address(address) => Builder::address(address.as_str())?,
    };
    let connection = builder
        .serve_at(OBJECT_PATH, interfaces::Root)?
        .serve_at(
            OBJECT_PATH,
            interfaces::Player {
                state: Arc::clone(&state),
                requests,
            },
        )?
        .build()
        .await?;
    let emitter = SignalEmitter::new(&connection, OBJECT_PATH)?;
    let mut owned = false;
    while let Some(update) = updates.recv().await {
        let (before, after, seeked) = {
            let mut state = lock(&state);
            let before = state.clone();
            let seeked = state.apply(update);
            (before, state.clone(), seeked)
        };
        if after.present() != owned {
            owned = own_name(&connection, after.present()).await;
        }
        if !owned {
            continue;
        }
        let properties = changed(&before, &after);
        if !properties.is_empty() {
            let values: HashMap<&str, Value<'_>> = properties
                .iter()
                .map(|property| (property.name(), property.value(&after)))
                .collect();
            if let Err(error) = zbus::fdo::Properties::properties_changed(
                &emitter,
                InterfaceName::from_static_str_unchecked(PLAYER_INTERFACE),
                values,
                (&[] as &[&str]).into(),
            )
            .await
            {
                log::warn!("cannot signal the MPRIS changes: {error}");
            }
        }
        if seeked
            && let Err(error) = interfaces::Player::seeked(&emitter, after.position_micros()).await
        {
            log::warn!("cannot signal the MPRIS seek: {error}");
        }
    }
    Ok(())
}

/// Requests or releases the bus name; returns whether it is owned.
async fn own_name(connection: &Connection, own: bool) -> bool {
    if own {
        match connection.request_name(BUS_NAME).await {
            Ok(()) => true,
            Err(error) => {
                log::warn!("cannot own {BUS_NAME}: {error}");
                false
            }
        }
    } else {
        if let Err(error) = connection.release_name(BUS_NAME).await {
            log::warn!("cannot release {BUS_NAME}: {error}");
        }
        false
    }
}

mod interfaces {
    //! The D-Bus interfaces, private: their macro adds undocumented public helpers.

    use super::*;

    pub(super) struct Root;

    #[interface(name = "org.mpris.MediaPlayer2")]
    impl Root {
        fn raise(&self) {}

        fn quit(&self) {}

        #[zbus(property)]
        fn can_quit(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn can_raise(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn has_track_list(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn identity(&self) -> &str {
            "Oxyfin"
        }

        #[zbus(property)]
        fn desktop_entry(&self) -> &str {
            crate::APP_ID
        }

        #[zbus(property)]
        fn supported_uri_schemes(&self) -> Vec<String> {
            Vec::new()
        }

        #[zbus(property)]
        fn supported_mime_types(&self) -> Vec<String> {
            Vec::new()
        }
    }

    pub(super) struct Player {
        pub(super) state: Shared,
        pub(super) requests: UnboundedSender<Request>,
    }

    impl Player {
        fn send(&self, request: Request) {
            let _ = self.requests.send(request);
        }

        fn state(&self) -> PlayerState {
            lock(&self.state).clone()
        }
    }

    #[interface(name = "org.mpris.MediaPlayer2.Player")]
    impl Player {
        fn next(&self) {
            self.send(Request::Next);
        }

        fn previous(&self) {
            self.send(Request::Previous);
        }

        fn pause(&self) {
            self.send(Request::Pause);
        }

        fn play_pause(&self) {
            self.send(Request::PlayPause);
        }

        fn stop(&self) {
            self.send(Request::Stop);
        }

        fn play(&self) {
            self.send(Request::Play);
        }

        fn seek(&self, offset: i64) {
            self.send(Request::SeekBy(offset as f64 / 1e6));
        }

        fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
            let state = self.state();
            let within = state
                .duration
                .and_then(micros)
                .is_some_and(|length| (0..=length).contains(&position));
            if within && track_id == state.track_id() {
                self.send(Request::SeekTo(position as f64 / 1e6));
            }
        }

        fn open_uri(&self, _uri: &str) -> zbus::fdo::Result<()> {
            Err(zbus::fdo::Error::NotSupported(
                "opening URIs is not supported".to_owned(),
            ))
        }

        /// Tells the desktop that playback jumped to `position`, in microseconds.
        #[zbus(signal)]
        pub(super) async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

        #[zbus(property)]
        fn playback_status(&self) -> &'static str {
            self.state().status()
        }

        #[zbus(property)]
        fn rate(&self) -> f64 {
            self.state().rate
        }

        #[zbus(property)]
        fn set_rate(&self, rate: f64) {
            if rate > 0.0 {
                self.send(Request::SetRate(rate));
            }
        }

        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, Value<'static>> {
            self.state().metadata()
        }

        #[zbus(property)]
        fn volume(&self) -> f64 {
            self.state().volume / 100.0
        }

        #[zbus(property)]
        fn set_volume(&self, volume: f64) {
            self.send(Request::SetVolume(volume));
        }

        #[zbus(property(emits_changed_signal = "false"))]
        fn position(&self) -> i64 {
            self.state().position_micros()
        }

        #[zbus(property)]
        fn minimum_rate(&self) -> f64 {
            rate_range().0
        }

        #[zbus(property)]
        fn maximum_rate(&self) -> f64 {
            rate_range().1
        }

        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            self.state().can_go_next()
        }

        #[zbus(property)]
        fn can_go_previous(&self) -> bool {
            self.state().can_go_previous()
        }

        #[zbus(property)]
        fn can_play(&self) -> bool {
            self.state().can_play()
        }

        #[zbus(property)]
        fn can_pause(&self) -> bool {
            self.state().can_play()
        }

        #[zbus(property)]
        fn can_seek(&self) -> bool {
            self.state().can_seek()
        }

        #[zbus(property(emits_changed_signal = "const"))]
        fn can_control(&self) -> bool {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyfin_api::ItemId;

    fn episode() -> PlaybackEvent {
        PlaybackEvent::Loading {
            item: ItemId::from("6c1b-2f"),
            title: PlaybackTitle {
                title: "Pilot".to_owned(),
                series_name: Some("The Show".to_owned()),
                episode_label: Some("S01E01".to_owned()),
                year: None,
                artwork: None,
            },
        }
    }

    fn ready(has_next: bool) -> PlaybackEvent {
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(90)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: true,
            has_next,
        }
    }

    fn playing() -> PlayerState {
        let mut state = PlayerState::default();
        state.apply(Update::Active(true));
        state.apply(Update::Event(episode()));
        state.apply(Update::Artwork(Some("http://localhost/art".to_owned())));
        state.apply(Update::Event(ready(true)));
        state
    }

    #[test]
    fn track_ids_are_valid_object_paths() {
        assert_eq!(
            track_path("6c1b-2f.x"),
            "/io/github/_0xsharkboy/Oxyfin/item/6c1b_2f_x"
        );
        assert!(ObjectPath::try_from(track_path("a/b c")).is_ok());
        assert!(ObjectPath::try_from(track_path("")).is_ok());
    }

    #[test]
    fn an_episode_shows_its_series_label_length_and_artwork() {
        let metadata = playing().metadata();
        assert_eq!(
            metadata["mpris:trackid"],
            Value::from(ObjectPath::try_from(track_path("6c1b-2f")).unwrap())
        );
        assert_eq!(
            metadata["xesam:title"],
            Value::from("S01E01 \u{b7} Pilot".to_owned())
        );
        assert_eq!(metadata["xesam:album"], Value::from("The Show".to_owned()));
        assert_eq!(
            metadata["xesam:artist"],
            Value::from(vec!["The Show".to_owned()])
        );
        assert_eq!(metadata["mpris:length"], Value::from(90_000_000i64));
        assert_eq!(
            metadata["mpris:artUrl"],
            Value::from("http://localhost/art".to_owned())
        );
    }

    #[test]
    fn a_movie_shows_its_title_alone() {
        let mut state = PlayerState::default();
        state.apply(Update::Event(PlaybackEvent::Loading {
            item: ItemId::from("m1"),
            title: PlaybackTitle {
                title: "Film".to_owned(),
                series_name: None,
                episode_label: None,
                year: None,
                artwork: None,
            },
        }));
        let metadata = state.metadata();
        assert_eq!(metadata["xesam:title"], Value::from("Film".to_owned()));
        assert!(!metadata.contains_key("xesam:album"));
        assert!(!metadata.contains_key("mpris:length"));
        assert!(!metadata.contains_key("mpris:artUrl"));
    }

    #[test]
    fn nothing_loaded_is_stopped_with_no_track() {
        let state = PlayerState::default();
        assert_eq!(state.status(), "Stopped");
        assert_eq!(state.track_id().as_str(), NO_TRACK);
        assert_eq!(state.metadata().len(), 1);
        assert!(!state.can_play() && !state.can_seek() && !state.can_go_next());
    }

    #[test]
    fn the_status_follows_pause_and_stop() {
        let mut state = playing();
        assert_eq!(state.status(), "Playing");
        state.apply(Update::Event(PlaybackEvent::Paused(true)));
        assert_eq!(state.status(), "Paused");
        state.apply(Update::Event(PlaybackEvent::Stopped));
        assert_eq!(state.status(), "Stopped");
        assert!(!state.present());
    }

    #[test]
    fn the_name_is_owned_only_while_the_player_shows_an_item() {
        let mut state = PlayerState::default();
        state.apply(Update::Active(true));
        assert!(!state.present());
        state.apply(Update::Event(episode()));
        assert!(state.present());
        state.apply(Update::Active(false));
        assert!(!state.present());
        assert_eq!(state.track, None);
    }

    #[test]
    fn navigation_and_seeking_wait_for_the_item_to_be_ready() {
        let mut state = PlayerState::default();
        state.apply(Update::Event(episode()));
        assert!(state.can_play());
        assert!(!state.can_seek() && !state.can_go_next() && !state.can_go_previous());
        state.apply(Update::Event(ready(false)));
        assert!(state.can_seek() && state.can_go_previous());
        assert!(!state.can_go_next());
    }

    #[test]
    fn the_next_episode_stays_reachable_during_the_countdown() {
        let mut state = playing();
        state.apply(Update::Event(PlaybackEvent::Ended));
        assert!(state.can_go_next() && state.can_go_previous() && state.can_seek());
    }

    #[test]
    fn changes_list_only_the_properties_that_differ() {
        let before = playing();
        let mut after = before.clone();
        after.apply(Update::Event(PlaybackEvent::Position(Duration::from_secs(
            5,
        ))));
        assert_eq!(changed(&before, &after), Vec::new());
        after.apply(Update::Event(PlaybackEvent::Paused(true)));
        assert_eq!(changed(&before, &after), vec![Property::PlaybackStatus]);
        let mut next = before.clone();
        next.apply(Update::Event(PlaybackEvent::Loading {
            item: ItemId::from("other"),
            title: PlaybackTitle {
                title: "Second".to_owned(),
                series_name: Some("The Show".to_owned()),
                episode_label: Some("S01E02".to_owned()),
                year: None,
                artwork: None,
            },
        }));
        assert_eq!(
            changed(&before, &next),
            vec![
                Property::Metadata,
                Property::CanGoNext,
                Property::CanGoPrevious,
                Property::CanSeek
            ]
        );
    }

    #[test]
    fn only_a_finished_seek_of_a_ready_item_is_signalled() {
        let mut state = playing();
        assert!(!state.apply(Update::Event(PlaybackEvent::Seeking)));
        assert!(state.apply(Update::Event(PlaybackEvent::Seeked)));
        state.apply(Update::Event(episode()));
        assert!(!state.apply(Update::Event(PlaybackEvent::Seeked)));
    }

    #[test]
    fn position_and_rate_are_reported_in_mpris_units() {
        let mut state = playing();
        state.apply(Update::Event(PlaybackEvent::Position(
            Duration::from_millis(1500),
        )));
        state.apply(Update::Event(PlaybackEvent::Speed(1.5)));
        state.apply(Update::Event(PlaybackEvent::Volume(40.0)));
        assert_eq!(state.position_micros(), 1_500_000);
        assert_eq!(Property::Rate.value(&state), Value::from(1.5));
        assert_eq!(Property::Volume.value(&state), Value::from(0.4));
        assert_eq!(rate_range(), (0.5, 2.0));
    }
}
