//! Switches the interface to French: the sign-in label, then the home and movie pages at 900 px.

mod common;

use common::{
    click, fixture, json, mount_browse_basics, run, server, start, wait_role, wait_until,
    wait_visible,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{Screen, ServerState};
use slint::{ComponentHandle, LogicalSize};
use wiremock::Mock;
use wiremock::matchers::{method, path};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";

#[test]
fn the_interface_follows_the_selected_language() {
    let flow = start(|_| async {
        let mock = server().await;
        mount_browse_basics(&mock, r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#).await;
        Mock::given(method("GET"))
            .and(path("/Items"))
            .respond_with(json(fixture("items_movies")))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/Items/{MOVIE}")))
            .respond_with(json(fixture("item_movie")))
            .mount(&mock)
            .await;
        mock
    });
    if let Err(error) = slint::select_bundled_translation("fr") {
        panic!("cannot select French: {error}");
    }
    let window = flow.window.as_weak();
    let uri = flow.mock.uri();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        wait_role(&window, "Se connecter", AccessibleRole::Button).await?;
        wait_visible(&window, "Utiliser Quick Connect").await?;

        window.invoke_server_confirmed(uri.as_str().into());
        wait_until("the server", || {
            window.get_server_status().state == ServerState::Found
        })
        .await?;
        window.invoke_sign_in("user".into(), "pass".into());
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_role(&window, "Accueil", AccessibleRole::Tab).await?;
        let movies = wait_role(&window, "Films", AccessibleRole::Tab).await?;
        click(&window, &movies);
        let card = wait_visible(&window, "Test Movie (2020)").await?;
        click(&window, &card);

        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;
        wait_visible(&window, "Avec Ada Actor, Ben Bit, Cleo Code, Dan Data").await?;
        wait_role(&window, "Retirer des favoris", AccessibleRole::Button).await?;
        window.window().set_size(LogicalSize::new(900., 700.));
        for label in ["Reprendre à 0:03", "Lire", "Marquer comme vu"] {
            let button = wait_role(&window, label, AccessibleRole::Button).await?;
            let (left, top) = (button.absolute_position().x, button.absolute_position().y);
            let (right, bottom) = (left + button.size().width, top + button.size().height);
            let texts: Vec<ElementHandle> =
                ElementHandle::find_by_element_id(&window, "Button::label-text")
                    .filter(|text| {
                        let (x, y) = (text.absolute_position().x, text.absolute_position().y);
                        x >= left && x <= right && y >= top && y <= bottom
                    })
                    .collect();
            let [text] = texts.as_slice() else {
                return Err(format!("{label}: expected one text, found {}", texts.len()));
            };
            let (x, y) = (text.absolute_position().x, text.absolute_position().y);
            let (width, height) = (text.size().width, text.size().height);
            if x + width > right + 0.5 || y + height > bottom + 0.5 {
                return Err(format!("the text of {label} overflows its button"));
            }
        }
        Ok(())
    });
}
