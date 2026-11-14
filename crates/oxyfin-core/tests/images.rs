//! Integration tests for the image cache.

mod common;

use common::session;
use oxyfin_api::{ImageType, ItemId};
use oxyfin_core::{
    CacheBudgets, DEFAULT_DISK_BUDGET, DEFAULT_MEMORY_BUDGET, ImageCache, ImageRef, ImageSize,
    Paths,
};
use tempfile::TempDir;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SIZE: ImageSize = ImageSize {
    max_width: 300,
    max_height: 450,
};

fn image(item: &str, tag: &str) -> ImageRef {
    ImageRef {
        item_id: ItemId::from(item),
        image_type: ImageType::Primary,
        tag: tag.to_owned(),
    }
}

async fn serve(mock: &MockServer, item: &str, body: &[u8]) {
    Mock::given(method("GET"))
        .and(path(format!("/Items/{item}/Images/Primary")))
        .and(query_param("maxWidth", "300"))
        .and(query_param("maxHeight", "450"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
        .mount(mock)
        .await;
}

async fn requests(mock: &MockServer) -> usize {
    mock.received_requests().await.map_or(0, |all| all.len())
}

fn cache(mock: &MockServer, dir: &TempDir, memory_bytes: u64) -> ImageCache {
    with_budgets(mock, dir, memory_bytes, u64::MAX)
}

fn with_budgets(
    mock: &MockServer,
    dir: &TempDir,
    memory_bytes: u64,
    disk_bytes: u64,
) -> ImageCache {
    let budgets = CacheBudgets {
        memory_bytes,
        disk_bytes,
    };
    ImageCache::new(session(mock), &Paths::under(dir.path()), budgets)
}

#[test]
fn default_budgets() {
    let budgets = CacheBudgets::default();
    assert_eq!(budgets.memory_bytes, DEFAULT_MEMORY_BUDGET);
    assert_eq!(DEFAULT_MEMORY_BUDGET, 64 * 1024 * 1024);
    assert_eq!(budgets.disk_bytes, DEFAULT_DISK_BUDGET);
}

#[tokio::test]
async fn second_get_hits_memory() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"pixels").await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 1024);
    let first = cache.get(&image("aaa", "t1"), SIZE).await.unwrap();
    let second = cache.get(&image("aaa", "t1"), SIZE).await.unwrap();
    assert_eq!(&*first, b"pixels");
    assert_eq!(first, second);
    assert_eq!(requests(&mock).await, 1);
}

#[tokio::test]
async fn new_cache_reads_disk() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"pixels").await;
    let dir = TempDir::new().unwrap();
    cache(&mock, &dir, 1024)
        .get(&image("aaa", "t1"), SIZE)
        .await
        .unwrap();
    let bytes = cache(&mock, &dir, 1024)
        .get(&image("aaa", "t1"), SIZE)
        .await
        .unwrap();
    assert_eq!(&*bytes, b"pixels");
    assert_eq!(requests(&mock).await, 1);
}

#[tokio::test]
async fn empty_disk_file_is_refetched() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"pixels").await;
    let dir = TempDir::new().unwrap();
    cache(&mock, &dir, 1024)
        .get(&image("aaa", "t1"), SIZE)
        .await
        .unwrap();
    let images = Paths::under(dir.path()).cache_dir.join("images");
    let files: Vec<_> = std::fs::read_dir(&images)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    std::fs::write(&files[0], b"").unwrap();
    let bytes = cache(&mock, &dir, 1024)
        .get(&image("aaa", "t1"), SIZE)
        .await
        .unwrap();
    assert_eq!(&*bytes, b"pixels");
    assert_eq!(requests(&mock).await, 2);
    assert_eq!(std::fs::read(&files[0]).unwrap(), b"pixels");
}

#[tokio::test]
async fn budget_evicts_oldest() {
    let mock = MockServer::start().await;
    for item in ["aaa", "bbb", "ccc"] {
        serve(&mock, item, b"123456").await;
    }
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 10);
    for item in ["aaa", "bbb", "ccc"] {
        cache.get(&image(item, "t"), SIZE).await.unwrap();
    }
    assert_eq!(requests(&mock).await, 3);
    std::fs::remove_dir_all(Paths::under(dir.path()).cache_dir).unwrap();
    cache.get(&image("ccc", "t"), SIZE).await.unwrap();
    assert_eq!(requests(&mock).await, 3);
    cache.get(&image("aaa", "t"), SIZE).await.unwrap();
    assert_eq!(requests(&mock).await, 4);
}

#[tokio::test]
async fn oversized_entry_is_returned_but_not_kept() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"0123456789").await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 4);
    let bytes = cache.get(&image("aaa", "t"), SIZE).await.unwrap();
    assert_eq!(bytes.len(), 10);
    std::fs::remove_dir_all(Paths::under(dir.path()).cache_dir).unwrap();
    cache.get(&image("aaa", "t"), SIZE).await.unwrap();
    assert_eq!(requests(&mock).await, 2);
}

#[tokio::test]
async fn different_tag_refetches() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"pixels").await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 1024);
    cache.get(&image("aaa", "t1"), SIZE).await.unwrap();
    cache.get(&image("aaa", "t2"), SIZE).await.unwrap();
    assert_eq!(requests(&mock).await, 2);
}

#[tokio::test]
async fn crafted_tag_stays_inside_the_cache_directory() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"pixels").await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 1024);
    cache
        .get(&image("aaa", "../../escape"), SIZE)
        .await
        .unwrap();
    assert!(!dir.path().join("escape").exists());
    assert!(!dir.path().join("cache").join("escape").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn read_only_disk_still_returns_bytes() {
    use std::os::unix::fs::PermissionsExt;

    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"pixels").await;
    let dir = TempDir::new().unwrap();
    let cache_dir = Paths::under(dir.path()).cache_dir;
    std::fs::create_dir_all(cache_dir.join("images")).unwrap();
    std::fs::set_permissions(
        cache_dir.join("images"),
        std::fs::Permissions::from_mode(0o500),
    )
    .unwrap();
    let cache = cache(&mock, &dir, 1024);
    let bytes = cache.get(&image("aaa", "t1"), SIZE).await.unwrap();
    std::fs::set_permissions(
        cache_dir.join("images"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    assert_eq!(&*bytes, b"pixels");
}

#[allow(clippy::unwrap_used)]
fn files_in(dir: &TempDir) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let images = Paths::under(dir.path()).cache_dir.join("images");
    std::fs::read_dir(images)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

#[allow(clippy::unwrap_used)]
fn age(path: &std::path::Path, secs: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[tokio::test]
async fn disk_cache_is_trimmed_to_its_budget() {
    let mock = MockServer::start().await;
    let bodies: [(&str, &[u8; 10]); 4] = [
        ("aaa", b"aaaaaaaaaa"),
        ("bbb", b"bbbbbbbbbb"),
        ("ccc", b"cccccccccc"),
        ("ddd", b"dddddddddd"),
    ];
    for (item, body) in bodies {
        serve(&mock, item, body).await;
    }
    let dir = TempDir::new().unwrap();
    let cache = with_budgets(&mock, &dir, 0, 35);
    for (item, _) in &bodies[..3] {
        cache.get(&image(item, "t"), SIZE).await.unwrap();
    }
    cache.trim().await;
    for (path, bytes) in files_in(&dir) {
        let rank = bodies.iter().position(|(_, b)| b[..] == bytes[..]).unwrap();
        age(&path, 1000 - rank as u64 * 100);
    }
    cache.get(&image("ddd", "t"), SIZE).await.unwrap();
    cache.trim().await;
    let mut left: Vec<_> = files_in(&dir).into_iter().map(|(_, b)| b).collect();
    left.sort();
    assert_eq!(
        left,
        vec![
            b"bbbbbbbbbb".to_vec(),
            b"cccccccccc".to_vec(),
            b"dddddddddd".to_vec()
        ]
    );
}

#[tokio::test]
async fn stale_tmp_files_are_removed() {
    let mock = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let images = Paths::under(dir.path()).cache_dir.join("images");
    std::fs::create_dir_all(&images).unwrap();
    let stale = images.join(".x.tmp");
    let fresh = images.join(".y.tmp");
    std::fs::write(&stale, b"partial").unwrap();
    std::fs::write(&fresh, b"partial").unwrap();
    age(&stale, 3600);
    let cache = cache(&mock, &dir, 1024);
    cache.trim().await;
    assert!(!stale.exists());
    assert!(fresh.exists());
}

#[allow(clippy::unwrap_used)]
fn disk_bytes(dir: &TempDir) -> u64 {
    files_in(dir)
        .iter()
        .map(|(_, bytes)| bytes.len() as u64)
        .sum()
}

#[tokio::test]
async fn writes_under_the_disk_budget_request_no_trim() {
    let mock = MockServer::start().await;
    for item in ["aaa", "bbb", "ccc"] {
        serve(&mock, item, b"0123456789").await;
    }
    let dir = TempDir::new().unwrap();
    let cache = with_budgets(&mock, &dir, 0, 100);
    cache.trim().await;
    let requests = cache.trim_requests();
    for item in ["aaa", "bbb", "ccc"] {
        cache.get(&image(item, "t"), SIZE).await.unwrap();
    }
    assert_eq!(cache.trim_requests(), requests);
}

#[tokio::test]
async fn going_over_the_disk_budget_trims_to_ninety_percent() {
    let mock = MockServer::start().await;
    let items: Vec<String> = (0..11).map(|n| format!("i{n:02}")).collect();
    for item in &items {
        serve(&mock, item, b"0123456789").await;
    }
    let dir = TempDir::new().unwrap();
    let cache = with_budgets(&mock, &dir, 0, 100);
    cache.trim().await;
    let requests = cache.trim_requests();
    for item in &items[..10] {
        cache.get(&image(item, "t"), SIZE).await.unwrap();
    }
    assert_eq!(cache.trim_requests(), requests);
    cache.get(&image(&items[10], "t"), SIZE).await.unwrap();
    assert_eq!(cache.trim_requests(), requests + 1);
    let start = std::time::Instant::now();
    while disk_bytes(&dir) > 90 {
        assert!(
            start.elapsed() < common::DEADLINE,
            "{} bytes",
            disk_bytes(&dir)
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Writes a `.x.tmp` file an hour old in the image directory; returns its path.
#[allow(clippy::unwrap_used)]
fn stale_tmp(dir: &TempDir) -> std::path::PathBuf {
    let images = Paths::under(dir.path()).cache_dir.join("images");
    std::fs::create_dir_all(&images).unwrap();
    let stale = images.join(".x.tmp");
    std::fs::write(&stale, b"partial").unwrap();
    age(&stale, 3600);
    stale
}

async fn wait_removed(path: &std::path::Path) {
    let start = std::time::Instant::now();
    while path.exists() {
        assert!(start.elapsed() < common::DEADLINE, "{path:?} still there");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

#[test]
fn cache_built_outside_a_runtime_trims_on_first_get() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mock = runtime.block_on(async {
        let mock = MockServer::start().await;
        serve(&mock, "aaa", b"pixels").await;
        mock
    });
    let dir = TempDir::new().unwrap();
    let stale = stale_tmp(&dir);
    let cache = cache(&mock, &dir, 1024);
    runtime.block_on(async {
        cache.get(&image("aaa", "t"), SIZE).await.unwrap();
        wait_removed(&stale).await;
    });
}
