//! Authorize another device: the Quick Connect code typed in the settings
//! approves the sign-in of another device, with the outcome shown inline.

mod common;

use common::playback_settings::{WIDTH, open_settings_with};
use std::time::Duration;

use common::{click, pause, wait_role, wait_until, wait_visible};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use slint::ComponentHandle;
use slint::platform::WindowEvent;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

/// Width of the settings column.
const COLUMN: f32 = 640.;

fn press(window: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn type_text(window: &AppWindow, text: &str) {
    for character in text.chars() {
        press(window, character.to_string());
    }
}

/// Focuses the code field with a click, empties it and types `code` in it.
async fn enter_code(window: &AppWindow, code: &str) -> Result<(), String> {
    let field = wait_role(window, "Quick Connect code", AccessibleRole::TextInput).await?;
    click(window, &field);
    press(window, slint::platform::Key::End);
    erase(window, 12);
    type_text(window, code);
    Ok(())
}

fn authorize_enabled(window: &AppWindow) -> bool {
    ElementHandle::find_by_accessible_label(window, "Authorize")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .and_then(|element| element.accessible_enabled())
        .unwrap_or(false)
}

fn answer(status: u16, code: &str) -> Mock {
    let response = ResponseTemplate::new(status);
    let response = if status == 200 {
        response.set_body_json(true)
    } else {
        response
    };
    Mock::given(method("POST"))
        .and(path("/QuickConnect/Authorize"))
        .and(query_param("code", code))
        .respond_with(response)
}

/// Fails unless `element` lies inside the settings column.
fn inside_column(element: &ElementHandle, what: &str) -> Result<(), String> {
    let left = (WIDTH - COLUMN) / 2.;
    let right = left + COLUMN;
    let start = element.absolute_position().x;
    let end = start + element.size().width;
    if start < left || end > right {
        return Err(format!(
            "{what} spans {start}..{end}, outside {left}..{right}"
        ));
    }
    Ok(())
}

fn erase(window: &AppWindow, count: usize) {
    for _ in 0..count {
        press(window, slint::platform::Key::Backspace);
    }
}

fn message_shown(window: &AppWindow, text: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, text).any(|element| element.size().height > 0.)
}

async fn authorize_button_disabled_when_empty(window: &AppWindow) -> Result<(), String> {
    wait_role(window, "Authorize", AccessibleRole::Button).await?;
    if authorize_enabled(window) {
        return Err("Authorize is enabled without a code".into());
    }
    enter_code(window, "   ").await?;
    pause(Duration::from_millis(50)).await;
    if authorize_enabled(window) {
        return Err("Authorize is enabled with only spaces".into());
    }
    enter_code(window, " - -").await?;
    pause(Duration::from_millis(50)).await;
    if authorize_enabled(window) {
        return Err("Authorize is enabled without a digit".into());
    }
    type_text(window, "4");
    wait_until("Authorize enabled with a code", || {
        authorize_enabled(window)
    })
    .await?;
    erase(window, 4);
    wait_until("Authorize disabled again", || !authorize_enabled(window)).await
}

async fn authorize_wrong_code_message(window: &AppWindow) -> Result<(), String> {
    enter_code(window, "000000").await?;
    press(window, slint::platform::Key::Return);
    wait_visible(window, "This code is wrong or expired").await?;
    type_text(window, "1");
    wait_until("the message to leave on edit", || {
        !message_shown(window, "This code is wrong or expired")
    })
    .await?;
    Ok(())
}

async fn authorize_failed_message(window: &AppWindow) -> Result<(), String> {
    enter_code(window, "222222").await?;
    press(window, slint::platform::Key::Return);
    wait_visible(window, "The device could not be authorized.").await?;
    Ok(())
}

async fn authorize_unavailable_message(window: &AppWindow) -> Result<(), String> {
    enter_code(window, "111111").await?;
    click(
        window,
        &wait_role(window, "Authorize", AccessibleRole::Button).await?,
    );
    wait_visible(window, "Quick Connect is turned off on this server").await?;
    if window.get_screen() != Screen::Settings {
        return Err("a disabled Quick Connect left the settings".into());
    }
    Ok(())
}

async fn authorize_success_message(window: &AppWindow) -> Result<(), String> {
    wait_visible(
        window,
        "Sign in another device: open Quick Connect on it, then type the code it shows.",
    )
    .await?;
    enter_code(window, "482-913").await?;
    let button = wait_role(window, "Authorize", AccessibleRole::Button).await?;
    let field = wait_role(window, "Quick Connect code", AccessibleRole::TextInput).await?;
    inside_column(&button, "the Authorize button")?;
    inside_column(&field, "the code field")?;
    let field_bottom = field.absolute_position().y + field.size().height;
    let button_bottom = button.absolute_position().y + button.size().height;
    // The input sits inside the padding of the field box, which ends with the button.
    if button_bottom < field_bottom || button_bottom > field_bottom + 16. {
        return Err(format!(
            "the button ends at {button_bottom}, beside a field ending at {field_bottom}"
        ));
    }
    click(window, &button);
    let message = wait_visible(window, "Device authorized").await?;
    inside_column(&message, "the message")?;
    wait_until("the field cleared", || {
        field
            .accessible_value()
            .is_some_and(|value| value.is_empty())
    })
    .await
}

#[test]
fn authorize_another_device() {
    let enabled = Mock::given(method("GET"))
        .and(path("/QuickConnect/Enabled"))
        .respond_with(ResponseTemplate::new(200).set_body_json(false));
    let mocks = vec![
        answer(200, "482913"),
        answer(404, "000000"),
        answer(401, "111111"),
        answer(500, "222222"),
        enabled,
    ];
    open_settings_with(mocks, async |window, _| {
        authorize_button_disabled_when_empty(window).await?;
        authorize_wrong_code_message(window).await?;
        authorize_unavailable_message(window).await?;
        authorize_failed_message(window).await?;
        authorize_success_message(window).await
    });
}
