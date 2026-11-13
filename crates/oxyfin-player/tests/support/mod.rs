//! Shared helpers for the player integration tests.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use oxyfin_player::{Player, PlayerEvent, PlayerOptions, VideoOutput};

/// Collects the events emitted by a player.
pub struct Events {
    receiver: Receiver<PlayerEvent>,
    seen: Mutex<Vec<PlayerEvent>>,
}

impl Events {
    /// Creates a collector and the callback to hand to [`Player::new`].
    pub fn new() -> (Self, impl Fn(PlayerEvent) + Send + 'static) {
        let (sender, receiver) = mpsc::channel();
        let events = Self {
            receiver,
            seen: Mutex::new(Vec::new()),
        };
        let callback = move |event| {
            let _ = sender.send(event);
        };
        (events, callback)
    }

    /// Waits for the first event matching `pred`, panicking with every event
    /// received so far if `timeout` expires.
    pub fn wait_for(&self, timeout: Duration, pred: impl Fn(&PlayerEvent) -> bool) -> PlayerEvent {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(remaining) {
                Ok(event) => {
                    self.seen.lock().unwrap().push(event.clone());
                    if pred(&event) {
                        return event;
                    }
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => panic!(
                    "no matching event within {timeout:?}; received: {:#?}",
                    self.seen.lock().unwrap()
                ),
            }
        }
    }

    /// Returns every event received so far, including those still queued.
    pub fn drain(&self) -> Vec<PlayerEvent> {
        let mut seen = self.seen.lock().unwrap();
        seen.extend(self.receiver.try_iter());
        seen.clone()
    }
}

/// Returns the path of a file in `tests/fixtures`.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Creates a player with no video and no audio output.
pub fn headless(on_event: impl Fn(PlayerEvent) + Send + 'static) -> Player {
    Player::new(
        PlayerOptions {
            video_output: VideoOutput::None,
        },
        on_event,
    )
    .unwrap()
}

/// One recorded request: the path and the headers as received.
pub type RecordedRequest = (String, Vec<(String, String)>);

/// A minimal HTTP/1.1 file server that records every request it receives.
pub struct HttpServer {
    address: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl HttpServer {
    /// Serves each `(url path, file)` pair on a free local port.
    pub fn serve(files: &[(&str, PathBuf)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let files: Arc<HashMap<String, PathBuf>> = Arc::new(
            files
                .iter()
                .map(|(path, file)| ((*path).to_owned(), file.clone()))
                .collect(),
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let accept_thread = {
            let requests = Arc::clone(&requests);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let files = Arc::clone(&files);
                    let requests = Arc::clone(&requests);
                    // mpv may keep one connection open while it opens another.
                    thread::spawn(move || handle(stream, &files, &requests));
                }
            })
        };

        Self {
            address,
            requests,
            stop,
            accept_thread: Some(accept_thread),
        }
    }

    /// Returns the absolute URL of `path` on this server.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /// Returns every request received so far as `(path, headers)`.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.address);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle(
    stream: TcpStream,
    files: &HashMap<String, PathBuf>,
    requests: &Mutex<Vec<RecordedRequest>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();

    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    requests.lock().unwrap().push((path.clone(), headers));

    let body = files.get(&path).and_then(|file| std::fs::read(file).ok());
    let _ = match body {
        Some(body) => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: none\r\nConnection: close\r\n\r\n",
                body.len()
            );
            writer.write_all(head.as_bytes()).and_then(|()| {
                if method == "HEAD" {
                    Ok(())
                } else {
                    writer.write_all(&body)
                }
            })
        }
        None => writer
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
    };
}
