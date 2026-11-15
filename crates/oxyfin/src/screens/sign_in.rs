//! Sign-in card and Quick Connect: server check, password sign-in, Quick Connect flow.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use oxyfin_core::{CoreError, ServerHandle};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use tokio::task::AbortHandle;

use crate::app::App;
use crate::present::core_error_message;
use crate::state::{Msg, Screen};
use crate::{AppWindow, Messages, ServerState, ServerStatus};

/// Delay between the last edit of the server address and its check.
pub const SERVER_CHECK_DEBOUNCE: Duration = Duration::from_millis(600);

/// State of the sign-in and Quick Connect screens.
#[derive(Default)]
pub(crate) struct SignInController {
    server: RefCell<Option<ServerHandle>>,
    checked: RefCell<SharedString>,
    request: Cell<u64>,
    debounce: Timer,
    attempt: Cell<u64>,
    quick_connect: RefCell<Option<AbortHandle>>,
}

fn error_text(window: &AppWindow, error: &CoreError) -> SharedString {
    let (message, detail) = core_error_message(error);
    window
        .global::<Messages>()
        .invoke_message_text(message, detail)
}

fn set_status(window: &AppWindow, state: ServerState, text: SharedString) {
    window.set_server_status(ServerStatus { state, text });
}

impl SignInController {
    /// The address was edited: check it once the user stops typing.
    pub(crate) fn server_changed(&self, app: &Rc<App>, address: SharedString) {
        self.invalidate();
        let Some(window) = app.window() else { return };
        if address.trim().is_empty() {
            self.debounce.stop();
            set_status(&window, ServerState::Idle, SharedString::default());
            return;
        }
        set_status(&window, ServerState::Checking, SharedString::default());
        let this = Rc::downgrade(app);
        self.debounce
            .start(TimerMode::SingleShot, SERVER_CHECK_DEBOUNCE, move || {
                if let Some(app) = this.upgrade() {
                    app.sign_in.check(&app, address.clone());
                }
            });
    }

    /// The address field lost the focus or was validated: check it now.
    pub(crate) fn server_confirmed(&self, app: &Rc<App>, address: SharedString) {
        if *self.checked.borrow() == address {
            return;
        }
        self.debounce.stop();
        self.check(app, address);
    }

    fn invalidate(&self) {
        self.request.set(self.request.get() + 1);
        self.server.borrow_mut().take();
        *self.checked.borrow_mut() = SharedString::default();
    }

    fn check(&self, app: &Rc<App>, address: SharedString) {
        self.invalidate();
        if address.trim().is_empty() {
            return;
        }
        let Some(window) = app.window() else { return };
        set_status(&window, ServerState::Checking, SharedString::default());
        *self.checked.borrow_mut() = address.clone();
        let request = self.request.get();
        let manager = app.manager();
        app.spawn(
            async move { manager.connect(&address).await },
            move |app, result| {
                let controller = &app.sign_in;
                if controller.request.get() != request {
                    return;
                }
                let Some(window) = app.window() else { return };
                match result {
                    Ok(server) => {
                        let text = window.global::<Messages>().invoke_server_found(
                            server.name.as_deref().unwrap_or_default().into(),
                            server.version.as_str().into(),
                        );
                        set_status(&window, ServerState::Found, text);
                        *controller.server.borrow_mut() = Some(server);
                    }
                    Err(error) => {
                        log::info!("server check failed: {error}");
                        set_status(&window, ServerState::Error, error_text(&window, &error));
                    }
                }
            },
        );
    }

    /// Signs in with a password on the server found.
    pub(crate) fn sign_in(&self, app: &Rc<App>, user: SharedString, password: SharedString) {
        let Some(window) = app.window() else { return };
        let Some(server) = self.server.borrow().clone() else {
            return;
        };
        if window.get_busy() || user.is_empty() {
            return;
        }
        window.set_busy(true);
        window.set_form_error(SharedString::default());
        let manager = app.manager();
        app.spawn(
            async move { manager.sign_in(&server, &user, &password).await },
            |app, result| {
                let Some(window) = app.window() else { return };
                window.set_busy(false);
                match result {
                    Ok(signed) => app.signed_in(signed),
                    Err(error) => {
                        log::info!("sign-in failed: {error}");
                        window.set_form_error(error_text(&window, &error));
                    }
                }
            },
        );
    }

    /// Opens Quick Connect when a server is found.
    pub(crate) fn open_quick_connect(&self, app: &Rc<App>) {
        let busy = app.window().is_none_or(|window| window.get_busy());
        if self.server.borrow().is_some() && !busy {
            app.dispatch(Msg::OpenQuickConnect);
        }
    }

    /// Reacts to a screen change.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: &Screen, previous: &Screen) {
        if *screen != Screen::QuickConnect {
            self.cancel_quick_connect();
        }
        match screen {
            Screen::SignIn if *previous != Screen::QuickConnect => self.reset(app),
            Screen::QuickConnect => self.start_quick_connect(app),
            _ => {}
        }
    }

    fn reset(&self, app: &Rc<App>) {
        let Some(window) = app.window() else { return };
        window.set_busy(false);
        window.set_form_error(SharedString::default());
        if window.get_server_address().is_empty()
            && let Some(url) = app.manager().last_server_url()
        {
            window.set_server_address(url.into());
        }
        self.debounce.stop();
        self.check(app, window.get_server_address());
    }

    /// Requests a Quick Connect code, then waits for its approval.
    pub(crate) fn start_quick_connect(&self, app: &Rc<App>) {
        self.cancel_quick_connect();
        let Some(window) = app.window() else { return };
        window.set_quick_connect_code(SharedString::default());
        window.set_quick_connect_digits(ModelRc::default());
        window.set_quick_connect_error(SharedString::default());
        window.set_quick_connect_can_retry(false);
        let Some(server) = self.server.borrow().clone() else {
            window.set_quick_connect_error(error_text(
                &window,
                &CoreError::InvalidServerUrl(String::new()),
            ));
            return;
        };
        let attempt = self.attempt.get();
        let manager = app.manager();
        let task = app.spawn(
            async move {
                manager
                    .start_quick_connect(&server)
                    .await
                    .map(|flow| (server, flow))
            },
            move |app, result| {
                let controller = &app.sign_in;
                if controller.attempt.get() != attempt {
                    return;
                }
                let (server, flow) = match result {
                    Ok(started) => started,
                    Err(error) => return controller.quick_connect_failed(app, &error),
                };
                if let Some(window) = app.window() {
                    let digits: Vec<SharedString> = flow
                        .code
                        .chars()
                        .map(|digit| digit.to_string().into())
                        .collect();
                    window.set_quick_connect_digits(ModelRc::new(VecModel::from(digits)));
                    window.set_quick_connect_code(flow.code.as_str().into());
                }
                let manager = app.manager();
                let task = app.spawn(
                    async move { manager.finish_quick_connect(&server, flow).await },
                    move |app, result| {
                        if app.sign_in.attempt.get() != attempt {
                            return;
                        }
                        app.sign_in.quick_connect.borrow_mut().take();
                        match result {
                            Ok(signed) => app.signed_in(signed),
                            Err(error) => app.sign_in.quick_connect_failed(app, &error),
                        }
                    },
                );
                *controller.quick_connect.borrow_mut() = Some(task);
            },
        );
        *self.quick_connect.borrow_mut() = Some(task);
    }

    fn quick_connect_failed(&self, app: &Rc<App>, error: &CoreError) {
        log::info!("Quick Connect failed: {error}");
        let Some(window) = app.window() else { return };
        window.set_quick_connect_error(error_text(&window, error));
        window.set_quick_connect_can_retry(matches!(error, CoreError::QuickConnectExpired));
    }

    fn cancel_quick_connect(&self) {
        self.attempt.set(self.attempt.get() + 1);
        if let Some(task) = self.quick_connect.borrow_mut().take() {
            task.abort();
        }
    }
}
