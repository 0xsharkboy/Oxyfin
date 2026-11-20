//! Coming back to a grid keeps its scroll position, filter and cards, without asking the server again.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    QueryLog, click, fixture, has_param, json, mount_browse_basics, pause, run, save_session,
    start_with, wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, CardData, Screen, TopBarPage};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, VecModel};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const TOTAL: usize = 120;

fn page(start: usize) -> String {
    let items: Vec<String> = (start..start + 60)
        .map(|index| {
            format!(
                r#"{{"Name":"Movie {index:03}","Id":"{index:032x}","Type":"Movie","IsFolder":false,"ProductionYear":2020,
                "UserData":{{"PlaybackPositionTicks":0,"PlayCount":0,"IsFavorite":false,"Played":false}}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"Items":[{}],"TotalRecordCount":{TOTAL},"StartIndex":{start}}}"#,
        items.join(",")
    )
}

struct Items {
    log: QueryLog,
}

impl Respond for Items {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("").to_owned();
        let start = if has_param(&query, "startIndex", "60") {
            60
        } else {
            0
        };
        self.log.lock().map(|mut log| log.push(query)).ok();
        json(page(start))
    }
}

/// Listing requests, leaving out the box sets the movie page looks up.
fn requests(log: &QueryLog) -> usize {
    log.lock()
        .map(|log| {
            log.iter()
                .filter(|query| !has_param(query, "includeItemTypes", "BoxSet"))
                .count()
        })
        .unwrap_or(0)
}

fn searches(log: &QueryLog) -> usize {
    log.lock()
        .map(|log| {
            log.iter()
                .filter(|query| query.contains("searchTerm="))
                .count()
        })
        .unwrap_or(0)
}

fn scroll_down(window: &AppWindow) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(640., 400.),
            delta_x: 0.,
            delta_y: -20000.,
        });
}

fn press(window: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn near(left: f32, right: f32) -> bool {
    (left - right).abs() < 0.5
}

#[test]
fn coming_back_keeps_the_grid_where_it_was() {
    let log: QueryLog = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::clone(&log);
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(Items { log: served })
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path_regex(r"^/Items/[0-9a-f]{32}$"))
                .respond_with(json(fixture("item_movie")))
                .mount(&mock)
                .await;
            mock
        },
        save_session,
    );
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let movies = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &movies);
        wait_visible(&window, "120 titles").await?;

        let unwatched = wait_role(&window, "Unwatched", AccessibleRole::Button).await?;
        click(&window, &unwatched);
        wait_until("the filtered page", || requests(&log) == 2).await?;
        wait_visible(&window, "Movie 000").await?;

        scroll_down(&window);
        wait_until("the second page", || {
            window.get_library_cards().row_count() == TOTAL
        })
        .await?;
        scroll_down(&window);
        pause(Duration::from_millis(300)).await;
        let scrolled = window.get_library_viewport_y();
        if scrolled >= 0. {
            return Err(format!("the grid did not scroll: {scrolled}"));
        }
        let asked = requests(&log);

        let card = wait_visible(&window, "Movie 119").await?;
        click(&window, &card);
        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;
        wait_visible(&window, "Drama").await?;
        press(&window, Key::Escape);
        wait_until("the library", || window.get_screen() == Screen::Library).await?;
        wait_visible(&window, "Movie 119").await?;
        pause(Duration::from_millis(300)).await;
        if !near(window.get_library_viewport_y(), scrolled) {
            return Err(format!(
                "the position changed from {scrolled} to {}",
                window.get_library_viewport_y()
            ));
        }
        if window.get_library_watch() != 1 || window.get_library_cards().row_count() != TOTAL {
            return Err("the filter or the cards were not kept".into());
        }
        if requests(&log) != asked {
            return Err("coming back asked the server again".into());
        }

        let settings = wait_role(&window, "Settings", AccessibleRole::Button).await?;
        click(&window, &settings);
        wait_until("the settings", || window.get_screen() == Screen::Settings).await?;
        press(&window, Key::Escape);
        wait_until("the library", || window.get_screen() == Screen::Library).await?;
        wait_visible(&window, "Movie 119").await?;
        pause(Duration::from_millis(300)).await;
        if !near(window.get_library_viewport_y(), scrolled) || requests(&log) != asked {
            return Err("the settings round trip moved the grid or reloaded it".into());
        }

        // The new tab starts at the top even though the old one was scrolled.
        let shows = wait_role(&window, "Shows", AccessibleRole::Tab).await?;
        click(&window, &shows);
        wait_until("the shows", || {
            window.get_top_bar_page() == TopBarPage::Shows
        })
        .await?;
        pause(Duration::from_millis(300)).await;
        if window.get_library_viewport_y() != 0. {
            return Err(format!(
                "the other tab starts at {}",
                window.get_library_viewport_y()
            ));
        }

        let movies = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &movies);
        wait_visible(&window, "120 titles").await?;
        scroll_down(&window);
        wait_until("the second page again", || {
            window.get_library_cards().row_count() == TOTAL
        })
        .await?;
        scroll_down(&window);
        pause(Duration::from_millis(300)).await;
        if window.get_library_viewport_y() >= 0. {
            return Err("the grid did not scroll before shrinking".into());
        }

        // A bound position is not clamped by the Flickable when the content shrinks.
        let few: Vec<CardData> = (0..5)
            .map(|index| CardData {
                title: format!("Few {index}").into(),
                ..CardData::default()
            })
            .collect();
        window.set_library_cards(ModelRc::new(VecModel::from(few)));
        wait_until("the shrunk grid", || window.get_library_viewport_y() == 0.).await?;

        let magnifier = wait_role(&window, "Search", AccessibleRole::Button).await?;
        click(&window, &magnifier);
        wait_role(&window, "Search titles", AccessibleRole::TextInput).await?;
        for character in "mov".chars() {
            press(&window, character.to_string());
        }
        press(&window, Key::Return);
        wait_visible(&window, "Movie 000").await?;
        scroll_down(&window);
        wait_until("the second page of results", || {
            window.get_search_cards().row_count() == TOTAL
        })
        .await?;
        scroll_down(&window);
        pause(Duration::from_millis(300)).await;
        let found = window.get_search_viewport_y();
        if found >= 0. {
            return Err(format!("the results did not scroll: {found}"));
        }
        let searched = searches(&log);

        let settings = wait_role(&window, "Settings", AccessibleRole::Button).await?;
        click(&window, &settings);
        wait_until("the settings", || window.get_screen() == Screen::Settings).await?;
        press(&window, Key::Escape);
        wait_until("the search", || window.get_screen() == Screen::Search).await?;
        wait_visible(&window, "Movie 119").await?;
        pause(Duration::from_millis(300)).await;
        if !near(window.get_search_viewport_y(), found) || searches(&log) != searched {
            return Err("the results moved or were searched again".into());
        }

        let movies = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &movies);
        wait_until("the library", || window.get_screen() == Screen::Library).await?;
        pause(Duration::from_millis(300)).await;
        if window.get_search_viewport_y() != 0. {
            return Err("the search position was kept after leaving the search".into());
        }
        Ok(())
    });
}
