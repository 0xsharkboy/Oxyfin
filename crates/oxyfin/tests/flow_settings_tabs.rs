//! The settings are not one of the top bar tabs: none of them is underlined there.

mod common;

use common::{click, mount_browse_basics, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;
use wiremock::MockServer;

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

#[test]
fn no_tab_is_underlined_on_settings() {
    let flow = start_with(
        |mock: MockServer| async move {
            mount_browse_basics(&mock, EMPTY).await;
            mock
        },
        save_session,
    );
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let settings = wait_role(&window, "Settings", AccessibleRole::Button).await?;
        click(&window, &settings);
        wait_until("the settings", || window.get_screen() == Screen::Settings).await?;

        for tab in ["Home", "Movies", "Shows", "Favorites"] {
            let element = wait_role(&window, tab, AccessibleRole::Tab).await?;
            if element.accessible_item_selected() != Some(false) {
                return Err(format!("the {tab} tab is selected on the settings"));
            }
        }
        Ok(())
    });
}
