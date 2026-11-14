//! Integration tests for the image cache.

mod common;

use common::session;
use oxyfin_api::{ImageType, ItemId};
use oxyfin_core::{ImageCache, ImageRef, ImageSize, Paths};
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

fn cache(mock: &MockServer, dir: &TempDir, budget: usize) -> ImageCache {
    ImageCache::new(session(mock), &Paths::under(dir.path()), budget)
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
