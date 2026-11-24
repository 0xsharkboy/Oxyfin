//! Integration tests for the image cache.

mod common;

use common::session;
use oxyfin_api::{ImageType, ItemId, MediaSourceId};
use oxyfin_core::{
    ARTWORK_FETCHES, CacheBudgets, DEFAULT_DISK_BUDGET, DEFAULT_MEMORY_BUDGET, IMAGE_FETCHES,
    ImageCache, ImagePriority, ImageRef, ImageSize, Paths, TrickplaySheet,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const PAGE: ImagePriority = ImagePriority::of_page(0);

const SIZE: ImageSize = ImageSize {
    max_width: 300,
    max_height: 450,
};

fn image(item: &str, tag: &str) -> ImageRef {
    ImageRef {
        item_id: ItemId::from(item),
        image_type: ImageType::Primary,
        tag: tag.to_owned(),
        index: None,
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
    let first = cache.get(&image("aaa", "t1"), SIZE, PAGE).await.unwrap();
    let second = cache.get(&image("aaa", "t1"), SIZE, PAGE).await.unwrap();
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
        .get(&image("aaa", "t1"), SIZE, PAGE)
        .await
        .unwrap();
    let bytes = cache(&mock, &dir, 1024)
        .get(&image("aaa", "t1"), SIZE, PAGE)
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
        .get(&image("aaa", "t1"), SIZE, PAGE)
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
        .get(&image("aaa", "t1"), SIZE, PAGE)
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
        cache.get(&image(item, "t"), SIZE, PAGE).await.unwrap();
    }
    assert_eq!(requests(&mock).await, 3);
    std::fs::remove_dir_all(Paths::under(dir.path()).cache_dir).unwrap();
    cache.get(&image("ccc", "t"), SIZE, PAGE).await.unwrap();
    assert_eq!(requests(&mock).await, 3);
    cache.get(&image("aaa", "t"), SIZE, PAGE).await.unwrap();
    assert_eq!(requests(&mock).await, 4);
}

#[tokio::test]
async fn oversized_entry_is_returned_but_not_kept() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"0123456789").await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 4);
    let bytes = cache.get(&image("aaa", "t"), SIZE, PAGE).await.unwrap();
    assert_eq!(bytes.len(), 10);
    std::fs::remove_dir_all(Paths::under(dir.path()).cache_dir).unwrap();
    cache.get(&image("aaa", "t"), SIZE, PAGE).await.unwrap();
    assert_eq!(requests(&mock).await, 2);
}

#[tokio::test]
async fn different_tag_refetches() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"pixels").await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 1024);
    cache.get(&image("aaa", "t1"), SIZE, PAGE).await.unwrap();
    cache.get(&image("aaa", "t2"), SIZE, PAGE).await.unwrap();
    assert_eq!(requests(&mock).await, 2);
}

#[tokio::test]
async fn crafted_tag_stays_inside_the_cache_directory() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"pixels").await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 1024);
    cache
        .get(&image("aaa", "../../escape"), SIZE, PAGE)
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
    let bytes = cache.get(&image("aaa", "t1"), SIZE, PAGE).await.unwrap();
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
        cache.get(&image(item, "t"), SIZE, PAGE).await.unwrap();
    }
    cache.trim().await;
    for (path, bytes) in files_in(&dir) {
        let rank = bodies.iter().position(|(_, b)| b[..] == bytes[..]).unwrap();
        age(&path, 1000 - rank as u64 * 100);
    }
    cache.get(&image("ddd", "t"), SIZE, PAGE).await.unwrap();
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
        cache.get(&image(item, "t"), SIZE, PAGE).await.unwrap();
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
        cache.get(&image(item, "t"), SIZE, PAGE).await.unwrap();
    }
    assert_eq!(cache.trim_requests(), requests);
    cache
        .get(&image(&items[10], "t"), SIZE, PAGE)
        .await
        .unwrap();
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
        cache.get(&image("aaa", "t"), SIZE, PAGE).await.unwrap();
        wait_removed(&stale).await;
    });
}

#[test]
fn trim_cancelled_before_it_runs_does_not_block_later_ones() {
    let first = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let dir = TempDir::new().unwrap();
    let stale = stale_tmp(&dir);
    let (mock, cache) = first.block_on(async {
        let mock = MockServer::start().await;
        serve(&mock, "aaa", b"pixels").await;
        let cache = cache(&mock, &dir, 1024);
        (mock, cache)
    });
    drop(first);
    assert!(stale.exists());
    let second = tokio::runtime::Runtime::new().unwrap();
    second.block_on(async {
        let _ = cache.get(&image("aaa", "t"), SIZE, PAGE).await;
        wait_removed(&stale).await;
    });
    drop(mock);
}

fn sheet(index: u32, layout: (u32, u32, u32)) -> TrickplaySheet {
    TrickplaySheet {
        item: ItemId::from("aaa"),
        media_source: MediaSourceId::from("src"),
        resolution: 320,
        index,
        layout,
    }
}

async fn serve_sheet(mock: &MockServer, index: u32, body: &[u8]) {
    Mock::given(method("GET"))
        .and(path(format!("/Videos/aaa/Trickplay/320/{index}.jpg")))
        .and(query_param("mediaSourceId", "src"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
        .mount(mock)
        .await;
}

#[tokio::test]
async fn trickplay_sheets_are_cached_in_memory_and_on_disk() {
    let mock = MockServer::start().await;
    serve_sheet(&mock, 0, b"sheet0").await;
    serve_sheet(&mock, 1, b"sheet1").await;
    let dir = TempDir::new().unwrap();
    let first = cache(&mock, &dir, 1024);
    let layout = (10_000, 10, 10);
    assert_eq!(
        &*first.trickplay_sheet(&sheet(0, layout)).await.unwrap(),
        b"sheet0"
    );
    assert_eq!(
        &*first.trickplay_sheet(&sheet(0, layout)).await.unwrap(),
        b"sheet0"
    );
    assert_eq!(
        &*first.trickplay_sheet(&sheet(1, layout)).await.unwrap(),
        b"sheet1"
    );
    assert_eq!(requests(&mock).await, 2);

    let second = cache(&mock, &dir, 1024);
    assert_eq!(
        &*second.trickplay_sheet(&sheet(1, layout)).await.unwrap(),
        b"sheet1"
    );
    assert_eq!(requests(&mock).await, 2);
}

#[tokio::test]
async fn a_trickplay_set_generated_again_is_fetched_again() {
    let mock = MockServer::start().await;
    serve_sheet(&mock, 0, b"sheet0").await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 1024);
    cache
        .trickplay_sheet(&sheet(0, (10_000, 10, 10)))
        .await
        .unwrap();
    cache
        .trickplay_sheet(&sheet(0, (5_000, 10, 10)))
        .await
        .unwrap();
    assert_eq!(requests(&mock).await, 2);
}

#[tokio::test]
async fn a_missing_trickplay_sheet_is_an_error_and_not_cached() {
    let mock = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 1024);
    assert!(
        cache
            .trickplay_sheet(&sheet(7, (10_000, 10, 10)))
            .await
            .is_err()
    );
    let images = Paths::under(dir.path()).cache_dir.join("images");
    assert!(!images.exists() || files_in(&dir).is_empty());
}

#[tokio::test]
async fn chapter_images_are_fetched_and_cached_by_index() {
    let mock = MockServer::start().await;
    for index in [1, 2] {
        Mock::given(method("GET"))
            .and(path(format!("/Items/aaa/Images/Chapter/{index}")))
            .and(query_param("tag", "t1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(format!("chapter {index}").into_bytes()),
            )
            .mount(&mock)
            .await;
    }
    let dir = TempDir::new().unwrap();
    let chapter = |index| ImageRef {
        item_id: ItemId::from("aaa"),
        image_type: ImageType::Chapter,
        tag: "t1".to_owned(),
        index: Some(index),
    };
    let first = cache(&mock, &dir, 1024)
        .get(&chapter(1), SIZE, PAGE)
        .await
        .unwrap();
    let second = cache(&mock, &dir, 1024)
        .get(&chapter(2), SIZE, PAGE)
        .await
        .unwrap();
    assert_eq!(&*first, b"chapter 1");
    assert_eq!(&*second, b"chapter 2");
    let again = cache(&mock, &dir, 1024)
        .get(&chapter(1), SIZE, PAGE)
        .await
        .unwrap();
    assert_eq!(&*again, b"chapter 1");
    assert_eq!(requests(&mock).await, 2);
}

/// Answers after a while, counting the requests being answered.
struct Counting(Arc<AtomicUsize>);

impl Respond for Counting {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.0.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200)
            .set_body_bytes(b"pixels".to_vec())
            .set_delay(std::time::Duration::from_secs(1))
    }
}

async fn wait_for(count: usize, answered: &AtomicUsize) {
    while answered.load(Ordering::SeqCst) < count {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn the_images_of_a_newer_page_and_its_artwork_are_downloaded_first() {
    const ROWS: usize = IMAGE_FETCHES - ARTWORK_FETCHES;
    let mock = MockServer::start().await;
    let asked = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .respond_with(Counting(Arc::clone(&asked)))
        .mount(&mock)
        .await;
    let dir = TempDir::new().unwrap();
    let cache = Arc::new(cache(&mock, &dir, 1024 * 1024));
    let mut gets = tokio::task::JoinSet::new();
    let mut ask = |name: String, priority: ImagePriority| {
        let cache = Arc::clone(&cache);
        gets.spawn(async move { cache.get(&image(&name, "t"), SIZE, priority).await });
    };
    for n in 0..ROWS {
        ask(format!("running{n}"), ImagePriority::of_page(1));
    }
    wait_for(ROWS, &asked).await;
    for n in 0..ROWS {
        ask(format!("older{n}"), ImagePriority::of_page(1));
    }
    for n in 0..ROWS {
        ask(format!("newer{n}"), ImagePriority::of_page(2));
    }
    ask("artwork".into(), ImagePriority::of_page(2).artwork());
    while let Some(got) = gets.join_next().await {
        assert!(got.unwrap().is_ok());
    }
    let paths: Vec<String> = mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect();
    assert!(paths[ROWS].contains("artwork"), "it waits for no row");
    let waited = &paths[ROWS + 1..];
    let newer = waited.iter().take(ROWS).collect::<Vec<_>>();
    assert!(newer.iter().all(|path| path.contains("newer")), "{paths:?}");
    assert!(waited[ROWS..].iter().all(|path| path.contains("older")));
}

#[tokio::test]
async fn disk_usage_counts_the_stored_images() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"123456").await;
    serve(&mock, "bbb", b"1234").await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 1024);
    assert_eq!(cache.disk_usage().await, 0);
    cache.get(&image("aaa", "t"), SIZE, PAGE).await.unwrap();
    cache.get(&image("bbb", "t"), SIZE, PAGE).await.unwrap();
    let images = Paths::under(dir.path()).cache_dir.join("images");
    std::fs::write(images.join(".partial.tmp"), b"half").unwrap();
    assert_eq!(cache.disk_usage().await, 10);
}

#[tokio::test]
async fn clear_empties_disk_and_memory() {
    let mock = MockServer::start().await;
    serve(&mock, "aaa", b"pixels").await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 1024);
    cache.get(&image("aaa", "t"), SIZE, PAGE).await.unwrap();
    assert_eq!(requests(&mock).await, 1);

    cache.clear().await.unwrap();
    assert_eq!(cache.disk_usage().await, 0);
    let images = Paths::under(dir.path()).cache_dir.join("images");
    let left = std::fs::read_dir(&images).map_or(0, |entries| entries.count());
    assert_eq!(left, 0);

    let bytes = cache.get(&image("aaa", "t"), SIZE, PAGE).await.unwrap();
    assert_eq!(&*bytes, b"pixels");
    assert_eq!(requests(&mock).await, 2);
    assert_eq!(cache.disk_usage().await, 6);
}

#[tokio::test]
async fn clear_without_a_cache_directory_succeeds() {
    let mock = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let cache = cache(&mock, &dir, 1024);
    cache.clear().await.unwrap();
    assert_eq!(cache.disk_usage().await, 0);
}
