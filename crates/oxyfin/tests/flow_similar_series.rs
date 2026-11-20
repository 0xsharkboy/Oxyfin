//! "More like this" on the series page: the row lists its items and opens them.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{
    click, fixture, json, mount_browse_basics, pause, run, save_session, start_with, wait_role,
    wait_until, wait_visible,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const SECOND: &str = "b4c9a0c1d2e34f5a8b6c7d8e9f0a1b2c";
const THIRD: &str = "c5d0b1d2e3f45a6b9c7d8e9f0a1b2c3d";

struct Counted {
    calls: Arc<AtomicUsize>,
    response: ResponseTemplate,
}

impl Respond for Counted {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.response.clone()
    }
}

fn scroll_down(window: &AppWindow) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(640., 400.),
            delta_x: 0.,
            delta_y: -800.,
        });
}

#[test]
fn series_similar_row_opens_item() {
    let similar = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let (similar_served, second_served) = (Arc::clone(&similar), Arc::clone(&second));
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, &fixture("next_up_series")).await;
            let similar_items = format!(
                r#"{{"Items":[
                    {{"Id":"{SECOND}","Name":"Second Feature (2021)","Type":"Movie","ProductionYear":2021}},
                    {{"Id":"{THIRD}","Name":"Third Show","Type":"Series","ProductionYear":2019}}
                ],"TotalRecordCount":2,"StartIndex":0}}"#
            );
            let second_item = fixture("item_movie")
                .replace(MOVIE, SECOND)
                .replace("A generated test movie.", "The second test movie.");
            let routes = [
                ("/Items".to_owned(), json(fixture("items_shows"))),
                (format!("/Items/{SERIES}"), json(fixture("item_series"))),
                (format!("/Shows/{SERIES}/Seasons"), json(fixture("seasons"))),
                (
                    format!("/Shows/{SERIES}/Episodes"),
                    json(fixture("episodes_season_2")),
                ),
            ];
            for (route, response) in routes {
                Mock::given(method("GET"))
                    .and(path(route))
                    .respond_with(response)
                    .mount(&mock)
                    .await;
            }
            Mock::given(method("GET"))
                .and(path(format!("/Items/{SERIES}/Similar")))
                .respond_with(Counted {
                    calls: similar_served,
                    response: json(similar_items),
                })
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Items/{SECOND}")))
                .respond_with(Counted {
                    calls: second_served,
                    response: json(second_item),
                })
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
        let shows = wait_role(&window, "Shows", AccessibleRole::Tab).await?;
        click(&window, &shows);
        let card = wait_visible(&window, "Test Show").await?;
        click(&window, &card);
        wait_until("the series", || window.get_screen() == Screen::Series).await?;
        wait_visible(&window, "A generated test show.").await?;
        wait_until("the similar items", || similar.load(Ordering::SeqCst) == 1).await?;
        for _ in 0..4 {
            scroll_down(&window);
            pause(Duration::from_millis(20)).await;
        }
        let row = wait_role(&window, "More like this", AccessibleRole::List).await?;
        let top = row.absolute_position().y;
        let bottom = top + row.size().height;
        let titles: Vec<String> = ElementHandle::find_by_element_type_name(&window, "PosterCard")
            .filter(|card| {
                let y = card.absolute_position().y;
                card.size().height > 0. && y >= top && y + card.size().height <= bottom
            })
            .filter_map(|card| card.accessible_label().map(|label| label.to_string()))
            .collect();
        if titles != ["Second Feature (2021)", "Third Show"] {
            return Err(format!("unexpected cards: {titles:?}"));
        }

        let card = wait_role(&window, "Second Feature (2021)", AccessibleRole::Button).await?;
        click(&window, &card);
        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;
        wait_until("the second movie", || second.load(Ordering::SeqCst) == 1).await?;
        wait_visible(&window, "The second test movie.").await?;
        Ok(())
    });
}
