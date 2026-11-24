//! The app opens on the home screen of its last run while the server answers, and signing out
//! leaves nothing of it.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{USER_ID, click, fixture, json, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use oxyfin_api::{ClientInfo, Server, UserId};
use oxyfin_core::Catalog;
use slint::{ComponentHandle, Model};
use wiremock::matchers::{any, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mount_home(mock: &MockServer, delay: Duration) {
    for (route, body) in [
        ("/UserViews", fixture("user_views")),
        ("/UserItems/Resume", fixture("user_items_resume")),
        ("/Shows/NextUp", fixture("shows_next_up")),
        ("/Items/Latest", fixture("items_latest")),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body).set_delay(delay))
            .mount(mock)
            .await;
    }
    Mock::given(any())
        .respond_with(ResponseTemplate::new(404))
        .with_priority(10)
        .mount(mock)
        .await;
}

#[test]
fn the_app_opens_on_the_home_screen_of_its_last_run() {
    let saved: Arc<Mutex<Option<PathBuf>>> = Arc::default();
    let file = Arc::clone(&saved);
    let flow = start_with(
        |mock| async move {
            mount_home(&mock, Duration::from_millis(1500)).await;
            mock
        },
        move |paths, vault, url| {
            save_session(paths, vault, url);
            // The last run, against a server that answered.
            let runtime = common::runtime();
            runtime.block_on(async {
                let earlier = MockServer::start().await;
                mount_home(&earlier, Duration::ZERO).await;
                let session = Server::new(&earlier.uri(), ClientInfo::new("Test", "id"))
                    .unwrap()
                    .restore_session("token", UserId::from(USER_ID), None);
                let catalog = Catalog::on_disk(session, paths);
                catalog.continue_watching(12).await.unwrap();
                catalog.next_up(12).await.unwrap();
                for library in catalog.libraries().await.unwrap() {
                    catalog.latest(&library.id, 16).await.unwrap();
                }
                catalog.save_answers();
                let pages = paths.cache_dir.join("pages.json");
                let text = std::fs::read_to_string(&pages).unwrap();
                let here = text.replace(&earlier.uri(), url.trim_end_matches('/'));
                std::fs::write(&pages, here).unwrap();
                *file.lock().unwrap() = Some(pages);
            });
        },
    );
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        let pages = saved
            .lock()
            .map_err(|_| "poisoned")?
            .clone()
            .ok_or("no file")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        if window.get_home_loading() || !window.get_home_banner().has {
            return Err("the home screen waits for the server".into());
        }
        let rows: Vec<String> = window
            .get_home_rows()
            .iter()
            .map(|row| row.title.to_string())
            .collect();
        let expected = [
            "Continue Watching",
            "Next Up",
            "Recently Added in Movies",
            "Recently Added in Shows",
        ];
        if rows != expected {
            return Err(format!("unexpected rows {rows:?}"));
        }
        wait_until("the new answers", || window.get_home_settled()).await?;
        let after: Vec<String> = window
            .get_home_rows()
            .iter()
            .map(|row| row.title.to_string())
            .collect();
        if after != expected || !window.get_home_banner().has {
            return Err(format!("the same answers changed the screen: {after:?}"));
        }

        window.invoke_open_settings();
        wait_until("the settings", || window.get_screen() == Screen::Settings).await?;
        let sign_out = wait_role(&window, "Sign out", AccessibleRole::Button).await?;
        click(&window, &sign_out);
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        tokio::time::sleep(Duration::from_millis(1300)).await;
        if pages.exists() {
            return Err("signing out left the last pages on disk".into());
        }
        Ok(())
    });
}
