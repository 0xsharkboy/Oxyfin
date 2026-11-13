//! Per-user item state: played and favorite.

use crate::{ItemId, Result, Session, UserItemDataDto};

impl Session {
    async fn set_flag(&self, route: &str, id: &ItemId, enabled: bool) -> Result<UserItemDataDto> {
        let route = format!("{route}/{id}");
        let query = [("userId", self.user_id().to_string())];
        let http = self.server().http();
        if enabled {
            http.post_empty(Some(self.token()), &route, &query).await
        } else {
            http.delete_empty(Some(self.token()), &route, &query).await
        }
    }

    /// Marks an item played or unplayed (`POST` or `DELETE UserPlayedItems/{id}`).
    pub async fn set_played(&self, id: &ItemId, played: bool) -> Result<UserItemDataDto> {
        self.set_flag("UserPlayedItems", id, played).await
    }

    /// Marks an item favorite or not (`POST` or `DELETE UserFavoriteItems/{id}`).
    pub async fn set_favorite(&self, id: &ItemId, favorite: bool) -> Result<UserItemDataDto> {
        self.set_flag("UserFavoriteItems", id, favorite).await
    }
}
