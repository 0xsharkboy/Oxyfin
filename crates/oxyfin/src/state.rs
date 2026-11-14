//! Pure application state machine: messages in, effects out.

/// Identifier of a session generation. Identifiers are never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationId(pub u64);

/// Screen currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Restoring the saved session.
    Starting,
    /// Sign-in form.
    SignIn,
    /// Quick Connect flow.
    QuickConnect,
    /// Home screen.
    Home,
    /// Video player.
    Player,
    /// Settings.
    Settings,
}

/// What the interface needs to know about the signed-in session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// Display name of the user.
    pub user_name: String,
    /// Server address.
    pub server_url: String,
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
    /// Show a message.
    Notify(Notice),
    /// Exit the application.
    Quit,
}

/// The application state.
#[derive(Debug)]
pub struct AppState {
    screen: Screen,
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
            screen: Screen::Starting,
            generation: None,
            next_id: 1,
            session: None,
            closing: None,
        }
    }

    /// Current screen.
    pub fn screen(&self) -> Screen {
        self.screen
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
        match msg {
            Msg::Restored(info) if self.screen == Screen::Starting => match info {
                Some(info) => {
                    let id = self.start_generation();
                    let mut effects = vec![
                        Effect::StartGeneration(id),
                        Effect::ValidateSession(id),
                        self.show(Screen::Home),
                    ];
                    self.push_token_warning(&info, &mut effects);
                    self.session = Some(info);
                    effects
                }
                None => vec![self.show(Screen::SignIn)],
            },
            Msg::StartupFailed if self.screen == Screen::Starting => {
                vec![
                    self.show(Screen::SignIn),
                    Effect::Notify(Notice::StorageError),
                ]
            }
            Msg::SignedIn(info) if matches!(self.screen, Screen::SignIn | Screen::QuickConnect) => {
                let id = self.start_generation();
                let mut effects = vec![Effect::StartGeneration(id), self.show(Screen::Home)];
                self.push_token_warning(&info, &mut effects);
                self.session = Some(info);
                effects
            }
            Msg::OpenQuickConnect if self.screen == Screen::SignIn => {
                vec![self.show(Screen::QuickConnect)]
            }
            Msg::CloseQuickConnect if self.screen == Screen::QuickConnect => {
                vec![self.show(Screen::SignIn)]
            }
            Msg::OpenPlayer if self.screen == Screen::Home => vec![self.show(Screen::Player)],
            Msg::ClosePlayer if self.screen == Screen::Player => vec![self.show(Screen::Home)],
            Msg::OpenSettings if self.screen == Screen::Home => {
                vec![self.show(Screen::Settings)]
            }
            Msg::CloseSettings if self.screen == Screen::Settings => {
                vec![self.show(Screen::Home)]
            }
            Msg::SignOut if self.screen == Screen::Settings => match self.end_generation() {
                Some(id) => vec![
                    Effect::EndGeneration {
                        id,
                        end: SessionEnd::SignOut,
                    },
                    self.show(Screen::SignIn),
                ],
                None => Vec::new(),
            },
            Msg::Expired(id) if self.generation == Some(id) => {
                self.end_generation();
                vec![
                    Effect::EndGeneration {
                        id,
                        end: SessionEnd::Expire,
                    },
                    self.show(Screen::SignIn),
                    Effect::Notify(Notice::SessionExpired),
                ]
            }
            Msg::CloseRequested => match self.end_generation() {
                Some(id) => {
                    self.closing = Some(id);
                    vec![Effect::EndGeneration {
                        id,
                        end: SessionEnd::Close,
                    }]
                }
                None => vec![Effect::Quit],
            },
            _ => Vec::new(),
        }
    }

    fn show(&mut self, screen: Screen) -> Effect {
        self.screen = screen;
        Effect::Show(screen)
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
            vec![end(g1, SessionEnd::SignOut), Effect::Show(Screen::SignIn)]
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
            vec![end(g1, SessionEnd::Close)]
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
        assert_eq!(s.update(Msg::ClosePlayer), vec![Effect::Show(Screen::Home)]);
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
