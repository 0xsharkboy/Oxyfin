//! Opening a title from a card of the same kind of screen holds nothing: the card pointed at is
//! gone with the page it was on.

mod common;

use common::answers::{Answers, id, visit_movies};
use common::{click, json, run, save_session, start_with, wait_role, wait_until, wait_visible};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;
use wiremock::Mock;
use wiremock::matchers::{method, path_regex};

#[test]
fn a_title_opened_from_a_similar_card_holds_nothing() {
    let server = Answers::holding(&[1, 2, 3]);
    let movie = |number: usize| {
        serde_json::json!({
            "Name": format!("Like {number}"), "Id": id(number), "Type": "Movie",
            "IsFolder": false, "ProductionYear": 2020
        })
    };
    let similar = serde_json::json!({"Items": [movie(2), movie(3)], "TotalRecordCount": 2});
    let flow = start_with(
        move |mock| async move {
            Mock::given(method("GET"))
                .and(path_regex("^/Items/[0-9a-f]{32}/Similar$"))
                .respond_with(json(similar.to_string()))
                .mount(&mock)
                .await;
            server.mount(mock).await
        },
        save_session,
    );
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        visit_movies(&window).await?;
        let card = wait_visible(&window, "Movie 1").await?;
        click(&window, &card);
        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;
        wait_role(&window, "Mark as watched", AccessibleRole::Button).await?;
        window
            .window()
            .set_size(slint::LogicalSize::new(1280., 2400.));
        let similar = wait_visible(&window, "Like 2").await?;
        click(&window, &similar);
        wait_until("the second movie", || {
            window.get_movie_data().title == "Movie 2" && window.get_movie_loaded()
        })
        .await?;
        if window.get_content_held() {
            return Err("the card left behind still holds the lists".into());
        }
        Ok(())
    });
}
