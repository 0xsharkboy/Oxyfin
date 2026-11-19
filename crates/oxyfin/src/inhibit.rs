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
//! while an item plays, unpaused, on the player screen, and releases it
//! otherwise.

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

/// Whether `event` can change the inhibition, so the app forwards only those,
/// and not the position reported every second.
pub fn concerns(event: &PlaybackEvent) -> bool {
    matches!(
        event,
        PlaybackEvent::Loading { .. }
            | PlaybackEvent::Ready { .. }
            | PlaybackEvent::Paused(_)
            | PlaybackEvent::Ended
            | PlaybackEvent::Seeked
            | PlaybackEvent::Stopped
            | PlaybackEvent::SessionExpired
            | PlaybackEvent::Error(_)
    )
}

/// What the inhibition depends on, updated from [`Update`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Playing {
    active: bool,
    ready: bool,
    paused: bool,
    ended: bool,
}

impl Playing {
    /// Takes `update` into account.
    pub fn apply(&mut self, update: &Update) {
        match update {
            Update::Active(active) => {
                self.active = *active;
                if !active {
                    *self = Self::default();
                }
            }
            Update::Event(event) => match event {
                PlaybackEvent::Loading { .. } => {
                    self.ready = false;
                    self.paused = false;
                    self.ended = false;
                }
                PlaybackEvent::Ready { .. } => self.ready = true,
                PlaybackEvent::Paused(paused) => self.paused = *paused,
                PlaybackEvent::Ended => self.ended = true,
                // A seek after the end plays again; a seek to the end ends anew.
                PlaybackEvent::Seeked => self.ended = false,
                PlaybackEvent::Stopped | PlaybackEvent::SessionExpired => self.ready = false,
                PlaybackEvent::Error(PlaybackError::Command { .. }) => {}
                PlaybackEvent::Error(_) => self.ready = false,
                _ => {}
            },
        }
    }

    /// Whether the screen should stay awake: an item plays, unpaused, on the player screen.
    pub fn wanted(&self) -> bool {
        self.active && self.ready && !self.paused && !self.ended
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
    /// screen must stay awake. Failing to inhibit only logs a warning.
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
    while let Some(message) = messages.recv().await {
        let mut release = None;
        match message {
            Message::Update(update) => playing.apply(&update),
            Message::Release(done) => release = Some(done),
        }
        // Only the latest state matters: quick play and pause changes never queue up calls.
        while release.is_none()
            && let Ok(message) = messages.try_recv()
        {
            match message {
                Message::Update(update) => playing.apply(&update),
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
            (true, false) => {
                if connection.is_none() {
                    connection = connect(&bus).await;
                }
                if let Some(connection) = &connection {
                    held = inhibit(connection).await;
                }
            }
            (false, true) => {
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

    fn ready() -> Update {
        event(PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(60)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: true,
            has_next: false,
        })
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

    fn after(updates: &[Update]) -> bool {
        let mut playing = Playing::default();
        for update in updates {
            playing.apply(update);
        }
        playing.wanted()
    }

    #[test]
    fn only_an_unpaused_item_on_the_player_keeps_the_screen_awake() {
        let active = Update::Active(true);
        assert!(!after(&[active.clone(), loading()]));
        assert!(after(&[active.clone(), loading(), ready()]));
        assert!(!after(&[loading(), ready()]));
        assert!(!after(&[
            active.clone(),
            ready(),
            event(PlaybackEvent::Paused(true))
        ]));
        assert!(after(&[
            active.clone(),
            ready(),
            event(PlaybackEvent::Paused(true)),
            event(PlaybackEvent::Paused(false)),
        ]));
        assert!(!after(&[active.clone(), ready(), Update::Active(false)]));
    }

    #[test]
    fn only_the_events_that_change_the_inhibition_are_forwarded() {
        for update in [ready(), loading()] {
            let Update::Event(event) = update else {
                unreachable!()
            };
            assert!(concerns(&event), "{event:?}");
        }
        for event in [
            PlaybackEvent::Paused(true),
            PlaybackEvent::Ended,
            PlaybackEvent::Seeked,
            PlaybackEvent::Stopped,
            PlaybackEvent::SessionExpired,
            PlaybackEvent::Error(PlaybackError::Network),
        ] {
            assert!(concerns(&event), "{event:?}");
        }
        for event in [
            PlaybackEvent::Position(Duration::from_secs(1)),
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
                !after(&[active.clone(), ready(), event(end.clone())]),
                "{end:?}"
            );
        }
        assert!(after(&[
            active.clone(),
            ready(),
            event(PlaybackEvent::Error(PlaybackError::Command {
                detail: "volume".into()
            })),
        ]));
        assert!(after(&[
            active.clone(),
            ready(),
            event(PlaybackEvent::Ended),
            event(PlaybackEvent::Seeked),
        ]));
        assert!(!after(&[
            active,
            ready(),
            event(PlaybackEvent::Ended),
            loading()
        ]));
    }
}
