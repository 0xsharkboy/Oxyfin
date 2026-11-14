//! Image cache: an in-memory LRU bounded in bytes, backed by a disk cache.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

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

/// Fetches item images, keeping them in memory and on disk.
///
/// Returns the raw encoded bytes; decoding is left to the UI.
pub struct ImageCache {
    session: Session,
    dir: PathBuf,
    budget: usize,
    memory: Mutex<Memory>,
}

impl ImageCache {
    /// A cache storing files under `paths.cache_dir` and keeping at most
    /// `memory_budget_bytes` of images in memory.
    pub fn new(session: Session, paths: &Paths, memory_budget_bytes: usize) -> Self {
        Self {
            session,
            dir: paths.cache_dir.join("images"),
            budget: memory_budget_bytes,
            memory: Mutex::new(Memory::default()),
        }
    }

    /// Returns the bytes of `image` scaled to fit `size`, from memory, disk or the server.
    pub async fn get(&self, image: &ImageRef, size: ImageSize) -> Result<Arc<[u8]>> {
        let name = file_name(image, size);
        if let Some(bytes) = self.lock().get(&name) {
            return Ok(bytes);
        }
        let path = self.dir.join(&name);
        let bytes: Arc<[u8]> = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes.into(),
            Err(_) => {
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
        let _ = self.try_write_disk(name, bytes).await;
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
