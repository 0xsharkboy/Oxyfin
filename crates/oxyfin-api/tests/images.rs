//! Image URL and download tests against a mock server.

mod common;

use common::{TEST_TOKEN, authorization_is, session};
use oxyfin_api::{Error, ImageRequest, ImageType};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn image_url_has_only_set_parameters() {
    let mock = MockServer::start().await;
    let session = session(&mock);
    let mut request = ImageRequest::new("abc".into(), ImageType::Primary);
    request.max_width = Some(300);
    assert_eq!(
        session.image_url(&request).as_str(),
        format!("{}/Items/abc/Images/Primary?maxWidth=300", mock.uri())
    );

    let bare = ImageRequest::new("abc".into(), ImageType::Backdrop);
    assert_eq!(
        session.image_url(&bare).as_str(),
        format!("{}/Items/abc/Images/Backdrop", mock.uri())
    );

    request.tag = Some("t1".to_owned());
    request.max_height = Some(450);
    request.quality = Some(90);
    assert_eq!(
        session.image_url(&request).as_str(),
        format!(
            "{}/Items/abc/Images/Primary?tag=t1&maxWidth=300&maxHeight=450&quality=90",
            mock.uri()
        )
    );
}

#[tokio::test]
async fn image_url_never_has_token() {
    let mock = MockServer::start().await;
    let session = session(&mock);
    let mut request = ImageRequest::new("abc".into(), ImageType::Primary);
    request.tag = Some("t1".to_owned());
    let url = session.image_url(&request).to_string();
    assert!(!url.contains(TEST_TOKEN), "{url}");
    assert!(!url.contains("api_key"), "{url}");
}

#[tokio::test]
async fn fetch_image_returns_bytes() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items/abc/Images/Primary"))
        .and(authorization_is(Some(TEST_TOKEN)))
        .respond_with(ResponseTemplate::new(200).set_body_raw(vec![0xFF, 0xD8, 0xFF], "image/jpeg"))
        .expect(1)
        .mount(&mock)
        .await;
    let request = ImageRequest::new("abc".into(), ImageType::Primary);
    let bytes = session(&mock).fetch_image(&request).await.unwrap();
    assert_eq!(bytes, [0xFF, 0xD8, 0xFF]);
}

#[tokio::test]
async fn fetch_image_reports_status_errors() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items/abc/Images/Primary"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(vec![0x00], "image/jpeg"))
        .mount(&mock)
        .await;
    let request = ImageRequest::new("abc".into(), ImageType::Primary);
    let err = session(&mock).fetch_image(&request).await.unwrap_err();
    assert!(matches!(err, Error::Status { status: 404, .. }), "{err:?}");
}
