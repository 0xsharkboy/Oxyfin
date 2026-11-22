//! The log file kept for bug reports: bounded in size, rotated once, with credentials scrubbed.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use log::Log;
use oxyfin_core::Paths;

/// Size at which the log file is rotated: the current and the rotated file stay under twice this.
pub const LOG_FILE_LIMIT: u64 = 1024 * 1024;

/// Bytes of the most recent log that "Copy logs" puts on the clipboard.
pub const RECENT_LOG_BYTES: usize = 256 * 1024;

/// Name of the current log file.
pub const LOG_FILE: &str = "oxyfin.log";

/// Name of the previous log file, replaced at each rotation.
pub const ROTATED_LOG_FILE: &str = "oxyfin.1.log";

const REDACTED: &str = "<redacted>";

/// Keys whose value never reaches the log file, compared without case.
const SECRET_KEYS: [&str; 6] = [
    "api_key",
    "apikey",
    "accesstoken",
    "token",
    "password",
    "pw",
];

/// The directory of the log files.
pub fn log_dir(paths: &Paths) -> PathBuf {
    paths.data_dir.join("logs")
}

/// A log file that is rotated once it would grow past its limit.
///
/// Writes are split into lines; each line is scrubbed of credentials before it is written.
pub struct LogFile {
    dir: PathBuf,
    limit: u64,
    file: Option<File>,
    size: u64,
    pending: Vec<u8>,
}

impl LogFile {
    /// Opens or creates the log file in `dir`, rotating it past `limit` bytes.
    pub fn open(dir: &Path, limit: u64) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(LOG_FILE))?;
        let size = file.metadata()?.len();
        Ok(Self {
            dir: dir.to_path_buf(),
            limit,
            file: Some(file),
            size,
            pending: Vec::new(),
        })
    }

    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        let line = scrub(&String::from_utf8_lossy(line));
        let len = line.len() as u64;
        if self.size > 0 && self.size + len > self.limit {
            self.rotate()?;
        }
        let file = match &mut self.file {
            Some(file) => file,
            None => self.file.insert(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(self.dir.join(LOG_FILE))?,
            ),
        };
        file.write_all(line.as_bytes())?;
        self.size += len;
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Closed first: Windows cannot rename an open file.
        self.file = None;
        std::fs::rename(self.dir.join(LOG_FILE), self.dir.join(ROTATED_LOG_FILE))?;
        self.size = 0;
        Ok(())
    }
}

impl Write for LogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            self.append(&line)?;
        }
        if self.pending.len() as u64 > self.limit {
            let line = std::mem::take(&mut self.pending);
            self.append(&line)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.file {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

/// `text` with the value of every credential key (`api_key=`, `Token="..."`, `password:`...)
/// replaced.
pub fn scrub(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for key in SECRET_KEYS {
        let mut from = 0;
        while let Some(found) = lower[from..].find(key) {
            let start = from + found;
            from = start + key.len();
            let preceded = start
                .checked_sub(1)
                .is_some_and(|before| bytes[before].is_ascii_alphanumeric());
            if preceded {
                continue;
            }
            if let Some(span) = value_after(bytes, start + key.len()) {
                spans.push(span);
            }
        }
    }
    spans.sort_unstable();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (start, end) in spans {
        if start < copied {
            continue;
        }
        out.push_str(&text[copied..start]);
        out.push_str(REDACTED);
        copied = end;
    }
    out.push_str(&text[copied..]);
    out
}

/// The span of the value assigned after a key ending at `at`, if a value follows.
fn value_after(bytes: &[u8], mut at: usize) -> Option<(usize, usize)> {
    let skip = |at: &mut usize, set: &[u8]| {
        while *at < bytes.len() && set.contains(&bytes[*at]) {
            *at += 1;
        }
    };
    skip(&mut at, b"\"' ");
    if at >= bytes.len() || !matches!(bytes[at], b'=' | b':') {
        return None;
    }
    at += 1;
    skip(&mut at, b"\"' ");
    let start = at;
    while at < bytes.len() && !b"&\"',;)} \t\r\n".contains(&bytes[at]) {
        at += 1;
    }
    (at > start).then_some((start, at))
}

/// The last `max` bytes of the log in `dir`, rotated file first, starting on a whole line and
/// scrubbed again; empty without a log.
pub fn recent(dir: &Path, max: usize) -> String {
    let mut bytes = std::fs::read(dir.join(ROTATED_LOG_FILE)).unwrap_or_default();
    bytes.extend(std::fs::read(dir.join(LOG_FILE)).unwrap_or_default());
    if bytes.len() > max {
        let cut = bytes.len() - max;
        let start = bytes[cut..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(cut, |line| cut + line + 1);
        bytes.drain(..start);
    }
    scrub(&String::from_utf8_lossy(&bytes))
}

/// Opens `dir` in the file manager, creating it first; the opener runs on its own thread.
pub fn open_folder(dir: &Path) {
    if let Err(error) = std::fs::create_dir_all(dir) {
        log::warn!("cannot create the log folder: {error}");
        return;
    }
    let mut command = opener(dir);
    let spawned = std::thread::Builder::new()
        .name("open-folder".into())
        .spawn(move || {
            // The exit status is ignored: some openers report failure after opening the folder.
            if let Err(error) = command.status() {
                log::warn!("cannot open the log folder: {error}");
            }
        });
    if let Err(error) = spawned {
        log::warn!("cannot open the log folder: {error}");
    }
}

/// The command that opens `dir` in the file manager of the platform.
pub fn opener(dir: &Path) -> std::process::Command {
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        // Inside Flatpak, xdg-open goes through the OpenURI portal.
        "xdg-open"
    };
    let mut command = std::process::Command::new(program);
    command.arg(dir);
    command
}

/// Logs to standard error as before and to the log file in `dir`, when it can be opened.
pub fn init(dir: Option<&Path>) {
    let env = || env_logger::Env::default().default_filter_or("info");
    let console = env_logger::Builder::from_env(env()).build();
    let mut failure = None;
    let file = dir.and_then(|dir| match LogFile::open(dir, LOG_FILE_LIMIT) {
        Ok(file) => Some(
            env_logger::Builder::from_env(env())
                .write_style(env_logger::WriteStyle::Never)
                .target(env_logger::Target::Pipe(Box::new(file)))
                .build(),
        ),
        Err(error) => {
            failure = Some(error);
            None
        }
    });
    let max = console.filter().max(
        file.as_ref()
            .map_or(log::LevelFilter::Off, |file| file.filter()),
    );
    if log::set_boxed_logger(Box::new(Tee { console, file })).is_ok() {
        log::set_max_level(max);
    }
    if let Some(error) = failure {
        log::warn!("cannot open the log file: {error}");
    }
}

struct Tee {
    console: env_logger::Logger,
    file: Option<env_logger::Logger>,
}

impl Log for Tee {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.console.enabled(metadata)
            || self
                .file
                .as_ref()
                .is_some_and(|file| file.enabled(metadata))
    }

    fn log(&self, record: &log::Record<'_>) {
        self.console.log(record);
        if let Some(file) = &self.file {
            file.log(record);
        }
    }

    fn flush(&self) {
        self.console.flush();
        if let Some(file) = &self.file {
            file.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap_or_default()
    }

    #[test]
    fn credentials_are_scrubbed() {
        let cases = [
            (
                "GET /Videos/a/stream?api_key=abc123&Static=true",
                "GET /Videos/a/stream?api_key=<redacted>&Static=true",
            ),
            (
                "/master.m3u8?DeviceId=d&ApiKey=deadbeef",
                "/master.m3u8?DeviceId=d&ApiKey=<redacted>",
            ),
            (
                r#"MediaBrowser Client="Oxyfin", Token="0123456789abcdef""#,
                r#"MediaBrowser Client="Oxyfin", Token="<redacted>""#,
            ),
            ("X-Emby-Token: secret", "X-Emby-Token: <redacted>"),
            (
                r#"{"Username":"a","Pw":"hunter2"}"#,
                r#"{"Username":"a","Pw":"<redacted>"}"#,
            ),
            (r#""AccessToken": "tok""#, r#""AccessToken": "<redacted>""#),
            ("password=p4ss word", "password=<redacted> word"),
        ];
        for (line, expected) in cases {
            assert_eq!(scrub(line), expected, "{line}");
        }
    }

    #[test]
    fn plain_text_is_left_alone() {
        for line in [
            "tokens are kept in memory only",
            "the token store is the keyring",
            "cannot reach https://jellyfin.example.org:8096/Users/Me: timed out",
            "keyword=value",
        ] {
            assert_eq!(scrub(line), line);
        }
    }

    #[test]
    fn lines_are_written_whole_and_scrubbed() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut file = LogFile::open(dir.path(), 1024).unwrap();
        file.write_all(b"first line api_").unwrap();
        assert_eq!(read(dir.path(), LOG_FILE), "");
        file.write_all(b"key=secret\nsecond").unwrap();
        file.write_all(b" line\n").unwrap();
        assert_eq!(
            read(dir.path(), LOG_FILE),
            "first line api_key=<redacted>\nsecond line\n"
        );
    }

    #[test]
    fn the_file_is_rotated_once_past_its_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut file = LogFile::open(dir.path(), 100).unwrap();
        let line = format!("{}\n", "x".repeat(39));
        for _ in 0..12 {
            file.write_all(line.as_bytes()).unwrap();
        }
        let current = read(dir.path(), LOG_FILE);
        let rotated = read(dir.path(), ROTATED_LOG_FILE);
        assert!(current.len() <= 100, "{}", current.len());
        assert_eq!(rotated.len(), 80);
        assert_eq!(current.len() + rotated.len(), 160);
        assert!(std::fs::read_dir(dir.path()).unwrap().count() == 2);
    }

    #[test]
    fn a_reopened_file_keeps_counting_its_size() {
        let dir = tempfile::TempDir::new().unwrap();
        LogFile::open(dir.path(), 100)
            .unwrap()
            .write_all(format!("{}\n", "a".repeat(79)).as_bytes())
            .unwrap();
        LogFile::open(dir.path(), 100)
            .unwrap()
            .write_all(format!("{}\n", "b".repeat(39)).as_bytes())
            .unwrap();
        assert_eq!(read(dir.path(), ROTATED_LOG_FILE).len(), 80);
        assert_eq!(read(dir.path(), LOG_FILE).len(), 40);
    }

    #[test]
    fn opening_fails_where_no_folder_can_be_made() {
        let dir = tempfile::TempDir::new().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"").unwrap();
        assert!(LogFile::open(&blocker.join("logs"), 100).is_err());
    }

    #[test]
    fn recent_keeps_the_last_whole_lines_of_both_files() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(recent(dir.path(), 100), "");
        std::fs::write(dir.path().join(ROTATED_LOG_FILE), "old one\nold two\n").unwrap();
        std::fs::write(dir.path().join(LOG_FILE), "new one\nnew api_key=k\n").unwrap();
        assert_eq!(
            recent(dir.path(), 1000),
            "old one\nold two\nnew one\nnew api_key=<redacted>\n"
        );
        assert_eq!(recent(dir.path(), 30), "new one\nnew api_key=<redacted>\n");
    }

    #[test]
    fn the_folder_opens_with_the_platform_opener() {
        let command = opener(Path::new("/tmp/logs"));
        let expected = if cfg!(target_os = "windows") {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        assert_eq!(command.get_program(), expected);
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new("/tmp/logs")]
        );
    }
}
