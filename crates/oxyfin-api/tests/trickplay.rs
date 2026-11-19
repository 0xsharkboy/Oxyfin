//! Trickplay info decoding and tile sheet download tests against a mock server.

mod common;

use common::{TEST_TOKEN, authorization_is, session, to_camel_case};
use oxyfin_api::{BaseItemDto, Error, ItemId, MediaSourceId, TrickplayInfoDto};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ITEM: &str = "af282b1c408f374beeee50519e9157a4";

/// Shaped like what Jellyfin 12.4 answered for a movie with generated trickplay.
fn item(trickplay: Value) -> Value {
    json!({"Id": ITEM, "Type": "Movie", "Trickplay": trickplay})
}

fn sheets() -> Value {
    json!({
        ITEM: {
            "160": {"Width": 160, "Height": 90, "TileWidth": 3, "TileHeight": 3,
                    "ThumbnailCount": 10, "Interval": 1000, "Bandwidth": 27125},
            "320": {"Width": 320, "Height": 180, "TileWidth": 10, "TileHeight": 10,
                    "ThumbnailCount": 10, "Interval": 1000, "Bandwidth": 90000}
        }
    })
}

#[allow(clippy::unwrap_used)]
async fn decoded(body: Value) -> BaseItemDto {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{ITEM}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(&mock)
        .await;
    session(&mock).item(&ItemId::from(ITEM)).await.unwrap()
}

#[tokio::test]
async fn trickplay_decodes_by_source_and_width_in_both_casings() {
    for body in [item(sheets()), to_camel_case(item(sheets()))] {
        let item = decoded(body).await;
        // Key normalization capitalizes the source key: `af28...` became `Af28...`.
        let widths = item
            .trickplay_for(&MediaSourceId::from(ITEM))
            .expect("trickplay of the source");
        assert_eq!(widths.keys().copied().collect::<Vec<_>>(), [160, 320]);
        assert_eq!(
            widths[&160],
            TrickplayInfoDto {
                width: 160,
                height: 90,
                tile_width: 3,
                tile_height: 3,
                thumbnail_count: 10,
                interval: 1000,
                bandwidth: 27125,
            }
        );
    }
}

#[tokio::test]
async fn trickplay_matches_a_dashed_source_id() {
    let item = decoded(item(sheets())).await;
    let dashed = MediaSourceId::from("af282b1c-408f-374b-eeee-50519e9157a4");
    assert!(item.trickplay_for(&dashed).is_some());
    assert!(item.trickplay_for(&MediaSourceId::from("0000")).is_none());
}

#[tokio::test]
async fn missing_or_null_trickplay_is_empty() {
    for trickplay in [
        json!(null),
        json!({}),
        json!({ITEM: null}),
        json!({ITEM: {}}),
    ] {
        let item = decoded(item(trickplay)).await;
        assert!(
            item.trickplay_for(&MediaSourceId::from(ITEM)).is_none(),
            "{:?}",
            item.trickplay
        );
    }
    let bare: BaseItemDto = serde_json::from_value(json!({"Id": "x", "Type": "Movie"})).unwrap();
    assert!(bare.trickplay.is_empty());
}

#[tokio::test]
async fn trickplay_sheet_returns_the_jpeg_with_auth() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/Videos/{ITEM}/Trickplay/320/2.jpg")))
        .and(query_param("mediaSourceId", "src1"))
        .and(authorization_is(Some(TEST_TOKEN)))
        .respond_with(ResponseTemplate::new(200).set_body_raw(vec![0xFF, 0xD8, 0xFF], "image/jpeg"))
        .expect(1)
        .mount(&mock)
        .await;
    let bytes = session(&mock)
        .trickplay_sheet(&ItemId::from(ITEM), &MediaSourceId::from("src1"), 320, 2)
        .await
        .unwrap();
    assert_eq!(bytes, [0xFF, 0xD8, 0xFF]);
}

#[tokio::test]
async fn trickplay_sheet_past_the_end_is_a_status_error() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/Videos/{ITEM}/Trickplay/320/9.jpg")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock)
        .await;
    let err = session(&mock)
        .trickplay_sheet(&ItemId::from(ITEM), &MediaSourceId::from(ITEM), 320, 9)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Status { status: 404, .. }), "{err:?}");
}
