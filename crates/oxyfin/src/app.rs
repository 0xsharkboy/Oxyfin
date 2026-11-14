//! The application: executes the state machine's effects and owns the current generation.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use oxyfin_api::{ItemId, Session};
use oxyfin_core::{
    Catalog, Config, CoreError, ImageCache, Paths, SessionManager, SignedIn, TokenStorage,
    TokenVault,
};
use slint::{CloseRequestResponse, ComponentHandle};
use tokio::runtime::Handle;
use tokio::task::{AbortHandle, JoinHandle};

use crate::generation::Generation;
use crate::present::device_name;
use crate::screens::home::HomeController;
use crate::screens::sign_in::SignInController;
use crate::state::{AppState, Effect, GenerationId, Msg, Notice, Screen, SessionInfo};
use crate::{AppWindow, Message};

/// The running application, living on the Slint thread.
///
/// Work runs on the Tokio runtime; its results come back to the Slint event
/// loop, where they are turned into [`Msg`]s for [`App::dispatch`].
pub struct App {
    this: Weak<App>,
    window: slint::Weak<AppWindow>,
    runtime: Handle,
    paths: Paths,
    manager: Arc<SessionManager>,
    state: RefCell<AppState>,
    shown: Cell<Screen>,
    generation: RefCell<Option<Generation>>,
    incoming: RefCell<Option<Session>>,
    ending: RefCell<Vec<JoinHandle<()>>>,
    quitting: Cell<bool>,
    pub(crate) sign_in: SignInController,
    pub(crate) home: HomeController,
}

impl App {
    /// Creates the application for `window`, then restores the saved session in the background.
    pub fn new(
        window: &AppWindow,
        runtime: Handle,
        paths: Paths,
        vault: TokenVault,
    ) -> Result<Rc<App>, CoreError> {
        let config = Config::load_or_create(&paths)?;
        let hostname = gethostname::gethostname();
        let name = device_name(hostname.to_str(), &config.device_id);
        let manager = Arc::new(SessionManager::new(paths.clone(), vault, name)?);
        let app = Rc::new_cyclic(|this| App {
            this: this.clone(),
            window: window.as_weak(),
            runtime,
            paths,
            manager,
            state: RefCell::new(AppState::new()),
            shown: Cell::new(Screen::Starting),
            generation: RefCell::new(None),
            incoming: RefCell::new(None),
            ending: RefCell::new(Vec::new()),
            quitting: Cell::new(false),
            sign_in: SignInController::default(),
            home: HomeController::default(),
        });
        app.bind(window);
        app.restore();
        Ok(app)
    }

    /// Applies `msg` to the state machine and executes the resulting effects.
    pub fn dispatch(self: &Rc<Self>, msg: Msg) {
        if self.quitting.get() {
            return;
        }
        let effects = self.state.borrow_mut().update(msg);
        for effect in effects {
            self.apply(effect);
        }
    }

    /// Takes the generation shutdowns still running, for `main` to wait on before exiting.
    pub fn take_pending_ends(&self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut *self.ending.borrow_mut())
    }

    pub(crate) fn window(&self) -> Option<AppWindow> {
        self.window.upgrade()
    }

    pub(crate) fn manager(&self) -> Arc<SessionManager> {
        Arc::clone(&self.manager)
    }

    pub(crate) fn runtime(&self) -> &Handle {
        &self.runtime
    }

    /// Identifier, catalog and image cache of the current generation.
    pub(crate) fn services(&self) -> Option<(GenerationId, Catalog, Arc<ImageCache>)> {
        self.generation.borrow().as_ref().map(|generation| {
            (
                generation.id,
                generation.catalog.clone(),
                Arc::clone(&generation.images),
            )
        })
    }

    /// Starts playing `item`. Playback is wired in a later step.
    pub(crate) fn play(&self, item: ItemId) {
        log::debug!("play {item} requested");
    }

    /// Runs `task` on the runtime, then `then` on the Slint thread with its output.
    /// Nothing is delivered once the task is aborted or the application quits.
    pub(crate) fn spawn<T, F, C>(&self, task: F, then: C) -> AbortHandle
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
        C: FnOnce(&Rc<App>, T) + 'static,
    {
        let task = self.runtime.spawn(task);
        let abort = task.abort_handle();
        self.on_ui(
            async move {
                match task.await {
                    Ok(value) => Some(value),
                    Err(error) if error.is_panic() => {
                        log::error!("background task panicked: {error}");
                        None
                    }
                    Err(_) => None,
                }
            },
            move |app, value| {
                if let Some(value) = value {
                    then(app, value);
                }
            },
        );
        abort
    }

    fn on_ui<T, F, C>(&self, future: F, then: C)
    where
        T: 'static,
        F: Future<Output = T> + 'static,
        C: FnOnce(&Rc<App>, T) + 'static,
    {
        let this = self.this.clone();
        let result = slint::spawn_local(async move {
            let value = future.await;
            if let Some(app) = this.upgrade()
                && !app.quitting.get()
            {
                then(&app, value);
            }
        });
        if let Err(error) = result {
            log::error!("cannot schedule work on the event loop: {error}");
        }
    }

    /// Hands a new session to the state machine, which starts its generation.
    pub(crate) fn signed_in(self: &Rc<Self>, signed: SignedIn) {
        let info = session_info(&signed);
        *self.incoming.borrow_mut() = Some(signed.session);
        self.dispatch(Msg::SignedIn(info));
        self.incoming.borrow_mut().take();
    }

    fn restore(&self) {
        let manager = self.manager();
        self.spawn(
            async move { manager.restore().await },
            |app, result| match result {
                Ok(Some(signed)) => {
                    let info = session_info(&signed);
                    *app.incoming.borrow_mut() = Some(signed.session);
                    app.dispatch(Msg::Restored(Some(info)));
                    app.incoming.borrow_mut().take();
                }
                Ok(None) => app.dispatch(Msg::Restored(None)),
                Err(error) => {
                    log::error!("cannot restore the saved session: {error}");
                    app.dispatch(Msg::StartupFailed);
                }
            },
        );
    }

    fn bind(&self, window: &AppWindow) {
        let this = self.this.clone();
        let with = move |action: fn(&Rc<App>)| {
            let this = this.clone();
            move || {
                if let Some(app) = this.upgrade() {
                    action(&app);
                }
            }
        };
        window.on_open_quick_connect(with(|app| app.sign_in.open_quick_connect(app)));
        window.on_cancel_quick_connect(with(|app| app.dispatch(Msg::CloseQuickConnect)));
        window.on_retry_quick_connect(with(|app| app.sign_in.start_quick_connect(app)));
        window.on_open_settings(with(|app| app.dispatch(Msg::OpenSettings)));

        let this = self.this.clone();
        window.on_server_changed(move |address| {
            if let Some(app) = this.upgrade() {
                app.sign_in.server_changed(&app, address);
            }
        });
        let this = self.this.clone();
        window.on_server_confirmed(move |address| {
            if let Some(app) = this.upgrade() {
                app.sign_in.server_confirmed(&app, address);
            }
        });
        let this = self.this.clone();
        window.on_play(move |id| {
            if let Some(app) = this.upgrade() {
                app.play(ItemId::from(id.as_str()));
            }
        });
        let this = self.this.clone();
        window.on_sign_in(move |user, password| {
            if let Some(app) = this.upgrade() {
                app.sign_in.sign_in(&app, user, password);
            }
        });
        let this = self.this.clone();
        window.window().on_close_requested(move || {
            match this.upgrade() {
                Some(app) => app.dispatch(Msg::CloseRequested),
                None => {
                    let _ = slint::quit_event_loop();
                }
            }
            CloseRequestResponse::HideWindow
        });
    }

    fn apply(self: &Rc<Self>, effect: Effect) {
        log::debug!("effect {effect:?}");
        match effect {
            Effect::StartGeneration(id) => {
                let Some(session) = self.incoming.borrow_mut().take() else {
                    log::error!("no session to start generation {id:?} with");
                    return;
                };
                let _guard = self.runtime.enter();
                *self.generation.borrow_mut() = Some(Generation::new(id, session, &self.paths));
            }
            Effect::ValidateSession(id) => {
                let session = match &*self.generation.borrow() {
                    Some(generation) if generation.id == id => generation.session.clone(),
                    _ => return,
                };
                let manager = self.manager();
                self.spawn(
                    async move { manager.validate(&session).await },
                    move |app, result| match result {
                        Ok(()) => {}
                        Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
                        Err(error) => log::warn!("cannot validate the restored session: {error}"),
                    },
                );
            }
            Effect::EndGeneration { id, end } => {
                let generation = self.generation.borrow_mut().take();
                let Some(generation) = generation.filter(|generation| generation.id == id) else {
                    log::error!("generation {id:?} is not the current one");
                    return;
                };
                let manager = self.manager();
                let (done, ended) = tokio::sync::oneshot::channel();
                let task = self.runtime.spawn(async move {
                    generation.end(end, &manager).await;
                    let _ = done.send(());
                });
                let mut ending = self.ending.borrow_mut();
                ending.retain(|task| !task.is_finished());
                ending.push(task);
                drop(ending);
                self.on_ui(ended, move |app, _| app.dispatch(Msg::GenerationEnded(id)));
            }
            Effect::Show(screen) => {
                let previous = self.shown.replace(screen);
                if let Some(window) = self.window() {
                    window.set_notice_visible(false);
                    window.set_screen(slint_screen(screen));
                }
                self.sign_in.shown(self, screen, previous);
                self.home.shown(self, screen);
            }
            Effect::Notify(notice) => {
                if let Some(window) = self.window() {
                    window.set_notice(notice_message(notice));
                    window.set_notice_detail(Default::default());
                    window.set_notice_visible(true);
                }
            }
            Effect::Quit => {
                self.quitting.set(true);
                if let Err(error) = slint::quit_event_loop() {
                    log::warn!("cannot quit the event loop: {error}");
                }
            }
        }
    }
}

fn session_info(signed: &SignedIn) -> SessionInfo {
    SessionInfo {
        user_name: signed.user_name.clone(),
        server_url: signed.server_url.clone(),
        token_in_file: signed.token_storage == TokenStorage::File,
        show_file_token_warning: signed.show_file_token_warning,
    }
}

fn slint_screen(screen: Screen) -> crate::Screen {
    match screen {
        Screen::Starting => crate::Screen::Starting,
        Screen::SignIn => crate::Screen::SignIn,
        Screen::QuickConnect => crate::Screen::QuickConnect,
        Screen::Home => crate::Screen::Home,
        Screen::Player => crate::Screen::Player,
        Screen::Settings => crate::Screen::Settings,
    }
}

fn notice_message(notice: Notice) -> Message {
    match notice {
        Notice::SessionExpired => Message::SessionExpired,
        Notice::FileTokenWarning => Message::FileTokenWarning,
        Notice::StorageError => Message::StorageError,
    }
}
