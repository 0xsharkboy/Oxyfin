//! The last answers of the server, shown again at once while they are asked for anew.
//!
//! They live as long as the session of one account. The answers of the home screen and the first
//! pages of the libraries are also kept on disk, for the next launch.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use oxyfin_api::ItemId;
use serde::{Deserialize, Serialize};
use tokio::time::{Duration, Instant};

use crate::catalog::{BrowseQuery, BrowseSource, SortField, WatchFilter};
use crate::models::{
    CollectionDetails, Episode, ItemSummary, Library, MovieDetails, Page, PersonDetails,
    PersonTitle, SeriesDetails,
};
use crate::{Paths, Result};

/// Answers kept besides those of the next launch; the least recently used ones are forgotten
/// first.
pub(crate) const KEPT_ANSWERS: usize = 96;

/// Answers the next launch may show kept, apart from the others: browsing many pages never
/// pushes Home out.
const KEPT_FOR_LAUNCH: usize = 96;

/// Items a kept list holds; a longer list is not kept.
const LONGEST_LIST: usize = 300;

/// Items of a listing kept: its first page.
pub(crate) const FIRST_PAGE: usize = 60;

/// Format of the file; a file of another format is ignored.
const VERSION: u32 = 1;

/// Largest file written or read, in bytes.
const LARGEST_FILE: u64 = 4 * 1024 * 1024;

/// How long answers gather before the file is written.
const SAVE_DELAY: Duration = Duration::from_secs(1);

const FILE_NAME: &str = "pages.json";

/// What a kept answer answers.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum PageKey {
    Resume(u32),
    NextUp(u32),
    Libraries,
    Latest(ItemId, u32),
    Browse(Listing),
    Genres(BrowseSource),
    Movie(ItemId),
    Series(ItemId),
    Episodes(ItemId, Option<ItemId>),
    Similar(ItemId, u32),
    Person(ItemId),
    PersonTitles(ItemId),
    Collection(ItemId),
}

impl PageKey {
    /// Whether the next launch may show the answer.
    fn on_disk(&self) -> bool {
        match self {
            Self::Resume(_)
            | Self::NextUp(_)
            | Self::Libraries
            | Self::Latest(..)
            | Self::Genres(_) => true,
            // The filters and the sort of a library start over at every launch.
            Self::Browse(listing) => listing.is_default(),
            _ => false,
        }
    }
}

/// A listing, whatever the page asked for.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct Listing {
    source: BrowseSource,
    sort: SortField,
    descending: bool,
    genre: Option<String>,
    watch: WatchFilter,
}

impl Listing {
    pub(crate) fn of(query: &BrowseQuery) -> Self {
        Self {
            source: query.source.clone(),
            sort: query.sort,
            descending: query.descending,
            genre: query.genre.clone(),
            watch: query.watch,
        }
    }

    fn is_default(&self) -> bool {
        self.sort == SortField::DateAdded
            && self.descending
            && self.genre.is_none()
            && self.watch == WatchFilter::All
    }
}

/// A kept answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum PageValue {
    Items(Vec<ItemSummary>),
    Libraries(Vec<Library>),
    Page(Page<ItemSummary>),
    Names(Vec<String>),
    #[serde(skip)]
    Movie(Box<MovieDetails>),
    #[serde(skip)]
    Series(Box<SeriesDetails>),
    #[serde(skip)]
    Episodes(Vec<Episode>),
    #[serde(skip)]
    Person(Box<PersonDetails>),
    #[serde(skip)]
    Titles(Vec<PersonTitle>),
    #[serde(skip)]
    Collection(Box<CollectionDetails>),
}

/// A type of answer that can be kept.
pub(crate) trait Kept: Clone + Sized {
    fn keep(&self) -> PageValue;
    fn kept(value: &PageValue) -> Option<Self>;
    fn length(&self) -> usize {
        0
    }
}

macro_rules! kept {
    ($type:ty, $variant:ident, list) => {
        impl Kept for $type {
            fn keep(&self) -> PageValue {
                PageValue::$variant(self.clone())
            }
            fn kept(value: &PageValue) -> Option<Self> {
                match value {
                    PageValue::$variant(value) => Some(value.clone()),
                    _ => None,
                }
            }
            fn length(&self) -> usize {
                self.len()
            }
        }
    };
    ($type:ty, $variant:ident) => {
        impl Kept for $type {
            fn keep(&self) -> PageValue {
                PageValue::$variant(Box::new(self.clone()))
            }
            fn kept(value: &PageValue) -> Option<Self> {
                match value {
                    PageValue::$variant(value) => Some((**value).clone()),
                    _ => None,
                }
            }
        }
    };
}

kept!(Vec<ItemSummary>, Items, list);
kept!(Vec<Library>, Libraries, list);
kept!(Vec<String>, Names, list);
kept!(Vec<Episode>, Episodes, list);
kept!(Vec<PersonTitle>, Titles, list);
kept!(MovieDetails, Movie);
kept!(SeriesDetails, Series);
kept!(PersonDetails, Person);
kept!(CollectionDetails, Collection);

impl Kept for Page<ItemSummary> {
    fn keep(&self) -> PageValue {
        let mut page = self.clone();
        page.items.truncate(FIRST_PAGE);
        PageValue::Page(page)
    }

    fn kept(value: &PageValue) -> Option<Self> {
        match value {
            PageValue::Page(page) => Some(page.clone()),
            _ => None,
        }
    }
}

struct Entry {
    value: PageValue,
    used: u64,
    /// When the server answered; `None` for an answer read from the disk.
    answered: Option<Instant>,
}

#[derive(Default)]
struct State {
    entries: HashMap<PageKey, Entry>,
    clock: u64,
    revision: u64,
    closed: bool,
    save_planned: bool,
    /// Grows with each save serialized: an older one is not written after it.
    saves: u64,
    /// What the file holds, not to write the same again.
    written: Vec<u8>,
}

struct Disk {
    file: PathBuf,
    account: String,
}

/// The answers kept for one account.
pub(crate) struct PageCache {
    state: Mutex<State>,
    disk: Option<Disk>,
    /// Held while a file is written, so that saves land in the order they were made.
    writing: Mutex<()>,
}

#[derive(Serialize)]
struct Saved<'a> {
    version: u32,
    account: &'a str,
    answers: Vec<(&'a PageKey, &'a PageValue)>,
}

#[derive(Deserialize)]
struct Head {
    version: u32,
    account: String,
}

#[derive(Deserialize)]
struct Answers {
    answers: Vec<(PageKey, PageValue)>,
}

impl std::fmt::Debug for PageCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageCache").finish_non_exhaustive()
    }
}

impl PageCache {
    /// Answers kept in memory only.
    pub(crate) fn in_memory() -> Self {
        Self {
            state: Mutex::default(),
            disk: None,
            writing: Mutex::default(),
        }
    }

    /// Answers kept for `account`, starting with those its last session left on disk.
    pub(crate) fn on_disk(paths: &Paths, account: &str) -> Self {
        let disk = Disk {
            file: paths.cache_dir.join(FILE_NAME),
            account: account.to_owned(),
        };
        let mut state = State::default();
        match disk.read() {
            Some(answers) => {
                for (key, value) in answers {
                    state.insert(key, value, None);
                }
            }
            None => disk.remove(),
        }
        Self {
            state: Mutex::new(state),
            disk: Some(disk),
            writing: Mutex::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The last answer to `key`.
    pub(crate) fn last<T: Kept>(&self, key: &PageKey) -> Option<T> {
        let mut state = self.lock();
        state.clock += 1;
        let clock = state.clock;
        let entry = state.entries.get_mut(key)?;
        entry.used = clock;
        T::kept(&entry.value)
    }

    /// Whether the server answered `key` within `age`.
    pub(crate) fn is_fresh(&self, key: &PageKey, age: Duration) -> bool {
        self.lock()
            .entries
            .get(key)
            .and_then(|entry| entry.answered)
            .is_some_and(|answered| answered.elapsed() < age)
    }

    /// Grows each time the answers are forgotten: an answer asked for before is not kept.
    pub(crate) fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// Asks with `fetch` and keeps the answer to `key`.
    pub(crate) async fn keep<T: Kept>(
        self: &Arc<Self>,
        key: PageKey,
        fetch: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let asked = self.revision();
        let answer = fetch.await?;
        self.store(asked, key, &answer);
        Ok(answer)
    }

    /// Keeps the answer to `key`, unless what was asked at revision `asked` was forgotten since.
    pub(crate) fn store<T: Kept>(self: &Arc<Self>, asked: u64, key: PageKey, answer: &T) {
        let on_disk = key.on_disk();
        {
            let mut state = self.lock();
            if state.closed || state.revision != asked {
                return;
            }
            if answer.length() > LONGEST_LIST {
                state.entries.remove(&key);
            } else {
                state.insert(key, answer.keep(), Some(Instant::now()));
            }
        }
        if on_disk {
            self.plan_save();
        }
    }

    /// Forgets every answer, in memory and on disk: what the user did made them wrong.
    pub(crate) fn forget(&self) {
        let mut state = self.lock();
        state.revision += 1;
        state.entries.clear();
        state.written.clear();
        // Under the lock: a file being written would come back after its removal.
        if let Some(disk) = &self.disk {
            disk.remove();
        }
    }

    /// Bytes the answers kept for the next launch take on disk.
    pub(crate) fn saved_bytes(&self) -> u64 {
        let Some(disk) = &self.disk else { return 0 };
        let _state = self.lock();
        fs::metadata(&disk.file).map_or(0, |meta| meta.len())
    }

    /// Forgets every answer and keeps none from now on: the account signed out.
    pub(crate) fn close(&self) {
        self.lock().closed = true;
        self.forget();
    }

    fn plan_save(self: &Arc<Self>) {
        if self.disk.is_none() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if std::mem::replace(&mut self.lock().save_planned, true) {
            return;
        }
        let this = Arc::clone(self);
        runtime.spawn(async move {
            tokio::time::sleep(SAVE_DELAY).await;
            let _ = tokio::task::spawn_blocking(move || this.save()).await;
        });
    }

    /// Writes the answers the next launch may show: serialized under the lock, written outside
    /// it, and put in place only if nothing was forgotten or saved since.
    pub(crate) fn save(&self) {
        let Some(disk) = &self.disk else { return };
        let (bytes, revision, number) = {
            let mut state = self.lock();
            state.save_planned = false;
            if state.closed || state.entries.is_empty() {
                return;
            }
            let mut answers: Vec<(&PageKey, &PageValue)> = state
                .entries
                .iter()
                .filter(|(key, _)| key.on_disk())
                .map(|(key, entry)| (key, &entry.value))
                .collect();
            answers.sort_by_cached_key(|(key, _)| format!("{key:?}"));
            let saved = Saved {
                version: VERSION,
                account: &disk.account,
                answers,
            };
            let bytes = match serde_json::to_vec(&saved) {
                Ok(bytes) => bytes,
                Err(error) => return log::warn!("cannot save the last pages: {error}"),
            };
            if bytes == state.written {
                return;
            }
            state.saves += 1;
            if bytes.len() as u64 > LARGEST_FILE {
                state.written.clear();
                return disk.remove();
            }
            (bytes, state.revision, state.saves)
        };
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        if self.lock().saves != number {
            return;
        }
        let tmp = match disk.write_tmp(&bytes) {
            Ok(tmp) => tmp,
            Err(error) => return log::warn!("cannot save the last pages: {error}"),
        };
        let mut state = self.lock();
        if state.closed || state.revision != revision || state.saves != number {
            let _ = fs::remove_file(&tmp);
            return;
        }
        match fs::rename(&tmp, &disk.file) {
            Ok(()) => state.written = bytes,
            Err(error) => {
                log::warn!("cannot save the last pages: {error}");
                let _ = fs::remove_file(&tmp);
            }
        }
    }
}

impl State {
    fn insert(&mut self, key: PageKey, value: PageValue, answered: Option<Instant>) {
        self.clock += 1;
        let entry = Entry {
            value,
            used: self.clock,
            answered,
        };
        let on_disk = key.on_disk();
        self.entries.insert(key, entry);
        let budget = if on_disk {
            KEPT_FOR_LAUNCH
        } else {
            KEPT_ANSWERS
        };
        let alike = |key: &PageKey| key.on_disk() == on_disk;
        while self.entries.keys().filter(|key| alike(key)).count() > budget {
            let oldest = self
                .entries
                .iter()
                .filter(|(key, _)| alike(key))
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(key) => self.entries.remove(&key),
                None => break,
            };
        }
    }
}

impl Disk {
    fn read(&self) -> Option<Vec<(PageKey, PageValue)>> {
        if fs::metadata(&self.file).ok()?.len() > LARGEST_FILE {
            return None;
        }
        let bytes = fs::read(&self.file).ok()?;
        let head: Head = serde_json::from_slice(&bytes).ok()?;
        if head.version != VERSION || head.account != self.account {
            return None;
        }
        let saved: Answers = serde_json::from_slice(&bytes).ok()?;
        Some(saved.answers)
    }

    /// Writes `bytes` to a file of this process beside the saved one, and returns its path.
    fn write_tmp(&self, bytes: &[u8]) -> std::io::Result<PathBuf> {
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.tmp();
        let _ = fs::remove_file(&tmp);
        let mut file = private(&tmp)?;
        file.write_all(bytes)?;
        Ok(tmp)
    }

    fn tmp(&self) -> PathBuf {
        self.file
            .with_extension(format!("json.{}.tmp", std::process::id()))
    }

    /// Removes the saved file and the files of every process writing it, even one that ended
    /// before it could rename its own.
    fn remove(&self) {
        let mut files = vec![self.file.clone()];
        let name = self.file.file_name().and_then(|name| name.to_str());
        if let (Some(dir), Some(name)) = (self.file.parent(), name)
            && let Ok(entries) = fs::read_dir(dir)
        {
            let prefix = format!("{name}.");
            files.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".tmp"))
            }));
        }
        for file in &files {
            match fs::remove_file(file) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    log::warn!("cannot remove the last pages: {error}");
                }
                _ => {}
            }
        }
    }
}

/// Removes the answers kept for the next launch under `paths`: no account starts from them.
pub(crate) fn remove_saved(paths: &Paths) {
    Disk {
        file: paths.cache_dir.join(FILE_NAME),
        account: String::new(),
    }
    .remove();
}

/// Creates a file only the user can read: titles tell what they watch.
fn private(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const ACCOUNT: &str = "user@https://server.example/";

    fn names(text: &str) -> Vec<String> {
        vec![text.to_owned()]
    }

    fn genres() -> PageKey {
        PageKey::Genres(BrowseSource::Favorites)
    }

    fn movie(id: &str) -> PageKey {
        PageKey::Movie(id.into())
    }

    fn on_disk(dir: &TempDir, account: &str) -> Arc<PageCache> {
        Arc::new(PageCache::on_disk(&Paths::under(dir.path()), account))
    }

    fn file(dir: &TempDir) -> PathBuf {
        Paths::under(dir.path()).cache_dir.join(FILE_NAME)
    }

    fn last(cache: &PageCache) -> Option<Vec<String>> {
        cache.last(&genres())
    }

    #[test]
    fn an_answer_asked_for_before_a_change_is_not_kept() {
        let cache = Arc::new(PageCache::in_memory());
        let asked = cache.revision();
        cache.store(asked, genres(), &names("before"));
        assert_eq!(last(&cache), Some(names("before")));
        cache.forget();
        assert_eq!(last(&cache), None);
        cache.store(asked, genres(), &names("late"));
        assert_eq!(last(&cache), None);
        cache.store(cache.revision(), genres(), &names("after"));
        assert_eq!(last(&cache), Some(names("after")));
    }

    #[test]
    fn the_least_recently_used_answers_go_first() {
        let cache = Arc::new(PageCache::in_memory());
        let keep = |index: usize| {
            let key = PageKey::Episodes(format!("{index}").as_str().into(), None);
            cache.store(cache.revision(), key, &Vec::<Episode>::new());
        };
        let kept = |index: usize| {
            let key = PageKey::Episodes(format!("{index}").as_str().into(), None);
            cache.last::<Vec<Episode>>(&key).is_some()
        };
        for index in 0..KEPT_ANSWERS {
            keep(index);
        }
        assert!(kept(0));
        keep(KEPT_ANSWERS);
        assert!(kept(0));
        assert!(!kept(1));
        assert_eq!(cache.lock().entries.len(), KEPT_ANSWERS);
    }

    #[test]
    fn the_pages_of_the_next_launch_outlive_the_others() {
        let cache = Arc::new(PageCache::in_memory());
        cache.store(cache.revision(), genres(), &names("home"));
        for index in 0..=KEPT_ANSWERS {
            let key = PageKey::Episodes(format!("{index}").as_str().into(), None);
            cache.store(cache.revision(), key, &Vec::<Episode>::new());
        }
        assert_eq!(last(&cache), Some(names("home")));
        let state = cache.lock();
        let others = state.entries.keys().filter(|key| !key.on_disk()).count();
        assert_eq!(others, KEPT_ANSWERS);
    }

    #[test]
    fn a_list_too_long_is_not_kept() {
        let cache = Arc::new(PageCache::in_memory());
        cache.store(cache.revision(), genres(), &names("a"));
        let long = vec![String::new(); LONGEST_LIST + 1];
        cache.store(cache.revision(), genres(), &long);
        assert_eq!(last(&cache), None);
    }

    #[tokio::test(start_paused = true)]
    async fn an_answer_is_fresh_for_a_while() {
        let cache = Arc::new(PageCache::in_memory());
        let age = Duration::from_secs(60);
        assert!(!cache.is_fresh(&genres(), age));
        cache.store(cache.revision(), genres(), &names("a"));
        assert!(cache.is_fresh(&genres(), age));
        tokio::time::advance(age).await;
        assert!(!cache.is_fresh(&genres(), age));
    }

    #[test]
    fn the_next_launch_of_the_same_account_shows_the_saved_answers() {
        let dir = TempDir::new().unwrap();
        let cache = on_disk(&dir, ACCOUNT);
        cache.store(cache.revision(), genres(), &names("Drama"));
        cache.store(cache.revision(), movie("m"), &names("not on disk"));
        cache.save();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(file(&dir)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let next = on_disk(&dir, ACCOUNT);
        assert_eq!(last(&next), Some(names("Drama")));
        assert_eq!(next.lock().entries.len(), 1);
        assert!(!next.is_fresh(&genres(), Duration::from_secs(60)));
    }

    #[test]
    fn another_account_sees_nothing_and_removes_the_file() {
        let dir = TempDir::new().unwrap();
        let cache = on_disk(&dir, ACCOUNT);
        cache.store(cache.revision(), genres(), &names("Drama"));
        cache.save();
        assert_eq!(last(&on_disk(&dir, "other@https://server.example/")), None);
        assert!(!file(&dir).exists());
    }

    #[test]
    fn another_format_and_unreadable_content_are_ignored() {
        let dir = TempDir::new().unwrap();
        let cache = on_disk(&dir, ACCOUNT);
        cache.store(cache.revision(), genres(), &names("Drama"));
        cache.save();
        let good = fs::read_to_string(file(&dir)).unwrap();
        let newer = good.replace("\"version\":1", "\"version\":2");
        assert_ne!(good, newer);
        for text in [newer.as_str(), &good[..good.len() / 2], "", "[]"] {
            fs::write(file(&dir), text).unwrap();
            assert_eq!(last(&on_disk(&dir, ACCOUNT)), None, "{text}");
            assert!(!file(&dir).exists());
        }
        fs::write(file(&dir), good).unwrap();
        assert_eq!(last(&on_disk(&dir, ACCOUNT)), Some(names("Drama")));
    }

    #[test]
    fn a_change_and_a_sign_out_remove_the_file() {
        let dir = TempDir::new().unwrap();
        let cache = on_disk(&dir, ACCOUNT);
        let save = |text: &str| {
            cache.store(cache.revision(), genres(), &names(text));
            cache.save();
        };
        save("Drama");
        cache.forget();
        assert!(!file(&dir).exists());
        cache.save();
        assert!(!file(&dir).exists(), "nothing is left to write");
        save("Drama");
        assert!(file(&dir).exists(), "the same answer is written again");
        cache.close();
        save("late");
        assert!(!file(&dir).exists());
        assert_eq!(last(&cache), None);
    }

    #[tokio::test(start_paused = true)]
    async fn answers_are_saved_together_after_a_while() {
        let dir = TempDir::new().unwrap();
        let cache = on_disk(&dir, ACCOUNT);
        cache.store(cache.revision(), genres(), &names("Drama"));
        cache.store(cache.revision(), PageKey::Libraries, &Vec::<Library>::new());
        assert!(!file(&dir).exists());
        tokio::time::sleep(SAVE_DELAY * 2).await;
        while !file(&dir).exists() {
            tokio::task::yield_now().await;
        }
        assert_eq!(last(&on_disk(&dir, ACCOUNT)), Some(names("Drama")));
    }

    /// Starts a save that waits for the file, and returns once its answers are serialized.
    fn save_held(cache: &Arc<PageCache>) -> std::thread::JoinHandle<()> {
        let serialized = cache.lock().saves + 1;
        let saving = Arc::clone(cache);
        let handle = std::thread::spawn(move || saving.save());
        while cache.lock().saves < serialized {
            std::thread::yield_now();
        }
        handle
    }

    #[test]
    fn a_save_writes_outside_the_lock_and_never_brings_back_a_forgotten_file() {
        let dir = TempDir::new().unwrap();
        let cache = on_disk(&dir, ACCOUNT);
        cache.store(cache.revision(), genres(), &names("Drama"));
        let writing = cache.writing.lock().unwrap();
        let saving = save_held(&cache);
        assert_eq!(
            last(&cache),
            Some(names("Drama")),
            "answers are read meanwhile"
        );
        cache.forget();
        drop(writing);
        saving.join().unwrap();
        assert!(!file(&dir).exists());
        let dir_left = file(&dir).parent().unwrap().to_owned();
        assert!(
            fs::read_dir(dir_left).unwrap().next().is_none(),
            "no file is left"
        );
    }

    #[test]
    fn an_older_save_is_not_written_after_a_newer_one() {
        let dir = TempDir::new().unwrap();
        let cache = on_disk(&dir, ACCOUNT);
        cache.store(cache.revision(), genres(), &names("old"));
        let writing = cache.writing.lock().unwrap();
        let older = save_held(&cache);
        cache.store(cache.revision(), genres(), &names("new"));
        let newer = save_held(&cache);
        drop(writing);
        older.join().unwrap();
        newer.join().unwrap();
        assert_eq!(last(&on_disk(&dir, ACCOUNT)), Some(names("new")));
    }

    #[test]
    fn removing_the_saved_pages_sweeps_the_files_of_every_process() {
        let dir = TempDir::new().unwrap();
        let cache_dir = file(&dir).parent().unwrap().to_owned();
        fs::create_dir_all(&cache_dir).unwrap();
        let swept = ["pages.json", "pages.json.1.tmp", "pages.json.424242.tmp"];
        let kept = ["other.json.1.tmp", "pages.jsonl"];
        for name in swept.iter().chain(&kept) {
            fs::write(cache_dir.join(name), b"{}").unwrap();
        }
        remove_saved(&Paths::under(dir.path()));
        for name in swept {
            assert!(!cache_dir.join(name).exists(), "{name} is left");
        }
        for name in kept {
            assert!(cache_dir.join(name).exists(), "{name} is removed");
        }
    }
}
