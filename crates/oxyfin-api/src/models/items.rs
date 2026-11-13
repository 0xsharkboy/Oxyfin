//! Library items and listings.

use super::enums::{BaseItemKind, ImageType, ItemFields, ItemSortBy, SortOrder};
use crate::{ItemId, Ticks};
use serde::Deserialize;
use std::collections::HashMap;

/// A page of results.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct QueryResult<T> {
    /// Items of this page.
    pub items: Vec<T>,
    /// Number of items matching the query, across all pages.
    pub total_record_count: u32,
    /// Index of the first item of this page.
    pub start_index: u32,
}

impl<T> Default for QueryResult<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total_record_count: 0,
            start_index: 0,
        }
    }
}

/// Per-user state of an item.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserItemDataDto {
    /// Whether the user has played the item.
    pub played: bool,
    /// Whether the user marked the item as favorite.
    pub is_favorite: bool,
    /// Resume position.
    pub playback_position_ticks: Ticks,
    /// Percentage played, when the server reports it.
    pub played_percentage: Option<f64>,
    /// Number of unplayed children, for folders.
    pub unplayed_item_count: Option<u32>,
    /// Number of times the item was played.
    pub play_count: u32,
}

/// A person credited on an item.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BaseItemPerson {
    /// Identifier of the person.
    pub id: Option<ItemId>,
    /// Name of the person.
    pub name: String,
    /// Character played, for actors.
    pub role: Option<String>,
    /// Kind of credit, such as `Actor` or `Director`.
    pub r#type: Option<String>,
    /// Tag of the person's primary image, if any.
    pub primary_image_tag: Option<String>,
}

/// A library item: movie, series, season, episode, folder and so on.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct BaseItemDto {
    /// Identifier of the item.
    pub id: ItemId,
    /// Display name.
    pub name: Option<String>,
    /// Kind of item.
    pub r#type: BaseItemKind,
    /// Synopsis.
    pub overview: Option<String>,
    /// Production year.
    pub production_year: Option<i32>,
    /// Premiere date, as an ISO 8601 string.
    pub premiere_date: Option<String>,
    /// Date the item was added, as an ISO 8601 string.
    pub date_created: Option<String>,
    /// Duration.
    pub run_time_ticks: Option<Ticks>,
    /// Genre names.
    pub genres: Vec<String>,
    /// Community rating.
    pub community_rating: Option<f32>,
    /// Parental rating, such as `PG-13`.
    pub official_rating: Option<String>,
    /// Library type, for library views.
    pub collection_type: Option<String>,
    /// Identifier of the series, for seasons and episodes.
    pub series_id: Option<ItemId>,
    /// Name of the series, for seasons and episodes.
    pub series_name: Option<String>,
    /// Identifier of the season, for episodes.
    pub season_id: Option<ItemId>,
    /// Episode or season number.
    pub index_number: Option<u32>,
    /// Season number, for episodes.
    pub parent_index_number: Option<u32>,
    /// Number of children, as scanned by the server.
    pub child_count: Option<u32>,
    /// State of the item for the signed-in user.
    pub user_data: Option<UserItemDataDto>,
    /// Image tags by image type.
    pub image_tags: HashMap<ImageType, String>,
    /// Tags of the item's backdrop images.
    pub backdrop_image_tags: Vec<String>,
    /// Item holding the backdrop to fall back on.
    pub parent_backdrop_item_id: Option<ItemId>,
    /// Tags of the fallback backdrop images.
    pub parent_backdrop_image_tags: Vec<String>,
    /// Tag of the series' primary image, for episodes.
    pub series_primary_image_tag: Option<String>,
    /// Cast and crew.
    pub people: Vec<BaseItemPerson>,
}

impl Default for BaseItemDto {
    fn default() -> Self {
        Self {
            id: ItemId::from(""),
            name: None,
            r#type: BaseItemKind::Other(String::new()),
            overview: None,
            production_year: None,
            premiere_date: None,
            date_created: None,
            run_time_ticks: None,
            genres: Vec::new(),
            community_rating: None,
            official_rating: None,
            collection_type: None,
            series_id: None,
            series_name: None,
            season_id: None,
            index_number: None,
            parent_index_number: None,
            child_count: None,
            user_data: None,
            image_tags: HashMap::new(),
            backdrop_image_tags: Vec::new(),
            parent_backdrop_item_id: None,
            parent_backdrop_image_tags: Vec::new(),
            series_primary_image_tag: None,
            people: Vec::new(),
        }
    }
}

/// Filters, sorting and paging of an item listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemsQuery {
    /// Only items inside this parent.
    pub parent_id: Option<ItemId>,
    /// Only items of these kinds.
    pub include_item_types: Vec<BaseItemKind>,
    /// Include descendants, not just direct children.
    pub recursive: bool,
    /// Sort fields, in priority order.
    pub sort_by: Vec<ItemSortBy>,
    /// Sort direction.
    pub sort_order: Option<SortOrder>,
    /// Only items of these genres, sent pipe delimited.
    pub genres: Vec<String>,
    /// Only played (`true`) or unplayed (`false`) items.
    pub is_played: Option<bool>,
    /// Only favorites (`true`) or non-favorites (`false`).
    pub is_favorite: Option<bool>,
    /// Free-text search.
    pub search_term: Option<String>,
    /// Index of the first item to return.
    pub start_index: Option<u32>,
    /// Maximum number of items to return.
    pub limit: Option<u32>,
    /// Optional fields to include.
    pub fields: Vec<ItemFields>,
}

fn join<'a>(values: impl Iterator<Item = &'a str>) -> String {
    values.collect::<Vec<_>>().join(",")
}

impl ItemsQuery {
    pub(crate) fn to_params(&self, user_id: &str) -> Vec<(&'static str, String)> {
        let mut params = Vec::new();
        if let Some(parent) = &self.parent_id {
            params.push(("parentId", parent.to_string()));
        }
        if !self.include_item_types.is_empty() {
            let kinds = join(self.include_item_types.iter().map(BaseItemKind::as_str));
            params.push(("includeItemTypes", kinds));
        }
        if self.recursive {
            params.push(("recursive", "true".to_owned()));
        }
        if !self.sort_by.is_empty() {
            params.push(("sortBy", join(self.sort_by.iter().map(ItemSortBy::as_str))));
        }
        if let Some(order) = self.sort_order {
            params.push(("sortOrder", order.as_str().to_owned()));
        }
        if !self.genres.is_empty() {
            params.push(("genres", self.genres.join("|")));
        }
        if let Some(played) = self.is_played {
            params.push(("isPlayed", played.to_string()));
        }
        if let Some(favorite) = self.is_favorite {
            params.push(("isFavorite", favorite.to_string()));
        }
        if let Some(term) = &self.search_term {
            params.push(("searchTerm", term.clone()));
        }
        if let Some(start) = self.start_index {
            params.push(("startIndex", start.to_string()));
        }
        if let Some(limit) = self.limit {
            params.push(("limit", limit.to_string()));
        }
        if !self.fields.is_empty() {
            params.push(("fields", join(self.fields.iter().map(ItemFields::as_str))));
        }
        params.push(("userId", user_id.to_owned()));
        params.push(("enableUserData", "true".to_owned()));
        params
    }
}
