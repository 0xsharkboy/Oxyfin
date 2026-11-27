//! The application: executes the state machine's effects and owns the current generation.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use oxyfin_api::{ItemId, Session};
use oxyfin_core::{
    Catalog, Config, CoreError, ImageCache, ImagePriority, Paths, SessionManager, SignedIn,
    TokenVault,
};
use oxyfin_core::{PlayMode, PlaybackCommand, Quality, UserState};
use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};
use tokio::runtime::Handle;
use tokio::task::{AbortHandle, JoinHandle};

use crate::back_cache::{BackCache, Parked};
use crate::generation::Generation;
#[cfg(feature = "mpv")]
use crate::generation::{Route, route};
use crate::present::device_name;
use crate::screens::cast::open_person;
use crate::screens::collection::CollectionController;
use crate::screens::home::HomeController;
use crate::screens::library::LibraryController;
use crate::screens::movie::MovieController;
use crate::screens::person::PersonController;
use crate::screens::player;
use crate::screens::search::SearchController;
use crate::screens::series::SeriesController;
use crate::screens::settings;
use crate::screens::sign_in::SignInController;
use crate::state::{
    AppState, Effect, GenerationId, LibraryTab, Msg, Notice, Screen, SessionEnd, SessionInfo, Tab,
};
use crate::{AppWindow, Message, MouseBack, NoticeAction, NoticeData, Shortcuts, TopBarPage};

/// Logical size of the next episode thumbnail, as laid out by the toast.
#[cfg(feature = "mpv")]
const NEXT_UP_THUMB_WIDTH: f32 = 160.0;
#[cfg(feature = "mpv")]
const NEXT_UP_THUMB_HEIGHT: f32 = 90.0;

/// Logical size of a chapter thumbnail in the chapters menu.
#[cfg(feature = "mpv")]
const CHAPTER_THUMB_WIDTH: f32 = 64.0;
#[cfg(feature = "mpv")]
const CHAPTER_THUMB_HEIGHT: f32 = 36.0;

/// Height of the artwork shown to the desktop over MPRIS, in pixels.
#[cfg(all(target_os = "linux", feature = "mpv"))]
const MPRIS_ART_HEIGHT: u32 = 512;

/// Opens a folder in the file manager.
type FolderOpener = dyn Fn(&std::path::Path);

/// Puts text on the clipboard of a window and tells whether it got there.
type Clipboard = dyn Fn(&AppWindow, &str) -> bool;

/// Changes of a list that wait until nothing points at it.
type Waiting = Vec<Box<dyn FnOnce(&Rc<App>)>>;

/// How long a notice that needs no action stays on screen.
pub const NOTICE_DURATION: Duration = Duration::from_secs(5);

/// How long the volume stays unchanged before it is saved.
pub const VOLUME_SAVE_DELAY: Duration = Duration::from_secs(1);

/// How long nothing on screen is pointed at before the changes that wait for it are shown.
const RELEASE_DELAY: Duration = Duration::from_millis(250);

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
    shown: RefCell<Screen>,
    /// Number of the page shown, growing with every page.
    page: Cell<u64>,
    generation: RefCell<Option<Generation>>,
    incoming: RefCell<Option<Session>>,
    ending: RefCell<Vec<JoinHandle<()>>>,
    quitting: Cell<bool>,
    pending_stops: Cell<u32>,
    notice_timer: Timer,
    volume_timer: Timer,
    /// A volume set but not saved yet.
    pending_volume: Cell<Option<f64>>,
    /// The lower quality the notice offers.
    advised: Cell<Option<Quality>>,
    sheets: RefCell<crate::images::SheetCache>,
    back_cache: RefCell<BackCache<Parked>>,
    waiting: RefCell<Waiting>,
    release_timer: Timer,
    #[cfg(feature = "mpv")]
    video: crate::video::VideoSource,
    #[cfg(feature = "mpv")]
    creating_player: Cell<Option<GenerationId>>,
    #[cfg(all(target_os = "linux", feature = "mpv"))]
    mpris: RefCell<Option<crate::mpris::Mpris>>,
    #[cfg(all(target_os = "linux", feature = "mpv"))]
    inhibitor: RefCell<Option<crate::inhibit::Inhibitor>>,
    discord: RefCell<Option<crate::discord::Discord>>,
    folder_opener: RefCell<Rc<FolderOpener>>,
    clipboard: RefCell<Rc<Clipboard>>,
    /// Hosts of the servers met, hidden from copied logs.
    known_hosts: RefCell<std::collections::BTreeSet<String>>,
    mpv_version: crate::logs::MpvVersion,
    pub(crate) sign_in: SignInController,
    pub(crate) home: HomeController,
    pub(crate) library: LibraryController,
    pub(crate) search: SearchController,
    pub(crate) movie: MovieController,
    pub(crate) person: PersonController,
    pub(crate) collection: CollectionController,
    pub(crate) series: SeriesController,
    pub(crate) hover: crate::hover::HoverController,
    pub(crate) menu: crate::menu::MenuController,
    pub(crate) prefetch: crate::prefetch::Prefetcher,
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
            shown: RefCell::new(Screen::Starting),
            page: Cell::new(0),
            generation: RefCell::new(None),
            incoming: RefCell::new(None),
            ending: RefCell::new(Vec::new()),
            quitting: Cell::new(false),
            pending_stops: Cell::new(0),
            notice_timer: Timer::default(),
            volume_timer: Timer::default(),
            pending_volume: Cell::new(None),
            advised: Cell::new(None),
            sheets: RefCell::default(),
            back_cache: RefCell::default(),
            waiting: RefCell::default(),
            release_timer: Timer::default(),
            #[cfg(feature = "mpv")]
            video: Default::default(),
            #[cfg(feature = "mpv")]
            creating_player: Cell::new(None),
            #[cfg(all(target_os = "linux", feature = "mpv"))]
            mpris: RefCell::new(None),
            #[cfg(all(target_os = "linux", feature = "mpv"))]
            inhibitor: RefCell::new(None),
            discord: RefCell::new(None),
            folder_opener: RefCell::new(Rc::new(crate::logs::open_folder)),
            clipboard: RefCell::new(Rc::new(|window: &AppWindow, text: &str| {
                window.invoke_copy_to_clipboard(text.into())
            })),
            known_hosts: RefCell::default(),
            mpv_version: Default::default(),
            sign_in: SignInController::default(),
            home: HomeController::default(),
            library: LibraryController::default(),
            search: SearchController::default(),
            movie: MovieController::default(),
            person: PersonController::default(),
            collection: CollectionController::default(),
            series: SeriesController::default(),
            hover: Default::default(),
            menu: Default::default(),
            prefetch: Default::default(),
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

    /// Reads the libmpv version for bug reports and logs it, without waiting for it.
    pub fn read_mpv_version(&self) {
        self.mpv_version.read_in_background(mpv_version);
    }

    /// Whether the window opens fullscreen, as the settings saved it.
    pub fn start_fullscreen(&self) -> bool {
        self.manager.start_fullscreen()
    }

    /// Sends server discovery requests to these addresses instead of the local network
    /// broadcast, for tests.
    pub fn set_discovery_targets(&self, targets: Vec<std::net::SocketAddr>) {
        self.manager.set_discovery_targets(targets);
    }

    /// Turns the background loading of what is likely to be opened next on or off, for tests.
    #[doc(hidden)]
    pub fn set_prefetch(&self, on: bool) {
        self.prefetch.enable(on);
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

    /// The current screen.
    pub(crate) fn screen(&self) -> Screen {
        self.state.borrow().screen()
    }

    /// The user state of item `id` changed to `user` on the current screen: a grid shows it on
    /// its card, other screens reload, and the screens below reload when they show again.
    pub(crate) fn item_changed(self: &Rc<Self>, id: &ItemId, user: &UserState) {
        let screen = self.screen();
        match &screen {
            Screen::Library(_) => self.library.patch(self, id, user),
            Screen::Search => self.search.patch(self, id, user),
            Screen::Home => self.home.changed(self),
            _ => self.refresh(&screen),
        }
        self.dispatch(Msg::ItemChanged(screen));
    }

    /// Priority of the images of the page shown: they go before those of the pages left.
    pub(crate) fn image_priority(&self) -> ImagePriority {
        ImagePriority::of_page(self.page.get())
    }

    /// Depth of the current screen in the stack, 0 for the root.
    pub(crate) fn top_depth(&self) -> usize {
        self.state.borrow().stack().len().saturating_sub(1)
    }

    /// Whether the stack holds `screen` at `depth`.
    pub(crate) fn holds(&self, depth: usize, screen: &Screen) -> bool {
        self.state.borrow().stack().get(depth) == Some(screen)
    }

    /// Keeps `page`, the content of `screen` at `depth`, for Back to show it again; nothing is
    /// kept for a screen that left that place or without a generation.
    pub(crate) fn park(&self, depth: usize, screen: Screen, page: Parked) {
        let Some((generation, ..)) = self.services() else {
            return;
        };
        if self.holds(depth, &screen) {
            self.back_cache
                .borrow_mut()
                .park(depth, screen, generation, page);
        }
    }

    /// Takes the page kept for `screen`, the current screen.
    pub(crate) fn unpark(&self, screen: &Screen) -> Option<Parked> {
        let (generation, ..) = self.services()?;
        let depth = self.top_depth();
        self.back_cache.borrow_mut().take(depth, screen, generation)
    }

    /// Runs `change`, which adds, removes or moves what is on screen, once the pointer rests on
    /// nothing it could move and no menu is open; at the latest when another page shows.
    pub(crate) fn when_free(self: &Rc<Self>, change: impl FnOnce(&Rc<App>) + 'static) {
        if self
            .window()
            .is_some_and(|window| window.get_content_held())
        {
            self.waiting.borrow_mut().push(Box::new(change));
        } else {
            change(self);
        }
    }

    /// Runs what [`App::when_free`] held back.
    pub(crate) fn run_waiting(self: &Rc<Self>) {
        let waiting = std::mem::take(&mut *self.waiting.borrow_mut());
        for change in waiting {
            change(self);
        }
    }

    /// The pointer left what it rested on, perhaps only to reach what is beside it.
    fn content_released(&self) {
        let this = self.this.clone();
        self.release_timer
            .start(TimerMode::SingleShot, RELEASE_DELAY, move || {
                let Some(app) = this.upgrade() else { return };
                if app
                    .window()
                    .is_some_and(|window| !window.get_content_held())
                {
                    app.run_waiting();
                }
            });
    }

    /// Forgets the answers kept for the pages: what the user did made them wrong.
    fn forget_answers(&self) {
        if let Some((_, catalog, _)) = self.services() {
            catalog.forget_answers();
        }
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

    /// Tells the user that the video of generation `id` cannot be shown, if it is still current.
    #[cfg(feature = "mpv")]
    pub fn video_failed(&self, id: GenerationId) {
        let current = self
            .generation
            .borrow()
            .as_ref()
            .map(|generation| generation.id);
        if current == Some(id) {
            self.notify(Message::LoadFailed);
        }
    }

    /// Shows the playback to the desktop over MPRIS and performs the calls
    /// it receives, until the app is gone.
    #[cfg(all(target_os = "linux", feature = "mpv"))]
    pub fn start_mpris(&self) {
        use crate::mpris::{Bus, Mpris, Update, perform};

        let (mpris, mut requests) = Mpris::start(&self.runtime, Bus::Session);
        mpris.update(Update::Active(
            self.state.borrow().screen() == Screen::Player,
        ));
        *self.mpris.borrow_mut() = Some(mpris);
        let this = self.this.clone();
        let result = slint::spawn_local(async move {
            while let Some(request) = requests.recv().await {
                let Some(window) = this.upgrade().and_then(|app| app.window()) else {
                    break;
                };
                perform(&window, request);
            }
        });
        if let Err(error) = result {
            log::warn!("cannot receive the MPRIS calls: {error}");
        }
    }

    /// Keeps the screen awake while an item plays on the player screen, until
    /// [`App::take_inhibitor`].
    #[cfg(all(target_os = "linux", feature = "mpv"))]
    pub fn start_inhibitor(&self) {
        use crate::inhibit::{Inhibitor, Update};

        let inhibitor = Inhibitor::start(&self.runtime, crate::mpris::Bus::Session);
        inhibitor.update(Update::Active(
            self.state.borrow().screen() == Screen::Player,
        ));
        *self.inhibitor.borrow_mut() = Some(inhibitor);
    }

    /// Takes the screen inhibitor, for `main` to release it before exiting.
    #[cfg(all(target_os = "linux", feature = "mpv"))]
    pub fn take_inhibitor(&self) -> Option<crate::inhibit::Inhibitor> {
        self.inhibitor.borrow_mut().take()
    }

    /// Shows what plays on Discord while the setting is on, until
    /// [`App::take_discord`].
    pub fn start_discord(&self) {
        match crate::discord::Discord::start(self.manager.discord()) {
            Ok(discord) => {
                discord.update(crate::discord::Update::Active(
                    self.state.borrow().screen() == Screen::Player,
                ));
                *self.discord.borrow_mut() = Some(discord);
            }
            Err(error) => log::warn!("cannot start the Discord activity: {error}"),
        }
    }

    /// Takes the Discord activity, for `main` to clear it before exiting.
    pub fn take_discord(&self) -> Option<crate::discord::Discord> {
        self.discord.borrow_mut().take()
    }

    fn discord_update(&self, update: crate::discord::Update) {
        if let Some(discord) = &*self.discord.borrow() {
            discord.update(update);
        }
    }

    #[cfg(all(target_os = "linux", feature = "mpv"))]
    fn inhibit_update(&self, update: crate::inhibit::Update) {
        if let Some(inhibitor) = &*self.inhibitor.borrow() {
            inhibitor.update(update);
        }
    }

    #[cfg(all(target_os = "linux", feature = "mpv"))]
    fn mpris_update(&self, update: crate::mpris::Update) {
        if let Some(mpris) = &*self.mpris.borrow() {
            mpris.update(update);
        }
    }

    /// Plays `item` on the player screen. Only the home screen and the detail pages start playback.
    pub(crate) fn play(self: &Rc<Self>, item: ItemId, mode: PlayMode) {
        if !self.state.borrow().screen().can_play() {
            return;
        }
        #[cfg(not(feature = "mpv"))]
        {
            log::info!("cannot play {item} ({mode:?}): this build has no player");
            self.notify(Message::PlaybackUnavailable);
        }
        #[cfg(feature = "mpv")]
        {
            let (id, handle) = {
                let current = self.generation.borrow();
                let Some(generation) = current.as_ref() else {
                    return;
                };
                let handle = generation
                    .playback
                    .as_ref()
                    .map(|playback| playback.handle.clone());
                (generation.id, handle)
            };
            match handle {
                Some(handle) => self.start_playback(&handle, item, mode),
                None if self.creating_player.get() == Some(id) => {}
                None => self.create_player(id, item, mode),
            }
        }
    }

    /// Opens the player screen and plays `item` on `handle`.
    #[cfg(feature = "mpv")]
    fn start_playback(
        self: &Rc<Self>,
        handle: &oxyfin_core::PlaybackHandle,
        item: ItemId,
        mode: PlayMode,
    ) {
        if let Some(window) = self.window() {
            player::reset(&window);
            player::show_quality(&window, self.manager.quality());
        }
        // The position of what plays changes from now on.
        self.forget_answers();
        self.check_subtitle_permission();
        handle.send(PlaybackCommand::Play { item, mode });
        self.dispatch(Msg::OpenPlayer);
    }

    /// Asks the server whether the user may search subtitles, which the track menu offers then.
    #[cfg(feature = "mpv")]
    fn check_subtitle_permission(&self) {
        // Another account may have signed in since the last answer.
        if let Some(window) = self.window() {
            window.global::<crate::SubtitleSearch>().set_allowed(false);
        }
        let Some((id, catalog, _)) = self.services() else {
            return;
        };
        self.spawn(
            async move { catalog.can_manage_subtitles().await },
            move |app, result| {
                if app.services().map(|(current, _, _)| current) != Some(id) {
                    return;
                }
                let allowed = result.unwrap_or_else(|error| {
                    log::warn!("cannot read the subtitle permission: {error}");
                    false
                });
                if let Some(window) = app.window() {
                    window
                        .global::<crate::SubtitleSearch>()
                        .set_allowed(allowed);
                }
            },
        );
    }

    /// Builds the player of generation `id` off the Slint thread, then plays `item` with it
    /// if the screen that asked for it is still shown.
    #[cfg(feature = "mpv")]
    fn create_player(&self, id: GenerationId, item: ItemId, mode: PlayMode) {
        let from = self.state.borrow().screen();
        use oxyfin_core::player_events;
        use oxyfin_player::{Player, PlayerOptions};

        self.creating_player.set(Some(id));
        let runtime = self.runtime.clone();
        let options = PlayerOptions {
            volume: Some(self.manager.volume()),
            ..PlayerOptions::default()
        };
        self.spawn(
            async move {
                runtime
                    .spawn_blocking(move || {
                        let (sink, events) = player_events();
                        Player::new(options, move |event| sink.send(event))
                            .map(|player| (Arc::new(player), events))
                            .map_err(|error| error.to_string())
                    })
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|built| built)
            },
            move |app, built| app.player_built(id, item, mode, from, built),
        );
    }

    /// Installs a player built for generation `id`, or releases it when that generation ended.
    #[cfg(feature = "mpv")]
    fn player_built(
        self: &Rc<Self>,
        id: GenerationId,
        item: ItemId,
        mode: PlayMode,
        from: Screen,
        built: Result<(Arc<oxyfin_player::Player>, oxyfin_core::PlayerEvents), String>,
    ) {
        use crate::generation::{Built, Playback};
        use oxyfin_core::{PlaybackController, TrackMemory, account_key};

        if self.creating_player.get() == Some(id) {
            self.creating_player.set(None);
        }
        let current = self
            .generation
            .borrow()
            .as_ref()
            .map(|generation| generation.id);
        let (player, events) = match built {
            Ok(built) => built,
            Err(error) => {
                log::error!("cannot create the player: {error}");
                if current == Some(id) {
                    self.notify(Message::LoadFailed);
                }
                return;
            }
        };
        let decision = crate::generation::built(id, current, &self.state.borrow().screen(), &from);
        let handle = {
            let mut current = self.generation.borrow_mut();
            match current.as_mut() {
                Some(generation) if decision != Built::Release && generation.playback.is_none() => {
                    let _guard = self.runtime.enter();
                    let session = &generation.session;
                    let account = account_key(
                        session.server().base_url().as_str(),
                        session.user_id().as_str(),
                    );
                    let (handle, updates) = PlaybackController::spawn(
                        session.clone(),
                        player.clone(),
                        events,
                        TrackMemory::load(&self.paths, &account),
                    );
                    handle.send(PlaybackCommand::SetSubtitleStyle(
                        self.manager.subtitle_style(),
                    ));
                    handle.send(PlaybackCommand::SetQuality(self.manager.quality()));
                    handle.send(PlaybackCommand::SetPrefs(self.manager.playback_prefs()));
                    self.pending_stops.set(0);
                    self.relay(id, updates);
                    *self.video.borrow_mut() = Some((id, Arc::clone(&player)));
                    generation.playback = Some(Playback {
                        handle: handle.clone(),
                        player,
                    });
                    handle
                }
                _ => {
                    drop(current);
                    crate::video::release_player(&self.runtime, player);
                    return;
                }
            }
        };
        if decision == Built::Play {
            self.start_playback(&handle, item, mode);
        }
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
    pub fn show_playback(self: &Rc<Self>, event: oxyfin_core::PlaybackEvent) {
        use oxyfin_core::PlaybackEvent;

        let Some(window) = self.window() else { return };
        if event == PlaybackEvent::StopReported {
            return self.stop_reported();
        }
        let stopped = event == PlaybackEvent::Stopped;
        let ended = event == PlaybackEvent::Ended;
        if stopped {
            self.sheets.borrow_mut().clear();
            self.forget_answers();
        }
        if let PlaybackEvent::NextUpCountdown { episode, .. }
        | PlaybackEvent::NextUpReady { episode } = &event
        {
            self.load_next_up_image(&window, episode);
        }
        #[cfg(target_os = "linux")]
        self.show_mpris(&event);
        #[cfg(target_os = "linux")]
        if crate::inhibit::concerns(&event) {
            self.inhibit_update(crate::inhibit::Update::Event(event.clone()));
        }
        if crate::discord::concerns(&event) {
            self.discord_update(crate::discord::Update::Event(event.clone()));
        }
        let chapters = match &event {
            PlaybackEvent::Chapters(chapters) => Some(chapters.clone()),
            _ => None,
        };
        let advised = match &event {
            PlaybackEvent::LowerQualityAdvised(quality) => Some(*quality),
            _ => None,
        };
        match player::apply(&window, event) {
            Some(message) if advised.is_some() => {
                self.advised.set(advised);
                self.show_notice(message, NoticeAction::LowerQuality);
            }
            Some(message) => self.notify(message),
            None => {}
        }
        if let Some(chapters) = chapters {
            self.load_chapter_images(&window, &chapters);
        }

        let on_player = self.state.borrow().screen() == Screen::Player;
        if stopped {
            let expected = self.pending_stops.get();
            if expected > 0 {
                self.pending_stops.set(expected - 1);
            } else if on_player {
                self.dispatch(Msg::ClosePlayer);
            }
            // The answer to the stop report reloads the screen shown.
            self.progress_changed(false);
        } else if ended && on_player && !window.get_player_has_next() {
            self.send_playback(PlaybackCommand::Stop);
            self.dispatch(Msg::ClosePlayer);
        }
    }

    /// The server knows where playback stopped: what was asked for before may hold the old
    /// position, so nothing of it is kept and the screen shown is asked for again.
    #[cfg(feature = "mpv")]
    fn stop_reported(self: &Rc<Self>) {
        self.forget_answers();
        self.progress_changed(true);
    }

    /// Playback moved the progress of an item: the screens below the one shown reload when
    /// they show again, and `refresh` reloads the one shown unless the player covers it.
    #[cfg(feature = "mpv")]
    fn progress_changed(self: &Rc<Self>, refresh: bool) {
        let stacked = self.state.borrow().stack().to_vec();
        let Some(top) = stacked
            .iter()
            .rev()
            .find(|screen| **screen != Screen::Player)
        else {
            return;
        };
        if refresh && stacked.last() == Some(top) {
            self.refresh(top);
        }
        self.dispatch(Msg::ItemChanged(top.clone()));
    }

    /// Forwards a playback event to MPRIS, with the artwork of a new item.
    #[cfg(all(target_os = "linux", feature = "mpv"))]
    fn show_mpris(&self, event: &oxyfin_core::PlaybackEvent) {
        use crate::mpris::Update;

        self.mpris_update(Update::Event(event.clone()));
        if let oxyfin_core::PlaybackEvent::Loading { title, .. } = event {
            let generation = self.generation.borrow();
            let art = generation
                .as_ref()
                .zip(title.artwork.as_ref())
                .map(|(generation, art)| {
                    let mut request =
                        oxyfin_api::ImageRequest::new(art.item_id.clone(), art.image_type.clone());
                    request.tag = Some(art.tag.clone());
                    request.max_height = Some(MPRIS_ART_HEIGHT);
                    generation.session.image_url(&request).to_string()
                });
            drop(generation);
            self.mpris_update(Update::Artwork(art));
        }
    }

    /// Loads the thumbnail of the next episode toast, once per episode.
    #[cfg(feature = "mpv")]
    fn load_next_up_image(&self, window: &AppWindow, episode: &oxyfin_core::Episode) {
        let id = slint::SharedString::from(episode.id.as_str());
        if window.get_player_next_up().id == id {
            return;
        }
        let (Some(thumb), Some((generation, _, images))) = (episode.thumb.clone(), self.services())
        else {
            return;
        };
        let size = crate::present::image_size(
            NEXT_UP_THUMB_WIDTH,
            NEXT_UP_THUMB_HEIGHT,
            window.window().scale_factor(),
        );
        let weak = window.as_weak();
        crate::images::load_into(self, generation, &images, thumb, size, move |image| {
            let Some(window) = weak.upgrade() else { return };
            let mut next = window.get_player_next_up();
            if next.id == id {
                next.image = image;
                next.has_image = true;
                window.set_player_next_up(next);
            }
        });
    }

    /// Loads the thumbnails of the chapters menu the window shows, each into
    /// its row while the window still shows those chapters.
    #[cfg(feature = "mpv")]
    fn load_chapter_images(&self, window: &AppWindow, chapters: &[oxyfin_core::Chapter]) {
        let Some((generation, _, images)) = self.services() else {
            return;
        };
        let size = crate::present::image_size(
            CHAPTER_THUMB_WIDTH,
            CHAPTER_THUMB_HEIGHT,
            window.window().scale_factor(),
        );
        let rows = window.get_player_chapters();
        for (index, chapter) in chapters.iter().enumerate() {
            let Some(image) = chapter.image.clone() else {
                continue;
            };
            let (weak, rows) = (window.as_weak(), rows.clone());
            crate::images::load_into(self, generation, &images, image, size, move |image| {
                show_chapter_image(&weak, &rows, index, image);
            });
        }

        let mut sheets: Vec<(oxyfin_core::TrickplaySheet, Vec<_>)> = Vec::new();
        for (index, tile) in player::chapter_tiles(window, chapters) {
            match sheets.iter_mut().find(|(sheet, _)| *sheet == tile.sheet) {
                Some((_, tiles)) => tiles.push((index, tile)),
                None => sheets.push((tile.sheet.clone(), vec![(index, tile)])),
            }
        }
        for (sheet, tiles) in sheets {
            let weak = window.as_weak();
            let decoded = self.sheets.borrow_mut().get(&sheet);
            if let Some(image) = decoded {
                show_chapter_tiles(&weak, &rows, &image, &tiles);
                continue;
            }
            let rows = rows.clone();
            let loaded = sheet.clone();
            crate::images::load_sheet(self, generation, &images, sheet, move |app, image| {
                let Some(image) = image else { return };
                app.sheets.borrow_mut().finish(&loaded, Some(image.clone()));
                show_chapter_tiles(&weak, &rows, &image, &tiles);
            });
        }
    }

    /// Shows `sheet` in the seek bar preview: at once when it is decoded
    /// already, otherwise once fetched and decoded off the Slint thread.
    fn load_trickplay_sheet(&self, sheet: oxyfin_core::TrickplaySheet) {
        let Some(window) = self.window() else { return };
        let decoded = self.sheets.borrow_mut().get(&sheet);
        if let Some(image) = decoded {
            player::show_sheet(&window, &sheet, image);
            return;
        }
        let Some((generation, _, images)) = self.services() else {
            return;
        };
        if !self.sheets.borrow_mut().start(&sheet) {
            return;
        }
        let loaded = sheet.clone();
        crate::images::load_sheet(self, generation, &images, sheet, move |app, image| {
            app.sheets.borrow_mut().finish(&loaded, image.clone());
            if let (Some(image), Some(window)) = (image, app.window()) {
                player::show_sheet(&window, &loaded, image);
            }
        });
    }

    /// Sends `command` to the current playback. The `Stopped` that answers a
    /// stop sent here is expected: the screen already left the player.
    fn send_playback(&self, command: PlaybackCommand) {
        if let PlaybackCommand::SetQuality(quality) = command
            && let Err(error) = self.manager.set_quality(quality)
        {
            log::warn!("cannot save the streaming quality: {error}");
        }
        if let PlaybackCommand::SetVolume(volume) = command {
            // Saved once the volume rests, not on each step of a drag or of the wheel.
            self.pending_volume.set(Some(volume));
            let this = self.this.clone();
            self.volume_timer
                .start(TimerMode::SingleShot, VOLUME_SAVE_DELAY, move || {
                    if let Some(app) = this.upgrade() {
                        app.save_volume();
                    }
                });
        }
        if command == PlaybackCommand::Stop {
            self.save_volume();
        }
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

    /// Saves the volume set last, if it is not saved yet.
    fn save_volume(&self) {
        self.volume_timer.stop();
        if let Some(volume) = self.pending_volume.take()
            && let Err(error) = self.manager.set_volume(volume)
        {
            log::warn!("cannot save the volume: {error}");
        }
    }

    /// Saves the subtitle style the settings picked and applies it to the
    /// current playback; the settings then show what is saved.
    fn set_subtitle_style(&self, choice: &crate::SubtitleChoice) {
        let style = settings::subtitle_style(choice);
        match self.manager.set_subtitle_style(style) {
            Ok(()) => self.send_playback(PlaybackCommand::SetSubtitleStyle(style)),
            Err(error) => {
                log::error!("cannot save the subtitle style: {error}");
                self.notify(Message::StorageError);
            }
        }
        if let Some(window) = self.window() {
            window.set_settings_subtitles(settings::subtitle_choice(self.manager.subtitle_style()));
        }
    }

    /// Saves whether the window opens fullscreen, from the next launch on; the
    /// settings then show what is saved.
    fn set_start_fullscreen(&self, on: bool) {
        if let Err(error) = self.manager.set_start_fullscreen(on) {
            log::error!("cannot save the fullscreen setting: {error}");
            self.notify(Message::StorageError);
        }
        if let Some(window) = self.window() {
            window.set_settings_start_fullscreen(self.manager.start_fullscreen());
        }
    }

    /// Saves whether Discord shows what plays and applies it at once; the
    /// settings then show what is saved.
    fn set_discord(&self, on: bool) {
        if let Err(error) = self.manager.set_discord(on) {
            log::error!("cannot save the Discord setting: {error}");
            self.notify(Message::StorageError);
        }
        let on = self.manager.discord();
        self.discord_update(crate::discord::Update::Enabled(on));
        if let Some(window) = self.window() {
            window.set_settings_discord(on);
        }
    }

    /// Saves the playback choices the settings picked and applies them to the
    /// current playback; the settings then show what is saved.
    fn set_playback_prefs(&self, choice: &crate::PlaybackChoice) {
        let prefs = settings::playback_prefs(choice, self.manager.playback_prefs());
        match self.manager.set_playback_prefs(prefs) {
            Ok(()) => self.send_playback(PlaybackCommand::SetPrefs(prefs)),
            Err(error) => {
                log::error!("cannot save the playback choices: {error}");
                self.notify(Message::StorageError);
            }
        }
        if let Some(window) = self.window() {
            window.set_settings_playback(settings::playback_choice(self.manager.playback_prefs()));
        }
    }

    /// Measures the image cache for the Diagnostics section of the settings.
    fn measure_image_cache(&self) {
        let Some((_, _, images)) = self.services() else {
            return;
        };
        self.spawn(async move { images.disk_usage().await }, |app, bytes| {
            if let Some(window) = app.window() {
                window.set_settings_image_cache_kb(settings::cache_kb(bytes));
            }
        });
    }

    fn clear_image_cache(&self) {
        let Some((_, _, images)) = self.services() else {
            return;
        };
        self.forget_answers();
        if let Some(window) = self.window() {
            window.set_settings_image_cache_kb(-1);
        }
        self.spawn(async move { images.clear().await }, |app, result| {
            if let Err(error) = result {
                log::warn!("{error}");
            }
            app.measure_image_cache();
        });
    }

    /// Remembers the host of a server, which copied logs never show.
    pub(crate) fn knows_server(&self, address: &str) {
        if let Some(host) = crate::logs::host_of(address) {
            self.known_hosts.borrow_mut().insert(host);
        }
    }

    fn copy_logs(&self) {
        let last = self.manager.last_server_url();
        for address in last.into_iter().chain(
            self.state
                .borrow()
                .session()
                .map(|info| info.server_url.clone()),
        ) {
            self.knows_server(&address);
        }
        let hosts: Vec<String> = self.known_hosts.borrow().iter().cloned().collect();
        let dir = crate::logs::log_dir(&self.paths);
        let mpv = self.mpv_version.get();
        let report = async move {
            tokio::task::spawn_blocking(move || {
                crate::logs::report(&dir, &crate::logs::header(&mpv), &hosts)
            })
            .await
        };
        self.spawn(report, |app, report| match report {
            Ok(text) => app.put_on_clipboard(&text),
            Err(error) => log::error!("cannot gather the logs: {error}"),
        });
    }

    fn put_on_clipboard(&self, text: &str) {
        let Some(window) = self.window() else { return };
        let clipboard = Rc::clone(&*self.clipboard.borrow());
        let copied = clipboard(&window, text);
        if !copied {
            log::warn!("the logs could not be copied to the clipboard");
        }
        window.set_settings_logs_copied(copied);
        window.set_settings_logs_copy_failed(!copied);
    }

    /// Replaces what puts text on the clipboard and tells whether it got there, for tests.
    #[doc(hidden)]
    pub fn set_clipboard(&self, clipboard: impl Fn(&AppWindow, &str) -> bool + 'static) {
        *self.clipboard.borrow_mut() = Rc::new(clipboard);
    }

    fn open_logs(&self) {
        let opener = Rc::clone(&*self.folder_opener.borrow());
        opener(&crate::logs::log_dir(&self.paths));
    }

    /// Replaces what opens the log folder, for tests that must not start a file manager.
    #[doc(hidden)]
    pub fn set_folder_opener(&self, opener: impl Fn(&std::path::Path) + 'static) {
        *self.folder_opener.borrow_mut() = Rc::new(opener);
    }

    /// Signs another device in with the Quick Connect `code` it shows, then
    /// tells the outcome in the settings.
    fn authorize_device(&self, code: &str) {
        let Some(window) = self.window() else { return };
        let Some((id, catalog, _)) = self.services() else {
            return;
        };
        if window.get_settings_authorize_state() == crate::AuthorizeState::Busy
            || !settings::code_ready(code)
        {
            return;
        }
        window.set_settings_authorize_state(crate::AuthorizeState::Busy);
        let code = code.to_owned();
        self.spawn(
            async move { catalog.authorize_device(&code).await },
            move |app, result| {
                let Some(window) = app.window() else { return };
                if app.services().is_none_or(|(current, ..)| current != id) {
                    window.set_settings_authorize_state(crate::AuthorizeState::Idle);
                    return;
                }
                if let Err(error) = &result {
                    log::warn!("cannot authorize the device: {error}");
                }
                if result == Err(CoreError::SessionExpired) {
                    window.set_settings_authorize_state(crate::AuthorizeState::Idle);
                    app.dispatch(Msg::Expired(id));
                    return;
                }
                let state = settings::authorize_state(&result);
                if state == crate::AuthorizeState::Done {
                    window.set_settings_authorize_code("".into());
                }
                window.set_settings_authorize_state(state);
            },
        );
    }

    /// Shows `message` in the notice banner, replacing the one shown. Messages
    /// that need no action leave after [`NOTICE_DURATION`]; the others stay
    /// until the user closes them.
    pub(crate) fn notify(&self, message: Message) {
        self.show_notice(message, NoticeAction::None);
    }

    /// Shows `message` with an Undo button, which calls [`crate::menu::MenuController::undo`].
    pub(crate) fn notify_undo(&self, message: Message) {
        self.show_notice(message, NoticeAction::Undo);
    }

    /// Shows `message`, the failure of an undo, with a Retry button that undoes again.
    pub(crate) fn notify_undo_failed(&self, message: Message) {
        self.show_notice(message, NoticeAction::RetryUndo);
    }

    fn show_notice(&self, message: Message, action: NoticeAction) {
        let Some(window) = self.window() else { return };
        let dismissible = is_persistent(message);
        window.set_notice(NoticeData {
            visible: true,
            message,
            detail: Default::default(),
            dismissible,
            action,
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

    /// The button of the notice was clicked: only the notice still showing acts.
    fn notice_action(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let notice = window.get_notice();
        if !notice.visible || notice.action == NoticeAction::None {
            return;
        }
        self.notice_timer.stop();
        hide_notice(&window);
        if notice.action != NoticeAction::LowerQuality {
            return self.menu.undo(self);
        }
        if let Some(quality) = self.advised.take() {
            player::show_quality(&window, quality);
            self.send_playback(PlaybackCommand::SetQuality(quality));
        }
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

    /// Reloads `screen` when it is one that loads its own content.
    fn refresh(self: &Rc<Self>, screen: &Screen) {
        match screen {
            Screen::Home => self.home.refresh(self),
            Screen::Library(_) => self.library.refresh(self),
            Screen::Search => self.search.refresh(self),
            Screen::Movie(_) => self.movie.refresh(self),
            Screen::Series(_) => self.series.refresh(self),
            Screen::Person(_) => self.person.refresh(self),
            Screen::Collection(_) => self.collection.refresh(self),
            _ => {}
        }
    }

    /// Hides the notice unless it still applies to `screen`.
    fn notice_on(&self, window: &AppWindow, screen: &Screen) {
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
        window.on_content_released(with(|app| app.content_released()));
        window.on_open_quick_connect(with(|app| app.sign_in.open_quick_connect(app)));
        window.on_cancel_quick_connect(with(|app| app.dispatch(Msg::CloseQuickConnect)));
        window.on_retry_quick_connect(with(|app| app.sign_in.start_quick_connect(app)));
        window.on_open_settings(with(|app| app.dispatch(Msg::OpenSettings)));
        window.on_close_settings(with(|app| app.dispatch(Msg::CloseSettings)));
        window.on_sign_out(with(|app| app.dispatch(Msg::SignOut)));
        let this = self.this.clone();
        window.on_settings_subtitles_changed(move |choice| {
            if let Some(app) = this.upgrade() {
                app.set_subtitle_style(&choice);
            }
        });
        let this = self.this.clone();
        window.on_settings_start_fullscreen_changed(move |on| {
            if let Some(app) = this.upgrade() {
                app.set_start_fullscreen(on);
            }
        });
        window.set_player_time_remaining(self.manager.time_remaining());
        let this = self.this.clone();
        window.on_player_time_remaining_changed(move |on| {
            if let Some(app) = this.upgrade()
                && let Err(error) = app.manager.set_time_remaining(on)
            {
                log::warn!("cannot save the time display: {error}");
            }
        });
        let this = self.this.clone();
        window.on_settings_discord_changed(move |on| {
            if let Some(app) = this.upgrade() {
                app.set_discord(on);
            }
        });
        let this = self.this.clone();
        window.on_settings_playback_changed(move |choice| {
            if let Some(app) = this.upgrade() {
                app.set_playback_prefs(&choice);
            }
        });
        window.on_settings_clear_image_cache(with(|app| app.clear_image_cache()));
        window.on_settings_copy_logs(with(|app| app.copy_logs()));
        window.on_settings_open_logs(with(|app| app.open_logs()));
        window.on_settings_code_ready(|code| settings::code_ready(&code));
        let this = self.this.clone();
        window.on_settings_authorize(move |code| {
            if let Some(app) = this.upgrade() {
                app.authorize_device(&code);
            }
        });
        window.on_dismiss_notice(with(|app| app.dismiss_notice()));
        window.on_notice_action(with(|app| app.notice_action()));
        window.on_library_retry(with(|app| app.library.retry(app)));
        window.on_search_retry(with(|app| app.search.retry(app)));
        window.on_search_retry_more(with(|app| app.search.retry_more(app)));
        window.on_search_near_end(with(|app| app.search.near_end(app)));
        window.on_back(with(|app| app.dispatch(Msg::Back)));
        window.global::<MouseBack>().on_pressed(with(|app| {
            // The shortcuts dialog covers the page.
            let covered = app
                .window()
                .is_some_and(|window| window.global::<Shortcuts>().get_open());
            if !covered {
                app.dispatch(Msg::Back);
            }
        }));
        window.on_movie_toggle_watched(with(|app| app.movie.toggle_watched(app)));
        window.on_movie_toggle_favorite(with(|app| app.movie.toggle_favorite(app)));
        window.on_movie_retry(with(|app| app.movie.retry(app)));
        window.on_series_toggle_watched(with(|app| app.series.toggle_watched(app)));
        window.on_series_toggle_favorite(with(|app| app.series.toggle_favorite(app)));
        window.on_series_retry(with(|app| app.series.retry(app)));
        window.on_series_retry_episodes(with(|app| app.series.retry_episodes(app)));
        window.on_person_retry(with(|app| app.person.retry(app)));
        window.on_collection_retry(with(|app| app.collection.retry(app)));
        window.on_collection_play(with(|app| app.collection.play(app)));
        window.on_collection_toggle_watched(with(|app| app.collection.toggle_watched(app)));
        window.on_collection_toggle_favorite(with(|app| app.collection.toggle_favorite(app)));
        let this = self.this.clone();
        window.on_movie_open_collection(move |index| {
            if let (Some(app), Ok(index)) = (this.upgrade(), usize::try_from(index)) {
                app.movie.open_collection(&app, index);
            }
        });
        let this = self.this.clone();
        window.on_collection_open(move |id| {
            if let Some(app) = this.upgrade() {
                app.collection.open(&app, &id);
            }
        });
        window.on_series_play(with(|app| app.series.play(app)));
        let this = self.this.clone();
        window.on_movie_open_similar(move |id| {
            if let Some(app) = this.upgrade() {
                app.movie.open_similar(&app, &id);
            }
        });
        let this = self.this.clone();
        window.on_movie_open_person(move |id| {
            if let Some(app) = this.upgrade() {
                open_person(&app, &id);
            }
        });
        let this = self.this.clone();
        window.on_series_open_person(move |id| {
            if let Some(app) = this.upgrade() {
                open_person(&app, &id);
            }
        });
        let this = self.this.clone();
        window.on_person_open(move |id| {
            if let Some(app) = this.upgrade() {
                app.person.open(&app, &id);
            }
        });
        let this = self.this.clone();
        window.on_person_select_filter(move |filter| {
            if let Some(app) = this.upgrade() {
                app.person.select_filter(&app, filter);
            }
        });
        let this = self.this.clone();
        window.on_series_open_similar(move |id| {
            if let Some(app) = this.upgrade() {
                app.series.open_similar(&app, &id);
            }
        });
        let this = self.this.clone();
        window.on_series_play_episode(move |id| {
            if let Some(app) = this.upgrade() {
                app.series.play_episode(&app, &id);
            }
        });
        let this = self.this.clone();
        window.on_series_select_season(move |index| {
            if let Some(app) = this.upgrade() {
                app.series
                    .select_season(&app, usize::try_from(index).unwrap_or(usize::MAX));
            }
        });
        let this = self.this.clone();
        window.on_movie_play(move |from_start| {
            if let Some(app) = this.upgrade() {
                let mode = if from_start {
                    PlayMode::FromStart
                } else {
                    PlayMode::Auto
                };
                app.movie.play(&app, mode);
            }
        });
        let this = self.this.clone();
        window
            .global::<crate::HoverCard>()
            .on_requested(move |id, progress| {
                if let Some(app) = this.upgrade() {
                    app.hover.request(&app, &id, progress);
                }
            });
        let this = self.this.clone();
        let menu = window.global::<crate::ContextMenu>();
        menu.on_requested(move |serial, id, from_resume| {
            if let Some(app) = this.upgrade() {
                app.menu.request(&app, serial, &id, from_resume);
            }
        });
        let this = self.this.clone();
        menu.on_chosen(move |serial, action| {
            if let Some(app) = this.upgrade() {
                app.menu.chosen(&app, serial, action);
            }
        });
        let weak = window.as_weak();
        menu.on_right_pressed_outside(move |x, y| {
            let weak = weak.clone();
            // Once the overlay is gone, the card under the pointer gets the press and opens its menu.
            Timer::single_shot(Duration::ZERO, move || {
                if let Some(window) = weak.upgrade() {
                    crate::menu::press_right(&window, x, y);
                }
            });
        });
        let this = self.this.clone();
        window.on_open_item(move |id| {
            if let Some(app) = this.upgrade() {
                app.home.open(&app, &id);
            }
        });
        let this = self.this.clone();
        window.on_library_open(move |id| {
            if let Some(app) = this.upgrade() {
                app.library.open(&app, &id);
            }
        });
        let this = self.this.clone();
        window.on_search_open(move |id| {
            if let Some(app) = this.upgrade() {
                app.search.open(&app, &id);
            }
        });
        let this = self.this.clone();
        window.on_search_edited(move |text| {
            if let Some(app) = this.upgrade() {
                app.search.edited(&app, &text);
            }
        });
        let this = self.this.clone();
        window.on_search_accepted(move |text| {
            if let Some(app) = this.upgrade() {
                app.search.accepted(&app, &text);
            }
        });
        window.on_library_retry_more(with(|app| app.library.retry_more(app)));
        window.on_library_near_end(with(|app| app.library.near_end(app)));
        window.on_library_direction_toggled(with(|app| app.library.toggle_direction(app)));
        let this = self.this.clone();
        window.on_open_tab(move |page| {
            if let (Some(app), Some(tab)) = (this.upgrade(), tab_of(page)) {
                app.dispatch(Msg::OpenTab(tab));
            }
        });
        let this = self.this.clone();
        window.on_library_watch_selected(move |index| {
            if let Some(app) = this.upgrade() {
                app.library.set_watch(&app, index);
            }
        });
        let this = self.this.clone();
        window.on_library_genre_selected(move |index| {
            if let Some(app) = this.upgrade() {
                app.library.set_genre(&app, index);
            }
        });
        let this = self.this.clone();
        window.on_library_sort_selected(move |index| {
            if let Some(app) = this.upgrade() {
                app.library.set_sort(&app, index);
            }
        });

        let this = self.this.clone();
        window.on_server_changed(move |address| {
            if let Some(app) = this.upgrade() {
                app.sign_in.server_changed(&app, address);
            }
        });
        let this = self.this.clone();
        window.on_found_server_chosen(move |address| {
            if let Some(app) = this.upgrade() {
                app.sign_in.found_server_chosen(&app, address);
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
                app.play(ItemId::from(id.as_str()), PlayMode::Auto);
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
        let (this, catalog) = (self.this.clone(), self.this.clone());
        crate::screens::subtitle_search::bind(
            window,
            self.runtime.clone(),
            &crate::locale::interface_language(),
            Rc::new(move || {
                catalog
                    .upgrade()
                    .and_then(|app| app.services())
                    .map(|(_, catalog, _)| catalog)
            }),
            Rc::new(move |command| {
                if let Some(app) = this.upgrade() {
                    app.send_playback(command);
                }
            }),
        );
        let this = self.this.clone();
        player::bind_preview(
            window,
            Rc::new(move |sheet| {
                if let Some(app) = this.upgrade() {
                    app.load_trickplay_sheet(sheet);
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
                self.back_cache.borrow_mut().clear();
                self.waiting.borrow_mut().clear();
                self.prefetch.stop();
                self.library.end_generation();
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
                match end {
                    SessionEnd::Close => generation.catalog.save_answers(),
                    SessionEnd::SignOut | SessionEnd::Expire => generation.catalog.close(),
                }
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
                self.run_waiting();
                self.prefetch.stop();
                let previous = self.shown.replace(screen.clone());
                self.page.set(self.page.get() + 1);
                self.back_cache
                    .borrow_mut()
                    .prune(self.state.borrow().stack());
                if let Some(window) = self.window() {
                    self.notice_on(&window, &screen);
                    if screen == Screen::Player && previous != Screen::Player {
                        player::enter(&window);
                    } else if screen != Screen::Player && previous == Screen::Player {
                        player::leave(&window);
                    }
                    #[cfg(all(target_os = "linux", feature = "mpv"))]
                    self.mpris_update(crate::mpris::Update::Active(screen == Screen::Player));
                    #[cfg(all(target_os = "linux", feature = "mpv"))]
                    self.inhibit_update(crate::inhibit::Update::Active(screen == Screen::Player));
                    self.discord_update(crate::discord::Update::Active(screen == Screen::Player));
                    let root = self.state.borrow().stack().first().cloned();
                    window.set_top_bar_page(top_bar_page(root.as_ref()));
                    if screen == Screen::Settings {
                        settings::show(&window, self.state.borrow().session());
                        self.measure_image_cache();
                        window.set_settings_subtitles(settings::subtitle_choice(
                            self.manager.subtitle_style(),
                        ));
                        window.set_settings_start_fullscreen(self.manager.start_fullscreen());
                        window.set_settings_discord(self.manager.discord());
                        window.set_settings_playback(settings::playback_choice(
                            self.manager.playback_prefs(),
                        ));
                    }
                    window.set_screen(slint_screen(&screen));
                }
                self.sign_in.shown(self, &screen, &previous);
                self.home.shown(self, &screen);
                self.library.shown(self, &screen);
                self.search.shown(self, &screen);
                self.movie.shown(self, &screen);
                self.series.shown(self, &screen);
                self.person.shown(self, &screen);
                self.collection.shown(self, &screen);
            }
            Effect::Discard(screen) => {
                self.home.discard(self, &screen);
                self.library.discard(self, &screen);
                self.search.discard(self, &screen);
                self.movie.discard(self, &screen);
                self.series.discard(self, &screen);
                self.person.discard(self, &screen);
                self.collection.discard(self, &screen);
            }
            Effect::Refresh(screen) => self.refresh(&screen),
            Effect::Notify(notice) => self.notify(notice_message(notice)),
            Effect::Quit => {
                self.save_volume();
                self.quitting.set(true);
                if let Err(error) = slint::quit_event_loop() {
                    log::warn!("cannot quit the event loop: {error}");
                }
            }
        }
    }
}

/// Shows `image` as the thumbnail of chapter `index`, if the window still
/// shows the chapters of `rows`.
#[cfg(feature = "mpv")]
fn show_chapter_image(
    weak: &slint::Weak<AppWindow>,
    rows: &slint::ModelRc<crate::ChapterRow>,
    index: usize,
    image: slint::Image,
) {
    use slint::Model;

    let Some(window) = weak.upgrade() else { return };
    if window.get_player_chapters() != *rows {
        return;
    }
    if let Some(mut row) = rows.row_data(index) {
        row.image = image;
        row.has_image = true;
        rows.set_row_data(index, row);
    }
}

/// Shows the `tiles` of the decoded trickplay `sheet` as chapter thumbnails.
#[cfg(feature = "mpv")]
fn show_chapter_tiles(
    weak: &slint::Weak<AppWindow>,
    rows: &slint::ModelRc<crate::ChapterRow>,
    sheet: &slint::Image,
    tiles: &[(usize, oxyfin_core::TrickplayTile)],
) {
    let Some(pixels) = sheet.to_rgba8() else {
        return;
    };
    for (index, tile) in tiles {
        if let Some(image) = crate::images::crop(&pixels, tile.x, tile.y, tile.width, tile.height) {
            show_chapter_image(weak, rows, *index, image);
        }
    }
}

/// The libmpv version for bug reports.
fn mpv_version() -> String {
    #[cfg(feature = "mpv")]
    {
        oxyfin_player::mpv_version().unwrap_or_else(|error| format!("unknown ({error})"))
    }
    #[cfg(not(feature = "mpv"))]
    {
        "not built in".to_owned()
    }
}

fn session_info(signed: &SignedIn) -> SessionInfo {
    SessionInfo {
        user_name: signed.user_name.clone(),
        server_url: signed.server_url.clone(),
        server_name: signed.server_name.clone(),
        token_storage: signed.token_storage,
        show_file_token_warning: signed.show_file_token_warning,
    }
}

fn slint_screen(screen: &Screen) -> crate::Screen {
    match screen {
        Screen::Starting => crate::Screen::Starting,
        Screen::SignIn => crate::Screen::SignIn,
        Screen::QuickConnect => crate::Screen::QuickConnect,
        Screen::Home => crate::Screen::Home,
        Screen::Library(_) => crate::Screen::Library,
        Screen::Search => crate::Screen::Search,
        Screen::Movie(_) => crate::Screen::Movie,
        Screen::Series(_) => crate::Screen::Series,
        Screen::Person(_) => crate::Screen::Person,
        Screen::Collection(_) => crate::Screen::Collection,
        Screen::Player => crate::Screen::Player,
        Screen::Settings => crate::Screen::Settings,
    }
}

fn top_bar_page(root: Option<&Screen>) -> TopBarPage {
    match root {
        Some(Screen::Home) => TopBarPage::Home,
        Some(Screen::Library(LibraryTab::Movies)) => TopBarPage::Movies,
        Some(Screen::Library(LibraryTab::Shows)) => TopBarPage::Shows,
        Some(Screen::Library(LibraryTab::Favorites)) => TopBarPage::Favorites,
        Some(Screen::Search) => TopBarPage::Search,
        _ => TopBarPage::None,
    }
}

fn tab_of(page: TopBarPage) -> Option<Tab> {
    match page {
        TopBarPage::Home => Some(Tab::Home),
        TopBarPage::Movies => Some(Tab::Library(LibraryTab::Movies)),
        TopBarPage::Shows => Some(Tab::Library(LibraryTab::Shows)),
        TopBarPage::Favorites => Some(Tab::Library(LibraryTab::Favorites)),
        TopBarPage::Search => Some(Tab::Search),
        TopBarPage::None => None,
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
        Message::SessionExpired
            | Message::FileTokenWarning
            | Message::StorageError
            | Message::SlowStream
    )
}

/// Whether a notice showing `message` stays when `screen` is shown.
fn keeps_notice(message: Message, screen: &Screen) -> bool {
    match message {
        Message::FileTokenWarning => screen.is_signed_in(),
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
        assert!(keeps_notice(Message::FileTokenWarning, &Screen::Settings));
        assert!(!keeps_notice(Message::FileTokenWarning, &Screen::SignIn));
        assert!(keeps_notice(Message::SessionExpired, &Screen::QuickConnect));
        assert!(!keeps_notice(Message::SessionExpired, &Screen::Home));
        assert!(!keeps_notice(Message::CommandFailed, &Screen::Player));
    }
}
