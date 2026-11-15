//! A failed next page shows a message with a retry, and the retry asks for the same page.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    QueryLog, click, exists, has_param, json, mount_browse_basics, pause, run, save_session,
    start_with, wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, Screen};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const TOTAL: usize = 70;

fn page(start: usize, count: usize) -> String {
    let items: Vec<String> = (start..start + count)
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
    second_page_calls: Arc<AtomicUsize>,
}

impl Respond for Items {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("").to_owned();
        self.log.lock().map(|mut log| log.push(query.clone())).ok();
        if !has_param(&query, "startIndex", "60") {
            return json(page(0, 60));
        }
        if self.second_page_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            ResponseTemplate::new(500)
        } else {
            json(page(60, TOTAL - 60))
        }
    }
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

#[test]
fn a_failed_next_page_offers_a_retry() {
    let log: QueryLog = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::clone(&log);
    let calls = Arc::new(AtomicUsize::new(0));
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(Items {
                    log: served,
                    second_page_calls: calls,
                })
                .mount(&mock)
                .await;
            mock
        },
        save_session,
    );
    let window = flow.window.as_weak();
    let count = move |log: &QueryLog, start: &str| {
        log.lock()
            .map(|log| {
                log.iter()
                    .filter(|query| has_param(query, "startIndex", start))
                    .count()
            })
            .unwrap_or(0)
    };

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let movies = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &movies);
        wait_visible(&window, "70 titles").await?;
        wait_visible(&window, "Movie 000").await?;

        scroll_down(&window);
        wait_visible(&window, "Cannot load more titles").await?;
        if count(&log, "60") != 1 {
            return Err("the second page was not requested once".into());
        }
        scroll_down(&window);
        pause(Duration::from_millis(300)).await;
        if count(&log, "60") != 1 {
            return Err("the failed page is asked again without a click".into());
        }
        if exists(&window, "Movie 061") {
            return Err("a card of the failed page is shown".into());
        }

        let retry = wait_role(&window, "Retry", AccessibleRole::Button).await?;
        click(&window, &retry);
        wait_visible(&window, "Movie 061").await?;
        if count(&log, "60") != 2 {
            return Err("the retry did not ask for the same page".into());
        }
        if exists(&window, "Cannot load more titles") {
            return Err("the failure message is left after the retry".into());
        }
        Ok(())
    });
}
