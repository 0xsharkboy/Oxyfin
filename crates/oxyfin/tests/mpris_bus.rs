//! The MPRIS service on a private D-Bus bus, called by a client as a desktop would.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use oxyfin::mpris::{BUS_NAME, Bus, Mpris, OBJECT_PATH, PLAYER_INTERFACE, Request, Update};
use oxyfin_api::ItemId;
use oxyfin_core::{PlaybackEvent, PlaybackTitle};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::timeout;
use zbus::Connection;
use zbus::export::futures_core::Stream;
use zbus::fdo::DBusProxy;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedValue};

const DEADLINE: Duration = Duration::from_secs(5);

/// A `dbus-daemon` of its own, so the test never touches the user's session bus.
struct PrivateBus(Child);

impl PrivateBus {
    fn start() -> (Self, String) {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon must be on the PATH");
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        (Self(child), address.trim().to_owned())
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn next<S: Stream + Unpin>(stream: &mut S) -> S::Item {
    timeout(
        DEADLINE,
        std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)),
    )
    .await
    .expect("no item before the deadline")
    .expect("the stream ended")
}

async fn request(requests: &mut UnboundedReceiver<Request>) -> Request {
    timeout(DEADLINE, requests.recv()).await.unwrap().unwrap()
}

async fn owned(dbus: &DBusProxy<'_>, expected: bool) {
    let name = zbus::names::BusName::try_from(BUS_NAME).unwrap();
    timeout(DEADLINE, async {
        while dbus.name_has_owner(name.clone()).await.unwrap() != expected {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{BUS_NAME} owned should be {expected}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "starts a private dbus-daemon: cargo test -p oxyfin --test mpris_bus -- --ignored"]
async fn the_desktop_sees_and_controls_the_playback() {
    let (_bus, address) = PrivateBus::start();
    let (mpris, mut requests) = Mpris::start(
        &tokio::runtime::Handle::current(),
        Bus::Address(address.clone()),
    );
    let client = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let dbus = DBusProxy::new(&client).await.unwrap();

    mpris.update(Update::Active(true));
    mpris.update(Update::Event(PlaybackEvent::Loading {
        item: ItemId::from("e1"),
        title: PlaybackTitle {
            title: "Pilot".to_owned(),
            series_name: Some("The Show".to_owned()),
            episode_label: Some("S01E01".to_owned()),
            year: None,
            artwork: None,
        },
    }));
    mpris.update(Update::Artwork(Some("http://localhost/art".to_owned())));
    mpris.update(Update::Event(PlaybackEvent::Ready {
        duration: Some(Duration::from_secs(90)),
        audio: Vec::new(),
        subtitles: Vec::new(),
        has_previous: true,
        has_next: false,
    }));
    owned(&dbus, true).await;

    let player = player(&client).await;
    let changes = zbus::Proxy::new(
        &client,
        BUS_NAME,
        OBJECT_PATH,
        "org.freedesktop.DBus.Properties",
    )
    .await
    .unwrap();
    let mut changes = changes.receive_signal("PropertiesChanged").await.unwrap();
    let mut seeks = player.receive_signal("Seeked").await.unwrap();

    let metadata: HashMap<String, OwnedValue> = player.get_property("Metadata").await.unwrap();
    assert_eq!(
        String::try_from(metadata["xesam:title"].clone()).unwrap(),
        "S01E01 \u{b7} Pilot"
    );
    assert_eq!(
        i64::try_from(metadata["mpris:length"].clone()).unwrap(),
        90_000_000
    );
    let track = ObjectPath::try_from(metadata["mpris:trackid"].clone()).unwrap();
    let status: String = player.get_property("PlaybackStatus").await.unwrap();
    assert_eq!(status, "Playing");
    assert!(player.get_property::<bool>("CanSeek").await.unwrap());
    assert!(!player.get_property::<bool>("CanGoNext").await.unwrap());

    player.call_method("PlayPause", &()).await.unwrap();
    assert_eq!(request(&mut requests).await, Request::PlayPause);
    player
        .call_method(
            "SetPosition",
            &(ObjectPath::try_from("/other").unwrap(), 1i64),
        )
        .await
        .unwrap();
    player
        .call_method("SetPosition", &(track, 30_000_000i64))
        .await
        .unwrap();
    assert_eq!(request(&mut requests).await, Request::SeekTo(30.0));
    player.call_method("Seek", &(-5_000_000i64)).await.unwrap();
    assert_eq!(request(&mut requests).await, Request::SeekBy(-5.0));

    mpris.update(Update::Event(PlaybackEvent::Paused(true)));
    let changed = next(&mut changes).await;
    let (interface, values, _): (String, HashMap<String, OwnedValue>, Vec<String>) =
        changed.body().deserialize().unwrap();
    assert_eq!(interface, PLAYER_INTERFACE);
    assert_eq!(
        String::try_from(values["PlaybackStatus"].clone()).unwrap(),
        "Paused"
    );

    mpris.update(Update::Event(PlaybackEvent::Position(Duration::from_secs(
        12,
    ))));
    mpris.update(Update::Event(PlaybackEvent::Seeked));
    let seeked: i64 = next(&mut seeks).await.body().deserialize().unwrap();
    assert_eq!(seeked, 12_000_000);
    let position: i64 = player.get_property("Position").await.unwrap();
    assert_eq!(position, 12_000_000);

    mpris.update(Update::Active(false));
    owned(&dbus, false).await;
}

async fn player(client: &Connection) -> zbus::Proxy<'static> {
    zbus::proxy::Builder::new(client)
        .destination(BUS_NAME)
        .unwrap()
        .path(OBJECT_PATH)
        .unwrap()
        .interface(PLAYER_INTERFACE)
        .unwrap()
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .unwrap()
}
