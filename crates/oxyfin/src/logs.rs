//! The log file kept for bug reports: bounded in size, rotated once, with credentials scrubbed.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;

use log::Log;
use oxyfin_core::Paths;

/// Size at which the log file is rotated: the current and the rotated file stay under twice this.
pub const LOG_FILE_LIMIT: u64 = 1024 * 1024;

/// Bytes of the most recent log that "Copy logs" puts on the clipboard, so that the report fits
/// the 65,536 characters of a GitHub issue body.
pub const RECENT_LOG_BYTES: usize = 48 * 1024;

/// Name of the current log file.
pub const LOG_FILE: &str = "oxyfin.log";

/// Name of the previous log file, replaced at each rotation.
pub const ROTATED_LOG_FILE: &str = "oxyfin.1.log";

const REDACTED: &str = "<redacted>";

/// Schemes of the URLs whose host is a server.
const SERVER_SCHEMES: [&str; 4] = ["http", "https", "ws", "wss"];

/// Domains of local networks, which complete the short name of a server.
const LOCAL_SUFFIXES: [&str; 5] = [".local", ".lan", ".home", ".internal", ".arpa"];

/// Extensions of source files, whose lines read like the ports of a server.
const SOURCE_EXTENSIONS: [&str; 8] = ["rs", "c", "h", "cc", "cpp", "hpp", "py", "js"];

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
        let file = open_log(&dir.join(LOG_FILE), false)?;
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
            None => self.file.insert(open_log(&self.dir.join(LOG_FILE), false)?),
        };
        file.write_all(line.as_bytes())?;
        self.size += len;
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Closed first: Windows cannot rename an open file.
        self.file = None;
        self.size = 0;
        let current = self.dir.join(LOG_FILE);
        match std::fs::rename(&current, self.dir.join(ROTATED_LOG_FILE)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                // Without a rotated file, the current one starts over so its size stays bounded.
                self.file = Some(open_log(&current, true)?);
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Opens the log file at `path` for appending, or emptied when `truncate`; only its owner
/// may read a file it creates.
fn open_log(path: &Path, truncate: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true);
    if truncate {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path)?;
    // A file made by an earlier version may still be readable by others; logging goes on if it
    // cannot be narrowed.
    #[cfg(unix)]
    let _ = file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600));
    Ok(file)
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
    skip(&mut at, b" ");
    // The Debug output of an `Option`: `Some("value")`.
    if bytes[at..].starts_with(b"some(") {
        at += "some(".len();
    }
    if let Some(&quote @ (b'"' | b'\'')) = bytes.get(at) {
        let start = at + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != quote {
            end += if bytes[end] == b'\\' { 2 } else { 1 };
        }
        let end = end.min(bytes.len());
        return (end > start).then_some((start, end));
    }
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

/// What "Copy logs" puts on the clipboard: `header`, then the recent log in `dir` with every
/// mention of a server replaced: the `hosts` the app knows, and those of the URLs in the log.
pub fn report(dir: &Path, header: &str, hosts: &[String]) -> String {
    let log = recent(dir, RECENT_LOG_BYTES);
    format!("{header}\n\n{}", redact_hosts(&log, hosts))
}

/// The host of a server address, which may lack its scheme.
pub fn host_of(address: &str) -> Option<String> {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let host = &rest[host_span(rest)?];
    Some(host.to_ascii_lowercase())
}

/// Where the host is in `rest`, the part of a URL after its scheme.
fn host_span(rest: &str) -> Option<std::ops::Range<usize>> {
    let end = rest
        .find(|c: char| c.is_whitespace() || "/?#\"'`<>\\,;(){}|".contains(c))
        .unwrap_or(rest.len());
    let start = rest[..end].rfind('@').map_or(0, |at| at + 1);
    let authority = &rest[start..end];
    let length = if authority.starts_with('[') {
        authority.find(']')? + 1
    } else {
        authority.find([':', ']']).unwrap_or(authority.len())
    };
    let host = authority[..length].trim_end_matches('.');
    (!host.is_empty()).then(|| start..start + host.len())
}

/// The hosts of the URLs in `text`.
fn hosts_in(text: &str) -> Vec<String> {
    text.match_indices("://")
        .filter_map(|(at, separator)| {
            let scheme = text[..at]
                .rsplit(|c: char| !c.is_ascii_alphanumeric())
                .next()?;
            SERVER_SCHEMES.contains(&scheme).then_some(())?;
            let rest = text[at + separator.len()..].lines().next()?;
            let host = &rest[host_span(rest)?];
            is_host(host).then(|| host.to_owned())
        })
        .collect()
}

/// Whether `host` can name a server: a name with a letter or a whole IP address.
fn is_host(host: &str) -> bool {
    let bare = host.trim_matches(['[', ']']);
    bare.chars().any(char::is_alphabetic) || bare.parse::<std::net::IpAddr>().is_ok()
}

/// `text` with every mention of `hosts` and of the hosts of its URLs replaced by `<server>`,
/// compared without case and only as whole words.
pub fn redact_hosts(text: &str, hosts: &[String]) -> String {
    let lower = text.to_ascii_lowercase();
    let mut wanted = hosts_in(&lower);
    wanted.extend(hosts.iter().map(|host| host.to_ascii_lowercase()));
    wanted.retain(|host| is_host(host));
    // An IPv6 address is also logged without its brackets.
    let bare = |host: &String| Some(host.strip_prefix('[')?.strip_suffix(']')?.to_owned());
    wanted.extend(wanted.iter().filter_map(bare).collect::<Vec<_>>());
    // The longest first: a host can end with another one.
    wanted.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    wanted.dedup();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for host in &wanted {
        for (start, _) in lower.match_indices(host.as_str()) {
            let end = start + host.len();
            let covered = spans.iter().any(|(from, to)| start < *to && *from < end);
            if covered {
                continue;
            }
            if let Some(end) = named_host_end(&lower, start, end, host) {
                spans.push((user_info_start(&lower, start), end));
            }
        }
    }
    spans.sort_unstable();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (start, end) in spans {
        if start >= copied {
            out.push_str(&text[copied..start]);
            out.push_str("<server>");
        }
        copied = copied.max(end);
    }
    out.push_str(&text[copied..]);
    out
}

/// The end of the host named by the text from `start` to `end`, which spells `host`; none when
/// that text is a part of another word or of another host.
fn named_host_end(text: &str, start: usize, end: usize, host: &str) -> Option<usize> {
    let word =
        |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') || byte >= 0x80;
    let (before, after) = (&text.as_bytes()[..start], &text.as_bytes()[end..]);
    let whole_before = match before.last() {
        // A longer name of the same domain is hidden up to the part known.
        Some(b'.') => host.contains('.'),
        Some(byte) => !word(*byte),
        None => true,
    };
    if !whole_before {
        return None;
    }
    let has_port = |rest: &[u8]| matches!(rest, [b':', digit, ..] if digit.is_ascii_digit());
    let in_url = before.ends_with(b"//") || before.ends_with(b"@");
    // A file and its line, as in a panic message, read like a host and its port.
    let in_path = !in_url && matches!(before.last(), Some(b'/' | b'\\'));
    let local_end = || {
        let labels = after
            .iter()
            .take_while(|byte| word(**byte) || **byte == b'.');
        let domain = text[end..end + labels.count()].trim_end_matches('.');
        let local = LOCAL_SUFFIXES.iter().any(|suffix| domain.ends_with(suffix));
        let source = domain
            .rsplit('.')
            .next()
            .is_some_and(|label| SOURCE_EXTENSIONS.contains(&label));
        let port = has_port(&after[domain.len()..]) && (in_url || !(in_path || source));
        (local || port).then_some(end + domain.len())
    };
    let whole_end = match after {
        // The short name of a server, completed with the domain of its network.
        [b'.', next, ..] if word(*next) => {
            return (!host.contains('.')).then(local_end).flatten();
        }
        [byte, ..] if *byte != b'.' => (!word(*byte)).then_some(end),
        _ => Some(end),
    }?;
    // A server can be named like a word of the log, or like the module a line comes from.
    if host == env!("CARGO_PKG_NAME") {
        return (in_url || has_port(after)).then_some(whole_end);
    }
    (!before.ends_with(b"::") && !after.starts_with(b"::")).then_some(whole_end)
}

/// Where the user name and password before the host at `start` begin, else `start`.
fn user_info_start(text: &str, start: usize) -> usize {
    if !text[..start].ends_with('@') {
        return start;
    }
    let token = text[..start]
        .rfind(|c: char| c.is_whitespace() || "\"'`<>()".contains(c))
        .map_or(0, |at| {
            at + text[at..].chars().next().map_or(1, char::len_utf8)
        });
    text[token..start]
        .rfind("://")
        .map_or(token, |at| token + at + "://".len())
}

/// The header of a copied log: the app version, the system and the libmpv version `mpv`.
pub fn header(mpv: &str) -> String {
    format!(
        "{}\nSystem: {}\nmpv: {mpv}",
        crate::launch::version_line(),
        system()
    )
}

/// The libmpv version, read once on a thread of its own: loading the library takes a while.
#[derive(Clone, Default)]
pub struct MpvVersion(Arc<OnceLock<String>>);

impl MpvVersion {
    /// Reads the version with `read` and logs it, without waiting for it.
    pub fn read_in_background(
        &self,
        read: impl FnOnce() -> String + Send + 'static,
    ) -> Option<JoinHandle<()>> {
        let slot = Arc::clone(&self.0);
        let spawned = std::thread::Builder::new()
            .name("mpv-version".into())
            .spawn(move || {
                // Read first: a disabled log line evaluates nothing.
                let version = slot.get_or_init(read);
                log::info!("mpv: {version}");
            });
        spawned
            .inspect_err(|error| log::warn!("cannot read the mpv version: {error}"))
            .ok()
    }

    /// The version, when it was read.
    pub fn get(&self) -> String {
        self.0
            .get()
            .cloned()
            .unwrap_or_else(|| "not read yet".to_owned())
    }
}

/// The operating system, its architecture and, on Linux, the desktop and session type.
pub fn system() -> String {
    let os = os_release()
        .and_then(|contents| pretty_name(&contents))
        .unwrap_or_else(|| std::env::consts::OS.to_owned());
    let mut system = format!("{os} ({})", std::env::consts::ARCH);
    let desktop: Vec<String> = ["XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .filter(|value| !value.is_empty())
        .collect();
    if !desktop.is_empty() {
        system.push_str(", ");
        system.push_str(&desktop.join(" "));
    }
    system
}

fn os_release() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    // Inside Flatpak, /etc holds the runtime's file and the host's is under /run/host.
    ["/run/host/os-release", "/etc/os-release"]
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
}

/// The `PRETTY_NAME` of an os-release file, without its quotes.
pub fn pretty_name(os_release: &str) -> Option<String> {
    os_release.lines().find_map(|line| {
        let value = line.strip_prefix("PRETTY_NAME=")?;
        let value = value.trim().trim_matches(['"', '\'']);
        (!value.is_empty()).then(|| value.to_owned())
    })
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

/// Whether a record may reach the log file: other crates stop at debug whatever `RUST_LOG`
/// asks, since their trace output can hold whole requests.
fn kept_in_file(metadata: &log::Metadata<'_>) -> bool {
    let target = metadata.target();
    metadata.level() <= log::Level::Debug
        || target == "oxyfin"
        || target.starts_with("oxyfin::")
        || target.starts_with("oxyfin_")
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
                .is_some_and(|file| kept_in_file(metadata) && file.enabled(metadata))
    }

    fn log(&self, record: &log::Record<'_>) {
        self.console.log(record);
        if let Some(file) = &self.file
            && kept_in_file(record.metadata())
        {
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
            ("password=p4ss&next=1", "password=<redacted>&next=1"),
            (
                r#"password="p4ss word, more" user=a"#,
                r#"password="<redacted>" user=a"#,
            ),
            (
                r#"{"Pw":"a \"quoted\" pass","Username":"a"}"#,
                r#"{"Pw":"<redacted>","Username":"a"}"#,
            ),
            ("token='it is, me' rest", "token='<redacted>' rest"),
            (
                r#"Credentials { password: Some("p4ss word"), user: "a" }"#,
                r#"Credentials { password: Some("<redacted>"), user: "a" }"#,
            ),
            ("token: Some(abc), next", "token: Some(<redacted>), next"),
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
    fn logging_goes_on_when_the_rotation_cannot_rename() {
        let dir = tempfile::TempDir::new().unwrap();
        let blocker = dir.path().join(ROTATED_LOG_FILE);
        std::fs::create_dir(&blocker).unwrap();
        std::fs::write(blocker.join("kept"), b"x").unwrap();
        let mut file = LogFile::open(dir.path(), 100).unwrap();
        for index in 0..6 {
            file.write_all(format!("{index:039}\n").as_bytes()).unwrap();
        }
        file.flush().unwrap();
        let current = read(dir.path(), LOG_FILE);
        assert_eq!(current, format!("{:039}\n{:039}\n", 4, 5));
    }

    #[test]
    fn logging_goes_on_when_the_file_was_removed() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut file = LogFile::open(dir.path(), 100).unwrap();
        file.write_all(format!("{}\n", "a".repeat(79)).as_bytes())
            .unwrap();
        std::fs::remove_file(dir.path().join(LOG_FILE)).unwrap();
        file.write_all(format!("{}\n", "b".repeat(39)).as_bytes())
            .unwrap();
        file.write_all(format!("{}\n", "c".repeat(39)).as_bytes())
            .unwrap();
        file.flush().unwrap();
        assert_eq!(
            read(dir.path(), LOG_FILE),
            format!("{}\n{}\n", "b".repeat(39), "c".repeat(39))
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_the_owner_reads_the_log() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let mode = |name: &str| {
            std::fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        std::fs::write(dir.path().join(LOG_FILE), b"old\n").unwrap();
        std::fs::set_permissions(
            dir.path().join(LOG_FILE),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let mut file = LogFile::open(dir.path(), 100).unwrap();
        assert_eq!(mode(LOG_FILE), 0o600, "an older file is narrowed");
        for _ in 0..4 {
            file.write_all(format!("{}\n", "x".repeat(39)).as_bytes())
                .unwrap();
        }
        assert_eq!(mode(ROTATED_LOG_FILE), 0o600);
        assert_eq!(mode(LOG_FILE), 0o600, "a new file is private");
    }

    #[test]
    fn other_crates_stop_at_debug_in_the_file() {
        let kept = |target: &str, level: log::Level| {
            kept_in_file(&log::Metadata::builder().target(target).level(level).build())
        };
        assert!(!kept("hyper_util::client", log::Level::Trace));
        assert!(!kept("reqwest::connect", log::Level::Trace));
        assert!(kept("reqwest::connect", log::Level::Debug));
        assert!(kept("oxyfin", log::Level::Trace));
        assert!(kept("oxyfin_core::playback", log::Level::Trace));
        assert!(kept("oxyfin::app", log::Level::Trace));
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
    fn the_report_fits_an_issue_and_hides_the_server() {
        let dir = tempfile::TempDir::new().unwrap();
        let line = "INFO oxyfin: GET https://Media.Example.org:8096/Items done\n";
        let log = line.repeat(4 * RECENT_LOG_BYTES / line.len());
        std::fs::write(dir.path().join(LOG_FILE), &log).unwrap();
        let report = report(
            dir.path(),
            "oxyfin 1.2.3",
            &["media.example.org".to_owned()],
        );
        assert!(
            report.starts_with("oxyfin 1.2.3\n\nINFO"),
            "{}",
            &report[..40]
        );
        assert!(
            report.chars().count() < 65_536,
            "{}",
            report.chars().count()
        );
        assert!(report.matches("/Items done").count() >= RECENT_LOG_BYTES / line.len() - 1);
        assert!(!report.to_ascii_lowercase().contains("example.org"));
        assert!(report.contains("GET https://<server>:8096/Items done"));
    }

    fn redacted(text: &str, hosts: &[&str]) -> String {
        let hosts: Vec<String> = hosts.iter().map(|host| (*host).to_owned()).collect();
        redact_hosts(text, &hosts)
    }

    #[test]
    fn every_server_known_or_logged_is_hidden() {
        for (text, hosts, expected) in [
            (
                "old https://old.example.org/a then new.example.com:8096",
                &["new.example.com"][..],
                "old https://<server>/a then <server>:8096",
            ),
            (
                "GET http://10.0.0.7:8096/Items\nretry of 10.0.0.7 failed",
                &[],
                "GET http://<server>:8096/Items\nretry of <server> failed",
            ),
            (
                "wss://Jelly.Example.org/socket, dns error for jelly.example.org.",
                &[],
                "wss://<server>/socket, dns error for <server>.",
            ),
            (
                "img.media.example.org and media.example.org.uk",
                &["media.example.org"],
                "img.<server> and media.example.org.uk",
            ),
            (
                "http://me:hunter2@box.lan/ and http://[fe80::1]:8096/",
                &[],
                "http://<server>/ and http://<server>:8096/",
            ),
        ] {
            assert_eq!(redacted(text, hosts), expected, "{text}");
        }
    }

    #[test]
    fn a_short_host_leaves_other_words_alone() {
        let text = "nas: the nasal snas nas-like nas_1 nas.rs unas, cannot resolve nas";
        assert_eq!(
            redacted(text, &["nas"]),
            "<server>: the nasal snas nas-like nas_1 nas.rs unas, cannot resolve <server>"
        );
        let text = "INFO  oxyfin::app] oxyfin 0.1.0, http://oxyfin:8096/ via oxyfin:8096";
        assert_eq!(
            redacted(text, &["oxyfin"]),
            "INFO  oxyfin::app] oxyfin 0.1.0, http://<server>:8096/ via <server>:8096"
        );
        assert_eq!(
            redacted("media::load of media", &["media"]),
            "media::load of <server>"
        );
        assert_eq!(redacted("file:///home/a/x", &[]), "file:///home/a/x");
    }

    #[test]
    fn a_short_host_is_hidden_with_the_domain_of_its_network() {
        for (text, expected) in [
            ("reached nas.home.arpa.", "reached <server>."),
            ("http to nas.lan, nas.Local", "http to <server>, <server>"),
            ("via nas.box:8096 again", "via <server>:8096 again"),
            ("x nas.internal/web", "x <server>/web"),
            (
                "nas.rs, nas.localized, nas.home.example, nas. Local",
                "nas.rs, nas.localized, nas.home.example, <server>. Local",
            ),
            ("my.nas.lan stays", "my.nas.lan stays"),
        ] {
            assert_eq!(redacted(text, &["nas"]), expected, "{text}");
        }
        assert_eq!(redacted("oxyfin.lan:8096", &["oxyfin"]), "<server>:8096");
    }

    #[test]
    fn a_source_line_named_like_a_short_host_stays() {
        for (text, expected) in [
            (
                "at nas.rs:42 and nas.rs:1370:5",
                "at nas.rs:42 and nas.rs:1370:5",
            ),
            (
                "panicked at src/nas.box:42:9",
                "panicked at src/nas.box:42:9",
            ),
            ("in C:\\x\\nas.c:7", "in C:\\x\\nas.c:7"),
            ("http://nas.box:80/x", "http://<server>:80/x"),
        ] {
            assert_eq!(redacted(text, &["nas"]), expected, "{text}");
        }
    }

    #[test]
    fn only_the_urls_of_servers_name_a_host() {
        for (text, expected) in [
            ("open fd://12 after 12 tries", "open fd://12 after 12 tries"),
            ("file://media/x of media", "file://media/x of media"),
            ("http://12/a and 12 more", "http://12/a and 12 more"),
            (
                "ws://10.0.0.7/a, 10.0.0.7 and 10.0.0.70",
                "ws://<server>/a, <server> and 10.0.0.70",
            ),
            ("http://box/a then box", "http://<server>/a then <server>"),
        ] {
            assert_eq!(redacted(text, &[]), expected, "{text}");
        }
        assert_eq!(redacted("12 tries", &["12"]), "12 tries");
    }

    #[test]
    fn an_ipv6_address_is_hidden_without_its_brackets() {
        assert_eq!(
            redacted("http://[fe80::1]:8096/ is fe80::1", &[]),
            "http://<server>:8096/ is <server>"
        );
        assert_eq!(
            redacted("to fe80::1, not fe80::12", &["[fe80::1]"]),
            "to <server>, not fe80::12"
        );
    }

    #[test]
    fn the_user_information_before_a_host_is_hidden() {
        for text in [
            "me:hunter2@box.lan",
            "\"me:p,a=ss@box.lan\"",
            "http://me:hunter2@box.lan",
        ] {
            let hidden = redacted(text, &["box.lan"]);
            assert!(hidden.contains("<server>"), "{hidden}");
            for part in ["me", "hunter2", "ss"] {
                assert!(!hidden.contains(&format!("{part}:")), "{hidden}");
                assert!(!hidden.contains(&format!("{part}@")), "{hidden}");
            }
        }
        assert_eq!(
            redacted("try me:hunter2@box.lan:80", &["box.lan"]),
            "try <server>:80"
        );
    }

    #[test]
    fn the_host_of_an_address_is_found_with_or_without_a_scheme() {
        assert_eq!(
            host_of("https://Jelly.example/").as_deref(),
            Some("jelly.example")
        );
        assert_eq!(host_of("nas:8096").as_deref(), Some("nas"));
        assert_eq!(host_of("http://[::1]:8096/x").as_deref(), Some("[::1]"));
        assert_eq!(host_of(""), None);
    }

    #[test]
    fn the_mpv_version_never_keeps_the_reader_waiting() {
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let version = MpvVersion::default();
        assert_eq!(version.get(), "not read yet");
        let reader = version.read_in_background(move || {
            wait.recv().ok();
            "mpv 9.9".to_owned()
        });
        assert_eq!(version.get(), "not read yet");
        release.send(()).unwrap();
        reader.unwrap().join().unwrap();
        assert_eq!(version.get(), "mpv 9.9");
    }

    #[test]
    fn the_header_names_the_version_and_the_system() {
        let header = header("mpv 0.40.0");
        let lines: Vec<&str> = header.lines().collect();
        assert_eq!(lines[0], crate::launch::version_line());
        assert!(lines[1].starts_with("System: "), "{header}");
        assert!(lines[1].contains(std::env::consts::ARCH), "{header}");
        assert_eq!(lines[2], "mpv: mpv 0.40.0");
    }

    #[test]
    fn the_pretty_name_is_read_from_os_release() {
        assert_eq!(
            pretty_name("NAME=NixOS\nPRETTY_NAME=\"NixOS 25.11 (Xantusia)\"\nID=nixos\n")
                .as_deref(),
            Some("NixOS 25.11 (Xantusia)")
        );
        assert_eq!(pretty_name("NAME=Arch\n"), None);
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
