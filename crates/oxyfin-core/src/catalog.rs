//! Browsing the library: home rows, listings, search, details and user state.

use oxyfin_api::{BaseItemDto, BaseItemKind, ItemId, ItemSortBy, ItemsQuery, Session, SortOrder};

use crate::models::{
    Episode, ItemSummary, Library, LibraryKind, MovieDetails, Page, Season, SeriesDetails,
    UserState,
};
use crate::{CoreError, Result};

/// What a listing browses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowseSource {
    /// The content of one library.
    Library {
        /// Identifier of the library.
        id: ItemId,
        /// Kind of content the library holds.
        kind: LibraryKind,
    },
    /// The favorite movies and series of the user.
    Favorites,
}

/// Field a listing is sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortField {
    /// Alphabetical order.
    Title,
    /// Date the item was added.
    DateAdded,
    /// Production year.
    Year,
}

/// Source, sorting, filters and paging of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowseQuery {
    /// What to browse.
    pub source: BrowseSource,
    /// Sort field.
    pub sort: SortField,
    /// Sort from the largest value to the smallest.
    pub descending: bool,
    /// Only items of this genre.
    pub genre: Option<String>,
    /// Only played (`true`) or unplayed (`false`) items.
    pub played: Option<bool>,
    /// Index of the first item.
    pub start: u32,
    /// Maximum number of items.
    pub limit: u32,
}

/// Read access to the library, and the played and favorite toggles.
#[derive(Clone, Debug)]
pub struct Catalog {
    session: Session,
}

fn summaries(items: &[BaseItemDto]) -> Vec<ItemSummary> {
    items.iter().filter_map(ItemSummary::from_dto).collect()
}

fn episodes_of(items: &[BaseItemDto]) -> Vec<Episode> {
    items.iter().filter_map(Episode::from_dto).collect()
}

fn page(result: oxyfin_api::QueryResult<BaseItemDto>) -> Page<ItemSummary> {
    Page {
        items: summaries(&result.items),
        total: result.total_record_count,
        start: result.start_index,
    }
}

fn kinds_of(kind: LibraryKind) -> Vec<BaseItemKind> {
    match kind {
        LibraryKind::Movies => vec![BaseItemKind::Movie],
        LibraryKind::Shows => vec![BaseItemKind::Series],
        LibraryKind::Mixed => vec![
            BaseItemKind::Movie,
            BaseItemKind::Series,
            BaseItemKind::Video,
        ],
        LibraryKind::Other => vec![BaseItemKind::Movie, BaseItemKind::Series],
    }
}

impl Catalog {
    /// Creates a catalog over a signed-in session.
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    /// Lists the movie, show, mixed and home video libraries of the user.
    ///
    /// Other views (music, live TV, playlists, box sets) are left out.
    pub async fn libraries(&self) -> Result<Vec<Library>> {
        let views = self.session.user_views().await?;
        Ok(views
            .items
            .iter()
            .map(Library::from_dto)
            .filter(|library| {
                matches!(
                    library.kind,
                    LibraryKind::Movies | LibraryKind::Shows | LibraryKind::Mixed
                )
            })
            .collect())
    }

    /// Lists the items the user can resume.
    pub async fn continue_watching(&self, limit: u32) -> Result<Vec<ItemSummary>> {
        let result = self.session.resume(limit).await?;
        Ok(summaries(&result.items))
    }

    /// Lists the next episodes to watch across all series.
    pub async fn next_up(&self, limit: u32) -> Result<Vec<ItemSummary>> {
        let result = self.session.next_up(None, limit).await?;
        Ok(summaries(&result.items))
    }

    /// Lists the items recently added to a library.
    pub async fn latest(&self, library: &ItemId, limit: u32) -> Result<Vec<ItemSummary>> {
        let items = self.session.latest(Some(library), limit).await?;
        Ok(summaries(&items))
    }

    /// Picks the hero item of the home screen: the first item to resume, else the first
    /// recent item with a backdrop.
    pub fn featured(
        continue_watching: &[ItemSummary],
        latest: &[ItemSummary],
    ) -> Option<ItemSummary> {
        continue_watching
            .first()
            .or_else(|| latest.iter().find(|item| item.backdrop.is_some()))
            .cloned()
    }

    /// Lists the items of a library, or the favorites.
    ///
    /// Movies and series are listed, plus videos for mixed libraries and favorites.
    pub async fn browse(&self, query: &BrowseQuery) -> Result<Page<ItemSummary>> {
        let mut items = ItemsQuery {
            recursive: true,
            sort_by: vec![match query.sort {
                SortField::Title => ItemSortBy::SortName,
                SortField::DateAdded => ItemSortBy::DateCreated,
                SortField::Year => ItemSortBy::ProductionYear,
            }],
            sort_order: Some(if query.descending {
                SortOrder::Descending
            } else {
                SortOrder::Ascending
            }),
            genres: query.genre.iter().cloned().collect(),
            is_played: query.played,
            start_index: Some(query.start),
            limit: Some(query.limit),
            ..ItemsQuery::default()
        };
        match &query.source {
            BrowseSource::Library { id, kind } => {
                items.parent_id = Some(id.clone());
                items.include_item_types = kinds_of(*kind);
            }
            BrowseSource::Favorites => {
                items.include_item_types = vec![
                    BaseItemKind::Movie,
                    BaseItemKind::Series,
                    BaseItemKind::Video,
                ];
                items.is_favorite = Some(true);
            }
        }
        Ok(page(self.session.items(&items).await?))
    }

    /// Lists the genres of a library.
    pub async fn genres(&self, library: &ItemId, kind: LibraryKind) -> Result<Vec<String>> {
        let result = self.session.genres(Some(library), &kinds_of(kind)).await?;
        Ok(result
            .items
            .into_iter()
            .filter_map(|genre| genre.name)
            .collect())
    }

    /// Searches movies and series.
    pub async fn search(&self, text: &str, start: u32, limit: u32) -> Result<Page<ItemSummary>> {
        let query = ItemsQuery {
            include_item_types: vec![BaseItemKind::Movie, BaseItemKind::Series],
            recursive: true,
            search_term: Some(text.to_owned()),
            start_index: Some(start),
            limit: Some(limit),
            ..ItemsQuery::default()
        };
        Ok(page(self.session.items(&query).await?))
    }

    /// Fetches the details of a movie.
    pub async fn movie(&self, id: &ItemId) -> Result<MovieDetails> {
        let dto = self.session.item(id).await?;
        MovieDetails::from_dto(&dto)
            .ok_or_else(|| CoreError::Decode(format!("item {id} is not a movie")))
    }

    /// Fetches the details of a series with its seasons and next episode.
    ///
    /// A failed next-episode lookup leaves `next_episode` empty instead of
    /// failing the page, except when the session expired.
    pub async fn series(&self, id: &ItemId) -> Result<SeriesDetails> {
        let dto = self.session.item(id).await?;
        let (seasons, next) =
            tokio::join!(self.session.seasons(id), self.next_episode_to_watch(id));
        let seasons = seasons?;
        let seasons: Vec<Season> = seasons.items.iter().filter_map(Season::from_dto).collect();
        let next_episode = match next {
            Ok(episode) => episode,
            Err(CoreError::SessionExpired) => return Err(CoreError::SessionExpired),
            Err(_) => None,
        };
        SeriesDetails::from_dto(&dto, seasons, next_episode)
            .ok_or_else(|| CoreError::Decode(format!("item {id} is not a series")))
    }

    /// Lists the episodes of one season.
    pub async fn episodes(&self, series: &ItemId, season: &ItemId) -> Result<Vec<Episode>> {
        let result = self.session.episodes(series, Some(season)).await?;
        Ok(episodes_of(&result.items))
    }

    /// Lists every episode of a series, in the order of the server.
    pub async fn all_episodes(&self, series: &ItemId) -> Result<Vec<Episode>> {
        let result = self.session.episodes(series, None).await?;
        Ok(episodes_of(&result.items))
    }

    /// Finds the episode to play next: the server's next up, else the first unplayed one.
    pub async fn next_episode_to_watch(&self, series: &ItemId) -> Result<Option<Episode>> {
        let next = self.session.next_up(Some(series), 1).await?;
        if let Some(episode) = episodes_of(&next.items).into_iter().next() {
            return Ok(Some(episode));
        }
        let all = self.all_episodes(series).await?;
        Ok(all.into_iter().find(|episode| !episode.user.played))
    }

    /// Marks an item played or unplayed.
    pub async fn set_played(&self, id: &ItemId, played: bool) -> Result<UserState> {
        let data = self.session.set_played(id, played).await?;
        Ok(UserState::from_dto(Some(&data)))
    }

    /// Marks an item favorite or not.
    pub async fn set_favorite(&self, id: &ItemId, favorite: bool) -> Result<UserState> {
        let data = self.session.set_favorite(id, favorite).await?;
        Ok(UserState::from_dto(Some(&data)))
    }
}
