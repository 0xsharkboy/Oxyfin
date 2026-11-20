//! Keeps the screen awake while a video plays.
//!
//! The screen saver and the idle actions of the desktop are inhibited through
//! the XDG desktop portal, which also works inside Flatpak, or through the
//! `org.freedesktop.ScreenSaver` service of the session when no portal
//! answers. libmpv cannot do it: its screen saver option needs a window of its
//! own, which the render API does not give it.
//!
//! The app forwards the player screen state and the playback events with
//! [`Inhibitor::update`]; a task on the Tokio runtime holds the inhibition
//! while an item plays, unpaused, on the player screen, keeps it through a
//! load and the next episode countdown, and releases it otherwise.

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use oxyfin_core::{PlaybackError, PlaybackEvent};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use zbus::Connection;
use zbus::connection::Builder;
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::mpris::Bus;

/// Well-known name of the XDG desktop portal.
pub const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
/// Object path of the XDG desktop portal.
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
/// Interface of the portal that inhibits session actions.
pub const PORTAL_INHIBIT: &str = "org.freedesktop.portal.Inhibit";
/// Interface of the request a portal call returns; closing it ends the inhibition.
pub const PORTAL_REQUEST: &str = "org.freedesktop.portal.Request";
/// Portal flag that inhibits the idle actions, such as the screen saver and blanking.
pub const IDLE: u32 = 8;
/// Well-known name, object path and interface of the screen saver service.
pub const SCREEN_SAVER: &str = "org.freedesktop.ScreenSaver";
/// Object path of the screen saver service.
pub const SCREEN_SAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
/// Why the screen is kept awake, as the desktop may show it.
pub const REASON: &str = "Playing a video";

// The bus may start a portal on demand; one that never answers must not hold back the fallback.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// A change the inhibition follows.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// The player screen was opened or closed.
    Active(bool),
    /// An event of the playback controller.
    Event(PlaybackEvent),
}

/// Whether `event` can change the inhibition, so the app forwards only those.
pub fn concerns(event: &PlaybackEvent) -> bool {
    matches!(
        event,
        PlaybackEvent::Loading { .. }
            | PlaybackEvent::Ready { .. }
            | PlaybackEvent::Position(_)
            | PlaybackEvent::Paused(_)
            | PlaybackEvent::Ended
            | PlaybackEvent::NextUpReady { .. }
            | PlaybackEvent::NextUpCancelled
            | PlaybackEvent::Stopped
            | PlaybackEvent::SessionExpired
            | PlaybackEvent::Error(_)
    )
}

/// What the inhibition should do, from [`Playing::wanted`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// Keep the screen awake.
    Hold,
    /// Let the screen sleep.
    Release,
    /// Leave the inhibition as it is, held or not: an item loads, or the next
    /// episode is about to.
    Keep,
}

/// What the inhibition depends on, updated from [`Update`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Playing {
    active: bool,
    loading: bool,
    ready: bool,
    paused: bool,
    has_next: bool,
    ended: bool,
    countdown: bool,
    /// The last position reported, or after the end the furthest one: only a
    /// position before it plays again, as a seek to the end may end before
    /// its position is reported.
    position: Option<Duration>,
}

impl Playing {
    /// Takes `update` into account.
    pub fn apply(&mut self, update: &Update) {
        match update {
            Update::Active(active) => {
                *self = Self {
                    active: *active,
                    ..if *active { *self } else { Self::default() }
                };
            }
            Update::Event(event) => match event {
                PlaybackEvent::Loading { .. } => {
                    *self = Self {
                        active: self.active,
                        loading: true,
                        ..Self::default()
                    };
                }
                PlaybackEvent::Ready { has_next, .. } => {
                    self.loading = false;
                    self.ready = true;
                    self.has_next = *has_next;
                }
                PlaybackEvent::Position(position) => {
                    if self.ended && self.position.is_some_and(|last| *position < last) {
                        self.ended = false;
                        self.countdown = false;
                    }
                    self.position = Some(*position);
                }
                PlaybackEvent::Paused(paused) => self.paused = *paused,
                PlaybackEvent::Ended => {
                    self.ended = true;
                    self.countdown = self.has_next;
                }
                // Waiting for the user, maybe for long: the screen may sleep.
                PlaybackEvent::NextUpReady { .. } | PlaybackEvent::NextUpCancelled => {
                    self.countdown = false;
                }
                PlaybackEvent::Error(PlaybackError::Command { .. }) => {}
                PlaybackEvent::Stopped
                | PlaybackEvent::SessionExpired
                | PlaybackEvent::Error(_) => {
                    *self = Self {
                        active: self.active,
                        ..Self::default()
                    };
                }
                _ => {}
            },
        }
    }

    /// What the inhibition should do: hold it while an item plays, unpaused,
    /// on the player screen; keep it as it is while an item loads or the next
    /// episode counts down; release it otherwise.
    pub fn wanted(&self) -> Want {
        if !self.active || self.paused {
            Want::Release
        } else if self.loading || (self.ended && self.countdown) {
            Want::Keep
        } else if self.ready && !self.ended {
            Want::Hold
        } else {
            Want::Release
        }
    }
}

enum Message {
    Update(Update),
    Release(oneshot::Sender<()>),
}

/// Handle on the task that keeps the screen awake.
pub struct Inhibitor {
    messages: UnboundedSender<Message>,
}

impl Inhibitor {
    /// Starts the task on `runtime`; it connects to `bus` the first time the
    /// screen must stay awake. Failing to inhibit only logs a warning, and is
    /// not tried again until the player screen opens anew.
    pub fn start(runtime: &Handle, bus: Bus) -> Self {
        let (messages, received) = unbounded_channel();
        runtime.spawn(run(bus, received));
        Self { messages }
    }

    /// Takes `update` into account.
    pub fn update(&self, update: Update) {
        let _ = self.messages.send(Message::Update(update));
    }

    /// Ends the inhibition, if held, and stops the task. The future resolves
    /// once the desktop was told, or right away when the task is gone.
    pub fn release(self) -> impl Future<Output = ()> {
        let (done, released) = oneshot::channel();
        let sent = self.messages.send(Message::Release(done)).is_ok();
        async move {
            if sent {
                let _ = released.await;
            }
        }
    }
}

/// An inhibition the desktop granted.
enum Held {
    /// The request handle the portal returned.
    Portal(OwnedObjectPath),
    /// The cookie the screen saver service returned.
    ScreenSaver(u32),
}

async fn run(bus: Bus, mut messages: UnboundedReceiver<Message>) {
    let mut playing = Playing::default();
    let mut connection = None;
    let mut held = None;
    // After a failure, asked again only once the player opens anew: every event would retry.
    let mut gave_up = false;
    while let Some(message) = messages.recv().await {
        let mut release = None;
        match message {
            Message::Update(update) => follow(&mut playing, &mut gave_up, &update),
            Message::Release(done) => release = Some(done),
        }
        // Only the latest state matters: quick play and pause changes never queue up calls.
        while release.is_none()
            && let Ok(message) = messages.try_recv()
        {
            match message {
                Message::Update(update) => follow(&mut playing, &mut gave_up, &update),
                Message::Release(done) => release = Some(done),
            }
        }
        if let Some(done) = release {
            if let (Some(connection), Some(held)) = (&connection, held.take()) {
                uninhibit(connection, held).await;
            }
            let _ = done.send(());
            return;
        }
        match (playing.wanted(), held.is_some()) {
            (Want::Hold, false) if !gave_up => {
                if connection.is_none() {
                    connection = connect(&bus).await;
                }
                if let Some(connection) = &connection {
                    held = inhibit(connection).await;
                }
                gave_up = held.is_none();
            }
            (Want::Release, true) => {
                if let (Some(connection), Some(held)) = (&connection, held.take()) {
                    uninhibit(connection, held).await;
                }
            }
            _ => {}
        }
    }
    if let (Some(connection), Some(held)) = (&connection, held) {
        uninhibit(connection, held).await;
    }
}

/// Applies `update` to `playing`; opening the player clears `gave_up`.
fn follow(playing: &mut Playing, gave_up: &mut bool, update: &Update) {
    if *update == Update::Active(true) && !playing.active {
        *gave_up = false;
    }
    playing.apply(update);
}

async fn connect(bus: &Bus) -> Option<Connection> {
    let connection = match bus {
        Bus::Session => Builder::session(),
        Bus::Address(address) => Builder::address(address.as_str()),
    };
    match connection {
        Ok(builder) => match builder.build().await {
            Ok(connection) => Some(connection),
            Err(error) => {
                log::warn!("cannot keep the screen awake: {error}");
                None
            }
        },
        Err(error) => {
            log::warn!("cannot keep the screen awake: {error}");
            None
        }
    }
}

async fn inhibit(connection: &Connection) -> Option<Held> {
    let portal = match ask(connection, portal_inhibit(connection.clone()), Held::Portal).await {
        Ok(handle) => {
            log::debug!("screen kept awake through the portal ({handle})");
            return Some(Held::Portal(handle));
        }
        Err(error) => error,
    };
    let screen_saver = screen_saver_inhibit(connection.clone());
    match ask(connection, screen_saver, Held::ScreenSaver).await {
        Ok(cookie) => {
            log::debug!("screen kept awake through the screen saver ({cookie})");
            Some(Held::ScreenSaver(cookie))
        }
        Err(error) => {
            log::warn!("cannot keep the screen awake: portal: {portal}; screen saver: {error}");
            None
        }
    }
}

/// Waits at most [`CALL_TIMEOUT`] for an inhibition; one granted later is
/// released as soon as it comes, so it never keeps the screen awake.
async fn ask<T: Send + 'static>(
    connection: &Connection,
    call: impl Future<Output = zbus::Result<T>> + Send + 'static,
    held: fn(T) -> Held,
) -> zbus::Result<T> {
    let mut call = tokio::spawn(call);
    match tokio::time::timeout(CALL_TIMEOUT, &mut call).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(zbus::Error::Failure(error.to_string())),
        Err(_) => {
            let connection = connection.clone();
            tokio::spawn(async move {
                if let Ok(Ok(granted)) = call.await {
                    log::warn!("an inhibition came after its timeout; releasing it");
                    uninhibit(&connection, held(granted)).await;
                }
            });
            Err(zbus::Error::Failure("no answer in time".to_owned()))
        }
    }
}

async fn timed<T>(call: impl Future<Output = zbus::Result<T>>) -> zbus::Result<T> {
    tokio::time::timeout(CALL_TIMEOUT, call)
        .await
        .unwrap_or_else(|_| Err(zbus::Error::Failure("no answer in time".to_owned())))
}

async fn portal_inhibit(connection: Connection) -> zbus::Result<OwnedObjectPath> {
    let options = HashMap::from([("reason", Value::from(REASON))]);
    let reply = connection
        .call_method(
            Some(PORTAL_NAME),
            PORTAL_PATH,
            Some(PORTAL_INHIBIT),
            "Inhibit",
            &("", IDLE, options),
        )
        .await?;
    reply.body().deserialize()
}

async fn screen_saver_inhibit(connection: Connection) -> zbus::Result<u32> {
    let reply = connection
        .call_method(
            Some(SCREEN_SAVER),
            SCREEN_SAVER_PATH,
            Some(SCREEN_SAVER),
            "Inhibit",
            &("Oxyfin", REASON),
        )
        .await?;
    reply.body().deserialize()
}

async fn uninhibit(connection: &Connection, held: Held) {
    let result = match &held {
        Held::Portal(handle) => timed(connection.call_method(
            Some(PORTAL_NAME),
            handle,
            Some(PORTAL_REQUEST),
            "Close",
            &(),
        ))
        .await
        .map(drop),
        Held::ScreenSaver(cookie) => timed(connection.call_method(
            Some(SCREEN_SAVER),
            SCREEN_SAVER_PATH,
            Some(SCREEN_SAVER),
            "UnInhibit",
            &(*cookie,),
        ))
        .await
        .map(drop),
    };
    match result {
        Ok(()) => log::debug!("screen no longer kept awake"),
        Err(error) => log::warn!("cannot let the screen sleep again: {error}"),
    }
}

#[cfg(test)]
mod tests {

    use oxyfin_api::ItemId;
    use oxyfin_core::PlaybackTitle;

    use super::*;

    fn event(event: PlaybackEvent) -> Update {
        Update::Event(event)
    }

    fn ready_with(has_next: bool) -> Update {
        event(PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(60)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: true,
            has_next,
        })
    }

    fn ready() -> Update {
        ready_with(false)
    }

    fn loading() -> Update {
        event(PlaybackEvent::Loading {
            item: ItemId::from("a"),
            title: PlaybackTitle {
                title: "A".into(),
                series_name: None,
                episode_label: None,
                artwork: None,
            },
        })
    }

    fn at(seconds: u64) -> Update {
        event(PlaybackEvent::Position(Duration::from_secs(seconds)))
    }

    fn paused(paused: bool) -> Update {
        event(PlaybackEvent::Paused(paused))
    }

    /// Whether the inhibition is held after `updates`, as the task decides it.
    fn held_after(updates: &[Update]) -> bool {
        let mut playing = Playing::default();
        let mut held = false;
        for update in updates {
            playing.apply(update);
            held = match playing.wanted() {
                Want::Hold => true,
                Want::Release => false,
                Want::Keep => held,
            };
        }
        held
    }

    #[test]
    fn only_an_unpaused_item_on_the_player_keeps_the_screen_awake() {
        let active = Update::Active(true);
        assert!(!held_after(&[active.clone(), loading()]));
        assert!(held_after(&[active.clone(), loading(), ready()]));
        assert!(!held_after(&[loading(), ready()]));
        assert!(!held_after(&[active.clone(), ready(), paused(true)]));
        assert!(held_after(&[
            active.clone(),
            ready(),
            paused(true),
            paused(false),
        ]));
        assert!(!held_after(&[
            active.clone(),
            ready(),
            Update::Active(false)
        ]));
    }

    #[test]
    fn only_the_events_that_change_the_inhibition_are_forwarded() {
        for update in [ready(), loading(), at(1)] {
            let Update::Event(event) = update else {
                unreachable!()
            };
            assert!(concerns(&event), "{event:?}");
        }
        for event in [
            PlaybackEvent::Paused(true),
            PlaybackEvent::Ended,
            PlaybackEvent::NextUpCancelled,
            PlaybackEvent::Stopped,
            PlaybackEvent::SessionExpired,
            PlaybackEvent::Error(PlaybackError::Network),
        ] {
            assert!(concerns(&event), "{event:?}");
        }
        for event in [
            PlaybackEvent::Seeked,
            PlaybackEvent::Volume(50.0),
            PlaybackEvent::SubtitleDelay(100),
        ] {
            assert!(!concerns(&event), "{event:?}");
        }
    }

    #[test]
    fn the_end_of_playback_lets_the_screen_sleep() {
        let active = Update::Active(true);
        for end in [
            PlaybackEvent::Ended,
            PlaybackEvent::Stopped,
            PlaybackEvent::SessionExpired,
            PlaybackEvent::Error(PlaybackError::Network),
        ] {
            assert!(
                !held_after(&[active.clone(), ready(), event(end.clone())]),
                "{end:?}"
            );
        }
        assert!(held_after(&[
            active.clone(),
            ready(),
            event(PlaybackEvent::Error(PlaybackError::Command {
                detail: "volume".into()
            })),
        ]));
        assert!(!held_after(&[
            active,
            ready(),
            event(PlaybackEvent::Ended),
            loading()
        ]));
    }

    #[test]
    fn loading_keeps_the_screen_awake_only_when_it_already_was() {
        let active = Update::Active(true);
        assert!(held_after(&[active.clone(), ready(), loading()]));
        assert!(held_after(&[active.clone(), ready(), loading(), ready()]));
        assert!(!held_after(&[
            active.clone(),
            ready(),
            paused(true),
            loading()
        ]));
        assert!(!held_after(&[active.clone(), loading()]));
        assert!(!held_after(&[
            active.clone(),
            ready(),
            loading(),
            event(PlaybackEvent::Error(PlaybackError::Network)),
        ]));
        assert!(!held_after(&[
            active,
            ready(),
            loading(),
            Update::Active(false)
        ]));
    }

    #[test]
    fn the_next_episode_countdown_keeps_the_screen_awake() {
        let active = Update::Active(true);
        let ended = event(PlaybackEvent::Ended);
        assert!(held_after(&[
            active.clone(),
            ready_with(true),
            ended.clone()
        ]));
        assert!(held_after(&[
            active.clone(),
            ready_with(true),
            ended.clone(),
            loading(),
            ready(),
        ]));
        assert!(!held_after(&[
            active.clone(),
            ready_with(true),
            ended.clone(),
            event(PlaybackEvent::NextUpCancelled),
        ]));
        assert!(!held_after(&[
            active.clone(),
            ready_with(true),
            ended.clone(),
            paused(true),
        ]));
        assert!(!held_after(&[active, ready_with(false), ended]));
    }

    #[test]
    fn waiting_for_the_user_to_start_the_next_episode_lets_the_screen_sleep() {
        let offer = PlaybackEvent::NextUpReady {
            episode: Box::new(oxyfin_core::Episode {
                id: ItemId::from("b"),
                series_id: ItemId::from("s"),
                series_name: None,
                season_id: None,
                title: "B".into(),
                season_number: None,
                number: None,
                label: None,
                overview: None,
                thumb: None,
                duration: None,
                user: oxyfin_core::UserState::default(),
            }),
        };
        assert!(concerns(&offer));
        assert!(!held_after(&[
            Update::Active(true),
            ready_with(true),
            event(PlaybackEvent::Ended),
            event(offer),
        ]));
    }

    #[test]
    fn the_end_lasts_until_the_position_goes_back() {
        let active = Update::Active(true);
        let ended = event(PlaybackEvent::Ended);
        let seeked = event(PlaybackEvent::Seeked);
        // A seek to the end may end before the seek completes.
        assert!(!held_after(&[
            active.clone(),
            ready(),
            at(100),
            ended.clone(),
            seeked.clone(),
            at(3600),
        ]));
        assert!(!held_after(&[
            active.clone(),
            ready(),
            at(100),
            ended.clone(),
            at(3600),
            seeked.clone(),
        ]));
        assert!(!held_after(&[
            active.clone(),
            ready(),
            ended.clone(),
            at(3600)
        ]));
        assert!(held_after(&[
            active.clone(),
            ready(),
            at(3600),
            ended.clone(),
            seeked,
            at(1200),
        ]));
        assert!(held_after(&[active, ready(), ended, at(3600), at(1200)]));
    }
}
