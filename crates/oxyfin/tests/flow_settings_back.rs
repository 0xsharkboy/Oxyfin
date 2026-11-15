//! The mouse back button leaves the settings, like Escape does.

mod common;

use common::{click, mount_browse_basics, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use wiremock::MockServer;

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

#[test]
fn the_mouse_back_button_leaves_the_settings() {
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

        let position = LogicalPosition::new(1150., 600.);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        for event in [
            WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Back,
            },
            WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Back,
            },
        ] {
            window.window().dispatch_event(event);
        }
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        Ok(())
    });
}
