//! The application: executes the state machine's effects and owns the current generation.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use oxyfin_api::{ItemId, Session};
use oxyfin_core::PlaybackCommand;
use oxyfin_core::{
    Catalog, Config, CoreError, ImageCache, Paths, SessionManager, SignedIn, TokenStorage,
    TokenVault,
};
use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};
use tokio::runtime::Handle;
use tokio::task::{AbortHandle, JoinHandle};

use crate::generation::Generation;
#[cfg(feature = "mpv")]
use crate::generation::{Route, route};
use crate::present::device_name;
use crate::screens::home::HomeController;
use crate::screens::player;
use crate::screens::settings;
use crate::screens::sign_in::SignInController;
use crate::state::{AppState, Effect, GenerationId, Msg, Notice, Screen, SessionInfo};
use crate::{AppWindow, Message, NoticeData};

/// Logical size of the next episode thumbnail, as laid out by the toast.
#[cfg(feature = "mpv")]
const NEXT_UP_THUMB_WIDTH: f32 = 160.0;
#[cfg(feature = "mpv")]
const NEXT_UP_THUMB_HEIGHT: f32 = 90.0;

/// How long a notice that needs no action stays on screen.
pub const NOTICE_DURATION: Duration = Duration::from_secs(5);

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
    pending_stops: Cell<u32>,
    notice_timer: Timer,
    #[cfg(feature = "mpv")]
    video: crate::video::VideoSource,
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
            pending_stops: Cell::new(0),
            notice_timer: Timer::default(),
            #[cfg(feature = "mpv")]
            video: Default::default(),
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

    /// The player of the current generation, published for the video renderer.
    #[cfg(feature = "mpv")]
    pub fn video_source(&self) -> crate::video::VideoSource {
        Rc::clone(&self.video)
    }

    /// Plays `item` on the player screen. Only the home screen starts playback.
    pub(crate) fn play(self: &Rc<Self>, item: ItemId) {
        if self.state.borrow().screen() != Screen::Home {
            return;
        }
        #[cfg(not(feature = "mpv"))]
        {
            log::info!("cannot play {item}: this build has no player");
            self.notify(Message::PlaybackUnavailable);
        }
        #[cfg(feature = "mpv")]
        {
            let handle = match self.playback() {
                Some(Ok(handle)) => handle,
                Some(Err(error)) => {
                    log::error!("cannot create the player: {error}");
                    self.notify(Message::LoadFailed);
                    return;
                }
                None => return,
            };
            if let Some(window) = self.window() {
                player::reset(&window);
            }
            handle.send(PlaybackCommand::Play {
                item,
                mode: oxyfin_core::PlayMode::Auto,
            });
            self.dispatch(Msg::OpenPlayer);
        }
    }

    /// The playback of the current generation, created at its first use.
    #[cfg(feature = "mpv")]
    fn playback(&self) -> Option<Result<oxyfin_core::PlaybackHandle, oxyfin_player::PlayerError>> {
        use oxyfin_core::{PlaybackController, player_events};
        use oxyfin_player::{Player, PlayerOptions};

        let mut current = self.generation.borrow_mut();
        let generation = current.as_mut()?;
        if let Some(playback) = &generation.playback {
            return Some(Ok(playback.handle.clone()));
        }
        let _guard = self.runtime.enter();
        let (sink, events) = player_events();
        let player = match Player::new(PlayerOptions::default(), move |event| sink.send(event)) {
            Ok(player) => Arc::new(player),
            Err(error) => return Some(Err(error)),
        };
        let (handle, updates) =
            PlaybackController::spawn(generation.session.clone(), player.clone(), events);
        self.pending_stops.set(0);
        self.relay(generation.id, updates);
        *self.video.borrow_mut() = Some((generation.id, Arc::clone(&player)));
        generation.playback = Some(crate::generation::Playback {
            handle: handle.clone(),
            player,
        });
        Some(Ok(handle))
    }

    /// Delivers the events of the playback controller of generation `id` on the Slint thread.
    #[cfg(feature = "mpv")]
    fn relay(
        &self,
        id: GenerationId,
        mut updates: tokio::sync::mpsc::UnboundedReceiver<oxyfin_core::PlaybackEvent>,
    ) {
        let this = self.this.clone();
        let result = slint::spawn_local(async move {
            while let Some(event) = updates.recv().await {
                let Some(app) = this.upgrade() else { break };
                if app.quitting.get() {
                    break;
                }
                let current = app.state.borrow().generation();
                match route(current, id, event) {
                    Route::Ignore => {}
                    Route::Expire(id) => app.dispatch(Msg::Expired(id)),
                    Route::Show(event) => app.show_playback(event),
                }
            }
        });
        if let Err(error) = result {
            log::error!("cannot relay the playback events: {error}");
        }
    }

    /// Stops publishing the player of generation `id`; the renderer goes at the next frame.
    #[cfg(feature = "mpv")]
    fn clear_video(&self, id: GenerationId) {
        let player = {
            let mut video = self.video.borrow_mut();
            match &*video {
                Some((published, _)) if *published == id => video.take(),
                _ => None,
            }
        };
        if let Some((_, player)) = player {
            crate::video::release_player(&self.runtime, player);
            if let Some(window) = self.window() {
                window.window().request_redraw();
            }
        }
    }

    /// Shows an event of the current playback, then leaves the player when playback is over.
    #[cfg(feature = "mpv")]
    fn show_playback(self: &Rc<Self>, event: oxyfin_core::PlaybackEvent) {
        use oxyfin_core::PlaybackEvent;

        let Some(window) = self.window() else { return };
        let stopped = event == PlaybackEvent::Stopped;
        let ended = event == PlaybackEvent::Ended;
        if let PlaybackEvent::NextUpCountdown { episode, .. } = &event {
            self.load_next_up_image(&window, episode);
        }
        if let Some(message) = player::apply(&window, event) {
            self.notify(message);
        }

        let on_player = self.state.borrow().screen() == Screen::Player;
        if stopped {
            let expected = self.pending_stops.get();
            if expected > 0 {
                self.pending_stops.set(expected - 1);
            } else if on_player {
                self.dispatch(Msg::ClosePlayer);
                return;
            }
            // The stop report is queued before this event: Continue Watching catches up.
            if self.shown.get() == Screen::Home {
                self.home.shown(self, Screen::Home);
            }
        } else if ended && on_player && !window.get_player_has_next() {
            self.send_playback(PlaybackCommand::Stop);
            self.dispatch(Msg::ClosePlayer);
        }
    }

    /// Loads the thumbnail of the next episode toast, once per episode.
    #[cfg(feature = "mpv")]
    fn load_next_up_image(&self, window: &AppWindow, episode: &oxyfin_core::Episode) {
        let id = slint::SharedString::from(episode.id.as_str());
        if window.get_player_next_up().id == id {
            return;
        }
        let (Some(thumb), Some((_, _, images))) = (episode.thumb.clone(), self.services()) else {
            return;
        };
        let size = crate::present::image_size(
            NEXT_UP_THUMB_WIDTH,
            NEXT_UP_THUMB_HEIGHT,
            window.window().scale_factor(),
        );
        let weak = window.as_weak();
        crate::images::load_into(&images, &self.runtime, thumb, size, move |image| {
            let Some(window) = weak.upgrade() else { return };
            let mut next = window.get_player_next_up();
            if next.id == id {
                next.image = image;
                next.has_image = true;
                window.set_player_next_up(next);
            }
        });
    }

    /// Sends `command` to the current playback. The `Stopped` that answers a
    /// stop sent here is expected: the screen already left the player.
    fn send_playback(&self, command: PlaybackCommand) {
        if let Some(playback) = self
            .generation
            .borrow()
            .as_ref()
            .and_then(|generation| generation.playback.as_ref())
        {
            if command == PlaybackCommand::Stop {
                self.pending_stops.set(self.pending_stops.get() + 1);
            }
            playback.handle.send(command);
        }
    }

    /// Shows `message` in the notice banner, replacing the one shown. Messages
    /// that need no action leave after [`NOTICE_DURATION`]; the others stay
    /// until the user closes them.
    fn notify(&self, message: Message) {
        let Some(window) = self.window() else { return };
        let dismissible = is_persistent(message);
        window.set_notice(NoticeData {
            visible: true,
            message,
            detail: Default::default(),
            dismissible,
        });
        if dismissible {
            self.notice_timer.stop();
            return;
        }
        let weak = window.as_weak();
        self.notice_timer
            .start(TimerMode::SingleShot, NOTICE_DURATION, move || {
                if let Some(window) = weak.upgrade() {
                    hide_notice(&window);
                }
            });
    }

    /// The user closed the notice; closing the file-token warning acknowledges it.
    fn dismiss_notice(&self) {
        let Some(window) = self.window() else { return };
        let notice = window.get_notice();
        if !notice.visible {
            return;
        }
        self.notice_timer.stop();
        hide_notice(&window);
        if notice.message == Message::FileTokenWarning {
            let manager = self.manager();
            self.runtime.spawn_blocking(move || {
                if let Err(error) = manager.acknowledge_file_token_warning() {
                    log::warn!("cannot record the file-token warning as seen: {error}");
                }
            });
        }
    }

    /// Hides the notice unless it still applies to `screen`.
    fn notice_on(&self, window: &AppWindow, screen: Screen) {
        let notice = window.get_notice();
        if notice.visible && !keeps_notice(notice.message, screen) {
            self.notice_timer.stop();
            hide_notice(window);
        }
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
        window.on_close_settings(with(|app| app.dispatch(Msg::CloseSettings)));
        window.on_sign_out(with(|app| app.dispatch(Msg::SignOut)));
        window.on_dismiss_notice(with(|app| app.dismiss_notice()));

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
        window.on_close_player(with(|app| app.dispatch(Msg::ClosePlayer)));
        let this = self.this.clone();
        player::bind(
            window,
            Rc::new(move |command| {
                if let Some(app) = this.upgrade() {
                    app.send_playback(command);
                }
            }),
        );
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
                let generation = {
                    let mut current = self.generation.borrow_mut();
                    match &*current {
                        Some(generation) if generation.id == id => current.take(),
                        _ => None,
                    }
                };
                let Some(generation) = generation else {
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
                self.on_ui(ended, move |app, _| {
                    #[cfg(feature = "mpv")]
                    app.clear_video(id);
                    app.dispatch(Msg::GenerationEnded(id));
                });
            }
            Effect::Show(screen) => {
                let previous = self.shown.replace(screen);
                if let Some(window) = self.window() {
                    self.notice_on(&window, screen);
                    if screen != Screen::Player {
                        player::leave(&window);
                    }
                    if screen == Screen::Settings {
                        settings::show(&window, self.state.borrow().session());
                    }
                    window.set_screen(slint_screen(screen));
                }
                self.sign_in.shown(self, screen, previous);
                self.home.shown(self, screen);
            }
            Effect::Notify(notice) => self.notify(notice_message(notice)),
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

fn hide_notice(window: &AppWindow) {
    let mut notice = window.get_notice();
    notice.visible = false;
    window.set_notice(notice);
}

/// Whether `message` stays until the user closes it, rather than leaving on its own.
fn is_persistent(message: Message) -> bool {
    matches!(
        message,
        Message::SessionExpired | Message::FileTokenWarning | Message::StorageError
    )
}

/// Whether a notice showing `message` stays when `screen` is shown.
fn keeps_notice(message: Message, screen: Screen) -> bool {
    match message {
        Message::FileTokenWarning => {
            matches!(screen, Screen::Home | Screen::Player | Screen::Settings)
        }
        Message::SessionExpired | Message::StorageError => {
            matches!(screen, Screen::SignIn | Screen::QuickConnect)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_messages_that_need_the_user_persist() {
        assert!(is_persistent(Message::SessionExpired));
        assert!(is_persistent(Message::FileTokenWarning));
        assert!(is_persistent(Message::StorageError));
        assert!(!is_persistent(Message::CommandFailed));
        assert!(!is_persistent(Message::PlaybackUnavailable));
        assert!(!is_persistent(Message::LoadFailed));
    }

    #[test]
    fn notices_stay_only_where_they_still_apply() {
        assert!(keeps_notice(Message::FileTokenWarning, Screen::Settings));
        assert!(!keeps_notice(Message::FileTokenWarning, Screen::SignIn));
        assert!(keeps_notice(Message::SessionExpired, Screen::QuickConnect));
        assert!(!keeps_notice(Message::SessionExpired, Screen::Home));
        assert!(!keeps_notice(Message::CommandFailed, Screen::Player));
    }
}
