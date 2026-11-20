//! The screen inhibition on a private D-Bus bus, granted by a fake portal or
//! a fake screen saver service as a desktop would.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use oxyfin::inhibit::{
    IDLE, Inhibitor, PORTAL_NAME, PORTAL_PATH, REASON, SCREEN_SAVER, SCREEN_SAVER_PATH, Update,
};
use oxyfin::mpris::Bus;
use oxyfin_core::PlaybackEvent;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::timeout;
use zbus::message::Header;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{Connection, interface};

const DEADLINE: Duration = Duration::from_secs(5);

/// A `dbus-daemon` of its own, so the test never touches the user's session bus. It
/// starts no service on demand, unlike a session bus that would start the real portal.
struct PrivateBus {
    child: Child,
    _config: tempfile::TempDir,
}

const CONFIG: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;

impl PrivateBus {
    fn start() -> (Self, String) {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("bus.conf");
        std::fs::write(&config, CONFIG).unwrap();
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon must be on the PATH");
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        (
            Self {
                child,
                _config: dir,
            },
            address.trim().to_owned(),
        )
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Portal {
    calls: UnboundedSender<String>,
    next: AtomicU32,
}

#[interface(name = "org.freedesktop.portal.Inhibit")]
impl Portal {
    async fn inhibit(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        window: &str,
        flags: u32,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let path = format!(
            "/org/freedesktop/portal/desktop/request/1/t{}",
            self.next.fetch_add(1, Ordering::SeqCst)
        );
        server
            .at(
                path.as_str(),
                Request {
                    calls: self.calls.clone(),
                },
            )
            .await?;
        let reason = options
            .get("reason")
            .and_then(|value| String::try_from(value.clone()).ok())
            .unwrap_or_default();
        let _ = self
            .calls
            .send(format!("inhibit {window:?} {flags} {reason} {path}"));
        Ok(OwnedObjectPath::try_from(path).unwrap())
    }
}

struct Request {
    calls: UnboundedSender<String>,
}

#[interface(name = "org.freedesktop.portal.Request")]
impl Request {
    async fn close(&self, #[zbus(header)] header: Header<'_>) {
        let path = header.path().map(ToString::to_string).unwrap_or_default();
        let _ = self.calls.send(format!("close {path}"));
    }
}

struct ScreenSaver {
    calls: UnboundedSender<String>,
}

#[interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    fn inhibit(&self, application: &str, reason: &str) -> u32 {
        let _ = self.calls.send(format!("inhibit {application} {reason}"));
        7
    }

    fn un_inhibit(&self, cookie: u32) {
        let _ = self.calls.send(format!("uninhibit {cookie}"));
    }
}

async fn call(calls: &mut UnboundedReceiver<String>) -> String {
    timeout(DEADLINE, calls.recv()).await.unwrap().unwrap()
}

async fn quiet(calls: &mut UnboundedReceiver<String>) {
    let extra = timeout(Duration::from_millis(300), calls.recv()).await;
    assert!(extra.is_err(), "unexpected call {extra:?}");
}

fn ready() -> Update {
    Update::Event(PlaybackEvent::Ready {
        duration: Some(Duration::from_secs(90)),
        audio: Vec::new(),
        subtitles: Vec::new(),
        has_previous: true,
        has_next: false,
    })
}

fn loading() -> Update {
    Update::Event(PlaybackEvent::Loading {
        item: "a".into(),
        title: oxyfin_core::PlaybackTitle {
            title: "A".into(),
            series_name: None,
            episode_label: None,
            artwork: None,
        },
    })
}

fn position(seconds: u64) -> Update {
    Update::Event(PlaybackEvent::Position(Duration::from_secs(seconds)))
}

async fn serve<I: zbus::object_server::Interface>(
    address: &str,
    name: &str,
    path: &str,
    interface: I,
) -> Connection {
    zbus::connection::Builder::address(address)
        .unwrap()
        .name(name.to_owned())
        .unwrap()
        .serve_at(path, interface)
        .unwrap()
        .build()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "starts a private dbus-daemon: cargo test -p oxyfin --test inhibit_bus -- --ignored"]
async fn the_portal_keeps_the_screen_awake_while_playing() {
    let (_bus, address) = PrivateBus::start();
    let (sender, mut calls) = unbounded_channel();
    let _portal = serve(
        &address,
        PORTAL_NAME,
        PORTAL_PATH,
        Portal {
            calls: sender,
            next: AtomicU32::new(0),
        },
    )
    .await;
    let inhibitor = Inhibitor::start(
        &tokio::runtime::Handle::current(),
        Bus::Address(address.clone()),
    );

    inhibitor.update(Update::Active(true));
    inhibitor.update(ready());
    let first = call(&mut calls).await;
    assert!(
        first.starts_with(&format!("inhibit \"\" {IDLE} {REASON} ")),
        "{first}"
    );
    inhibitor.update(Update::Event(PlaybackEvent::Paused(true)));
    assert_eq!(
        call(&mut calls).await,
        "close /org/freedesktop/portal/desktop/request/1/t0"
    );
    inhibitor.update(Update::Event(PlaybackEvent::Paused(false)));
    assert!(call(&mut calls).await.ends_with("/t1"));
    inhibitor.update(Update::Active(false));
    assert!(call(&mut calls).await.starts_with("close "));

    inhibitor.update(Update::Active(true));
    inhibitor.update(ready());
    assert!(call(&mut calls).await.ends_with("/t2"));
    timeout(DEADLINE, inhibitor.release()).await.unwrap();
    assert_eq!(
        call(&mut calls).await,
        "close /org/freedesktop/portal/desktop/request/1/t2"
    );
    quiet(&mut calls).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "starts a private dbus-daemon: cargo test -p oxyfin --test inhibit_bus -- --ignored"]
async fn the_screen_saver_service_is_the_fallback_without_a_portal() {
    let (_bus, address) = PrivateBus::start();
    let (sender, mut calls) = unbounded_channel();
    let _screen_saver = serve(
        &address,
        SCREEN_SAVER,
        SCREEN_SAVER_PATH,
        ScreenSaver { calls: sender },
    )
    .await;
    let inhibitor = Inhibitor::start(
        &tokio::runtime::Handle::current(),
        Bus::Address(address.clone()),
    );

    inhibitor.update(Update::Active(true));
    inhibitor.update(ready());
    assert_eq!(call(&mut calls).await, format!("inhibit Oxyfin {REASON}"));
    inhibitor.update(loading());
    quiet(&mut calls).await;
    inhibitor.update(ready());
    inhibitor.update(position(89));
    inhibitor.update(Update::Event(PlaybackEvent::Ended));
    assert_eq!(call(&mut calls).await, "uninhibit 7");
    inhibitor.update(position(90));
    quiet(&mut calls).await;
    inhibitor.update(position(30));
    assert_eq!(call(&mut calls).await, format!("inhibit Oxyfin {REASON}"));
    timeout(DEADLINE, inhibitor.release()).await.unwrap();
    assert_eq!(call(&mut calls).await, "uninhibit 7");
    quiet(&mut calls).await;
}

/// A portal that grants the inhibition only after the app stopped waiting.
struct SlowPortal {
    calls: UnboundedSender<String>,
}

#[interface(name = "org.freedesktop.portal.Inhibit")]
impl SlowPortal {
    async fn inhibit(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        _window: &str,
        _flags: u32,
        _options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        std::thread::sleep(Duration::from_secs(6));
        let path = "/org/freedesktop/portal/desktop/request/1/late";
        server
            .at(
                path,
                Request {
                    calls: self.calls.clone(),
                },
            )
            .await?;
        let _ = self.calls.send(format!("granted {path}"));
        Ok(OwnedObjectPath::try_from(path).unwrap())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "starts a private dbus-daemon: cargo test -p oxyfin --test inhibit_bus -- --ignored"]
async fn a_portal_grant_after_the_timeout_is_released() {
    let (_bus, address) = PrivateBus::start();
    let (sender, mut calls) = unbounded_channel();
    let _portal = serve(
        &address,
        PORTAL_NAME,
        PORTAL_PATH,
        SlowPortal {
            calls: sender.clone(),
        },
    )
    .await;
    let _screen_saver = serve(
        &address,
        SCREEN_SAVER,
        SCREEN_SAVER_PATH,
        ScreenSaver { calls: sender },
    )
    .await;
    let inhibitor = Inhibitor::start(
        &tokio::runtime::Handle::current(),
        Bus::Address(address.clone()),
    );

    inhibitor.update(Update::Active(true));
    inhibitor.update(ready());
    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push(
            timeout(Duration::from_secs(10), calls.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(
        seen,
        [
            format!("inhibit Oxyfin {REASON}"),
            "granted /org/freedesktop/portal/desktop/request/1/late".to_owned(),
            "close /org/freedesktop/portal/desktop/request/1/late".to_owned(),
        ]
    );
    timeout(DEADLINE, inhibitor.release()).await.unwrap();
    assert_eq!(call(&mut calls).await, "uninhibit 7");
}
