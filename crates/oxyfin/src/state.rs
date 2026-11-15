//! Pure application state machine: messages in, effects out.

/// Identifier of a session generation. Identifiers are never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationId(pub u64);

/// A library page of the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibraryTab {
    /// Movies.
    Movies,
    /// Series.
    Shows,
    /// Favorites.
    Favorites,
}

/// A top-level destination of the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Home screen.
    Home,
    /// A library page.
    Library(LibraryTab),
    /// Search.
    Search,
}

impl Tab {
    fn screen(self) -> Screen {
        match self {
            Tab::Home => Screen::Home,
            Tab::Library(tab) => Screen::Library(tab),
            Tab::Search => Screen::Search,
        }
    }
}

/// Screen currently shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// Restoring the saved session.
    Starting,
    /// Sign-in form.
    SignIn,
    /// Quick Connect flow.
    QuickConnect,
    /// Home screen.
    Home,
    /// A library page.
    Library(LibraryTab),
    /// Search.
    Search,
    /// Detail page of a movie or video, by item identifier.
    Movie(String),
    /// Detail page of a series, by item identifier.
    Series(String),
    /// Video player.
    Player,
    /// Settings.
    Settings,
}

impl Screen {
    /// Whether the screen belongs to a signed-in session.
    pub fn is_signed_in(&self) -> bool {
        !matches!(
            self,
            Screen::Starting | Screen::SignIn | Screen::QuickConnect
        )
    }

    /// Whether playback can start from the screen: the home screen and the detail pages.
    pub fn can_play(&self) -> bool {
        matches!(self, Screen::Home | Screen::Movie(_) | Screen::Series(_))
    }

    fn has_controller(&self) -> bool {
        matches!(
            self,
            Screen::Home
                | Screen::Library(_)
                | Screen::Search
                | Screen::Movie(_)
                | Screen::Series(_)
        )
    }

    fn is_detail(&self) -> bool {
        matches!(self, Screen::Movie(_) | Screen::Series(_))
    }
}

/// What the interface needs to know about the signed-in session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// Display name of the user.
    pub user_name: String,
    /// Server address.
    pub server_url: String,
    /// Display name of the server, when it published one.
    pub server_name: Option<String>,
    /// Whether the token is stored in a file rather than the keyring.
    pub token_in_file: bool,
    /// Whether the file-token warning must be shown.
    pub show_file_token_warning: bool,
}

/// Input of the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// Startup finished, with the restored session if any.
    Restored(Option<SessionInfo>),
    /// The session store could not be read at startup.
    StartupFailed,
    /// A sign-in or Quick Connect succeeded.
    SignedIn(SessionInfo),
    /// The user opened Quick Connect.
    OpenQuickConnect,
    /// The user left Quick Connect.
    CloseQuickConnect,
    /// The user picked a tab of the top bar.
    OpenTab(Tab),
    /// The user opened a detail page: only `Screen::Movie` or `Screen::Series`.
    OpenItem(Screen),
    /// The user went back to the previous screen.
    Back,
    /// Playback was requested.
    OpenPlayer,
    /// The player was closed.
    ClosePlayer,
    /// The user opened the settings.
    OpenSettings,
    /// The user left the settings.
    CloseSettings,
    /// The user asked to sign out.
    SignOut,
    /// The server rejected the token of a generation.
    Expired(GenerationId),
    /// The window is being closed.
    CloseRequested,
    /// A generation finished shutting down.
    GenerationEnded(GenerationId),
}

/// Message the interface shows to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// The session expired.
    SessionExpired,
    /// The token is stored in a plain file.
    FileTokenWarning,
    /// The session store failed.
    StorageError,
}

/// Why a generation ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    /// The server rejected the token.
    Expire,
    /// The user signed out.
    SignOut,
    /// The application is closing.
    Close,
}

/// Output of the state machine, applied by the executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Create the resources of a new generation.
    StartGeneration(GenerationId),
    /// Check the restored session against the server.
    ValidateSession(GenerationId),
    /// Shut down a generation.
    EndGeneration {
        /// Generation to end.
        id: GenerationId,
        /// Reason for ending it.
        end: SessionEnd,
    },
    /// Switch to a screen.
    Show(Screen),
    /// A screen left the stack: release its controller.
    Discard(Screen),
    /// A screen came back from the player: reload it.
    Refresh(Screen),
    /// Show a message.
    Notify(Notice),
    /// Exit the application.
    Quit,
}

/// The application state.
#[derive(Debug)]
pub struct AppState {
    stack: Vec<Screen>,
    generation: Option<GenerationId>,
    next_id: u64,
    session: Option<SessionInfo>,
    closing: Option<GenerationId>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    /// Creates the state of an application that is starting.
    pub fn new() -> Self {
        Self {
            stack: vec![Screen::Starting],
            generation: None,
            next_id: 1,
            session: None,
            closing: None,
        }
    }

    /// Current screen: the top of the stack.
    pub fn screen(&self) -> Screen {
        self.top().clone()
    }

    /// The screens of the session, from the root to the current one.
    pub fn stack(&self) -> &[Screen] {
        &self.stack
    }

    /// The live generation, if any.
    pub fn generation(&self) -> Option<GenerationId> {
        self.generation
    }

    /// The signed-in session, if any.
    pub fn session(&self) -> Option<&SessionInfo> {
        self.session.as_ref()
    }

    /// Applies a message and returns the effects to execute, in order.
    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        if let Some(closing) = self.closing {
            return match msg {
                Msg::GenerationEnded(id) if id == closing => {
                    self.closing = None;
                    vec![Effect::Quit]
                }
                _ => Vec::new(),
            };
        }
        let on_starting = *self.top() == Screen::Starting;
        let on_connected = self.top().is_signed_in();
        let on_player = *self.top() == Screen::Player;
        match msg {
            Msg::Restored(info) if on_starting => match info {
                Some(info) => {
                    let id = self.start_generation();
                    let mut effects = vec![
                        Effect::StartGeneration(id),
                        Effect::ValidateSession(id),
                        self.root(Screen::Home),
                    ];
                    self.push_token_warning(&info, &mut effects);
                    self.session = Some(info);
                    effects
                }
                None => vec![self.root(Screen::SignIn)],
            },
            Msg::StartupFailed if on_starting => {
                vec![
                    self.root(Screen::SignIn),
                    Effect::Notify(Notice::StorageError),
                ]
            }
            Msg::SignedIn(info) if matches!(self.top(), Screen::SignIn | Screen::QuickConnect) => {
                let id = self.start_generation();
                let mut effects = vec![Effect::StartGeneration(id), self.root(Screen::Home)];
                self.push_token_warning(&info, &mut effects);
                self.session = Some(info);
                effects
            }
            Msg::OpenQuickConnect if *self.top() == Screen::SignIn => {
                vec![self.root(Screen::QuickConnect)]
            }
            Msg::CloseQuickConnect if *self.top() == Screen::QuickConnect => {
                vec![self.root(Screen::SignIn)]
            }
            Msg::OpenTab(tab) if on_connected && !on_player => {
                let target = tab.screen();
                if self.stack == [target.clone()] {
                    return Vec::new();
                }
                if self.stack[0] == target {
                    let mut effects = self.pop_to_root();
                    effects.push(Effect::Show(target));
                    return effects;
                }
                let mut effects = self.discard_all();
                effects.push(self.root(target));
                effects
            }
            Msg::OpenItem(screen)
                if on_connected
                    && !on_player
                    && *self.top() != Screen::Settings
                    && screen.is_detail() =>
            {
                vec![self.push(screen)]
            }
            Msg::Back if on_connected && !on_player && self.stack.len() > 1 => self.pop(),
            Msg::OpenPlayer if on_connected && !on_player && *self.top() != Screen::Settings => {
                vec![self.push(Screen::Player)]
            }
            Msg::ClosePlayer if on_player => {
                self.stack.pop();
                let top = self.top().clone();
                vec![Effect::Show(top.clone()), Effect::Refresh(top)]
            }
            Msg::OpenSettings if on_connected && !on_player && *self.top() != Screen::Settings => {
                vec![self.push(Screen::Settings)]
            }
            Msg::CloseSettings if *self.top() == Screen::Settings => self.pop(),
            Msg::SignOut if *self.top() == Screen::Settings => match self.end_generation() {
                Some(id) => {
                    let mut effects = vec![Effect::EndGeneration {
                        id,
                        end: SessionEnd::SignOut,
                    }];
                    effects.extend(self.discard_all());
                    effects.push(self.root(Screen::SignIn));
                    effects
                }
                None => Vec::new(),
            },
            Msg::Expired(id) if self.generation == Some(id) => {
                self.end_generation();
                let mut effects = vec![Effect::EndGeneration {
                    id,
                    end: SessionEnd::Expire,
                }];
                effects.extend(self.discard_all());
                effects.push(self.root(Screen::SignIn));
                effects.push(Effect::Notify(Notice::SessionExpired));
                effects
            }
            Msg::CloseRequested => match self.end_generation() {
                Some(id) => {
                    self.closing = Some(id);
                    let mut effects = vec![Effect::EndGeneration {
                        id,
                        end: SessionEnd::Close,
                    }];
                    effects.extend(self.discard_all());
                    effects
                }
                None => vec![Effect::Quit],
            },
            _ => Vec::new(),
        }
    }

    fn top(&self) -> &Screen {
        self.stack.last().unwrap_or(&Screen::Starting)
    }

    /// Replaces the whole stack by `screen`.
    fn root(&mut self, screen: Screen) -> Effect {
        self.stack = vec![screen.clone()];
        Effect::Show(screen)
    }

    fn push(&mut self, screen: Screen) -> Effect {
        self.stack.push(screen.clone());
        Effect::Show(screen)
    }

    /// Pops the top screen and shows the one below.
    fn pop(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some(popped) = self.stack.pop()
            && popped.has_controller()
        {
            effects.push(Effect::Discard(popped));
        }
        effects.push(Effect::Show(self.top().clone()));
        effects
    }

    /// Pops every screen above the root, discarding their controllers.
    fn pop_to_root(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        while self.stack.len() > 1 {
            if let Some(popped) = self.stack.pop()
                && popped.has_controller()
            {
                effects.push(Effect::Discard(popped));
            }
        }
        effects
    }

    /// Empties the stack, discarding the controllers from the top down.
    fn discard_all(&mut self) -> Vec<Effect> {
        self.stack
            .drain(..)
            .rev()
            .filter(Screen::has_controller)
            .map(Effect::Discard)
            .collect()
    }

    fn start_generation(&mut self) -> GenerationId {
        let id = GenerationId(self.next_id);
        self.next_id += 1;
        self.generation = Some(id);
        id
    }

    fn end_generation(&mut self) -> Option<GenerationId> {
        self.session = None;
        self.generation.take()
    }

    fn push_token_warning(&self, info: &SessionInfo, effects: &mut Vec<Effect>) {
        if info.show_file_token_warning {
            effects.push(Effect::Notify(Notice::FileTokenWarning));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(warn: bool) -> SessionInfo {
        SessionInfo {
            user_name: "alice".into(),
            server_url: "https://jf.example".into(),
            server_name: None,
            token_in_file: warn,
            show_file_token_warning: warn,
        }
    }

    fn signed_in() -> (AppState, GenerationId) {
        let mut s = AppState::new();
        s.update(Msg::Restored(None));
        s.update(Msg::SignedIn(info(false)));
        let g = s.generation().unwrap();
        (s, g)
    }

    fn end(id: GenerationId, end: SessionEnd) -> Effect {
        Effect::EndGeneration { id, end }
    }

    #[test]
    fn restored_session_goes_home_and_validates() {
        let mut s = AppState::new();
        let fx = s.update(Msg::Restored(Some(info(false))));
        let g1 = s.generation().unwrap();
        assert_eq!(
            fx,
            vec![
                Effect::StartGeneration(g1),
                Effect::ValidateSession(g1),
                Effect::Show(Screen::Home)
            ]
        );
        assert_eq!(s.screen(), Screen::Home);
        assert_eq!(s.session(), Some(&info(false)));
    }

    #[test]
    fn restored_session_with_warning_notifies() {
        let mut s = AppState::new();
        let fx = s.update(Msg::Restored(Some(info(true))));
        assert_eq!(fx.last(), Some(&Effect::Notify(Notice::FileTokenWarning)));
    }

    #[test]
    fn no_session_shows_sign_in() {
        let mut s = AppState::new();
        assert_eq!(
            s.update(Msg::Restored(None)),
            vec![Effect::Show(Screen::SignIn)]
        );
        let mut s = AppState::new();
        assert_eq!(
            s.update(Msg::StartupFailed),
            vec![
                Effect::Show(Screen::SignIn),
                Effect::Notify(Notice::StorageError)
            ]
        );
    }

    #[test]
    fn file_token_warning_is_notified_once() {
        let mut s = AppState::new();
        s.update(Msg::Restored(None));
        let fx = s.update(Msg::SignedIn(info(true)));
        let n = fx
            .iter()
            .filter(|e| **e == Effect::Notify(Notice::FileTokenWarning))
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn expiry_from_player_ends_generation_and_shows_sign_in() {
        let (mut s, g1) = signed_in();
        s.update(Msg::OpenPlayer);
        assert_eq!(s.screen(), Screen::Player);
        assert_eq!(
            s.update(Msg::Expired(g1)),
            vec![
                end(g1, SessionEnd::Expire),
                Effect::Discard(Screen::Home),
                Effect::Show(Screen::SignIn),
                Effect::Notify(Notice::SessionExpired)
            ]
        );
        assert_eq!(s.generation(), None);
        assert_eq!(s.session(), None);
    }

    #[test]
    fn second_expiry_is_ignored() {
        let (mut s, g1) = signed_in();
        assert!(!s.update(Msg::Expired(g1)).is_empty());
        assert!(s.update(Msg::Expired(g1)).is_empty());
    }

    #[test]
    fn stale_expiry_is_ignored() {
        let (mut s, g1) = signed_in();
        s.update(Msg::OpenSettings);
        s.update(Msg::SignOut);
        s.update(Msg::SignedIn(info(false)));
        assert_ne!(s.generation(), Some(g1));
        assert!(s.update(Msg::Expired(g1)).is_empty());
        assert_eq!(s.screen(), Screen::Home);
    }

    #[test]
    fn sign_out_then_sign_in_starts_a_new_generation() {
        let (mut s, g1) = signed_in();
        s.update(Msg::OpenSettings);
        assert_eq!(
            s.update(Msg::SignOut),
            vec![
                end(g1, SessionEnd::SignOut),
                Effect::Discard(Screen::Home),
                Effect::Show(Screen::SignIn)
            ]
        );
        assert_eq!(s.generation(), None);
        s.update(Msg::SignedIn(info(false)));
        let g2 = s.generation().unwrap();
        assert_ne!(g2, g1);
    }

    #[test]
    fn close_waits_for_the_generation() {
        let (mut s, g1) = signed_in();
        assert_eq!(
            s.update(Msg::CloseRequested),
            vec![end(g1, SessionEnd::Close), Effect::Discard(Screen::Home)]
        );
        assert!(s.update(Msg::OpenPlayer).is_empty());
        assert_eq!(s.update(Msg::GenerationEnded(g1)), vec![Effect::Quit]);
    }

    #[test]
    fn close_without_session_quits() {
        let mut s = AppState::new();
        s.update(Msg::Restored(None));
        assert_eq!(s.update(Msg::CloseRequested), vec![Effect::Quit]);
    }

    #[test]
    fn close_while_expiring_quits() {
        let (mut s, g1) = signed_in();
        s.update(Msg::Expired(g1));
        assert_eq!(s.update(Msg::CloseRequested), vec![Effect::Quit]);
    }

    #[test]
    fn player_returns_to_home() {
        let (mut s, _) = signed_in();
        s.update(Msg::OpenPlayer);
        assert_eq!(
            s.update(Msg::ClosePlayer),
            vec![Effect::Show(Screen::Home), Effect::Refresh(Screen::Home)]
        );
    }

    #[test]
    fn settings_return_to_home() {
        let (mut s, _) = signed_in();
        assert_eq!(
            s.update(Msg::OpenSettings),
            vec![Effect::Show(Screen::Settings)]
        );
        assert_eq!(
            s.update(Msg::CloseSettings),
            vec![Effect::Show(Screen::Home)]
        );
    }

    #[test]
    fn quick_connect_cancel_returns_to_sign_in() {
        let mut s = AppState::new();
        s.update(Msg::Restored(None));
        assert_eq!(
            s.update(Msg::OpenQuickConnect),
            vec![Effect::Show(Screen::QuickConnect)]
        );
        assert_eq!(
            s.update(Msg::CloseQuickConnect),
            vec![Effect::Show(Screen::SignIn)]
        );
    }

    fn movie(id: &str) -> Screen {
        Screen::Movie(id.into())
    }

    fn series(id: &str) -> Screen {
        Screen::Series(id.into())
    }

    fn movies() -> Screen {
        Screen::Library(LibraryTab::Movies)
    }

    fn on_movies_grid() -> AppState {
        let (mut s, _) = signed_in();
        s.update(Msg::OpenTab(Tab::Library(LibraryTab::Movies)));
        s
    }

    #[test]
    fn tab_replaces_the_stack() {
        let (mut s, _) = signed_in();
        assert_eq!(
            s.update(Msg::OpenTab(Tab::Library(LibraryTab::Movies))),
            vec![Effect::Discard(Screen::Home), Effect::Show(movies())]
        );
        assert_eq!(s.stack(), [movies()]);
        assert_eq!(
            s.update(Msg::OpenTab(Tab::Search)),
            vec![Effect::Discard(movies()), Effect::Show(Screen::Search)]
        );
        assert_eq!(s.stack(), [Screen::Search]);
    }

    #[test]
    fn leaving_the_search_discards_its_controller() {
        let (mut s, _) = signed_in();
        s.update(Msg::OpenTab(Tab::Search));
        assert_eq!(
            s.update(Msg::OpenTab(Tab::Home)),
            vec![Effect::Discard(Screen::Search), Effect::Show(Screen::Home)]
        );
    }

    #[test]
    fn same_tab_does_nothing() {
        let (mut s, _) = signed_in();
        assert!(s.update(Msg::OpenTab(Tab::Home)).is_empty());
        assert_eq!(s.stack(), [Screen::Home]);
    }

    #[test]
    fn item_then_back_returns_to_the_grid() {
        let mut s = on_movies_grid();
        assert_eq!(
            s.update(Msg::OpenItem(movie("m"))),
            vec![Effect::Show(movie("m"))]
        );
        assert_eq!(s.stack(), [movies(), movie("m")]);
        assert_eq!(
            s.update(Msg::Back),
            vec![Effect::Discard(movie("m")), Effect::Show(movies())]
        );
        assert_eq!(s.stack(), [movies()]);
    }

    #[test]
    fn player_from_a_detail_returns_and_refreshes() {
        let (mut s, _) = signed_in();
        s.update(Msg::OpenItem(series("s")));
        assert_eq!(
            s.update(Msg::OpenPlayer),
            vec![Effect::Show(Screen::Player)]
        );
        assert_eq!(
            s.update(Msg::ClosePlayer),
            vec![Effect::Show(series("s")), Effect::Refresh(series("s"))]
        );
        assert_eq!(s.stack(), [Screen::Home, series("s")]);
    }

    #[test]
    fn back_on_a_tab_does_nothing() {
        let mut s = on_movies_grid();
        assert!(s.update(Msg::Back).is_empty());
        assert_eq!(s.stack(), [movies()]);
    }

    #[test]
    fn settings_from_a_detail_back_to_it() {
        let (mut s, _) = signed_in();
        s.update(Msg::OpenItem(movie("m")));
        assert_eq!(
            s.update(Msg::OpenSettings),
            vec![Effect::Show(Screen::Settings)]
        );
        assert_eq!(s.update(Msg::CloseSettings), vec![Effect::Show(movie("m"))]);
        s.update(Msg::OpenSettings);
        assert_eq!(s.update(Msg::Back), vec![Effect::Show(movie("m"))]);
        assert_eq!(s.stack(), [Screen::Home, movie("m")]);
    }

    #[test]
    fn back_to_home_emits_no_refresh() {
        let (mut s, _) = signed_in();
        s.update(Msg::OpenItem(movie("m")));
        assert_eq!(
            s.update(Msg::Back),
            vec![Effect::Discard(movie("m")), Effect::Show(Screen::Home)]
        );
    }

    #[test]
    fn player_from_home_refreshes_home() {
        let (mut s, _) = signed_in();
        s.update(Msg::OpenPlayer);
        assert_eq!(
            s.update(Msg::ClosePlayer),
            vec![Effect::Show(Screen::Home), Effect::Refresh(Screen::Home)]
        );
    }

    #[test]
    fn expiry_with_three_screens_discards_them_all() {
        let (mut s, g1) = signed_in();
        s.update(Msg::OpenItem(movie("m")));
        s.update(Msg::OpenItem(series("s")));
        assert_eq!(
            s.update(Msg::Expired(g1)),
            vec![
                end(g1, SessionEnd::Expire),
                Effect::Discard(series("s")),
                Effect::Discard(movie("m")),
                Effect::Discard(Screen::Home),
                Effect::Show(Screen::SignIn),
                Effect::Notify(Notice::SessionExpired)
            ]
        );
        assert_eq!(s.stack(), [Screen::SignIn]);
    }

    #[test]
    fn tab_during_a_detail_discards_it() {
        let mut s = on_movies_grid();
        s.update(Msg::OpenItem(movie("m")));
        assert_eq!(
            s.update(Msg::OpenTab(Tab::Library(LibraryTab::Shows))),
            vec![
                Effect::Discard(movie("m")),
                Effect::Discard(movies()),
                Effect::Show(Screen::Library(LibraryTab::Shows))
            ]
        );
        assert_eq!(s.stack(), [Screen::Library(LibraryTab::Shows)]);
    }

    #[test]
    fn the_origin_tab_during_a_detail_returns_to_it() {
        let mut s = on_movies_grid();
        s.update(Msg::OpenItem(movie("m")));
        assert_eq!(
            s.update(Msg::OpenTab(Tab::Library(LibraryTab::Movies))),
            vec![Effect::Discard(movie("m")), Effect::Show(movies())]
        );
    }

    #[test]
    fn item_ignored_when_signed_out() {
        let mut s = AppState::new();
        s.update(Msg::Restored(None));
        assert!(s.update(Msg::OpenItem(movie("m"))).is_empty());
        assert!(s.update(Msg::OpenTab(Tab::Search)).is_empty());
        assert_eq!(s.stack(), [Screen::SignIn]);
    }

    #[test]
    fn only_details_can_be_opened_as_items() {
        let (mut s, _) = signed_in();
        assert!(s.update(Msg::OpenItem(Screen::Settings)).is_empty());
        assert!(s.update(Msg::OpenItem(Screen::Home)).is_empty());
    }

    #[test]
    fn nothing_opens_over_the_settings() {
        let (mut s, _) = signed_in();
        s.update(Msg::OpenSettings);
        assert!(s.update(Msg::OpenItem(movie("m"))).is_empty());
        assert!(s.update(Msg::OpenPlayer).is_empty());
        assert_eq!(s.stack(), [Screen::Home, Screen::Settings]);
    }

    #[test]
    fn nothing_opens_over_the_player() {
        let (mut s, _) = signed_in();
        s.update(Msg::OpenPlayer);
        assert!(s.update(Msg::OpenSettings).is_empty());
        assert!(s.update(Msg::OpenItem(movie("m"))).is_empty());
        assert!(s.update(Msg::OpenTab(Tab::Search)).is_empty());
        assert!(s.update(Msg::Back).is_empty());
        assert_eq!(s.stack(), [Screen::Home, Screen::Player]);
    }

    #[test]
    fn senseless_navigation_is_ignored() {
        let mut s = AppState::new();
        s.update(Msg::Restored(None));
        assert!(s.update(Msg::OpenPlayer).is_empty());
        assert!(s.update(Msg::ClosePlayer).is_empty());
        assert!(s.update(Msg::GenerationEnded(GenerationId(9))).is_empty());
        assert!(s.update(Msg::Restored(Some(info(false)))).is_empty());
    }
}
