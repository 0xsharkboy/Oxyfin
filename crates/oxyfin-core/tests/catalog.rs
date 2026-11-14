//! Integration tests for the catalog service.

mod common;

use common::{TEST_USER_ID, fixture, session};
use oxyfin_api::ItemId;
use oxyfin_core::{
    BrowseQuery, BrowseSource, Catalog, CoreError, ItemKind, ItemSummary, LibraryKind, SortField,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MOVIE_ID: &str = "76282b1c408f374beeee50519e9157a4";
const SERIES_ID: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";

async fn mount(mock: &MockServer, verb: &str, route: &str, body: Value) {
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(mock)
        .await;
}

fn catalog(mock: &MockServer) -> Catalog {
    Catalog::new(session(mock))
}

fn query(source: BrowseSource) -> BrowseQuery {
    BrowseQuery {
        source,
        sort: SortField::Title,
        descending: false,
        genre: None,
        played: None,
        start: 0,
        limit: 20,
    }
}

fn summary(id: &str, with_backdrop: bool) -> ItemSummary {
    let mut dto = serde_json::from_value::<oxyfin_api::BaseItemDto>(json!({
        "Id": id, "Name": id, "Type": "Movie"
    }))
    .unwrap_or_else(|err| panic!("bad item: {err}"));
    if with_backdrop {
        dto.backdrop_image_tags = vec!["tag".to_owned()];
    }
    ItemSummary::from_dto(&dto).unwrap_or_else(|| panic!("unsupported item"))
}

#[tokio::test]
async fn libraries_lists_views() {
    let mock = MockServer::start().await;
    mount(&mock, "GET", "/UserViews", fixture("user_views")).await;
    let libraries = catalog(&mock).libraries().await.unwrap();
    assert_eq!(libraries.len(), 2);
    assert_eq!(libraries[1].kind, LibraryKind::Shows);
}

#[tokio::test]
async fn libraries_keep_video_libraries() {
    let mock = MockServer::start().await;
    let view = |id: &str, collection: Option<&str>| {
        let mut view = json!({"Id": id, "Name": id, "Type": "CollectionFolder"});
        if let Some(collection) = collection {
            view["CollectionType"] = json!(collection);
        }
        view
    };
    let items = [
        view("movies", Some("movies")),
        view("tvshows", Some("tvshows")),
        view("music", Some("music")),
        view("livetv", Some("livetv")),
        view("playlists", Some("playlists")),
        view("untyped", None),
        view("home", Some("homevideos")),
        view("boxsets", Some("boxsets")),
    ];
    mount(
        &mock,
        "GET",
        "/UserViews",
        json!({"Items": items, "TotalRecordCount": 8, "StartIndex": 0}),
    )
    .await;
    let libraries = catalog(&mock).libraries().await.unwrap();
    let kinds: Vec<_> = libraries.iter().map(|library| library.kind).collect();
    assert_eq!(
        kinds,
        [
            LibraryKind::Movies,
            LibraryKind::Shows,
            LibraryKind::Mixed,
            LibraryKind::Mixed
        ]
    );
}

#[tokio::test]
async fn continue_watching_skips_unknown_kinds() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .and(query_param("limit", "12"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("user_items_resume")))
        .mount(&mock)
        .await;
    let items = catalog(&mock).continue_watching(12).await.unwrap();
    let kinds: Vec<_> = items.iter().map(|item| item.kind).collect();
    assert_eq!(kinds, [ItemKind::Movie, ItemKind::Series]);
}

#[tokio::test]
async fn next_up_lists_episodes() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .and(query_param("limit", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("shows_next_up")))
        .mount(&mock)
        .await;
    let items = catalog(&mock).next_up(5).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, ItemKind::Episode);
}

#[tokio::test]
async fn latest_sends_parent_and_limit() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .and(query_param("parentId", "lib1"))
        .and(query_param("limit", "8"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_latest")))
        .mount(&mock)
        .await;
    let items = catalog(&mock)
        .latest(&ItemId::from("lib1"), 8)
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
}

#[test]
fn featured_prefers_continue_watching() {
    let resume = [summary("r1", false), summary("r2", true)];
    let latest = [summary("l1", true)];
    let featured = Catalog::featured(&resume, &latest).unwrap();
    assert_eq!(featured.id, ItemId::from("r1"));
}

#[test]
fn featured_falls_back_to_latest_with_backdrop() {
    let latest = [summary("l1", false), summary("l2", true)];
    let featured = Catalog::featured(&[], &latest).unwrap();
    assert_eq!(featured.id, ItemId::from("l2"));
    assert!(Catalog::featured(&[], &[summary("l3", false)]).is_none());
}

#[tokio::test]
async fn browse_shows_sends_filters_and_sort() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("parentId", "lib2"))
        .and(query_param("recursive", "true"))
        .and(query_param("includeItemTypes", "Series"))
        .and(query_param("sortBy", "DateCreated"))
        .and(query_param("sortOrder", "Descending"))
        .and(query_param("genres", "Drama"))
        .and(query_param("isPlayed", "false"))
        .and(query_param("startIndex", "40"))
        .and(query_param("limit", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_search")))
        .mount(&mock)
        .await;
    let mut q = query(BrowseSource::Library {
        id: ItemId::from("lib2"),
        kind: LibraryKind::Shows,
    });
    q.sort = SortField::DateAdded;
    q.descending = true;
    q.genre = Some("Drama".to_owned());
    q.played = Some(false);
    q.start = 40;
    let page = catalog(&mock).browse(&q).await.unwrap();
    assert!(page.total > 0);
}

#[tokio::test]
async fn browse_movies_sends_movie_kind_and_year_sort() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("includeItemTypes", "Movie"))
        .and(query_param("sortBy", "ProductionYear"))
        .and(query_param("sortOrder", "Ascending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_movies")))
        .mount(&mock)
        .await;
    let mut q = query(BrowseSource::Library {
        id: ItemId::from("lib1"),
        kind: LibraryKind::Movies,
    });
    q.sort = SortField::Year;
    let page = catalog(&mock).browse(&q).await.unwrap();
    assert_eq!(page.items.len(), 1);
}

#[tokio::test]
async fn browse_favorites_sends_is_favorite() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("isFavorite", "true"))
        .and(query_param("recursive", "true"))
        .and(query_param("includeItemTypes", "Movie,Series,Video"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_movies")))
        .mount(&mock)
        .await;
    let page = catalog(&mock)
        .browse(&query(BrowseSource::Favorites))
        .await
        .unwrap();
    assert_eq!(page.start, 0);
}

#[tokio::test]
async fn genres_returns_names() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Genres"))
        .and(query_param("parentId", "lib2"))
        .and(query_param("includeItemTypes", "Series"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("genres")))
        .mount(&mock)
        .await;
    let genres = catalog(&mock)
        .genres(&ItemId::from("lib2"), LibraryKind::Shows)
        .await
        .unwrap();
    assert_eq!(genres, ["Animation", "Comedy", "Drama"]);
}

#[tokio::test]
async fn search_sends_term_and_paging() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("searchTerm", "test"))
        .and(query_param("recursive", "true"))
        .and(query_param("includeItemTypes", "Movie,Series"))
        .and(query_param("startIndex", "10"))
        .and(query_param("limit", "30"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_search")))
        .mount(&mock)
        .await;
    let page = catalog(&mock).search("test", 10, 30).await.unwrap();
    assert!(!page.items.is_empty());
}

#[tokio::test]
async fn movie_fetches_details() {
    let mock = MockServer::start().await;
    mount(
        &mock,
        "GET",
        &format!("/Items/{MOVIE_ID}"),
        fixture("item_movie"),
    )
    .await;
    let movie = catalog(&mock).movie(&ItemId::from(MOVIE_ID)).await.unwrap();
    assert_eq!(movie.genres, ["Drama", "Comedy"]);
}

async fn mount_series(mock: &MockServer) {
    mount(
        mock,
        "GET",
        &format!("/Items/{SERIES_ID}"),
        fixture("item_series"),
    )
    .await;
    mount(
        mock,
        "GET",
        &format!("/Shows/{SERIES_ID}/Seasons"),
        fixture("shows_seasons"),
    )
    .await;
}

#[tokio::test]
async fn series_includes_the_next_episode() {
    let mock = MockServer::start().await;
    mount_series(&mock).await;
    mount(&mock, "GET", "/Shows/NextUp", fixture("shows_next_up")).await;
    let series = catalog(&mock)
        .series(&ItemId::from(SERIES_ID))
        .await
        .unwrap();
    assert_eq!(series.seasons.len(), 2);
    assert_eq!(series.next_episode.unwrap().title, "Test Show S01E02");
}

#[tokio::test]
async fn series_survives_a_failed_next_episode_lookup() {
    let mock = MockServer::start().await;
    mount_series(&mock).await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;
    let series = catalog(&mock)
        .series(&ItemId::from(SERIES_ID))
        .await
        .unwrap();
    assert_eq!(series.seasons.len(), 2);
    assert_eq!(series.next_episode, None);
}

#[tokio::test]
async fn series_next_episode_401_is_session_expired() {
    let mock = MockServer::start().await;
    mount_series(&mock).await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock)
        .await;
    let error = catalog(&mock)
        .series(&ItemId::from(SERIES_ID))
        .await
        .unwrap_err();
    assert_eq!(error, CoreError::SessionExpired);
}

#[tokio::test]
async fn episodes_sends_season_id() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES_ID}/Episodes")))
        .and(query_param("seasonId", "season1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("shows_episodes")))
        .mount(&mock)
        .await;
    let episodes = catalog(&mock)
        .episodes(&ItemId::from(SERIES_ID), &ItemId::from("season1"))
        .await
        .unwrap();
    assert_eq!(episodes.len(), 4);
}

#[tokio::test]
async fn all_episodes_sends_no_season() {
    let mock = MockServer::start().await;
    mount(
        &mock,
        "GET",
        &format!("/Shows/{SERIES_ID}/Episodes"),
        fixture("shows_episodes"),
    )
    .await;
    let episodes = catalog(&mock)
        .all_episodes(&ItemId::from(SERIES_ID))
        .await
        .unwrap();
    assert_eq!(episodes.len(), 4);
    let requests = mock.received_requests().await.unwrap();
    assert!(!requests[0].url.query().unwrap_or("").contains("seasonId"));
}

#[tokio::test]
async fn next_episode_uses_next_up_with_series_id() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .and(query_param("seriesId", SERIES_ID))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("shows_next_up")))
        .mount(&mock)
        .await;
    let episode = catalog(&mock)
        .next_episode_to_watch(&ItemId::from(SERIES_ID))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(episode.title, "Test Show S01E02");
}

#[tokio::test]
async fn next_episode_falls_back_to_first_unplayed() {
    let mock = MockServer::start().await;
    mount(
        &mock,
        "GET",
        "/Shows/NextUp",
        json!({"Items": [], "TotalRecordCount": 0, "StartIndex": 0}),
    )
    .await;
    mount(
        &mock,
        "GET",
        &format!("/Shows/{SERIES_ID}/Episodes"),
        fixture("shows_episodes"),
    )
    .await;
    let episode = catalog(&mock)
        .next_episode_to_watch(&ItemId::from(SERIES_ID))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(episode.title, "Test Show S01E02");
}

#[tokio::test]
async fn set_played_posts_and_returns_state() {
    let mock = MockServer::start().await;
    mount(
        &mock,
        "POST",
        &format!("/UserPlayedItems/{MOVIE_ID}"),
        fixture("user_played_items"),
    )
    .await;
    let state = catalog(&mock)
        .set_played(&ItemId::from(MOVIE_ID), true)
        .await
        .unwrap();
    assert!(state.played);
    let requests = mock.received_requests().await.unwrap();
    assert!(requests[0].url.query().unwrap_or("").contains(TEST_USER_ID));
}

#[tokio::test]
async fn set_favorite_deletes_to_unset() {
    let mock = MockServer::start().await;
    mount(
        &mock,
        "DELETE",
        &format!("/UserFavoriteItems/{MOVIE_ID}"),
        fixture("user_favorite_items"),
    )
    .await;
    catalog(&mock)
        .set_favorite(&ItemId::from(MOVIE_ID), false)
        .await
        .unwrap();
}

#[tokio::test]
async fn catalog_401_is_session_expired() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock)
        .await;
    let error = catalog(&mock).libraries().await.unwrap_err();
    assert_eq!(error, CoreError::SessionExpired);
}

#[tokio::test]
async fn browse_mixed_sends_movie_series_and_video_kinds() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("parentId", "lib3"))
        .and(query_param("includeItemTypes", "Movie,Series,Video"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_movies")))
        .mount(&mock)
        .await;
    let q = query(BrowseSource::Library {
        id: ItemId::from("lib3"),
        kind: LibraryKind::Mixed,
    });
    let page = catalog(&mock).browse(&q).await.unwrap();
    assert_eq!(page.items.len(), 1);
}
