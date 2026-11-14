//! Resources owned by one signed-in session.

use oxyfin_api::Session;
use oxyfin_core::{CacheBudgets, Catalog, ImageCache, Paths, SessionManager};

use crate::state::{GenerationId, SessionEnd};

/// What lives as long as one signed-in session: the session and the services bound to it.
pub struct Generation {
    /// Identifier given by the state machine.
    pub id: GenerationId,
    /// The authenticated session.
    pub session: Session,
    /// Library browsing for this session.
    pub catalog: Catalog,
    /// Image cache for this session.
    pub images: ImageCache,
}

impl Generation {
    /// Creates the services of a session. Must run inside the Tokio runtime,
    /// where the image cache starts its background trim.
    pub fn new(id: GenerationId, session: Session, paths: &Paths) -> Self {
        Self {
            id,
            catalog: Catalog::new(session.clone()),
            images: ImageCache::new(session.clone(), paths, CacheBudgets::default()),
            session,
        }
    }

    /// Shuts the generation down for the given reason.
    pub async fn end(self, end: SessionEnd, manager: &SessionManager) {
        let result = match end {
            SessionEnd::Expire => manager.expire().await,
            SessionEnd::SignOut => manager.sign_out(&self.session).await,
            SessionEnd::Close => Ok(()),
        };
        if let Err(error) = result {
            log::warn!("ending session generation {:?} failed: {error}", self.id);
        }
    }
}
