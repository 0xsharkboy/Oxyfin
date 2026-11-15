//! Switches the interface to French: the sign-in label, the home page and the movie actions.

mod common;

use common::{
    click, fixture, json, mount_browse_basics, pause, run, server, start, wait_role, wait_until,
    wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{Screen, ServerState};
use slint::{ComponentHandle, LogicalSize};
use std::time::Duration;
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
        for width in [900., 760.] {
            window.window().set_size(LogicalSize::new(width, 700.));
            pause(Duration::from_millis(200)).await;
            let header = wait_role(&window, "Test Movie (2020)", AccessibleRole::Groupbox).await?;
            let header_bottom = header.absolute_position().y + header.size().height;
            let mut boxes = Vec::new();
            for label in [
                "Reprendre à 0:03",
                "Marquer comme vu",
                "Retirer des favoris",
            ] {
                let button = wait_role(&window, label, AccessibleRole::Button).await?;
                let (x, y) = (button.absolute_position().x, button.absolute_position().y);
                let (w, h) = (button.size().width, button.size().height);
                if x < -0.5 || x + w > width + 0.5 {
                    return Err(format!("{label} leaves the {width} px window: x={x} w={w}"));
                }
                if y + h > header_bottom + 0.5 {
                    return Err(format!("{label} sticks out of the header at {width} px"));
                }
                boxes.push((label, x, y, w, h));
            }
            for (index, first) in boxes.iter().enumerate() {
                for second in boxes.iter().skip(index + 1) {
                    let apart = first.1 + first.3 <= second.1 + 0.5
                        || second.1 + second.3 <= first.1 + 0.5
                        || first.2 + first.4 <= second.2 + 0.5
                        || second.2 + second.4 <= first.2 + 0.5;
                    if !apart {
                        return Err(format!("{} overlaps {} at {width} px", first.0, second.0));
                    }
                }
            }
        }
        Ok(())
    });
}
