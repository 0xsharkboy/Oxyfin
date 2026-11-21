//! Subtitle search from the player: the track menu offers it only with the
//! permission, the dialog lists what the server finds, downloads one and has
//! the controller reload the subtitles, and shows errors inline.

mod common;

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use common::{click, pause, wait_role, wait_until, wait_visible};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::{player, subtitle_search};
use oxyfin::{AppWindow, Screen, SubtitleSearch};
use oxyfin_api::{ClientInfo, ItemId, Server, UserId};
use oxyfin_core::{AudioOption, Catalog, PlaybackCommand, PlaybackEvent, PlaybackTitle};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ITEM: &str = "76282b1c408f374beeee50519e9157a4";
const ENTRY: &str = "Find subtitles online…";
const FIRST: &str = "night of the living dead (1969)";
const SECOND: &str = "Night.Of.The.Living.Dead.1968.720p.BluRay.x264-Hv";
const THIRD: &str = "Night of the Living Dead (1968)";
const THIRD_ID: &str = "e02a34f56233e3e5e7765144ca8f184f_srt-fre-4438193";
const NOT_ADDED: &str = "The server did not add this subtitle. Try another one.";
const SERVER_ERROR: &str = "The server ran into an error. Please try again later.";

type Sent = Rc<RefCell<Vec<PlaybackCommand>>>;

fn search_route(language: &str) -> String {
    format!("/Items/{ITEM}/RemoteSearch/Subtitles/{language}")
}

fn download_route(id: &str) -> String {
    format!("/Items/{ITEM}/RemoteSearch/Subtitles/{id}")
}

/// Results of a search captured on a test server, the first marked a perfect match.
fn results() -> serde_json::Value {
    serde_json::json!([
        {
            "Id": "e02a34f56233e3e5e7765144ca8f184f_srt-fre-4437550",
            "ProviderName": "Open Subtitles",
            "Name": FIRST,
            "Format": "srt",
            "DownloadCount": 3005,
            "IsHashMatch": true,
            "Forced": false,
            "HearingImpaired": false
        },
        {
            "Id": "e02a34f56233e3e5e7765144ca8f184f_srt-fre-4438262",
            "ProviderName": "Open Subtitles",
            "Name": SECOND,
            "Format": "srt",
            "DownloadCount": 2416,
            "IsHashMatch": false,
            "Forced": false,
            "HearingImpaired": true
        },
        {
            "Id": THIRD_ID,
            "ProviderName": "Open Subtitles",
            "Name": THIRD,
            "Format": "srt",
            "DownloadCount": 1480
        }
    ])
}

async fn mount(mock: &MockServer) {
    Mock::given(method("GET"))
        .and(path(search_route("fre")))
        .respond_with(ResponseTemplate::new(200).set_body_json(results()))
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path(search_route("ger")))
        .respond_with(ResponseTemplate::new(500))
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path(download_route(
            "e02a34f56233e3e5e7765144ca8f184f_srt-fre-4437550",
        )))
        .respond_with(ResponseTemplate::new(204).set_delay(Duration::from_millis(400)))
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path(download_route(
            "e02a34f56233e3e5e7765144ca8f184f_srt-fre-4438262",
        )))
        .respond_with(ResponseTemplate::new(500))
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path(download_route(THIRD_ID)))
        .respond_with(ResponseTemplate::new(204))
        .mount(mock)
        .await;
}

fn find(window: &AppWindow, label: &str, role: AccessibleRole) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role) && element.size().height > 0.)
}

fn shown(window: &AppWindow, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, label)
        .any(|element| element.size().height > 0. && element.size().width > 0.)
}

fn open_track_menu(window: &AppWindow) -> Result<(), String> {
    let button = find(window, "Audio and subtitles", AccessibleRole::Button)
        .ok_or("no track menu button")?;
    click(window, &button);
    Ok(())
}

fn close_menus(window: &AppWindow) {
    for event in [
        WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        },
        WindowEvent::KeyReleased {
            text: slint::platform::Key::Escape.into(),
        },
    ] {
        window.window().dispatch_event(event);
    }
}

/// The row of the result named `name`.
fn row(window: &AppWindow, name: &str) -> Option<ElementHandle> {
    find(window, name, AccessibleRole::ListItem)
}

/// The Download button on the row of the result named `name`.
fn download_button(window: &AppWindow, name: &str) -> Option<ElementHandle> {
    let row = row(window, name)?;
    let top = row.absolute_position().y;
    let bottom = top + row.size().height;
    ElementHandle::find_by_accessible_label(window, "Download").find(|button| {
        let y = button.absolute_position().y;
        button.accessible_role() == Some(AccessibleRole::Button)
            && button.size().height > 0.
            && y >= top
            && y < bottom
    })
}

async fn entry_hidden_without_permission(window: &AppWindow) -> Result<(), String> {
    window.global::<SubtitleSearch>().set_allowed(false);
    open_track_menu(window)?;
    wait_role(window, "Audio and subtitles", AccessibleRole::List).await?;
    pause(Duration::from_millis(50)).await;
    if shown(window, ENTRY) {
        return Err("the entry shows without the permission".into());
    }
    close_menus(window);
    window.global::<SubtitleSearch>().set_allowed(true);
    open_track_menu(window)?;
    let entry = wait_role(window, ENTRY, AccessibleRole::Button).await?;
    let reset = wait_role(window, "Reset the subtitle delay", AccessibleRole::Button).await?;
    if entry.absolute_position().y <= reset.absolute_position().y {
        return Err("the entry is not at the end of the track menu".into());
    }
    let menu = wait_role(window, "Audio and subtitles", AccessibleRole::List).await?;
    let menu_bottom = menu.absolute_position().y + menu.size().height;
    if entry.absolute_position().y + entry.size().height > menu_bottom + 0.5 {
        return Err("the entry overflows the track menu".into());
    }
    close_menus(window);
    Ok(())
}

async fn language_chips_order(window: &AppWindow) -> Result<(), String> {
    open_track_menu(window)?;
    click(
        window,
        &wait_role(window, ENTRY, AccessibleRole::Button).await?,
    );
    let dialog = wait_role(window, "Find subtitles", AccessibleRole::Groupbox).await?;
    let french = wait_role(window, "French", AccessibleRole::Button).await?;
    let english = wait_role(window, "English", AccessibleRole::Button).await?;
    let other = wait_role(window, "Other…", AccessibleRole::Button).await?;
    let order = [&french, &english, &other].map(|chip| chip.absolute_position().x);
    if !(order[0] < order[1] && order[1] < order[2]) {
        return Err(format!("the chips are not in order: {order:?}"));
    }
    if french.accessible_checked() != Some(true) || english.accessible_checked() == Some(true) {
        return Err("the interface language is not the one chosen".into());
    }
    let left = dialog.absolute_position().x;
    let right = left + dialog.size().width;
    if (dialog.size().width - 640.).abs() > 0.5 {
        return Err(format!("the dialog is {} wide", dialog.size().width));
    }
    for chip in [&french, &english, &other] {
        let x = chip.absolute_position().x;
        if x < left || x + chip.size().width > right {
            return Err("a chip leaves the dialog".into());
        }
    }
    Ok(())
}

async fn dialog_lists_results_and_downloads(window: &AppWindow, sent: &Sent) -> Result<(), String> {
    wait_until("the results", || row(window, FIRST).is_some()).await?;
    wait_visible(window, "Perfect match").await?;
    wait_visible(window, "Hearing impaired").await?;
    let first = row(window, FIRST).ok_or("no first row")?;
    let dialog = wait_role(window, "Find subtitles", AccessibleRole::Groupbox).await?;
    let right = dialog.absolute_position().x + dialog.size().width;
    let button = download_button(window, FIRST).ok_or("no Download on the first row")?;
    if button.absolute_position().x + button.size().width > right {
        return Err("the Download button leaves the dialog".into());
    }
    if first.absolute_position().x + first.size().width > right + 0.5 {
        return Err("the row leaves the dialog".into());
    }
    click(window, &button);
    wait_visible(window, "Downloading…").await?;
    if !sent.borrow().is_empty() {
        return Err("the subtitles reload before the download ends".into());
    }
    let expected = PlaybackCommand::ReloadSubtitles {
        select_language: "fre".to_owned(),
    };
    wait_until("the reload command", || sent.borrow().contains(&expected)).await?;
    // Until the player shows the new file, the download is not over.
    pause(Duration::from_millis(100)).await;
    if !shown(window, "Downloading…") || shown(window, "Added") {
        return Err("the row is added before the player has the file".into());
    }
    player::apply(window, PlaybackEvent::SubtitlesReloaded { added: true });
    wait_visible(window, "Added").await?;
    if download_button(window, FIRST).is_some() {
        return Err("the added row still offers Download".into());
    }
    Ok(())
}

async fn dialog_errors_inline(window: &AppWindow, sent: &Sent) -> Result<(), String> {
    let before = sent.borrow().len();
    click(
        window,
        &download_button(window, SECOND).ok_or("no Download on the second row")?,
    );
    wait_visible(window, SERVER_ERROR).await?;
    wait_until("Download offered again", || {
        download_button(window, SECOND).is_some()
    })
    .await?;

    click(
        window,
        &download_button(window, THIRD).ok_or("no Download on the third row")?,
    );
    wait_until("the reload command", || sent.borrow().len() == before + 1).await?;
    player::apply(window, PlaybackEvent::SubtitlesReloaded { added: false });
    wait_visible(window, NOT_ADDED).await?;
    wait_until("Download offered again", || {
        download_button(window, THIRD).is_some()
    })
    .await?;
    let before = sent.borrow().len();

    click(
        window,
        &wait_role(window, "Other…", AccessibleRole::Button).await?,
    );
    let german = wait_role(window, "German", AccessibleRole::ListItem).await?;
    click(window, &german);
    wait_role(window, "German", AccessibleRole::Button).await?;
    wait_visible(window, SERVER_ERROR).await?;
    if row(window, FIRST).is_some() {
        return Err("the French results stay after a failed search".into());
    }
    if !shown(window, "Find subtitles") || window.get_player_has_error() {
        return Err("the error left the dialog or reached the player".into());
    }
    if window.get_screen() != Screen::Player {
        return Err("the error left the player".into());
    }
    if sent.borrow().len() != before {
        return Err(format!("errors sent commands: {:?}", sent.borrow()));
    }
    click(
        window,
        &wait_role(window, "Close", AccessibleRole::Button).await?,
    );
    wait_until("the dialog closed", || !shown(window, "Find subtitles")).await
}

fn audio(index: i32, language: &str, selected: bool) -> AudioOption {
    AudioOption {
        index,
        label: language.to_owned(),
        language: Some(language.to_owned()),
        channels: Some(2),
        selected,
    }
}

fn run(scenario: impl Future<Output = Result<(), String>> + 'static) {
    let outcome = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&outcome);
    slint::spawn_local(async move {
        *slot.borrow_mut() = Some(scenario.await);
        let _ = slint::quit_event_loop();
    })
    .unwrap_or_else(|err| panic!("cannot spawn the scenario: {err}"));
    slint::run_event_loop().unwrap_or_else(|err| panic!("event loop failed: {err}"));
    match outcome.borrow_mut().take() {
        Some(Ok(())) => {}
        Some(Err(error)) => panic!("{error}"),
        None => panic!("the scenario did not finish"),
    }
}

#[test]
fn subtitle_search_from_the_player() {
    i_slint_backend_testing::init_integration_test_with_system_time();
    let runtime = common::runtime();
    let mock = runtime.block_on(async {
        let mock = MockServer::start().await;
        mount(&mock).await;
        mock
    });
    let session = Server::new(&mock.uri(), ClientInfo::new("Test", "test-device"))
        .unwrap_or_else(|err| panic!("cannot create the server: {err}"))
        .restore_session("token", UserId::from(common::USER_ID), None);
    let catalog = Catalog::new(session);

    let window = AppWindow::new().unwrap_or_else(|err| panic!("cannot create the window: {err}"));
    let _ = slint::select_bundled_translation("en");
    window.window().set_size(LogicalSize::new(1280., 800.));
    window
        .show()
        .unwrap_or_else(|err| panic!("cannot show the window: {err}"));
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));

    let sent: Sent = Rc::default();
    let recorder = Rc::clone(&sent);
    let send: Rc<dyn Fn(PlaybackCommand)> =
        Rc::new(move |command| recorder.borrow_mut().push(command));
    player::bind(&window, Rc::clone(&send));
    subtitle_search::bind(
        &window,
        runtime.handle().clone(),
        "fr",
        Rc::new(move || Some(catalog.clone())),
        send,
    );
    player::reset(&window);
    window.set_screen(Screen::Player);
    player::apply(
        &window,
        PlaybackEvent::Loading {
            item: ItemId::from(ITEM),
            title: PlaybackTitle {
                title: "Night of the Living Dead".to_owned(),
                series_name: None,
                episode_label: None,
                year: Some(1968),
                artwork: None,
            },
        },
    );
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(5760)),
            audio: vec![audio(2, "eng", true), audio(3, "fra", false)],
            subtitles: Vec::new(),
            has_previous: true,
            has_next: false,
        },
    );

    let _guard = runtime.enter();
    let scenario_window = window.clone_strong();
    let scenario_sent = Rc::clone(&sent);
    run(async move {
        let window = &scenario_window;
        entry_hidden_without_permission(window).await?;
        language_chips_order(window).await?;
        dialog_lists_results_and_downloads(window, &scenario_sent).await?;
        dialog_errors_inline(window, &scenario_sent).await
    });
    drop(window);
    drop(mock);
}
