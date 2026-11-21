//! Shows what plays on the Discord profile of the user, when they opt in.
//!
//! The local Discord app listens on an IPC socket (a Unix socket, or a named
//! pipe on Windows). A thread of its own talks to it, so a slow or missing
//! Discord never holds the UI back: the app forwards the setting, the player
//! screen state and the playback events with [`Discord::update`], and the
//! thread connects only while something plays with the setting on, retries
//! quietly every [`RETRY`] when Discord is not running, and clears the
//! activity when playback stops, the player closes or the setting goes off.
//!
//! The activity is built by [`oxyfin_core::presence_for`]: titles and times
//! only, never the server address, item ids or user names.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use oxyfin_core::{PlaybackError, PlaybackEvent, PlaybackTitle, Presence, presence_for};
use serde_json::{Value, json};

/// Public ID of the Oxyfin application on Discord.
pub const DISCORD_APP_ID: &str = "1573080372043055284";
/// How often a missing Discord is looked for again, and the activity refreshed at most.
pub const RETRY: Duration = Duration::from_secs(15);
/// Wait after a change before sending it, so the position reported next is taken into account.
pub const SETTLE: Duration = Duration::from_millis(300);
/// How far the shown times may drift from the playback before they are sent again.
pub const DRIFT: i64 = 2;
/// Activity type "Watching".
pub const WATCHING: u8 = 3;

#[cfg(unix)]
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_FRAME: u32 = 64 * 1024;
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

/// A change the activity follows.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// The setting was turned on or off.
    Enabled(bool),
    /// The player screen was opened or closed.
    Active(bool),
    /// An event of the playback controller.
    Event(PlaybackEvent),
}

/// Whether `event` can change the activity, so the app forwards only those.
pub fn concerns(event: &PlaybackEvent) -> bool {
    matches!(
        event,
        PlaybackEvent::Loading { .. }
            | PlaybackEvent::Ready { .. }
            | PlaybackEvent::Position(_)
            | PlaybackEvent::Paused(_)
            | PlaybackEvent::Seeked
            | PlaybackEvent::Stopped
            | PlaybackEvent::SessionExpired
            | PlaybackEvent::Error(_)
    )
}

/// What the activity depends on, updated from [`Update`]s.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Watching {
    enabled: bool,
    active: bool,
    title: Option<PlaybackTitle>,
    ready: bool,
    duration: Option<Duration>,
    position: Duration,
    paused: bool,
    /// Set by Ready and Seeked: the next position, such as a resume point, is sent soon.
    jumped: bool,
}

impl Watching {
    /// Takes `update` into account; returns whether the activity should be
    /// sent again soon rather than at the next refresh.
    pub fn apply(&mut self, update: &Update) -> bool {
        match update {
            Update::Enabled(enabled) => self.enabled = *enabled,
            Update::Active(active) => {
                self.active = *active;
                if !active {
                    self.reset();
                }
            }
            Update::Event(event) => match event {
                PlaybackEvent::Loading { title, .. } => {
                    self.reset();
                    self.title = Some(title.clone());
                    return false;
                }
                PlaybackEvent::Ready { duration, .. } => {
                    self.ready = true;
                    self.duration = *duration;
                    self.jumped = true;
                }
                PlaybackEvent::Position(position) => {
                    self.position = *position;
                    return std::mem::take(&mut self.jumped);
                }
                PlaybackEvent::Paused(paused) => self.paused = *paused,
                PlaybackEvent::Seeked => self.jumped = true,
                PlaybackEvent::Error(PlaybackError::Command { .. }) => return false,
                PlaybackEvent::Stopped
                | PlaybackEvent::SessionExpired
                | PlaybackEvent::Error(_) => self.reset(),
                _ => return false,
            },
        }
        true
    }

    /// The activity to show at Unix time `now`, in seconds, or `None` to show none.
    pub fn presence(&self, now: i64) -> Option<Presence> {
        if !(self.enabled && self.active && self.ready) {
            return None;
        }
        let title = self.title.as_ref()?;
        Some(presence_for(
            title,
            self.position,
            self.duration,
            self.paused,
            None,
            now,
        ))
    }

    /// Whether the setting is on.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    fn reset(&mut self) {
        *self = Self {
            enabled: self.enabled,
            active: self.active,
            ..Self::default()
        };
    }
}

/// Whether `next` shows something else than `shown`, beyond a drift of the times.
pub fn differs(shown: Option<&Presence>, next: Option<&Presence>) -> bool {
    match (shown, next) {
        (None, None) => false,
        (Some(shown), Some(next)) => {
            let drifted = |a: Option<i64>, b: Option<i64>| match (a, b) {
                (Some(a), Some(b)) => (a - b).abs() > DRIFT,
                (a, b) => a.is_some() != b.is_some(),
            };
            shown.details != next.details
                || shown.state != next.state
                || shown.large_image != next.large_image
                || shown.large_text != next.large_text
                || drifted(shown.start, next.start)
                || drifted(shown.end, next.end)
        }
        _ => true,
    }
}

/// The `SET_ACTIVITY` command showing `presence`, or clearing the activity.
pub fn set_activity(presence: Option<&Presence>, pid: u32, nonce: &str) -> Value {
    let mut args = json!({ "pid": pid });
    if let Some(presence) = presence {
        // Discord rejects texts shorter than two characters.
        let text = |text: &str| (text.chars().count() >= 2).then(|| text.to_owned());
        let mut activity = json!({
            "type": WATCHING,
            "instance": false,
            "assets": { "large_image": presence.large_image },
        });
        if let Some(large_text) = text(&presence.large_text) {
            activity["assets"]["large_text"] = json!(large_text);
        }
        if let Some(details) = text(&presence.details) {
            activity["details"] = json!(details);
        }
        if let Some(state) = text(&presence.state) {
            activity["state"] = json!(state);
        }
        if let Some(start) = presence.start {
            activity["timestamps"] = json!({ "start": start.saturating_mul(1000) });
            if let Some(end) = presence.end {
                activity["timestamps"]["end"] = json!(end.saturating_mul(1000));
            }
        }
        args["activity"] = activity;
    }
    json!({ "cmd": "SET_ACTIVITY", "args": args, "nonce": nonce })
}

/// A byte stream to the Discord app.
pub trait Stream: Read + Write + Send {}

impl<T: Read + Write + Send> Stream for T {}

/// A connection to the Discord app, after the handshake.
pub struct Ipc {
    stream: Box<dyn Stream>,
    nonce: u64,
}

impl Ipc {
    /// Says hello to Discord on `stream` and waits until it is ready.
    pub fn handshake(stream: Box<dyn Stream>) -> io::Result<Self> {
        let mut ipc = Self { stream, nonce: 0 };
        let hello = json!({ "v": 1, "client_id": DISCORD_APP_ID });
        ipc.write(OP_HANDSHAKE, &hello)?;
        let answer = ipc.read()?;
        if answer["evt"] != "READY" {
            return Err(refused(&answer));
        }
        log::debug!("Discord is ready");
        Ok(ipc)
    }

    /// Shows `presence`, or clears the activity, and waits for Discord to accept it.
    pub fn set_activity(&mut self, presence: Option<&Presence>) -> io::Result<()> {
        self.nonce += 1;
        let nonce = self.nonce.to_string();
        self.write(
            OP_FRAME,
            &set_activity(presence, std::process::id(), &nonce),
        )?;
        loop {
            let answer = self.read()?;
            if answer["nonce"] != nonce.as_str() {
                continue;
            }
            if answer["evt"] == "ERROR" {
                return Err(refused(&answer));
            }
            log::debug!(
                "Discord accepted {}: cmd={} evt={}",
                if presence.is_some() {
                    "the activity"
                } else {
                    "clearing the activity"
                },
                answer["cmd"],
                answer["evt"],
            );
            return Ok(());
        }
    }

    fn write(&mut self, opcode: u32, payload: &Value) -> io::Result<()> {
        let body = serde_json::to_vec(payload).map_err(io::Error::other)?;
        let length = u32::try_from(body.len()).map_err(io::Error::other)?;
        let mut frame = Vec::with_capacity(8 + body.len());
        frame.extend_from_slice(&opcode.to_le_bytes());
        frame.extend_from_slice(&length.to_le_bytes());
        frame.extend_from_slice(&body);
        self.stream.write_all(&frame)?;
        self.stream.flush()
    }

    fn read(&mut self) -> io::Result<Value> {
        loop {
            let mut header = [0; 8];
            self.stream.read_exact(&mut header)?;
            let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
            if length > MAX_FRAME {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Discord sent a frame too large",
                ));
            }
            let mut body = vec![0; length as usize];
            self.stream.read_exact(&mut body)?;
            match opcode {
                OP_FRAME => {
                    return serde_json::from_slice(&body)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
                }
                OP_PING => {
                    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    self.write(OP_PONG, &body)?;
                }
                OP_CLOSE => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "Discord closed the connection",
                    ));
                }
                _ => {}
            }
        }
    }
}

// Only the code and the message of a refusal: other fields may describe the Discord user.
fn refused(answer: &Value) -> io::Error {
    io::Error::other(format!(
        "Discord refused: {} {}",
        answer["data"]["code"], answer["data"]["message"]
    ))
}

/// Opens a stream to the Discord app, as [`connect`] does.
pub type Connector = Box<dyn FnMut() -> io::Result<Box<dyn Stream>> + Send>;

/// Where the Discord app may listen, most likely first.
#[cfg(unix)]
pub fn socket_dirs() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .filter_map(std::env::var_os)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect();
    bases.push(PathBuf::from("/tmp"));
    let mut dirs = Vec::new();
    for base in bases {
        for sub in ["", "app/com.discordapp.Discord", "snap.discord"] {
            let dir = base.join(sub);
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// Opens a stream to the first Discord app listening in `dirs`.
#[cfg(unix)]
pub fn connect(dirs: &[PathBuf]) -> io::Result<Box<dyn Stream>> {
    for dir in dirs {
        for index in 0..10 {
            let path = dir.join(format!("discord-ipc-{index}"));
            if let Ok(stream) = std::os::unix::net::UnixStream::connect(&path) {
                stream.set_read_timeout(Some(IO_TIMEOUT))?;
                stream.set_write_timeout(Some(IO_TIMEOUT))?;
                return Ok(Box::new(stream));
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Discord is not running",
    ))
}

/// Where the Discord app may listen.
#[cfg(windows)]
pub fn socket_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from(r"\\.\pipe")]
}

/// Opens a stream to the first Discord app listening in `dirs`.
#[cfg(windows)]
pub fn connect(dirs: &[PathBuf]) -> io::Result<Box<dyn Stream>> {
    for dir in dirs {
        for index in 0..10 {
            let path = dir.join(format!("discord-ipc-{index}"));
            if let Ok(pipe) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
            {
                return Ok(Box::new(pipe));
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Discord is not running",
    ))
}

/// Where the Discord app may listen: nowhere known on this system.
#[cfg(not(any(unix, windows)))]
pub fn socket_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// Opens a stream to the Discord app: not supported on this system.
#[cfg(not(any(unix, windows)))]
pub fn connect(_dirs: &[PathBuf]) -> io::Result<Box<dyn Stream>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Discord is not supported",
    ))
}

enum Command {
    Update(Update),
    Quit(Sender<()>),
}

/// The thread showing the activity on Discord.
pub struct Discord {
    commands: Sender<Command>,
}

impl Discord {
    /// Starts the thread, reaching Discord in the usual places.
    pub fn start(enabled: bool) -> io::Result<Self> {
        let dirs = socket_dirs();
        Self::start_with(enabled, Box::new(move || connect(&dirs)), RETRY)
    }

    /// Starts the thread, reaching Discord with `connector` and looking for
    /// it again every `retry` while it is missing.
    pub fn start_with(enabled: bool, connector: Connector, retry: Duration) -> io::Result<Self> {
        let (commands, receiver) = mpsc::channel();
        let mut watching = Watching::default();
        watching.apply(&Update::Enabled(enabled));
        thread::Builder::new()
            .name("discord".into())
            .spawn(move || Worker::new(connector, retry).run(watching, &receiver))?;
        Ok(Self { commands })
    }

    /// Forwards a change; never blocks.
    pub fn update(&self, update: Update) {
        let _ = self.commands.send(Command::Update(update));
    }

    /// Clears the activity and stops the thread, waiting at most `grace`.
    pub fn shutdown(self, grace: Duration) {
        let (done, finished) = mpsc::channel();
        if self.commands.send(Command::Quit(done)).is_ok() && finished.recv_timeout(grace).is_err()
        {
            log::warn!("the Discord activity was not cleared before exit");
        }
    }
}

struct Worker {
    connector: Connector,
    retry: Duration,
    ipc: Option<Ipc>,
    shown: Option<Presence>,
    next_attempt: Instant,
}

impl Worker {
    fn new(connector: Connector, retry: Duration) -> Self {
        Self {
            connector,
            retry,
            ipc: None,
            shown: None,
            next_attempt: Instant::now(),
        }
    }

    fn run(mut self, mut watching: Watching, commands: &Receiver<Command>) {
        let mut due: Option<Instant> = None;
        let mut refresh = Instant::now() + self.retry;
        loop {
            let deadline = due.map_or(refresh, |due| due.min(refresh));
            match commands.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Command::Update(update)) => {
                    if watching.apply(&update) && due.is_none() {
                        due = Some(Instant::now() + SETTLE);
                    }
                }
                Ok(Command::Quit(done)) => {
                    self.leave();
                    let _ = done.send(());
                    return;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.leave();
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            let now = Instant::now();
            let tick = refresh <= now;
            if due.is_some_and(|due| due <= now) || tick {
                due = None;
                refresh = now + self.retry;
                self.sync(&watching, tick);
            }
        }
    }

    /// Shows what `watching` calls for. On a refresh `tick`, the activity
    /// shown is sent again even unchanged, so a Discord that restarted since
    /// gets it back.
    fn sync(&mut self, watching: &Watching, tick: bool) {
        let Some(presence) = watching.presence(unix_now()) else {
            self.leave();
            self.disconnect();
            return;
        };
        let resend = tick && self.shown.is_some();
        if !resend && !differs(self.shown.as_ref(), Some(&presence)) {
            return;
        }
        if self.ipc.is_none() {
            if Instant::now() < self.next_attempt {
                return;
            }
            self.next_attempt = Instant::now() + self.retry;
            match (self.connector)().and_then(Ipc::handshake) {
                Ok(ipc) => self.ipc = Some(ipc),
                Err(error) => {
                    log::debug!("cannot reach Discord: {error}");
                    return;
                }
            }
        }
        let Some(ipc) = self.ipc.as_mut() else { return };
        match ipc.set_activity(Some(&presence)) {
            Ok(()) => self.shown = Some(presence),
            Err(error) => {
                log::warn!("Discord did not take the activity: {error}");
                self.ipc = None;
                self.shown = None;
            }
        }
    }

    /// Closes the connection while nothing is shown; the next activity
    /// connects again at once.
    fn disconnect(&mut self) {
        if self.ipc.take().is_some() {
            self.next_attempt = Instant::now();
        }
    }

    /// Clears the activity shown, if any.
    fn leave(&mut self) {
        if self.shown.take().is_none() {
            return;
        }
        if let Some(ipc) = self.ipc.as_mut()
            && let Err(error) = ipc.set_activity(None)
        {
            log::debug!("cannot clear the Discord activity: {error}");
            self.ipc = None;
        }
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyfin_api::ItemId;

    fn title() -> PlaybackTitle {
        PlaybackTitle {
            title: "The Pact".into(),
            series_name: Some("Les Veilleurs".into()),
            episode_label: Some("S02E05".into()),
            year: None,
            artwork: None,
        }
    }

    fn loading() -> Update {
        Update::Event(PlaybackEvent::Loading {
            item: ItemId::from("0000000000000000000000000000000a"),
            title: title(),
        })
    }

    fn ready() -> Update {
        Update::Event(PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(2400)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: true,
            has_next: false,
        })
    }

    fn playing() -> Watching {
        let mut watching = Watching::default();
        for update in [
            Update::Enabled(true),
            Update::Active(true),
            loading(),
            ready(),
        ] {
            watching.apply(&update);
        }
        watching
    }

    #[test]
    fn shows_nothing_while_off_or_off_the_player() {
        let mut watching = playing();
        assert!(watching.presence(100).is_some());
        watching.apply(&Update::Enabled(false));
        assert_eq!(watching.presence(100), None);

        let mut watching = playing();
        watching.apply(&Update::Active(false));
        watching.apply(&Update::Active(true));
        assert_eq!(watching.presence(100), None);
    }

    #[test]
    fn shows_nothing_before_ready_or_after_a_stop() {
        let mut watching = Watching::default();
        for update in [Update::Enabled(true), Update::Active(true), loading()] {
            watching.apply(&update);
        }
        assert_eq!(watching.presence(100), None);

        let mut watching = playing();
        assert!(watching.apply(&Update::Event(PlaybackEvent::Stopped)));
        assert_eq!(watching.presence(100), None);
    }

    #[test]
    fn follows_the_position_and_the_pause() {
        let mut watching = playing();
        watching.apply(&Update::Event(PlaybackEvent::Position(
            Duration::from_secs(60),
        )));
        let presence = watching.presence(1000).unwrap();
        assert_eq!(presence.start, Some(940));
        assert_eq!(presence.end, Some(940 + 2400));

        assert!(watching.apply(&Update::Event(PlaybackEvent::Paused(true))));
        let presence = watching.presence(1000).unwrap();
        assert_eq!(presence.start, None);
        assert_eq!(presence.state, "S02E05 · The Pact · Paused");
        assert!(watching.apply(&Update::Event(PlaybackEvent::Seeked)));
    }

    #[test]
    fn the_first_position_after_a_jump_is_sent_soon() {
        let mut watching = playing();
        let position = |secs| Update::Event(PlaybackEvent::Position(Duration::from_secs(secs)));
        assert!(watching.apply(&position(600)));
        assert!(!watching.apply(&position(601)));
        watching.apply(&Update::Event(PlaybackEvent::Seeked));
        assert!(watching.apply(&position(900)));
        assert!(!watching.apply(&position(901)));
    }

    #[test]
    fn small_drifts_are_not_sent_again() {
        let watching = playing();
        let shown = watching.presence(1000);
        assert!(!differs(shown.as_ref(), watching.presence(1002).as_ref()));
        assert!(differs(shown.as_ref(), watching.presence(1003).as_ref()));
        assert!(differs(shown.as_ref(), None));
        assert!(!differs(None, None));
    }

    #[test]
    fn the_command_shows_titles_and_times_only() {
        let presence = playing().presence(1000).unwrap();
        let command = set_activity(Some(&presence), 42, "7");
        assert_eq!(command["cmd"], "SET_ACTIVITY");
        assert_eq!(command["nonce"], "7");
        assert_eq!(command["args"]["pid"], 42);
        let activity = &command["args"]["activity"];
        assert_eq!(activity["type"], 3);
        assert_eq!(activity["details"], "Les Veilleurs");
        assert_eq!(activity["state"], "S02E05 · The Pact");
        assert_eq!(activity["timestamps"]["start"], 1_000_000);
        assert_eq!(activity["timestamps"]["end"], 2_400_000 + 1_000_000);
        assert_eq!(activity["assets"]["large_image"], "oxyfin");
        assert!(!command.to_string().contains("http"));

        let clear = set_activity(None, 42, "8");
        assert!(clear["args"].get("activity").is_none());
    }

    #[test]
    fn short_texts_and_missing_times_are_left_out() {
        let presence = Presence {
            details: "M".into(),
            state: String::new(),
            start: None,
            end: None,
            large_image: "oxyfin".into(),
            large_text: "Oxyfin".into(),
        };
        let activity = set_activity(Some(&presence), 1, "1")["args"]["activity"].clone();
        assert!(activity.get("details").is_none());
        assert!(activity.get("state").is_none());
        assert!(activity.get("timestamps").is_none());
    }

    #[cfg(unix)]
    mod ipc {
        use super::*;
        use std::os::unix::net::{UnixListener, UnixStream};
        use std::path::Path;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        const WAIT: Duration = Duration::from_secs(5);
        /// Forwarded by the fakes when the app closes the connection.
        const HUNG_UP: u32 = u32::MAX;

        fn read_frame(stream: &mut UnixStream) -> Option<(u32, Value)> {
            let mut header = [0; 8];
            stream.read_exact(&mut header).ok()?;
            let opcode = u32::from_le_bytes(header[..4].try_into().ok()?);
            let length = u32::from_le_bytes(header[4..].try_into().ok()?);
            let mut body = vec![0; length as usize];
            stream.read_exact(&mut body).ok()?;
            Some((opcode, serde_json::from_slice(&body).ok()?))
        }

        fn write_frame(stream: &mut UnixStream, opcode: u32, payload: &Value) {
            let body = serde_json::to_vec(payload).unwrap();
            let mut frame = opcode.to_le_bytes().to_vec();
            frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
            frame.extend_from_slice(&body);
            stream.write_all(&frame).unwrap();
        }

        /// A fake Discord: forwards every frame received, answers as Discord does.
        fn fake_discord(dir: &Path, refuse: bool) -> Receiver<(u32, Value)> {
            let listener = UnixListener::bind(dir.join("discord-ipc-0")).unwrap();
            let (frames, received) = mpsc::channel();
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    while let Some((opcode, payload)) = read_frame(&mut stream) {
                        let answer = if opcode == OP_HANDSHAKE {
                            json!({ "cmd": "DISPATCH", "evt": "READY", "data": { "v": 1 } })
                        } else if refuse {
                            json!({ "cmd": payload["cmd"], "evt": "ERROR", "nonce": payload["nonce"],
                                    "data": { "code": 4000, "message": "bad" } })
                        } else {
                            json!({ "cmd": payload["cmd"], "evt": null, "nonce": payload["nonce"] })
                        };
                        write_frame(&mut stream, OP_FRAME, &answer);
                        if frames.send((opcode, payload)).is_err() {
                            return;
                        }
                    }
                    if frames.send((HUNG_UP, Value::Null)).is_err() {
                        return;
                    }
                }
            });
            received
        }

        /// A fake Discord that quits after showing the first activity, as when the user closes it.
        fn short_lived_discord(dir: &Path) -> Receiver<(u32, Value)> {
            let socket = dir.join("discord-ipc-0");
            let listener = UnixListener::bind(&socket).unwrap();
            let (frames, received) = mpsc::channel();
            thread::spawn(move || {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                while let Some((opcode, payload)) = read_frame(&mut stream) {
                    let answer = if opcode == OP_HANDSHAKE {
                        json!({ "cmd": "DISPATCH", "evt": "READY", "data": { "v": 1 } })
                    } else {
                        json!({ "cmd": payload["cmd"], "evt": null, "nonce": payload["nonce"] })
                    };
                    write_frame(&mut stream, OP_FRAME, &answer);
                    let shown = opcode == OP_FRAME;
                    let _ = frames.send((opcode, payload));
                    if shown {
                        break;
                    }
                }
                drop(listener);
                let _ = std::fs::remove_file(&socket);
                let _ = stream.shutdown(std::net::Shutdown::Both);
                let _ = frames.send((HUNG_UP, Value::Null));
            });
            received
        }

        fn start(dir: &Path, enabled: bool, retry: Duration) -> (Discord, Arc<AtomicUsize>) {
            let attempts = Arc::new(AtomicUsize::new(0));
            let count = Arc::clone(&attempts);
            let dirs = vec![dir.to_path_buf()];
            let connector: Connector = Box::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                connect(&dirs)
            });
            (
                Discord::start_with(enabled, connector, retry).unwrap(),
                attempts,
            )
        }

        fn play(discord: &Discord) {
            for update in [Update::Active(true), loading(), ready()] {
                discord.update(update);
            }
        }

        #[test]
        fn shows_then_clears_the_activity() {
            let dir = tempfile::tempdir().unwrap();
            let frames = fake_discord(dir.path(), false);
            let (discord, _) = start(dir.path(), true, RETRY);
            play(&discord);

            let (opcode, hello) = frames.recv_timeout(WAIT).unwrap();
            assert_eq!(opcode, OP_HANDSHAKE);
            assert_eq!(hello["client_id"], DISCORD_APP_ID);
            let (_, set) = frames.recv_timeout(WAIT).unwrap();
            assert_eq!(set["cmd"], "SET_ACTIVITY");
            assert_eq!(set["args"]["activity"]["details"], "Les Veilleurs");
            assert_eq!(set["args"]["activity"]["type"], WATCHING);

            discord.update(Update::Event(PlaybackEvent::Paused(true)));
            let (_, paused) = frames.recv_timeout(WAIT).unwrap();
            assert_eq!(
                paused["args"]["activity"]["state"],
                "S02E05 · The Pact · Paused"
            );

            discord.update(Update::Active(false));
            let (_, clear) = frames.recv_timeout(WAIT).unwrap();
            assert_eq!(clear["cmd"], "SET_ACTIVITY");
            assert!(clear["args"].get("activity").is_none());
            discord.shutdown(WAIT);
        }

        #[test]
        fn a_resume_point_is_sent_at_once() {
            let dir = tempfile::tempdir().unwrap();
            let frames = fake_discord(dir.path(), false);
            let (discord, _) = start(dir.path(), true, RETRY);
            play(&discord);
            frames.recv_timeout(WAIT).unwrap();
            frames.recv_timeout(WAIT).unwrap();

            discord.update(Update::Event(PlaybackEvent::Position(Duration::from_secs(
                600,
            ))));
            let (_, resumed) = frames.recv_timeout(Duration::from_secs(3)).unwrap();
            let start = resumed["args"]["activity"]["timestamps"]["start"]
                .as_i64()
                .unwrap();
            let elapsed = unix_now() * 1000 - start;
            assert!(
                (598_000..=603_000).contains(&elapsed),
                "{elapsed} ms elapsed"
            );
            discord.shutdown(WAIT);
        }

        #[test]
        fn turning_off_clears_the_activity() {
            let dir = tempfile::tempdir().unwrap();
            let frames = fake_discord(dir.path(), false);
            let (discord, _) = start(dir.path(), true, RETRY);
            play(&discord);
            frames.recv_timeout(WAIT).unwrap();
            frames.recv_timeout(WAIT).unwrap();

            discord.update(Update::Enabled(false));
            let (_, clear) = frames.recv_timeout(WAIT).unwrap();
            assert!(clear["args"].get("activity").is_none());
            discord.shutdown(WAIT);
        }

        #[test]
        fn exiting_clears_the_activity() {
            let dir = tempfile::tempdir().unwrap();
            let frames = fake_discord(dir.path(), false);
            let (discord, _) = start(dir.path(), true, RETRY);
            play(&discord);
            frames.recv_timeout(WAIT).unwrap();
            frames.recv_timeout(WAIT).unwrap();

            discord.shutdown(WAIT);
            let (_, clear) = frames.recv_timeout(WAIT).unwrap();
            assert!(clear["args"].get("activity").is_none());
        }

        #[test]
        fn off_never_connects() {
            let dir = tempfile::tempdir().unwrap();
            let frames = fake_discord(dir.path(), false);
            let (discord, attempts) = start(dir.path(), false, Duration::from_millis(50));
            play(&discord);
            thread::sleep(Duration::from_millis(500));
            assert_eq!(attempts.load(Ordering::SeqCst), 0);
            assert!(frames.try_recv().is_err());
            discord.shutdown(WAIT);
        }

        #[test]
        fn a_missing_discord_is_looked_for_quietly_at_each_retry() {
            let dir = tempfile::tempdir().unwrap();
            let (discord, attempts) = start(dir.path(), true, Duration::from_millis(200));
            play(&discord);
            thread::sleep(Duration::from_millis(1100));
            let tried = attempts.load(Ordering::SeqCst);
            assert!((3..=7).contains(&tried), "{tried} attempts");

            let frames = fake_discord(dir.path(), false);
            let (opcode, _) = frames.recv_timeout(WAIT).unwrap();
            assert_eq!(opcode, OP_HANDSHAKE);
            let (_, set) = frames.recv_timeout(WAIT).unwrap();
            assert_eq!(set["cmd"], "SET_ACTIVITY");
            discord.shutdown(WAIT);
        }

        #[test]
        fn the_activity_comes_back_after_discord_restarts() {
            let dir = tempfile::tempdir().unwrap();
            let frames = short_lived_discord(dir.path());
            let (discord, _) = start(dir.path(), true, Duration::from_millis(200));
            play(&discord);
            // Paused, the activity has no times that drift and would be sent again anyway.
            discord.update(Update::Event(PlaybackEvent::Paused(true)));
            assert_eq!(frames.recv_timeout(WAIT).unwrap().0, OP_HANDSHAKE);
            assert_eq!(frames.recv_timeout(WAIT).unwrap().0, OP_FRAME);
            assert_eq!(frames.recv_timeout(WAIT).unwrap().0, HUNG_UP);

            let frames = fake_discord(dir.path(), false);
            let (opcode, _) = frames.recv_timeout(WAIT).unwrap();
            assert_eq!(opcode, OP_HANDSHAKE);
            let (_, set) = frames.recv_timeout(WAIT).unwrap();
            assert_eq!(
                set["args"]["activity"]["state"],
                "S02E05 · The Pact · Paused"
            );
            discord.shutdown(WAIT);
        }

        #[test]
        fn the_connection_closes_while_nothing_plays() {
            let dir = tempfile::tempdir().unwrap();
            let frames = fake_discord(dir.path(), false);
            let (discord, attempts) = start(dir.path(), true, RETRY);
            play(&discord);
            frames.recv_timeout(WAIT).unwrap();
            frames.recv_timeout(WAIT).unwrap();

            discord.update(Update::Event(PlaybackEvent::Stopped));
            let (_, clear) = frames.recv_timeout(WAIT).unwrap();
            assert!(clear["args"].get("activity").is_none());
            assert_eq!(frames.recv_timeout(WAIT).unwrap().0, HUNG_UP);

            discord.update(loading());
            discord.update(ready());
            let (opcode, _) = frames.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(opcode, OP_HANDSHAKE);
            let (_, set) = frames.recv_timeout(WAIT).unwrap();
            assert_eq!(set["cmd"], "SET_ACTIVITY");
            assert_eq!(attempts.load(Ordering::SeqCst), 2);
            discord.shutdown(WAIT);
        }

        #[test]
        fn a_refused_activity_is_tried_again_later() {
            let dir = tempfile::tempdir().unwrap();
            let frames = fake_discord(dir.path(), true);
            let (discord, attempts) = start(dir.path(), true, Duration::from_millis(200));
            play(&discord);
            frames.recv_timeout(WAIT).unwrap();
            frames.recv_timeout(WAIT).unwrap();
            thread::sleep(Duration::from_millis(700));
            assert!(attempts.load(Ordering::SeqCst) >= 2);
            discord.shutdown(WAIT);
        }
    }
}
