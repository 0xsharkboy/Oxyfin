//! Opening the context menu of an element with a right click and reading or choosing its rows.

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, ContextMenu};
use slint::ComponentHandle;
use slint::platform::{PointerEventButton, WindowEvent};

use super::{DEADLINE, click};

/// Right-clicks `element` near its top left corner.
pub fn right_click(window: &AppWindow, element: &ElementHandle) {
    let at = element.absolute_position();
    let position = slint::LogicalPosition::new(at.x + 20., at.y + 20.);
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Right,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Right,
        },
    ] {
        window.window().dispatch_event(event);
    }
}

/// Waits for the open menu titled `title` and returns the labels of its rows, in order.
pub async fn wait_menu(window: &AppWindow, title: &str) -> Result<Vec<String>, String> {
    let start = std::time::Instant::now();
    loop {
        let panel = ElementHandle::find_by_accessible_label(window, title).find(|element| {
            element.accessible_role() == Some(AccessibleRole::List) && element.size().width > 0.
        });
        if let Some(panel) = panel {
            let mut rows: Vec<ElementHandle> = panel
                .query_descendants()
                .match_accessible_role(AccessibleRole::Button)
                .find_all()
                .into_iter()
                .filter(|row| row.size().height > 0.)
                .collect();
            rows.sort_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y));
            return Ok(rows
                .iter()
                .filter_map(|row| row.accessible_label().map(|label| label.to_string()))
                .collect());
        }
        if start.elapsed() > DEADLINE {
            return Err(format!("timed out waiting for the menu {title}"));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Clicks the row `label` of the open menu.
pub fn choose(window: &AppWindow, label: &str) -> Result<(), String> {
    let menu = window.global::<ContextMenu>();
    if !menu.get_open() {
        return Err(format!("no menu is open to choose {label}"));
    }
    let row = ElementHandle::find_by_accessible_label(window, label)
        .find(|row| row.accessible_role() == Some(AccessibleRole::Button) && row.size().height > 0.)
        .ok_or_else(|| format!("no row {label}"))?;
    click(window, &row);
    Ok(())
}

/// Whether the menu is open.
pub fn is_open(window: &AppWindow) -> bool {
    window.global::<ContextMenu>().get_open()
}
