//! Per-user item state: played, favorite and the resume position.

use serde::Serialize;

use crate::{ItemId, Result, Session, Ticks, UserItemDataDto};

/// Fields of the user data an update changes; the others are left as they are.
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct UserDataUpdate {
    playback_position_ticks: Ticks,
}

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

    /// Takes an item out of Continue Watching by clearing its resume position
    /// (`POST UserItems/{id}/UserData`), leaving its played state as it is.
    pub async fn hide_from_resume(&self, id: &ItemId) -> Result<UserItemDataDto> {
        let route = format!("UserItems/{id}/UserData");
        let query = [("userId", self.user_id().to_string())];
        let body = UserDataUpdate {
            playback_position_ticks: Ticks(0),
        };
        self.server()
            .http()
            .post_json(Some(self.token()), &route, &query, &body)
            .await
    }
}
