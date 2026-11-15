//! Helpers shared by the flow tests: a simulated Jellyfin 12.4 server and the app setup.
#![allow(dead_code)]

use std::cell::RefCell;
use std::future::Future;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use oxyfin::{App, AppWindow};
use oxyfin_core::{Config, MemoryStore, Paths, SavedSession, TokenVault, account_key};
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

/// A vault whose primary and fallback stores are fresh in-memory stores, and a
/// second vault over the same stores for the test to inspect.
pub fn vault() -> (TokenVault, TokenVault) {
    let primary = Arc::new(MemoryStore::default());
    let fallback = Arc::new(MemoryStore::default());
    (
        TokenVault::new(primary.clone(), fallback.clone()),
        TokenVault::new(primary, fallback),
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
    /// A view of the token stores the app uses.
    pub vault: TokenVault,
    _dir: TempDir,
}

/// Starts the testing backend, a simulated server prepared by `mount`, and the app.
pub fn start<F, Fut>(mount: F) -> Flow
where
    F: FnOnce(MockServer) -> Fut,
    Fut: Future<Output = MockServer>,
{
    start_with(mount, |_, _, _| {})
}

/// Like [`start`], with `prepare` filling the directories and the vault before the app starts.
pub fn start_with<F, Fut>(mount: F, prepare: impl FnOnce(&Paths, &TokenVault, &str)) -> Flow
where
    F: FnOnce(MockServer) -> Fut,
    Fut: Future<Output = MockServer>,
{
    i_slint_backend_testing::init_integration_test_with_system_time();
    let runtime = runtime();
    let mock = runtime.block_on(async { mount(MockServer::start().await).await });
    let (dir, paths) = paths();
    let (vault, view) = vault();
    prepare(&paths, &vault, &mock.uri());
    let window = AppWindow::new().unwrap_or_else(|err| panic!("cannot create the window: {err}"));
    let app = App::new(&window, runtime.handle().clone(), paths, vault)
        .unwrap_or_else(|err| panic!("cannot create the app: {err}"));
    window
        .show()
        .unwrap_or_else(|err| panic!("cannot show the window: {err}"));
    Flow {
        runtime,
        mock,
        window,
        app,
        vault: view,
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

/// Id of the fixture user.
pub const USER_ID: &str = "28dd79b9f3564a398568f3d0ca90f9de";

/// Saves a signed-in session for `server_url`, as a previous run would have.
pub fn save_session(paths: &Paths, vault: &TokenVault, server_url: &str) {
    let session_url = format!("{}/", server_url.trim_end_matches('/'));
    let mut config =
        Config::load_or_create(paths).unwrap_or_else(|err| panic!("cannot create config: {err}"));
    config.session = Some(SavedSession {
        server_url: session_url.clone(),
        user_id: USER_ID.to_owned(),
        user_name: "oxyfin".to_owned(),
        server_id: None,
        server_name: None,
    });
    config.last_server_url = Some(server_url.to_owned());
    config
        .save(paths)
        .unwrap_or_else(|err| panic!("cannot save config: {err}"));
    vault
        .save(
            &account_key(&session_url, USER_ID),
            "5dabe43042264dc4960513e5e30552e3",
        )
        .unwrap_or_else(|err| panic!("cannot save the token: {err}"));
}
