//! Media segment request tests against a mock server.

mod common;

use common::{TEST_TOKEN, authorization_is, session, to_camel_case};
use oxyfin_api::{Error, ItemId, MediaSegmentType, Ticks};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ITEM: &str = "76282b1c408f374beeee50519e9157a4";

/// Shaped like the answer of Jellyfin 10.10 and later, which no test server has data for.
fn segments() -> Value {
    json!({
        "Items": [
            {
                "Id": "0f1e2d3c4b5a69788796a5b4c3d2e1f0",
                "ItemId": ITEM,
                "Type": "Intro",
                "StartTicks": 50_000_000,
                "EndTicks": 900_000_000_i64,
            },
            {
                "Id": "1f1e2d3c4b5a69788796a5b4c3d2e1f0",
                "ItemId": ITEM,
                "Type": "Outro",
                "StartTicks": 12_000_000_000_i64,
                "EndTicks": 12_600_000_000_i64,
            },
            {
                "Id": "2f1e2d3c4b5a69788796a5b4c3d2e1f0",
                "ItemId": ITEM,
                "Type": "Sponsor",
                "StartTicks": 0,
                "EndTicks": 10,
            },
        ],
        "TotalRecordCount": 3,
        "StartIndex": 0,
    })
}

async fn mount(mock: &MockServer, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(format!("/MediaSegments/{ITEM}")))
        .and(header("accept", "application/json"))
        .and(authorization_is(Some(TEST_TOKEN)))
        .respond_with(response)
        .expect(1)
        .mount(mock)
        .await;
}

#[tokio::test]
async fn media_segments_decode() {
    for body in [segments(), to_camel_case(segments())] {
        let mock = MockServer::start().await;
        mount(&mock, ResponseTemplate::new(200).set_body_json(&body)).await;
        let found = session(&mock)
            .media_segments(&ItemId::from(ITEM))
            .await
            .unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].r#type, MediaSegmentType::Intro);
        assert_eq!(found[0].item_id, ItemId::from(ITEM));
        assert_eq!(found[0].start_ticks, Ticks(50_000_000));
        assert_eq!(found[0].end_ticks, Ticks(900_000_000));
        assert_eq!(found[1].r#type, MediaSegmentType::Outro);
        assert_eq!(
            found[2].r#type,
            MediaSegmentType::Other("Sponsor".to_owned())
        );
    }
}

#[tokio::test]
async fn no_segments_is_an_empty_list() {
    let mock = MockServer::start().await;
    let empty = json!({"Items": [], "TotalRecordCount": 0, "StartIndex": 0});
    mount(&mock, ResponseTemplate::new(200).set_body_json(empty)).await;
    let found = session(&mock)
        .media_segments(&ItemId::from(ITEM))
        .await
        .unwrap();
    assert!(found.is_empty());
}

#[tokio::test]
async fn not_found_is_an_empty_list() {
    let mock = MockServer::start().await;
    let problem = json!({"title": "Not Found", "status": 404});
    mount(&mock, ResponseTemplate::new(404).set_body_json(problem)).await;
    let found = session(&mock)
        .media_segments(&ItemId::from(ITEM))
        .await
        .unwrap();
    assert!(found.is_empty());
}

#[tokio::test]
async fn other_failures_are_errors() {
    let mock = MockServer::start().await;
    mount(&mock, ResponseTemplate::new(401)).await;
    let err = session(&mock)
        .media_segments(&ItemId::from(ITEM))
        .await
        .unwrap_err();
    assert!(err.is_unauthorized(), "{err:?}");

    let mock = MockServer::start().await;
    mount(&mock, ResponseTemplate::new(500)).await;
    let err = session(&mock)
        .media_segments(&ItemId::from(ITEM))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Status { status: 500, .. }), "{err:?}");
}
