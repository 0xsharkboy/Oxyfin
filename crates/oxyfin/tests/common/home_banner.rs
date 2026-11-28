//! A home screen with one featured item, from Continue Watching or from a library.

use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer};

use super::{fixture, json, mount_me};

const MOVIES: &str = "f137a2dd21bbc1b99aa5c0f6bf02a805";
const SHOWS: &str = "a656b907eb3a73532e40e44b968d0225";
/// The series of [`EPISODE_TO_RESUME`].
pub const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
/// An empty list.
pub const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

/// An episode 24 minutes long, 12 of them watched.
pub const EPISODE_TO_RESUME: &str = r#"{"Items":[{"Name":"Sword","Id":"0407e3d02b7d70abe4a2d37241b02fea",
    "Type":"Episode","IsFolder":false,"RunTimeTicks":14400000000,"ParentIndexNumber":1,"IndexNumber":2,
    "SeriesName":"Northern Saga","SeriesId":"9ea196e7ccc3ae17d2c3f75c3ab34406",
    "Overview":"A raid on the coast.",
    "UserData":{"PlayedPercentage":50,"PlaybackPositionTicks":7200000000,"PlayCount":1,
    "IsFavorite":false,"Played":false}}],"TotalRecordCount":1,"StartIndex":0}"#;

/// A movie 100 minutes long, never started, with a backdrop.
pub const RECENT_MOVIE: &str = r#"[{"Name":"Fresh Arrival","Id":"76282b1c408f374beeee50519e9157a4",
    "Type":"Movie","IsFolder":false,"RunTimeTicks":60000000000,"ProductionYear":2020,
    "UserData":{"PlaybackPositionTicks":0,"PlayCount":0,"IsFavorite":false,"Played":false},
    "ImageTags":{},"BackdropImageTags":["b1"]}]"#;

/// Mounts a home screen whose resume list is `resume` and whose movies are `latest`.
pub async fn mount_banner(
    mock: MockServer,
    resume: &'static str,
    latest: &'static str,
) -> MockServer {
    mount_me(&mock).await;
    Mock::given(method("GET"))
        .and(path("/UserViews"))
        .respond_with(json(fixture("user_views")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .and(query_param("fields", "Overview"))
        .respond_with(json(resume))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .respond_with(json(EMPTY))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .and(query_param("parentId", MOVIES))
        .respond_with(json(latest))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .and(query_param("parentId", SHOWS))
        .respond_with(json("[]"))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{SERIES}")))
        .respond_with(json(fixture("item_series")))
        .mount(&mock)
        .await;
    mock
}
