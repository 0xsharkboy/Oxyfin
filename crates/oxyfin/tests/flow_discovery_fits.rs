//! Three servers found on the network in a default window: the sign-in card starts at the top
//! and scrolls to its last button.

mod common;

use common::discovery::{reply_as, responder};
use common::{pause, run, server, start, wait_role, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::Screen;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, Model};
use std::time::Duration;

#[test]
fn sign_in_card_scrolls_with_three_servers() {
    let flow = start(|_| server());
    let uri = flow.mock.uri();
    let targets = ["Maison", "Salon", "Bureau"]
        .iter()
        .map(|name| responder(Some(reply_as(name, name, &uri))))
        .collect();
    flow.app.set_discovery_targets(targets);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        wait_until("three servers", || {
            window.get_found_servers().row_count() == 3
        })
        .await?;
        wait_role(&window, "Bureau", AccessibleRole::Button).await?;

        let size = window
            .window()
            .size()
            .to_logical(window.window().scale_factor());
        let heading = ElementHandle::find_by_accessible_label(&window, "Sign in")
            .find(|element| element.accessible_role() == Some(AccessibleRole::Text))
            .ok_or("no heading")?;
        if heading.absolute_position().y < 0. {
            return Err(format!(
                "the heading is cut off at {}",
                heading.absolute_position().y
            ));
        }

        window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(size.width / 2., size.height / 2.),
                delta_x: 0.,
                delta_y: -1000.,
            });
        pause(Duration::from_millis(100)).await;
        let quick_connect = wait_role(&window, "Use Quick Connect", AccessibleRole::Button).await?;
        let bottom = quick_connect.absolute_position().y + quick_connect.size().height;
        if bottom > size.height {
            return Err(format!(
                "Quick Connect stays below the {} px window: {bottom}",
                size.height
            ));
        }
        Ok(())
    });
}
