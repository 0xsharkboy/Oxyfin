//! Item model and browsing query tests against a mock server.

mod common;

use common::{
    TEST_TOKEN, TEST_USER_ID, authorization_is, fixture, session, to_pascal_case, variants,
};
use oxyfin_api::{
    BaseItemDto, BaseItemKind, ImageType, ItemFields, ItemId, ItemSortBy, ItemsQuery, SortOrder,
    Ticks,
};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn version_of(label: &str) -> &str {
    label.split('/').next().unwrap_or(label)
}

async fn mount(mock: &MockServer, route: &str, body: &Value) {
    Mock::given(method("GET"))
        .and(path(route))
        .and(header("accept", "application/json"))
        .and(authorization_is(Some(TEST_TOKEN)))
        .and(query_param("userId", TEST_USER_ID))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(mock)
        .await;
}

fn movie(items: &[BaseItemDto]) -> &BaseItemDto {
    items
        .iter()
        .find(|item| item.r#type == BaseItemKind::Movie)
        .unwrap_or_else(|| panic!("no movie in {items:?}"))
}

fn assert_test_movie(item: &BaseItemDto, label: &str) {
    assert_eq!(item.name.as_deref(), Some("Test Movie (2020)"), "{label}");
    assert_eq!(item.production_year, Some(2020), "{label}");
    assert!(item.genres.iter().any(|g| g == "Drama"), "{label}");
    assert!(item.image_tags.contains_key(&ImageType::Primary), "{label}");
}

#[tokio::test]
async fn user_views_decodes_all_fixtures() {
    for (label, body) in variants("user_views") {
        let captured = to_pascal_case(fixture(version_of(&label), "user_views"));
        let mock = MockServer::start().await;
        mount(&mock, "/UserViews", &body).await;
        let page = session(&mock).user_views().await.unwrap();
        assert_eq!(
            page.items.len(),
            captured["Items"].as_array().unwrap().len(),
            "{label}"
        );
        assert!(!page.items.is_empty(), "{label}");
        let first = &captured["Items"][0];
        assert_eq!(page.items[0].id.as_str(), first["Id"].as_str().unwrap());
        assert_eq!(
            page.items[0].collection_type.as_deref(),
            first["CollectionType"].as_str(),
            "{label}"
        );
    }
}

#[tokio::test]
async fn items_decode_movies_all_fixtures() {
    for (label, body) in variants("items_movies") {
        let mock = MockServer::start().await;
        mount(&mock, "/Items", &body).await;
        let page = session(&mock).items(&ItemsQuery::default()).await.unwrap();
        assert_test_movie(movie(&page.items), &label);
    }
}

#[tokio::test]
async fn items_decode_search_all_fixtures() {
    for (label, body) in variants("items_search") {
        let mock = MockServer::start().await;
        mount(&mock, "/Items", &body).await;
        let query = ItemsQuery {
            search_term: Some("test".into()),
            ..ItemsQuery::default()
        };
        let page = session(&mock).items(&query).await.unwrap();
        assert!(!page.items.is_empty(), "{label}");
        assert!(page.total_record_count >= 1, "{label}");
    }
}

#[tokio::test]
async fn item_movie_decodes_all_fixtures() {
    for (label, body) in variants("item_movie") {
        let captured = to_pascal_case(fixture(version_of(&label), "item_movie"));
        let id = ItemId::from(captured["Id"].as_str().unwrap());
        let mock = MockServer::start().await;
        mount(&mock, &format!("/Items/{id}"), &body).await;
        let item = session(&mock).item(&id).await.unwrap();
        assert_eq!(item.id, id, "{label}");
        assert_eq!(item.r#type, BaseItemKind::Movie, "{label}");
        assert_test_movie(&item, &label);
        assert!(item.run_time_ticks.is_some_and(|t| t > Ticks(0)), "{label}");
        assert!(item.user_data.is_some(), "{label}");
    }
}

#[tokio::test]
async fn item_series_decodes_all_fixtures() {
    for (label, body) in variants("item_series") {
        let captured = to_pascal_case(fixture(version_of(&label), "item_series"));
        let id = ItemId::from(captured["Id"].as_str().unwrap());
        let mock = MockServer::start().await;
        mount(&mock, &format!("/Items/{id}"), &body).await;
        let item = session(&mock).item(&id).await.unwrap();
        assert_eq!(item.r#type, BaseItemKind::Series, "{label}");
        assert_eq!(item.name.as_deref(), Some("Test Show"), "{label}");
        assert!(item.genres.iter().any(|g| g == "Animation"), "{label}");
    }
}

#[tokio::test]
async fn item_episode_decodes_all_fixtures() {
    for (label, body) in variants("item_episode") {
        let captured = to_pascal_case(fixture(version_of(&label), "item_episode"));
        let id = ItemId::from(captured["Id"].as_str().unwrap());
        let mock = MockServer::start().await;
        mount(&mock, &format!("/Items/{id}"), &body).await;
        let item = session(&mock).item(&id).await.unwrap();
        assert_eq!(item.r#type, BaseItemKind::Episode, "{label}");
        assert_eq!(item.series_name.as_deref(), Some("Test Show"), "{label}");
        assert_eq!(
            item.index_number.map(i64::from),
            captured["IndexNumber"].as_i64(),
            "{label}"
        );
        assert_eq!(
            item.parent_index_number.map(i64::from),
            captured["ParentIndexNumber"].as_i64(),
            "{label}"
        );
        assert_eq!(
            item.series_id.as_ref().map(ItemId::as_str),
            captured["SeriesId"].as_str(),
            "{label}"
        );
        assert_eq!(
            item.season_id.as_ref().map(ItemId::as_str),
            captured["SeasonId"].as_str(),
            "{label}"
        );
    }
}

#[tokio::test]
async fn latest_decodes_bare_array() {
    for (label, body) in variants("items_latest") {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Items/Latest"))
            .and(authorization_is(Some(TEST_TOKEN)))
            .and(query_param("userId", TEST_USER_ID))
            .and(query_param("parentId", "abc"))
            .and(query_param("limit", "16"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let items = session(&mock)
            .latest(Some(&ItemId::from("abc")), 16)
            .await
            .unwrap();
        assert_eq!(items.len(), body.as_array().unwrap().len(), "{label}");
        assert!(!items.is_empty(), "{label}");
    }
}

#[tokio::test]
async fn resume_contains_the_started_movie() {
    for (label, body) in variants("user_items_resume") {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/UserItems/Resume"))
            .and(authorization_is(Some(TEST_TOKEN)))
            .and(query_param("userId", TEST_USER_ID))
            .and(query_param("limit", "12"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let page = session(&mock).resume(12).await.unwrap();
        let item = movie(&page.items);
        let data = item
            .user_data
            .as_ref()
            .unwrap_or_else(|| panic!("no user data"));
        assert!(data.playback_position_ticks > Ticks(0), "{label}");
        assert_eq!(item.name.as_deref(), Some("Test Movie (2020)"), "{label}");
    }
}

#[tokio::test]
async fn next_up_is_second_episode() {
    for (label, body) in variants("shows_next_up") {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Shows/NextUp"))
            .and(authorization_is(Some(TEST_TOKEN)))
            .and(query_param("userId", TEST_USER_ID))
            .and(query_param("seriesId", "s1"))
            .and(query_param("limit", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let page = session(&mock)
            .next_up(Some(&ItemId::from("s1")), 5)
            .await
            .unwrap();
        assert!(
            page.items
                .iter()
                .any(|item| item.r#type == BaseItemKind::Episode
                    && item.parent_index_number == Some(1)
                    && item.index_number == Some(2)),
            "{label}"
        );
    }
}

#[tokio::test]
async fn seasons_decode_two_seasons() {
    for (label, body) in variants("shows_seasons") {
        let mock = MockServer::start().await;
        mount(&mock, "/Shows/series1/Seasons", &body).await;
        let page = session(&mock)
            .seasons(&ItemId::from("series1"))
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2, "{label}");
        assert!(
            page.items.iter().all(|i| i.r#type == BaseItemKind::Season),
            "{label}"
        );
    }
}

#[tokio::test]
async fn episodes_decode_two_episodes_of_a_season() {
    for (label, body) in variants("shows_episodes") {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Shows/series1/Episodes"))
            .and(authorization_is(Some(TEST_TOKEN)))
            .and(query_param("userId", TEST_USER_ID))
            .and(query_param("seasonId", "season1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let page = session(&mock)
            .episodes(&ItemId::from("series1"), Some(&ItemId::from("season1")))
            .await
            .unwrap();
        assert!(!page.items.is_empty(), "{label}");
        assert!(
            page.items.iter().all(|i| i.r#type == BaseItemKind::Episode),
            "{label}"
        );
    }
}

#[tokio::test]
async fn genres_decode_as_other_kind() {
    for (label, body) in variants("genres") {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Genres"))
            .and(authorization_is(Some(TEST_TOKEN)))
            .and(query_param("userId", TEST_USER_ID))
            .and(query_param("parentId", "lib"))
            .and(query_param("includeItemTypes", "Movie,Series"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let page = session(&mock)
            .genres(
                Some(&ItemId::from("lib")),
                &[BaseItemKind::Movie, BaseItemKind::Series],
            )
            .await
            .unwrap();
        let names: Vec<_> = page
            .items
            .iter()
            .filter_map(|i| i.name.as_deref())
            .collect();
        assert!(names.contains(&"Drama"), "{label}");
        assert!(names.contains(&"Comedy"), "{label}");
        assert!(
            page.items
                .iter()
                .all(|i| i.r#type == BaseItemKind::Other("Genre".into())),
            "{label}"
        );
    }
}

#[tokio::test]
async fn items_query_serializes_parameters() {
    let full = ItemsQuery {
        parent_id: Some(ItemId::from("parent")),
        include_item_types: vec![BaseItemKind::Movie, BaseItemKind::Series],
        recursive: true,
        sort_by: vec![ItemSortBy::SortName, ItemSortBy::ProductionYear],
        sort_order: Some(SortOrder::Descending),
        genres: vec!["Drama".into()],
        is_played: Some(false),
        is_favorite: Some(true),
        search_term: Some("te st".into()),
        start_index: Some(20),
        limit: Some(50),
        fields: vec![ItemFields::Overview, ItemFields::Genres],
    };
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"Items": []})))
        .mount(&mock)
        .await;
    let s = session(&mock);
    s.items(&full).await.unwrap();
    s.items(&ItemsQuery::default()).await.unwrap();
    s.items(&ItemsQuery {
        genres: vec!["Drama".into(), "Comedy".into()],
        ..ItemsQuery::default()
    })
    .await
    .unwrap();

    let requests = mock.received_requests().await.unwrap();
    let pairs = |i: usize| -> Vec<(String, String)> {
        requests[i]
            .url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    };
    let expected: Vec<(String, String)> = [
        ("parentId", "parent"),
        ("includeItemTypes", "Movie,Series"),
        ("recursive", "true"),
        ("sortBy", "SortName,ProductionYear"),
        ("sortOrder", "Descending"),
        ("genres", "Drama"),
        ("isPlayed", "false"),
        ("isFavorite", "true"),
        ("searchTerm", "te st"),
        ("startIndex", "20"),
        ("limit", "50"),
        ("fields", "Overview,Genres"),
        ("userId", TEST_USER_ID),
        ("enableUserData", "true"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    assert_eq!(pairs(0), expected);
    assert_eq!(
        pairs(1),
        vec![
            ("userId".to_string(), TEST_USER_ID.to_string()),
            ("enableUserData".to_string(), "true".to_string())
        ]
    );
    assert!(
        pairs(2).contains(&("genres".to_string(), "Drama|Comedy".to_string())),
        "{:?}",
        pairs(2)
    );
    let raw = requests[2].url.query().unwrap();
    assert!(raw.contains("genres=Drama%7CComedy"), "{raw}");
}

#[test]
fn minimal_item_decodes() {
    let item: BaseItemDto = serde_json::from_value(json!({"Id": "x", "Type": "Movie"})).unwrap();
    assert_eq!(item.id, ItemId::from("x"));
    assert_eq!(item.r#type, BaseItemKind::Movie);
    assert!(item.name.is_none());
    assert!(item.genres.is_empty());
    assert!(item.image_tags.is_empty());
    assert!(item.user_data.is_none());
}

#[test]
fn unknown_kind_is_kept() {
    let page: oxyfin_api::QueryResult<BaseItemDto> = serde_json::from_value(json!({
        "Items": [{"Id": "a", "Type": "Movie"}, {"Id": "b", "Type": "Trailer"}],
        "TotalRecordCount": 2
    }))
    .unwrap();
    assert_eq!(page.items[0].r#type, BaseItemKind::Movie);
    assert_eq!(page.items[1].r#type, BaseItemKind::Other("Trailer".into()));
    assert_eq!(page.total_record_count, 2);
}
