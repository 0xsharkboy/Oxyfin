//! Image cache: an in-memory LRU bounded in bytes, backed by a bounded disk cache.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use oxyfin_api::{ImageRequest, Session};

use crate::{ImageRef, Paths, Result};

/// Bounding box requested for an image, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageSize {
    /// Maximum width.
    pub max_width: u32,
    /// Maximum height.
    pub max_height: u32,
}

/// Default memory budget of the image cache: 64 MB.
pub const DEFAULT_MEMORY_BUDGET: u64 = 64 * 1024 * 1024;

/// Default disk budget of the image cache: 500 MB.
pub const DEFAULT_DISK_BUDGET: u64 = 500 * 1024 * 1024;

/// Byte budgets of an [`ImageCache`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheBudgets {
    /// Images kept in memory, in bytes (see [`DEFAULT_MEMORY_BUDGET`]).
    pub memory_bytes: u64,
    /// Images kept on disk, in bytes (see [`DEFAULT_DISK_BUDGET`]).
    pub disk_bytes: u64,
}

impl Default for CacheBudgets {
    fn default() -> Self {
        Self {
            memory_bytes: DEFAULT_MEMORY_BUDGET,
            disk_bytes: DEFAULT_DISK_BUDGET,
        }
    }
}

/// Temporary files younger than this may belong to a write in progress.
const STALE_TMP_AGE: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Memory {
    entries: HashMap<String, Arc<[u8]>>,
    order: VecDeque<String>,
    used: usize,
}

impl Memory {
    fn get(&mut self, key: &str) -> Option<Arc<[u8]>> {
        let bytes = self.entries.get(key)?.clone();
        self.order.retain(|other| other != key);
        self.order.push_back(key.to_owned());
        Some(bytes)
    }

    fn insert(&mut self, key: String, bytes: Arc<[u8]>, budget: usize) {
        if let Some(old) = self.entries.remove(&key) {
            self.used -= old.len();
            self.order.retain(|other| *other != key);
        }
        if bytes.len() > budget {
            return;
        }
        while self.used + bytes.len() > budget {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.used -= evicted.len();
            }
        }
        self.used += bytes.len();
        self.order.push_back(key.clone());
        self.entries.insert(key, bytes);
    }
}

struct Trimmer {
    dir: PathBuf,
    budget: u64,
    running: AtomicBool,
    pending: AtomicBool,
    requests: AtomicU64,
    /// Bytes on disk: measured by each pass, raised by each write in between.
    total: AtomicU64,
    /// A pass has completed.
    ran: AtomicBool,
    pass: tokio::sync::Mutex<()>,
}

/// Clears [`Trimmer::running`] when a trim task is cancelled.
struct Running {
    trimmer: Arc<Trimmer>,
    armed: bool,
}

impl Running {
    /// The task ended normally: `running` may already belong to another task.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.armed {
            self.trimmer.running.store(false, Ordering::SeqCst);
        }
    }
}

impl Trimmer {
    fn request(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.pending.store(true, Ordering::SeqCst);
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        // Built outside the task so that a task dropped before its first poll resets the flag.
        let mut running = Running {
            trimmer: self.clone(),
            armed: true,
        };
        handle.spawn(async move {
            let this = running.trimmer.clone();
            loop {
                while this.pending.swap(false, Ordering::SeqCst) {
                    this.run().await;
                }
                this.running.store(false, Ordering::SeqCst);
                if !this.pending.load(Ordering::SeqCst) || this.running.swap(true, Ordering::SeqCst)
                {
                    break;
                }
            }
            running.disarm();
        });
    }

    /// Requests the first pass when none has run or is running, as when the
    /// cache was built outside a runtime.
    fn ensure_ran(self: &Arc<Self>) {
        if !self.ran.load(Ordering::SeqCst) && !self.running.load(Ordering::SeqCst) {
            self.request();
        }
    }

    /// Counts a write of `len` bytes; requests a trim once over the budget.
    fn wrote(self: &Arc<Self>, len: u64) {
        let total = self
            .total
            .fetch_add(len, Ordering::SeqCst)
            .saturating_add(len);
        if total > self.budget {
            self.request();
        }
    }

    async fn run(&self) {
        let _guard = self.pass.lock().await;
        let _ = self.trim().await;
        self.ran.store(true, Ordering::SeqCst);
    }

    async fn trim(&self) -> std::io::Result<()> {
        let mut entries = match tokio::fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.total.store(0, Ordering::SeqCst);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let now = SystemTime::now();
        let mut files = Vec::new();
        let mut total = 0u64;
        while let Some(entry) = entries.next_entry().await? {
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') && name.ends_with(".tmp") {
                let age = now.duration_since(modified).unwrap_or_default();
                if age >= STALE_TMP_AGE {
                    let _ = tokio::fs::remove_file(entry.path()).await;
                }
                continue;
            }
            total += meta.len();
            files.push((modified, entry.path(), meta.len()));
        }
        if total > self.budget {
            let low_water = self.budget - self.budget / 10;
            files.sort();
            for (_, path, len) in files {
                if total <= low_water {
                    break;
                }
                if tokio::fs::remove_file(&path).await.is_ok() {
                    total -= len;
                }
            }
        }
        self.total.store(total, Ordering::SeqCst);
        Ok(())
    }
}

/// Fetches item images, keeping them in memory and on disk.
///
/// Returns the raw encoded bytes; decoding is left to the UI.
///
/// The disk part is trimmed in a background task, so `get` never waits for it.
/// A pass runs at construction and measures the disk usage; writes add to that
/// measure, and a write that takes it over the budget requests a pass, which
/// trims down to 90 % of the budget. At most one trim runs at a time and a
/// request made while one runs schedules one more pass. Files go oldest first
/// by modification time, which is the write order: reading a file does not
/// refresh it. Orphaned `.*.tmp` files older than a minute are removed by the
/// same pass.
pub struct ImageCache {
    session: Session,
    dir: PathBuf,
    budget: usize,
    memory: Mutex<Memory>,
    trimmer: Arc<Trimmer>,
}

impl ImageCache {
    /// A cache storing files under `paths.cache_dir`, within `budgets`.
    ///
    /// Starts a background trim of the disk cache when called inside a Tokio
    /// runtime, and otherwise on the first [`ImageCache::get`].
    pub fn new(session: Session, paths: &Paths, budgets: CacheBudgets) -> Self {
        let dir = paths.cache_dir.join("images");
        let trimmer = Arc::new(Trimmer {
            dir: dir.clone(),
            budget: budgets.disk_bytes,
            running: AtomicBool::new(false),
            pending: AtomicBool::new(false),
            requests: AtomicU64::new(0),
            total: AtomicU64::new(0),
            ran: AtomicBool::new(false),
            pass: tokio::sync::Mutex::new(()),
        });
        trimmer.request();
        Self {
            session,
            dir,
            budget: usize::try_from(budgets.memory_bytes).unwrap_or(usize::MAX),
            memory: Mutex::new(Memory::default()),
            trimmer,
        }
    }

    /// Runs a trim pass and returns when it is done, for deterministic tests.
    #[doc(hidden)]
    pub async fn trim(&self) {
        self.trimmer.run().await;
    }

    /// Number of background trims requested so far, for deterministic tests.
    #[doc(hidden)]
    pub fn trim_requests(&self) -> u64 {
        self.trimmer.requests.load(Ordering::SeqCst)
    }

    /// Returns the bytes of `image` scaled to fit `size`, from memory, disk or the server.
    pub async fn get(&self, image: &ImageRef, size: ImageSize) -> Result<Arc<[u8]>> {
        self.trimmer.ensure_ran();
        let name = file_name(image, size);
        if let Some(bytes) = self.lock().get(&name) {
            return Ok(bytes);
        }
        let path = self.dir.join(&name);
        let bytes: Arc<[u8]> = match tokio::fs::read(&path).await {
            // An empty file is a write cut short, by a power loss for instance.
            Ok(bytes) if !bytes.is_empty() => bytes.into(),
            _ => {
                let mut request =
                    ImageRequest::new(image.item_id.clone(), image.image_type.clone());
                request.tag = Some(image.tag.clone());
                request.max_width = Some(size.max_width);
                request.max_height = Some(size.max_height);
                let bytes: Arc<[u8]> = self.session.fetch_image(&request).await?.into();
                self.write_disk(&name, &bytes).await;
                bytes
            }
        };
        self.lock().insert(name, bytes.clone(), self.budget);
        Ok(bytes)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn write_disk(&self, name: &str, bytes: &[u8]) {
        if self.try_write_disk(name, bytes).await.is_ok() {
            self.trimmer.wrote(bytes.len() as u64);
        }
    }

    async fn try_write_disk(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        let tmp = self
            .dir
            .join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
        let result = async {
            tokio::fs::write(&tmp, bytes).await?;
            tokio::fs::rename(&tmp, self.dir.join(name)).await
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        result
    }
}

fn file_name(image: &ImageRef, size: ImageSize) -> String {
    format!(
        "{}-{}-{}-{}x{}",
        sanitize(&image.item_id.to_string()),
        image.image_type.as_str(),
        sanitize(&image.tag),
        size.max_width,
        size.max_height
    )
}

fn sanitize(part: &str) -> String {
    part.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
