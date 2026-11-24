//! A server whose movies, delay and failures a flow changes while the app runs.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, Screen};
use slint::Model;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::{click, has_param, json, mount_browse_basics, wait_role, wait_until};

pub const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

/// How long a slow answer takes; what it answers is what the server held when asked.
pub const SLOW: Duration = Duration::from_millis(900);

/// What the server holds and how it answers.
#[derive(Default)]
pub struct Held {
    /// Numbers of the movies, in the order of the library.
    pub movies: Vec<usize>,
    /// Numbers of the movies played.
    pub played: Vec<usize>,
    /// Listings take [`SLOW`] to answer.
    pub slow: bool,
    /// Listings answer with an error.
    pub failing: bool,
    /// Movies whose poster is listed.
    pub posters: bool,
    /// Paths and queries asked for.
    pub asked: Vec<String>,
}

#[derive(Clone, Default)]
pub struct Answers(Arc<Mutex<Held>>);

pub fn id(number: usize) -> String {
    format!("{number:032x}")
}

impl Answers {
    pub fn holding(movies: &[usize]) -> Self {
        let answers = Self::default();
        answers.held().movies = movies.to_vec();
        answers
    }

    pub fn held(&self) -> MutexGuard<'_, Held> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Number of requests so far whose path and query hold every part of `parts`.
    pub fn asked(&self, parts: &[&str]) -> usize {
        self.held()
            .asked
            .iter()
            .filter(|asked| parts.iter().all(|part| asked.contains(part)))
            .count()
    }

    fn movie(held: &Held, number: usize) -> serde_json::Value {
        let mut movie = serde_json::json!({
            "Name": format!("Movie {number}"), "Id": id(number), "Type": "Movie",
            "IsFolder": false, "ProductionYear": 2020,
            "UserData": {"PlaybackPositionTicks": 0, "PlayCount": 0, "IsFavorite": false,
                "Played": held.played.contains(&number)}
        });
        if held.posters {
            movie["ImageTags"] = serde_json::json!({"Primary": "tag"});
        }
        movie
    }

    /// Mounts the server on `mock`.
    pub async fn mount(self, mock: MockServer) -> MockServer {
        Mock::given(method("GET"))
            .and(path("/Items"))
            .respond_with(Listing(self.clone()))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("^/Items/[0-9a-f]{32}$"))
            .respond_with(One(self.clone()))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("^/Items/[0-9a-f]{32}/Images/Primary$"))
            .respond_with(Counted(self.clone(), ResponseTemplate::new(404)))
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path_regex("^/UserPlayedItems/[0-9a-f]{32}$"))
            .respond_with(MarkPlayed(self.clone()))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path_regex("^/Items/[0-9a-f]{32}/Similar$"))
            .respond_with(json(EMPTY))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path("/Genres"))
            .respond_with(Counted(self.clone(), json(super::fixture("genres"))))
            .mount(&mock)
            .await;
        mount_browse_basics(&mock, EMPTY).await;
        mock
    }
}

impl Answers {
    fn log(&self, request: &Request) {
        let query = request.url.query().unwrap_or("");
        self.held()
            .asked
            .push(format!("{}?{query}", request.url.path()));
    }
}

struct Listing(Answers);

impl Respond for Listing {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.0.log(request);
        let held = self.0.held();
        if held.failing {
            return ResponseTemplate::new(500);
        }
        let query = request.url.query().unwrap_or("");
        let movies = has_param(query, "includeItemTypes", "Movie");
        let all: &[usize] = if movies { &held.movies } else { &[] };
        let param = |key: &str| {
            query.split('&').find_map(|pair| {
                pair.strip_prefix(&format!("{key}="))
                    .and_then(|value| value.parse::<usize>().ok())
            })
        };
        let start = param("startIndex").unwrap_or(0).min(all.len());
        let end = (start + param("limit").unwrap_or(all.len())).min(all.len());
        let items: Vec<serde_json::Value> = all[start..end]
            .iter()
            .map(|number| Answers::movie(&held, *number))
            .collect();
        let answer = json(
            serde_json::json!({"Items": items, "TotalRecordCount": all.len(), "StartIndex": start})
                .to_string(),
        );
        match held.slow {
            true => answer.set_delay(SLOW),
            false => answer,
        }
    }
}

struct One(Answers);

impl Respond for One {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.0.log(request);
        let held = self.0.held();
        let wanted = request.url.path().rsplit('/').next().unwrap_or("");
        let answer = match held.movies.iter().find(|number| id(**number) == wanted) {
            Some(number) => json(Answers::movie(&held, *number).to_string()),
            None => ResponseTemplate::new(404),
        };
        match held.slow {
            true => answer.set_delay(SLOW),
            false => answer,
        }
    }
}

struct Counted(Answers, ResponseTemplate);

impl Respond for Counted {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.0.log(request);
        self.1.clone()
    }
}

struct MarkPlayed(Answers);

impl Respond for MarkPlayed {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let wanted = request.url.path().rsplit('/').next().unwrap_or("");
        let mut held = self.0.held();
        let number = held.movies.iter().copied().find(|n| id(*n) == wanted);
        held.played.extend(number);
        json(r#"{"PlaybackPositionTicks":0,"PlayCount":1,"IsFavorite":false,"Played":true}"#)
    }
}

/// Titles of the library cards.
pub fn titles(window: &AppWindow) -> Vec<String> {
    window
        .get_library_cards()
        .iter()
        .map(|card| card.title.to_string())
        .collect()
}

/// Whether the card of movie `number` has its watched badge; `None` without that card.
pub fn watched(window: &AppWindow, number: usize) -> Option<bool> {
    window
        .get_library_cards()
        .iter()
        .find(|card| card.id == id(number).as_str())
        .map(|card| card.watched)
}

/// Clicks the tab `label` of the top bar.
pub async fn open_tab(window: &AppWindow, label: &str) -> Result<(), String> {
    let tab = wait_role(window, label, AccessibleRole::Tab).await?;
    click(window, &tab);
    Ok(())
}

/// Waits for the home screen, then opens Movies and waits for its cards.
pub async fn visit_movies(window: &AppWindow) -> Result<(), String> {
    wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
    open_tab(window, "Movies").await?;
    wait_until("the movies", || !titles(window).is_empty()).await
}
