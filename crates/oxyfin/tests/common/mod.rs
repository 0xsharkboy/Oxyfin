//! Helpers shared by the flow tests: a simulated Jellyfin 12.4 server and the app setup.
#![allow(dead_code)]

use std::cell::RefCell;
use std::future::Future;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use oxyfin::{App, AppWindow};
use oxyfin_core::{MemoryStore, Paths, TokenVault};
use slint::ComponentHandle;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// How long a wait helper waits before failing the test.
pub const DEADLINE: Duration = Duration::from_secs(5);

/// Name the fixture server publishes.
pub const SERVER_NAME: &str = "49667124a8b5";

/// Version the fixture server publishes.
pub const SERVER_VERSION: &str = "12.4.0";

/// Quick Connect code of the fixtures.
pub const QUICK_CONNECT_CODE: &str = "668908";

/// Text of `tests/fixtures/<name>.json`.
pub fn fixture(name: &str) -> String {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(format!("{name}.json"));
    std::fs::read_to_string(&file)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", file.display()))
}

/// A JSON response with `body`.
pub fn json(body: impl Into<Vec<u8>>) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.into(), "application/json")
}

/// Mounts `GET /System/Info/Public`.
pub async fn mount_public_info(mock: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .respond_with(json(fixture("system_info_public")))
        .mount(mock)
        .await;
}

/// Mounts a successful password sign-in and `GET /Users/Me`.
pub async fn mount_sign_in(mock: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/Users/AuthenticateByName"))
        .respond_with(json(fixture("authenticate_by_name")))
        .mount(mock)
        .await;
    mount_me(mock).await;
}

/// Mounts `GET /Users/Me`.
pub async fn mount_me(mock: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/Users/Me"))
        .respond_with(json(fixture("users_me")))
        .mount(mock)
        .await;
}

/// Mounts a Quick Connect flow whose code is approved at the first check.
pub async fn mount_quick_connect(mock: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/QuickConnect/Enabled"))
        .respond_with(json("true"))
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/QuickConnect/Initiate"))
        .respond_with(json(fixture("quick_connect_initiate")))
        .mount(mock)
        .await;
    let approved = fixture("quick_connect_connect")
        .replace("\"Authenticated\": false", "\"Authenticated\": true");
    Mock::given(method("GET"))
        .and(path("/QuickConnect/Connect"))
        .respond_with(json(approved))
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/Users/AuthenticateWithQuickConnect"))
        .respond_with(json(fixture("authenticate_by_name")))
        .mount(mock)
        .await;
    mount_me(mock).await;
}

/// A server answering the public info, password sign-in and Quick Connect routes.
pub async fn server() -> MockServer {
    let mock = MockServer::start().await;
    mount_public_info(&mock).await;
    mount_sign_in(&mock).await;
    mount_quick_connect(&mock).await;
    mock
}

/// Application directories under a fresh temporary directory.
pub fn paths() -> (TempDir, Paths) {
    let dir = TempDir::new().unwrap_or_else(|err| panic!("cannot create a temp dir: {err}"));
    let paths = Paths::under(dir.path());
    (dir, paths)
}

/// A vault whose primary and fallback stores are fresh in-memory stores.
pub fn vault() -> TokenVault {
    TokenVault::new(
        Arc::new(MemoryStore::default()),
        Arc::new(MemoryStore::default()),
    )
}

/// A multi-thread runtime owned by the test.
pub fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|err| panic!("cannot build the runtime: {err}"))
}

/// Everything a flow needs: the window, the app, and what must outlive them.
pub struct Flow {
    /// Runtime the app runs its tasks on.
    pub runtime: Runtime,
    /// Simulated server.
    pub mock: MockServer,
    /// Main window.
    pub window: AppWindow,
    /// The app under test.
    pub app: Rc<App>,
    _dir: TempDir,
}

/// Starts the testing backend, a simulated server prepared by `mount`, and the app.
pub fn start<F, Fut>(mount: F) -> Flow
where
    F: FnOnce(MockServer) -> Fut,
    Fut: Future<Output = MockServer>,
{
    i_slint_backend_testing::init_integration_test_with_system_time();
    let runtime = runtime();
    let mock = runtime.block_on(async { mount(MockServer::start().await).await });
    let (dir, paths) = paths();
    let window = AppWindow::new().unwrap_or_else(|err| panic!("cannot create the window: {err}"));
    let app = App::new(&window, runtime.handle().clone(), paths, vault())
        .unwrap_or_else(|err| panic!("cannot create the app: {err}"));
    window
        .show()
        .unwrap_or_else(|err| panic!("cannot show the window: {err}"));
    Flow {
        runtime,
        mock,
        window,
        app,
        _dir: dir,
    }
}

/// Runs `scenario` on the Slint event loop and fails the test with its error.
pub fn run<Fut>(flow: &Flow, scenario: Fut)
where
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let _guard = flow.runtime.enter();
    let outcome = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&outcome);
    slint::spawn_local(async move {
        let result = scenario.await;
        *slot.borrow_mut() = Some(result);
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

/// Waits until `condition` holds, or fails with `what` after [`DEADLINE`].
pub async fn wait_until(what: &str, condition: impl Fn() -> bool) -> Result<(), String> {
    let start = std::time::Instant::now();
    while !condition() {
        if start.elapsed() > DEADLINE {
            return Err(format!("timed out waiting for {what}"));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

/// Sleeps on the event loop without blocking it.
pub async fn pause(duration: Duration) {
    tokio::time::sleep(duration).await;
}
