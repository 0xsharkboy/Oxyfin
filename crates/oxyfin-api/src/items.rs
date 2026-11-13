//! Browsing the library: views, listings, details, resume and next up.

use crate::models::{BaseItemDto, BaseItemKind, ItemsQuery, QueryResult};
use crate::{ItemId, Result, Session};

impl Session {
    fn user_param(&self) -> (&'static str, String) {
        ("userId", self.user_id().to_string())
    }

    async fn get_items<T: serde::de::DeserializeOwned>(
        &self,
        route: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        self.server()
            .http()
            .get(Some(self.token()), route, query)
            .await
    }

    /// Fetches the libraries of the user (`GET UserViews`).
    pub async fn user_views(&self) -> Result<QueryResult<BaseItemDto>> {
        self.get_items("UserViews", &[self.user_param()]).await
    }

    /// Lists items matching `query` (`GET Items`).
    pub async fn items(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItemDto>> {
        let params = query.to_params(self.user_id().as_str());
        self.get_items("Items", &params).await
    }

    /// Fetches one item (`GET Items/{id}`).
    pub async fn item(&self, id: &ItemId) -> Result<BaseItemDto> {
        self.get_items(&format!("Items/{id}"), &[self.user_param()])
            .await
    }

    /// Lists recently added items, optionally inside `parent_id` (`GET Items/Latest`).
    pub async fn latest(&self, parent_id: Option<&ItemId>, limit: u32) -> Result<Vec<BaseItemDto>> {
        let mut params = vec![self.user_param()];
        if let Some(parent) = parent_id {
            params.push(("parentId", parent.to_string()));
        }
        params.push(("limit", limit.to_string()));
        self.get_items("Items/Latest", &params).await
    }

    /// Lists items the user can resume (`GET UserItems/Resume`).
    pub async fn resume(&self, limit: u32) -> Result<QueryResult<BaseItemDto>> {
        let params = [self.user_param(), ("limit", limit.to_string())];
        self.get_items("UserItems/Resume", &params).await
    }

    /// Lists the next episodes to watch, optionally for one series (`GET Shows/NextUp`).
    pub async fn next_up(
        &self,
        series_id: Option<&ItemId>,
        limit: u32,
    ) -> Result<QueryResult<BaseItemDto>> {
        let mut params = vec![self.user_param()];
        if let Some(series) = series_id {
            params.push(("seriesId", series.to_string()));
        }
        params.push(("limit", limit.to_string()));
        self.get_items("Shows/NextUp", &params).await
    }

    /// Lists the seasons of a series (`GET Shows/{id}/Seasons`).
    pub async fn seasons(&self, series_id: &ItemId) -> Result<QueryResult<BaseItemDto>> {
        self.get_items(&format!("Shows/{series_id}/Seasons"), &[self.user_param()])
            .await
    }

    /// Lists the episodes of a series, optionally of one season (`GET Shows/{id}/Episodes`).
    pub async fn episodes(
        &self,
        series_id: &ItemId,
        season_id: Option<&ItemId>,
    ) -> Result<QueryResult<BaseItemDto>> {
        let mut params = vec![self.user_param()];
        if let Some(season) = season_id {
            params.push(("seasonId", season.to_string()));
        }
        self.get_items(&format!("Shows/{series_id}/Episodes"), &params)
            .await
    }

    /// Lists genres, optionally within `parent_id` and for items of `kinds` (`GET Genres`).
    pub async fn genres(
        &self,
        parent_id: Option<&ItemId>,
        kinds: &[BaseItemKind],
    ) -> Result<QueryResult<BaseItemDto>> {
        let mut params = vec![self.user_param()];
        if let Some(parent) = parent_id {
            params.push(("parentId", parent.to_string()));
        }
        if !kinds.is_empty() {
            let kinds: Vec<&str> = kinds.iter().map(BaseItemKind::as_str).collect();
            params.push(("includeItemTypes", kinds.join(",")));
        }
        self.get_items("Genres", &params).await
    }
}
