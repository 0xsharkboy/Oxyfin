//! Integration tests for the catalog service.

mod common;

use common::{TEST_USER_ID, fixture, session};
use oxyfin_api::ItemId;
use oxyfin_core::{
    BrowseQuery, BrowseSource, Catalog, CollectionLink, ContentKind, CoreError, ItemKind,
    ItemSummary, LibraryKind, MovieDetails, PersonKind, SortField, WatchFilter,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

struct NoParam(&'static str);

impl Match for NoParam {
    fn matches(&self, request: &Request) -> bool {
        !request.url.query_pairs().any(|(key, _)| key == self.0)
    }
}

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
        watch: WatchFilter::All,
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
    q.start = 40;
    let page = catalog(&mock).browse(&q).await.unwrap();
    assert!(page.total > 0);
}

#[tokio::test]
async fn unplayed_sends_is_played_false() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("isPlayed", "false"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_movies")))
        .expect(1)
        .mount(&mock)
        .await;
    let mut q = query(BrowseSource::All(ContentKind::Movies));
    q.watch = WatchFilter::Unplayed;
    catalog(&mock).browse(&q).await.unwrap();
}

#[tokio::test]
async fn played_sends_is_played_true() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("isPlayed", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_movies")))
        .expect(1)
        .mount(&mock)
        .await;
    let mut q = query(BrowseSource::All(ContentKind::Movies));
    q.watch = WatchFilter::Played;
    catalog(&mock).browse(&q).await.unwrap();
}

#[tokio::test]
async fn all_sends_no_is_played() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(NoParam("isPlayed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_movies")))
        .expect(1)
        .mount(&mock)
        .await;
    catalog(&mock)
        .browse(&query(BrowseSource::All(ContentKind::Movies)))
        .await
        .unwrap();
}

async fn mount_in_progress_lists(mock: &MockServer) {
    mount(
        mock,
        "GET",
        "/UserItems/Resume",
        fixture("user_items_resume"),
    )
    .await;
    mount(mock, "GET", "/Shows/NextUp", fixture("shows_next_up")).await;
}

fn in_progress(source: BrowseSource) -> BrowseQuery {
    let mut q = query(source);
    q.watch = WatchFilter::InProgress;
    q
}

#[tokio::test]
async fn in_progress_lists_resumable_movies_and_started_series() {
    let mock = MockServer::start().await;
    mount_in_progress_lists(&mock).await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("ids", format!("{MOVIE_ID},{SERIES_ID}")))
        .and(query_param("includeItemTypes", "Series"))
        .and(NoParam("isPlayed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_search")))
        .expect(1)
        .mount(&mock)
        .await;
    let page = catalog(&mock)
        .browse(&in_progress(BrowseSource::All(ContentKind::Shows)))
        .await
        .unwrap();
    assert!(page.total > 0);
}

#[tokio::test]
async fn in_progress_without_candidates_returns_an_empty_page() {
    let mock = MockServer::start().await;
    let empty = json!({"Items": [], "TotalRecordCount": 0, "StartIndex": 0});
    mount(&mock, "GET", "/UserItems/Resume", empty.clone()).await;
    mount(&mock, "GET", "/Shows/NextUp", empty).await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_movies")))
        .expect(0)
        .mount(&mock)
        .await;
    let mut q = in_progress(BrowseSource::Favorites);
    q.start = 60;
    let page = catalog(&mock).browse(&q).await.unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.total, 0);
    assert_eq!(page.start, 60);
}

#[tokio::test]
async fn in_progress_keeps_genre_sort_and_paging() {
    let mock = MockServer::start().await;
    mount_in_progress_lists(&mock).await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("parentId", "lib2"))
        .and(query_param("includeItemTypes", "Series"))
        .and(query_param("genres", "Drama"))
        .and(query_param("sortBy", "ProductionYear"))
        .and(query_param("sortOrder", "Descending"))
        .and(query_param("startIndex", "40"))
        .and(query_param("limit", "20"))
        .and(query_param("ids", format!("{MOVIE_ID},{SERIES_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_search")))
        .expect(1)
        .mount(&mock)
        .await;
    let mut q = in_progress(BrowseSource::Library {
        id: ItemId::from("lib2"),
        kind: LibraryKind::Shows,
    });
    q.genre = Some("Drama".to_owned());
    q.sort = SortField::Year;
    q.descending = true;
    q.start = 40;
    catalog(&mock).browse(&q).await.unwrap();
}

#[tokio::test]
async fn in_progress_propagates_an_expired_session_from_either_list() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock)
        .await;
    mount(&mock, "GET", "/Shows/NextUp", fixture("shows_next_up")).await;
    let err = catalog(&mock)
        .browse(&in_progress(BrowseSource::All(ContentKind::Movies)))
        .await
        .unwrap_err();
    assert_eq!(err, CoreError::SessionExpired);

    let mock = MockServer::start().await;
    mount(
        &mock,
        "GET",
        "/UserItems/Resume",
        fixture("user_items_resume"),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock)
        .await;
    let err = catalog(&mock)
        .browse(&in_progress(BrowseSource::All(ContentKind::Movies)))
        .await
        .unwrap_err();
    assert_eq!(err, CoreError::SessionExpired);
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
        .and(query_param("includeItemTypes", "Movie,Series,Video,BoxSet"))
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
async fn genres_of_a_library() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Genres"))
        .and(query_param("parentId", "lib2"))
        .and(query_param("includeItemTypes", "Series"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("genres")))
        .mount(&mock)
        .await;
    let genres = catalog(&mock)
        .genres(&BrowseSource::Library {
            id: ItemId::from("lib2"),
            kind: LibraryKind::Shows,
        })
        .await
        .unwrap();
    assert_eq!(genres, ["Animation", "Comedy", "Drama"]);
}

#[tokio::test]
async fn genres_of_all_movies() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Genres"))
        .and(query_param("includeItemTypes", "Movie"))
        .and(NoParam("parentId"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("genres")))
        .mount(&mock)
        .await;
    let genres = catalog(&mock)
        .genres(&BrowseSource::All(ContentKind::Movies))
        .await
        .unwrap();
    assert_eq!(genres, ["Animation", "Comedy", "Drama"]);
}

#[tokio::test]
async fn genres_of_favorites_cover_movies_series_and_videos() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Genres"))
        .and(query_param("includeItemTypes", "Movie,Series,Video"))
        .and(NoParam("parentId"))
        .and(NoParam("isFavorite"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("genres")))
        .mount(&mock)
        .await;
    let genres = catalog(&mock)
        .genres(&BrowseSource::Favorites)
        .await
        .unwrap();
    assert_eq!(genres.len(), 3);
}

#[tokio::test]
async fn browse_all_movies_has_no_parent() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("recursive", "true"))
        .and(query_param("includeItemTypes", "Movie"))
        .and(NoParam("parentId"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_movies")))
        .mount(&mock)
        .await;
    let page = catalog(&mock)
        .browse(&query(BrowseSource::All(ContentKind::Movies)))
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
}

#[tokio::test]
async fn browse_all_shows() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("recursive", "true"))
        .and(query_param("includeItemTypes", "Series"))
        .and(NoParam("parentId"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_search")))
        .mount(&mock)
        .await;
    let page = catalog(&mock)
        .browse(&query(BrowseSource::All(ContentKind::Shows)))
        .await
        .unwrap();
    assert!(page.total > 0);
}

#[tokio::test]
async fn search_sends_term_and_paging() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("searchTerm", "test"))
        .and(query_param("recursive", "true"))
        .and(query_param("includeItemTypes", "Movie,Series,BoxSet"))
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

#[tokio::test]
async fn similar_maps_summaries() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE_ID}/Similar")))
        .and(query_param("userId", TEST_USER_ID))
        .and(query_param("limit", "12"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("items_movies")))
        .expect(1)
        .mount(&mock)
        .await;
    let items = catalog(&mock)
        .similar(&ItemId::from(MOVIE_ID), 12)
        .await
        .unwrap();
    let titles: Vec<_> = items.iter().map(|item| item.title.as_str()).collect();
    assert_eq!(titles, ["Test Movie (2020)"]);
    assert_eq!(items[0].kind, ItemKind::Movie);
}

fn people_movie(people: Value) -> oxyfin_api::BaseItemDto {
    serde_json::from_value(json!({"Id": "m", "Name": "M", "Type": "Movie", "People": people}))
        .unwrap_or_else(|err| panic!("bad item: {err}"))
}

#[test]
fn cast_order_actors_first() {
    let dto = people_movie(json!([
        {"Name": "Writer One", "Id": "w1", "Type": "Writer"},
        {"Name": "Director One", "Id": "d1", "Type": "Director", "PrimaryImageTag": "t"},
        {"Name": "Actor One", "Id": "a1", "Role": "Hero", "Type": "Actor"},
        {"Name": "Grip One", "Type": "Grip"},
        {"Name": "Composer One", "Id": "c1", "Type": "Composer"},
        {"Name": "Actor Two", "Id": "a2", "Role": "", "Type": "GuestStar"},
        {"Name": "Producer One", "Id": "p1", "Type": "Producer"},
        {"Name": "Actor Three", "Id": "a3", "Role": "Sidekick", "Type": "Actor"}
    ]));
    let details = MovieDetails::from_dto(&dto).unwrap_or_else(|| panic!("not a movie"));
    let names: Vec<_> = details.cast.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Actor One",
            "Actor Two",
            "Actor Three",
            "Director One",
            "Writer One",
            "Composer One",
            "Producer One",
            "Grip One"
        ]
    );
    let kinds: Vec<_> = details.cast.iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        [
            PersonKind::Actor,
            PersonKind::Actor,
            PersonKind::Actor,
            PersonKind::Director,
            PersonKind::Writer,
            PersonKind::Composer,
            PersonKind::Producer,
            PersonKind::Other
        ]
    );
    assert_eq!(details.cast[0].id, Some(ItemId::from("a1")));
    assert_eq!(details.cast[0].role.as_deref(), Some("Hero"));
    assert_eq!(details.cast[1].role, None);
    assert_eq!(details.cast[7].id, None);
    let photo = details.cast[3]
        .image
        .as_ref()
        .unwrap_or_else(|| panic!("no photo"));
    assert_eq!(photo.item_id, ItemId::from("d1"));
    assert_eq!(photo.index, None);
}

const PERSON_ID: &str = "8db589eea9a68887bf01dfcc4020e817";

#[tokio::test]
async fn person_details_optional_fields() {
    let mock = MockServer::start().await;
    mount(
        &mock,
        "GET",
        &format!("/Items/{PERSON_ID}"),
        json!({
            "Id": PERSON_ID, "Name": "Song Kang-ho", "Type": "Person",
            "Overview": "A South Korean actor.",
            "PremiereDate": "1967-01-17T00:00:00.0000000Z",
            "ProductionLocations": ["Gimhae, South Korea"],
            "ImageTags": {"Primary": "tag"}
        }),
    )
    .await;
    mount(
        &mock,
        "GET",
        "/Items/bare",
        json!({"Id": "bare", "Name": "Nobody", "Type": "Person", "Overview": "", "ProductionLocations": [""]}),
    )
    .await;
    let catalog = catalog(&mock);
    let person = catalog.person(&ItemId::from(PERSON_ID)).await.unwrap();
    assert_eq!(person.id, ItemId::from(PERSON_ID));
    assert_eq!(person.name, "Song Kang-ho");
    assert_eq!(person.overview.as_deref(), Some("A South Korean actor."));
    assert_eq!(person.birth.as_deref(), Some("1967-01-17"));
    assert_eq!(person.birth_place.as_deref(), Some("Gimhae, South Korea"));
    let image = person.image.unwrap_or_else(|| panic!("no image"));
    assert_eq!(image.item_id, ItemId::from(PERSON_ID));
    assert_eq!(image.index, None);

    let bare = catalog.person(&ItemId::from("bare")).await.unwrap();
    assert_eq!(bare.name, "Nobody");
    assert_eq!(bare.overview, None);
    assert_eq!(bare.birth, None);
    assert_eq!(bare.birth_place, None);
    assert_eq!(bare.image, None);
}

#[tokio::test]
async fn person_titles_role_from_people() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("personIds", PERSON_ID))
        .and(query_param("recursive", "true"))
        .and(query_param("includeItemTypes", "Movie,Series"))
        .and(query_param("sortBy", "PremiereDate"))
        .and(query_param("sortOrder", "Descending"))
        .and(query_param("fields", "People"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Items": [
                {"Id": "s1", "Name": "A Show", "Type": "Series", "ProductionYear": 2021, "People": [
                    {"Name": "Someone", "Id": "x", "Role": "Other", "Type": "Actor"},
                    {"Name": "Song Kang-ho", "Id": PERSON_ID, "Role": "Detective", "Type": "Actor"}
                ]},
                {"Id": "m1", "Name": "A Movie", "Type": "Movie", "ProductionYear": 2003, "People": [
                    {"Name": "Song Kang-ho", "Id": PERSON_ID, "Role": "", "Type": "Director"}
                ]},
                {"Id": "m2", "Name": "No People", "Type": "Movie"},
                {"Id": "b1", "Name": "A Box Set", "Type": "BoxSet"}
            ],
            "TotalRecordCount": 4
        })))
        .expect(1)
        .mount(&mock)
        .await;
    let titles = catalog(&mock)
        .person_titles(&ItemId::from(PERSON_ID))
        .await
        .unwrap();
    let found: Vec<_> = titles
        .iter()
        .map(|title| (title.summary.title.as_str(), title.role.as_deref()))
        .collect();
    assert_eq!(
        found,
        [
            ("A Show", Some("Detective")),
            ("A Movie", None),
            ("No People", None)
        ]
    );
    assert_eq!(titles[0].summary.kind, ItemKind::Series);
}

const BOX_SET_ID: &str = "b0000000000000000000000000000001";
const OTHER_BOX_SET_ID: &str = "b0000000000000000000000000000002";

async fn mount_box_sets(mock: &MockServer, calls: u64) {
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("includeItemTypes", "BoxSet"))
        .and(query_param("recursive", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Items": [
                {"Id": OTHER_BOX_SET_ID, "Name": "Other Collection", "Type": "BoxSet"},
                {"Id": BOX_SET_ID, "Name": "Northbound Collection", "Type": "BoxSet"}
            ],
            "TotalRecordCount": 2
        })))
        .expect(calls)
        .mount(mock)
        .await;
    let children = [
        (
            OTHER_BOX_SET_ID,
            json!([{"Id": "m9", "Name": "Elsewhere", "Type": "Movie"}]),
        ),
        (
            BOX_SET_ID,
            json!([
                {"Id": "m1", "Name": "The Long Coast", "Type": "Movie"},
                {"Id": MOVIE_ID, "Name": "Northbound", "Type": "Movie"}
            ]),
        ),
    ];
    for (parent, items) in children {
        Mock::given(method("GET"))
            .and(path("/Items"))
            .and(query_param("parentId", parent))
            .and(query_param("enableImages", "false"))
            .and(query_param("enableUserData", "false"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"Items": items, "TotalRecordCount": 1})),
            )
            .expect(calls)
            .mount(mock)
            .await;
    }
}

#[tokio::test]
async fn collections_of_finds_box_set() {
    let mock = MockServer::start().await;
    mount_box_sets(&mock, 1).await;
    let catalog = catalog(&mock);
    let found = catalog
        .collections_of(&ItemId::from(MOVIE_ID))
        .await
        .unwrap();
    assert_eq!(
        found,
        [CollectionLink {
            id: ItemId::from(BOX_SET_ID),
            name: "Northbound Collection".to_owned(),
        }]
    );
    let none = catalog
        .collections_of(&ItemId::from("not-in-any"))
        .await
        .unwrap();
    assert!(none.is_empty());
}

#[tokio::test]
async fn collections_cached_once() {
    let mock = MockServer::start().await;
    mount_box_sets(&mock, 1).await;
    let catalog = catalog(&mock);
    let clone = catalog.clone();
    let (movie, other) = (ItemId::from(MOVIE_ID), ItemId::from("m9"));
    let (first, second) =
        tokio::join!(catalog.collections_of(&movie), clone.collections_of(&other));
    assert_eq!(first.unwrap().len(), 1);
    assert_eq!(second.unwrap()[0].name, "Other Collection");
    let again = catalog.clone().collections_of(&ItemId::from("m1")).await;
    assert_eq!(again.unwrap()[0].id, ItemId::from(BOX_SET_ID));
}

#[tokio::test]
async fn collections_not_cached_after_failure() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("includeItemTypes", "BoxSet"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&mock)
        .await;
    mount_box_sets(&mock, 1).await;
    let catalog = catalog(&mock);
    assert!(
        catalog
            .collections_of(&ItemId::from(MOVIE_ID))
            .await
            .is_err()
    );
    let found = catalog.collections_of(&ItemId::from(MOVIE_ID)).await;
    assert_eq!(found.unwrap().len(), 1);
}

#[tokio::test]
async fn collection_orders_by_premiere() {
    let mock = MockServer::start().await;
    mount(
        &mock,
        "GET",
        &format!("/Items/{BOX_SET_ID}"),
        json!({
            "Id": BOX_SET_ID, "Name": "Northbound Collection", "Type": "BoxSet",
            "Overview": "Expeditions to the north.",
            "UserData": {"Played": false, "IsFavorite": true, "PlaybackPositionTicks": 0}
        }),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("parentId", BOX_SET_ID))
        .and(query_param("sortBy", "PremiereDate"))
        .and(query_param("sortOrder", "Ascending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Items": [
                {"Id": "m3", "Name": "The Surveyor", "Type": "Movie", "ProductionYear": 2024,
                 "PremiereDate": "2024-02-01T00:00:00.0000000Z"},
                {"Id": "m1", "Name": "The Long Coast", "Type": "Movie", "ProductionYear": 2019,
                 "PremiereDate": "2019-05-01T00:00:00.0000000Z",
                 "UserData": {"Played": true, "IsFavorite": false, "PlaybackPositionTicks": 0}},
                {"Id": "m4", "Name": "Undated", "Type": "Movie"},
                {"Id": "m2", "Name": "Northbound", "Type": "Movie", "ProductionYear": 2023,
                 "PremiereDate": "2023-03-01T00:00:00.0000000Z"}
            ],
            "TotalRecordCount": 4
        })))
        .expect(1)
        .mount(&mock)
        .await;
    let details = catalog(&mock)
        .collection(&ItemId::from(BOX_SET_ID))
        .await
        .unwrap();
    assert_eq!(details.summary.kind, ItemKind::BoxSet);
    assert_eq!(details.summary.title, "Northbound Collection");
    assert!(details.summary.user.favorite);
    assert_eq!(
        details.overview.as_deref(),
        Some("Expeditions to the north.")
    );
    let titles: Vec<_> = details
        .items
        .iter()
        .map(|item| item.title.as_str())
        .collect();
    assert_eq!(
        titles,
        ["The Long Coast", "Northbound", "The Surveyor", "Undated"]
    );
    assert_eq!(details.watched, 1);
    assert_eq!(details.first_unwatched, Some(ItemId::from("m2")));
}

#[tokio::test]
async fn collection_fully_watched_has_no_first_unwatched() {
    let mock = MockServer::start().await;
    mount(
        &mock,
        "GET",
        &format!("/Items/{BOX_SET_ID}"),
        json!({"Id": BOX_SET_ID, "Name": "Done", "Type": "BoxSet"}),
    )
    .await;
    mount(
        &mock,
        "GET",
        "/Items",
        json!({"Items": [
            {"Id": "m1", "Name": "Seen", "Type": "Movie",
             "UserData": {"Played": true, "IsFavorite": false, "PlaybackPositionTicks": 0}}
        ], "TotalRecordCount": 1}),
    )
    .await;
    let details = catalog(&mock)
        .collection(&ItemId::from(BOX_SET_ID))
        .await
        .unwrap();
    assert_eq!(details.watched, 1);
    assert_eq!(details.first_unwatched, None);
}

#[tokio::test]
async fn search_includes_box_sets() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("searchTerm", "north"))
        .and(query_param("includeItemTypes", "Movie,Series,BoxSet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Items": [
                {"Id": BOX_SET_ID, "Name": "Northbound Collection", "Type": "BoxSet"},
                {"Id": MOVIE_ID, "Name": "Northbound", "Type": "Movie"}
            ],
            "TotalRecordCount": 2
        })))
        .expect(1)
        .mount(&mock)
        .await;
    let page = catalog(&mock).search("north", 0, 20).await.unwrap();
    let kinds: Vec<_> = page.items.iter().map(|item| item.kind).collect();
    assert_eq!(kinds, [ItemKind::BoxSet, ItemKind::Movie]);
}

#[tokio::test]
async fn collections_of_reads_every_box_set() {
    let mock = MockServer::start().await;
    let ids: Vec<String> = (0..9).map(|n| format!("b{n:031}")).collect();
    let sets: Vec<Value> = ids
        .iter()
        .map(|id| json!({"Id": id, "Name": format!("Set {id}"), "Type": "BoxSet"}))
        .collect();
    mount(
        &mock,
        "GET",
        "/Items",
        json!({"Items": sets, "TotalRecordCount": 9}),
    )
    .await;
    for (n, id) in ids.iter().enumerate() {
        Mock::given(method("GET"))
            .and(path("/Items"))
            .and(query_param("parentId", id.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Items": [{"Id": format!("m{n}"), "Name": "Movie", "Type": "Movie"}],
                "TotalRecordCount": 1
            })))
            .expect(1)
            .with_priority(1)
            .mount(&mock)
            .await;
    }
    let catalog = catalog(&mock);
    for (n, id) in ids.iter().enumerate() {
        let found = catalog
            .collections_of(&ItemId::from(format!("m{n}").as_str()))
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, ItemId::from(id.as_str()));
    }
}

#[test]
fn cast_keeps_the_crew_of_a_long_cast() {
    let mut people: Vec<Value> = (0..30)
        .map(|n| json!({"Name": format!("Actor {n}"), "Id": format!("a{n}"), "Type": "Actor"}))
        .collect();
    people.push(json!({"Name": "The Director", "Id": "d", "Type": "Director"}));
    let details = MovieDetails::from_dto(&people_movie(Value::Array(people)))
        .unwrap_or_else(|| panic!("not a movie"));
    assert_eq!(details.cast.len(), 20);
    assert_eq!(details.cast[18].name, "Actor 18");
    assert_eq!(details.cast[19].name, "The Director");
    assert_eq!(details.cast[19].kind, PersonKind::Director);
}

#[test]
fn cast_joins_the_credits_of_one_person() {
    let dto = people_movie(json!([
        {"Name": "Ada Lind", "Id": "a1", "Role": "Captain", "Type": "Actor"},
        {"Name": "Ada Lind", "Id": "a1", "Role": "Director", "Type": "Director"},
        {"Name": "Ada Lind", "Id": "a1", "Role": "Narrator", "Type": "Actor"},
        {"Name": "Karl Vinter", "Id": "w1", "Type": "Writer"},
        {"Name": "Karl Vinter", "Id": "w1", "Type": "Producer"},
        {"Name": "Unnamed", "Type": "Actor"},
        {"Name": "Unnamed", "Type": "Actor"}
    ]));
    let details = MovieDetails::from_dto(&dto).unwrap_or_else(|| panic!("not a movie"));
    let names: Vec<_> = details.cast.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Ada Lind", "Unnamed", "Unnamed", "Karl Vinter"]);
    let ada = &details.cast[0];
    assert_eq!(ada.kind, PersonKind::Actor);
    assert_eq!(ada.role.as_deref(), Some("Captain / Narrator"));
    assert_eq!(ada.also, Some(PersonKind::Director));
    let karl = &details.cast[3];
    assert_eq!(karl.kind, PersonKind::Writer);
    assert_eq!(karl.also, None);
}

/// Box set listings, then the items of the two box sets, the second answering `status`.
async fn mount_failing_box_set(mock: &MockServer, status: u16) {
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("includeItemTypes", "BoxSet"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Items": [
                {"Id": BOX_SET_ID, "Name": "Northbound Collection", "Type": "BoxSet"},
                {"Id": OTHER_BOX_SET_ID, "Name": "Other Collection", "Type": "BoxSet"}
            ],
            "TotalRecordCount": 2
        })))
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("parentId", BOX_SET_ID))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Items": [{"Id": MOVIE_ID, "Name": "Northbound", "Type": "Movie"}],
            "TotalRecordCount": 1
        })))
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("parentId", OTHER_BOX_SET_ID))
        .respond_with(ResponseTemplate::new(status))
        .expect(1)
        .mount(mock)
        .await;
}

#[tokio::test]
async fn collections_skip_a_box_set_whose_items_fail() {
    let mock = MockServer::start().await;
    mount_failing_box_set(&mock, 500).await;
    let catalog = catalog(&mock);
    let found = catalog
        .collections_of(&ItemId::from(MOVIE_ID))
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, ItemId::from(BOX_SET_ID));
    let none = catalog.collections_of(&ItemId::from("m9")).await.unwrap();
    assert!(none.is_empty());
}

#[tokio::test]
async fn collections_end_the_session_when_a_box_set_answers_401() {
    let mock = MockServer::start().await;
    mount_failing_box_set(&mock, 401).await;
    let result = catalog(&mock).collections_of(&ItemId::from(MOVIE_ID)).await;
    assert_eq!(result, Err(CoreError::SessionExpired));
}

#[tokio::test]
async fn collections_listed_again_after_half_an_hour() {
    let mock = MockServer::start().await;
    mount_box_sets(&mock, 2).await;
    let catalog = catalog(&mock);
    let movie = ItemId::from(MOVIE_ID);
    assert_eq!(catalog.collections_of(&movie).await.unwrap().len(), 1);
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(20 * 60)).await;
    tokio::time::resume();
    assert_eq!(catalog.collections_of(&movie).await.unwrap().len(), 1);
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(11 * 60)).await;
    tokio::time::resume();
    assert_eq!(catalog.collections_of(&movie).await.unwrap().len(), 1);
}

#[tokio::test]
async fn authorize_device_sends_the_code_without_spaces() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/QuickConnect/Authorize"))
        .and(query_param("code", "482913"))
        .respond_with(ResponseTemplate::new(200).set_body_json(true))
        .expect(1)
        .mount(&mock)
        .await;
    assert_eq!(catalog(&mock).authorize_device(" 482 913 ").await, Ok(true));
}

#[tokio::test]
async fn authorize_device_reports_a_wrong_code_and_a_disabled_server() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/QuickConnect/Authorize"))
        .and(query_param("code", "000000"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/QuickConnect/Authorize"))
        .and(query_param("code", "111111"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock)
        .await;
    mount(&mock, "GET", "/QuickConnect/Enabled", json!(false)).await;
    assert_eq!(catalog(&mock).authorize_device("000000").await, Ok(false));
    assert_eq!(
        catalog(&mock).authorize_device("111111").await,
        Err(CoreError::QuickConnectDisabled)
    );
}

#[tokio::test]
async fn authorize_device_with_an_empty_code_asks_nothing() {
    let mock = MockServer::start().await;
    assert_eq!(catalog(&mock).authorize_device("  ").await, Ok(false));
    assert!(
        mock.received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}
