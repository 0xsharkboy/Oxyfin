//! Browsing the library: home rows, listings, search, details and user state.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

use oxyfin_api::{
    BaseItemDto, BaseItemKind, ItemFields, ItemId, ItemSortBy, ItemsQuery, RemoteSubtitleInfo,
    Session, SortOrder, Ticks,
};

use crate::models::{
    CollectionDetails, CollectionLink, Episode, ItemBrief, ItemKind, ItemSummary, Library,
    LibraryKind, MenuTarget, MovieDetails, Page, PersonDetails, PersonTitle, Season, SeriesDetails,
    UserState,
};
use crate::pages::{Kept, Listing, PageCache, PageKey};
use crate::{CoreError, Paths, Result, account_key};

/// Kind of content the Movies and Shows tabs list across all libraries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContentKind {
    /// Movies.
    Movies,
    /// Series.
    Shows,
}

impl ContentKind {
    fn item_kind(self) -> BaseItemKind {
        match self {
            Self::Movies => BaseItemKind::Movie,
            Self::Shows => BaseItemKind::Series,
        }
    }
}

/// What a listing browses.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrowseSource {
    /// The content of one library.
    Library {
        /// Identifier of the library.
        id: ItemId,
        /// Kind of content the library holds.
        kind: LibraryKind,
    },
    /// All movies or all series of the server, across libraries.
    All(ContentKind),
    /// The favorite movies, series, videos and box sets of the user.
    Favorites,
}

/// Watch state a listing is restricted to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WatchFilter {
    /// No restriction.
    #[default]
    All,
    /// Items not played yet.
    Unplayed,
    /// Movies and videos that can be resumed, and series with a next episode.
    InProgress,
    /// Items played to the end.
    Played,
}

/// Cap on each list of candidates for the in-progress filter.
const IN_PROGRESS_CAP: u32 = 200;

/// Requests listing the items of box sets sent at the same time.
const BOX_SET_REQUESTS: usize = 4;

/// How long the box sets listed stay valid before they are listed again.
const BOX_SET_LIFETIME: Duration = Duration::from_secs(30 * 60);

type ChildrenAnswer = (
    usize,
    oxyfin_api::Result<oxyfin_api::QueryResult<BaseItemDto>>,
);

/// Stores the item identifiers of one box set answer at its index. A box set whose items cannot
/// be listed keeps no item, unless the session expired.
fn store_children(
    items: &mut [Vec<ItemId>],
    joined: Option<std::result::Result<ChildrenAnswer, tokio::task::JoinError>>,
) -> Result<()> {
    let Some(joined) = joined else {
        return Ok(());
    };
    let (index, result) =
        joined.map_err(|error| CoreError::Decode(format!("box set task: {error}")))?;
    let ids = match result.map_err(CoreError::from) {
        Ok(answer) => answer.items.into_iter().map(|dto| dto.id).collect(),
        Err(CoreError::SessionExpired) => return Err(CoreError::SessionExpired),
        Err(error) => {
            log::warn!("cannot list the items of a box set: {error}");
            Vec::new()
        }
    };
    if let Some(slot) = items.get_mut(index) {
        *slot = ids;
    }
    Ok(())
}

/// Field a listing is sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// Watch state to keep.
    pub watch: WatchFilter,
    /// Index of the first item.
    pub start: u32,
    /// Maximum number of items.
    pub limit: u32,
}

/// Box sets of the user with the identifiers of their items.
type BoxSets = Vec<(CollectionLink, Vec<ItemId>)>;

/// The box sets last listed, with the time they were.
type KeptBoxSets = Mutex<Option<(Instant, Arc<BoxSets>)>>;

/// Read access to the library, and the played and favorite toggles.
///
/// Clones share the box sets, listed once per catalog and again every half hour.
#[derive(Clone, Debug)]
pub struct Catalog {
    session: Session,
    box_sets: Arc<KeptBoxSets>,
    pages: Arc<PageCache>,
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

fn movies_series_videos() -> Vec<BaseItemKind> {
    vec![
        BaseItemKind::Movie,
        BaseItemKind::Series,
        BaseItemKind::Video,
    ]
}

fn kinds_of(kind: LibraryKind) -> Vec<BaseItemKind> {
    match kind {
        LibraryKind::Movies => vec![BaseItemKind::Movie],
        LibraryKind::Shows => vec![BaseItemKind::Series],
        LibraryKind::Mixed => movies_series_videos(),
        LibraryKind::Other => vec![BaseItemKind::Movie, BaseItemKind::Series],
    }
}

impl Catalog {
    /// Creates a catalog over a signed-in session.
    pub fn new(session: Session) -> Self {
        Self {
            session,
            box_sets: Arc::default(),
            pages: Arc::new(PageCache::in_memory()),
        }
    }

    /// The same, keeping the home screen and the first pages of the libraries under
    /// `paths.cache_dir` for the next launch, where they start from.
    pub fn on_disk(session: Session, paths: &Paths) -> Self {
        let account = account_key(
            session.server().base_url().as_str(),
            session.user_id().as_str(),
        );
        Self {
            pages: Arc::new(PageCache::on_disk(paths, &account)),
            ..Self::new(session)
        }
    }

    /// Forgets every kept answer: what the user just did made them wrong. Answers on their way
    /// are not kept either.
    pub fn forget_answers(&self) {
        self.pages.forget();
    }

    /// Forgets every kept answer, in memory and on disk, and keeps none from now on: the account
    /// signed out.
    pub fn close(&self) {
        self.pages.close();
    }

    /// Writes the answers the next launch starts from, without waiting for more of them.
    pub fn save_answers(&self) {
        self.pages.save();
    }

    /// Whether the server answered [`Catalog::browse`] for the first page of `query` within `age`.
    pub fn browsed_within(&self, query: &BrowseQuery, age: Duration) -> bool {
        self.pages
            .is_fresh(&PageKey::Browse(Listing::of(query)), age)
    }

    /// Whether the server answered [`Catalog::libraries`], which the home screen lists, within
    /// `age`.
    pub fn listed_libraries_within(&self, age: Duration) -> bool {
        self.pages.is_fresh(&PageKey::Libraries, age)
    }

    fn last<T: Kept>(&self, key: PageKey) -> Option<T> {
        self.pages.last(&key)
    }

    /// Runs a change of the user's state, which makes the kept answers wrong: those asked for
    /// before it, and those asked for while it is on its way.
    async fn change(
        &self,
        change: impl Future<Output = oxyfin_api::Result<oxyfin_api::UserItemDataDto>>,
    ) -> Result<UserState> {
        self.pages.forget();
        let changed = change.await;
        self.pages.forget();
        Ok(UserState::from_dto(Some(&changed?)))
    }

    /// Lists the movie, show, mixed and home video libraries of the user.
    ///
    /// Other views (music, live TV, playlists, box sets) are left out.
    pub async fn libraries(&self) -> Result<Vec<Library>> {
        self.pages
            .keep(PageKey::Libraries, async {
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
            })
            .await
    }

    /// The last answer of [`Catalog::libraries`].
    pub fn last_libraries(&self) -> Option<Vec<Library>> {
        self.last(PageKey::Libraries)
    }

    /// Lists the items the user can resume.
    pub async fn continue_watching(&self, limit: u32) -> Result<Vec<ItemSummary>> {
        self.pages
            .keep(PageKey::Resume(limit), async {
                let result = self.session.resume(limit).await?;
                Ok(summaries(&result.items))
            })
            .await
    }

    /// The last answer of [`Catalog::continue_watching`].
    pub fn last_continue_watching(&self, limit: u32) -> Option<Vec<ItemSummary>> {
        self.last(PageKey::Resume(limit))
    }

    /// Lists the next episodes to watch across all series.
    pub async fn next_up(&self, limit: u32) -> Result<Vec<ItemSummary>> {
        self.pages
            .keep(PageKey::NextUp(limit), async {
                let result = self.session.next_up(None, limit).await?;
                Ok(summaries(&result.items))
            })
            .await
    }

    /// The last answer of [`Catalog::next_up`].
    pub fn last_next_up(&self, limit: u32) -> Option<Vec<ItemSummary>> {
        self.last(PageKey::NextUp(limit))
    }

    /// Lists the items recently added to a library.
    pub async fn latest(&self, library: &ItemId, limit: u32) -> Result<Vec<ItemSummary>> {
        self.pages
            .keep(PageKey::Latest(library.clone(), limit), async {
                let items = self.session.latest(Some(library), limit).await?;
                Ok(summaries(&items))
            })
            .await
    }

    /// The last answer of [`Catalog::latest`].
    pub fn last_latest(&self, library: &ItemId, limit: u32) -> Option<Vec<ItemSummary>> {
        self.last(PageKey::Latest(library.clone(), limit))
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

    /// Lists the items of a library, of the whole server, or the favorites.
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
            is_played: match query.watch {
                WatchFilter::Unplayed => Some(false),
                WatchFilter::Played => Some(true),
                WatchFilter::All | WatchFilter::InProgress => None,
            },
            start_index: Some(query.start),
            limit: Some(query.limit),
            ..ItemsQuery::default()
        };
        match &query.source {
            BrowseSource::Library { id, kind } => {
                items.parent_id = Some(id.clone());
                items.include_item_types = kinds_of(*kind);
            }
            BrowseSource::All(kind) => {
                items.include_item_types = vec![kind.item_kind()];
            }
            BrowseSource::Favorites => {
                items.include_item_types = movies_series_videos();
                items.include_item_types.push(BaseItemKind::BoxSet);
                items.is_favorite = Some(true);
            }
        }
        let fetch = async {
            if query.watch == WatchFilter::InProgress {
                items.ids = self.in_progress_ids().await?;
                if items.ids.is_empty() {
                    return Ok(Page {
                        items: Vec::new(),
                        total: 0,
                        start: query.start,
                    });
                }
            }
            Ok(page(self.session.items(&items).await?))
        };
        if query.start > 0 {
            return fetch.await;
        }
        self.pages
            .keep(PageKey::Browse(Listing::of(query)), fetch)
            .await
    }

    /// The first page last answered by [`Catalog::browse`] for `query`, whatever the page it asks
    /// for.
    pub fn last_browse(&self, query: &BrowseQuery) -> Option<Page<ItemSummary>> {
        self.last(PageKey::Browse(Listing::of(query)))
    }

    /// Collects the movies and videos to resume and the series with a next episode.
    async fn in_progress_ids(&self) -> Result<Vec<ItemId>> {
        let (resume, next_up) = tokio::join!(
            self.session.resume(IN_PROGRESS_CAP),
            self.session.next_up(None, IN_PROGRESS_CAP)
        );
        let (resume, next_up) = (
            resume.map_err(CoreError::from),
            next_up.map_err(CoreError::from),
        );
        let expired = |result: &Result<_>| matches!(result, Err(CoreError::SessionExpired));
        if expired(&resume) || expired(&next_up) {
            return Err(CoreError::SessionExpired);
        }
        let (resume, next_up) = (resume?, next_up?);
        let resumable = summaries(&resume.items)
            .into_iter()
            .filter(|item| matches!(item.kind, ItemKind::Movie | ItemKind::Video))
            .map(|item| item.id);
        let series = summaries(&next_up.items)
            .into_iter()
            .filter_map(|item| item.series_id);
        let mut ids: Vec<ItemId> = Vec::new();
        for id in resumable.chain(series) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// Lists the genres of a source.
    ///
    /// Favorites fall back to the genres of every movie, series and video.
    pub async fn genres(&self, source: &BrowseSource) -> Result<Vec<String>> {
        self.pages
            .keep(PageKey::Genres(source.clone()), async {
                let result = match source {
                    BrowseSource::Library { id, kind } => {
                        self.session.genres(Some(id), &kinds_of(*kind)).await?
                    }
                    BrowseSource::All(kind) => {
                        self.session.genres(None, &[kind.item_kind()]).await?
                    }
                    BrowseSource::Favorites => {
                        self.session.genres(None, &movies_series_videos()).await?
                    }
                };
                Ok(result
                    .items
                    .into_iter()
                    .filter_map(|genre| genre.name)
                    .collect())
            })
            .await
    }

    /// The last answer of [`Catalog::genres`].
    pub fn last_genres(&self, source: &BrowseSource) -> Option<Vec<String>> {
        self.last(PageKey::Genres(source.clone()))
    }

    /// Searches movies, series and box sets.
    pub async fn search(&self, text: &str, start: u32, limit: u32) -> Result<Page<ItemSummary>> {
        let query = ItemsQuery {
            include_item_types: vec![
                BaseItemKind::Movie,
                BaseItemKind::Series,
                BaseItemKind::BoxSet,
            ],
            recursive: true,
            search_term: Some(text.to_owned()),
            start_index: Some(start),
            limit: Some(limit),
            ..ItemsQuery::default()
        };
        Ok(page(self.session.items(&query).await?))
    }

    /// Lists the items similar to `id`.
    pub async fn similar(&self, id: &ItemId, limit: u32) -> Result<Vec<ItemSummary>> {
        self.pages
            .keep(PageKey::Similar(id.clone(), limit), async {
                let result = self.session.similar(id, limit).await?;
                Ok(summaries(&result.items))
            })
            .await
    }

    /// The last answer of [`Catalog::similar`].
    pub fn last_similar(&self, id: &ItemId, limit: u32) -> Option<Vec<ItemSummary>> {
        self.last(PageKey::Similar(id.clone(), limit))
    }

    /// Lists the box sets the movie or series `id` belongs to.
    ///
    /// The box sets of the user and their items are listed on the first call, then again once
    /// they are half an hour old; a failed listing is asked again on the next call.
    pub async fn collections_of(&self, id: &ItemId) -> Result<Vec<CollectionLink>> {
        let box_sets = {
            let mut kept = self.box_sets.lock().await;
            match &*kept {
                Some((listed, box_sets)) if listed.elapsed() < BOX_SET_LIFETIME => {
                    Arc::clone(box_sets)
                }
                _ => {
                    let box_sets = Arc::new(self.list_box_sets().await?);
                    *kept = Some((Instant::now(), Arc::clone(&box_sets)));
                    box_sets
                }
            }
        };
        Ok(box_sets
            .iter()
            .filter(|(_, items)| items.contains(id))
            .map(|(link, _)| link.clone())
            .collect())
    }

    async fn list_box_sets(&self) -> Result<BoxSets> {
        let query = ItemsQuery {
            include_item_types: vec![BaseItemKind::BoxSet],
            recursive: true,
            sort_by: vec![ItemSortBy::SortName],
            bare: true,
            ..ItemsQuery::default()
        };
        let links: Vec<CollectionLink> = self
            .session
            .items(&query)
            .await?
            .items
            .into_iter()
            .filter(|dto| dto.r#type == BaseItemKind::BoxSet)
            .map(|dto| CollectionLink {
                id: dto.id,
                name: dto.name.unwrap_or_default(),
            })
            .collect();
        let mut items: Vec<Vec<ItemId>> = vec![Vec::new(); links.len()];
        let mut children = tokio::task::JoinSet::new();
        for (index, link) in links.iter().enumerate() {
            if children.len() >= BOX_SET_REQUESTS {
                store_children(&mut items, children.join_next().await)?;
            }
            let (session, parent) = (self.session.clone(), link.id.clone());
            children.spawn(async move {
                let query = ItemsQuery {
                    parent_id: Some(parent),
                    bare: true,
                    ..ItemsQuery::default()
                };
                (index, session.items(&query).await)
            });
        }
        while !children.is_empty() {
            store_children(&mut items, children.join_next().await)?;
        }
        Ok(links.into_iter().zip(items).collect())
    }

    /// Fetches a box set with its items in premiere order.
    pub async fn collection(&self, id: &ItemId) -> Result<CollectionDetails> {
        self.pages
            .keep(PageKey::Collection(id.clone()), async {
                let query = ItemsQuery {
                    parent_id: Some(id.clone()),
                    sort_by: vec![ItemSortBy::PremiereDate],
                    sort_order: Some(SortOrder::Ascending),
                    ..ItemsQuery::default()
                };
                let (dto, children) =
                    tokio::join!(self.session.item(id), self.session.items(&query));
                let (dto, children) = (
                    dto.map_err(CoreError::from),
                    children.map_err(CoreError::from),
                );
                if matches!(dto, Err(CoreError::SessionExpired))
                    || matches!(children, Err(CoreError::SessionExpired))
                {
                    return Err(CoreError::SessionExpired);
                }
                let (dto, children) = (dto?, children?);
                CollectionDetails::from_dto(&dto, &children.items)
                    .ok_or_else(|| CoreError::Decode(format!("item {id} is not a box set")))
            })
            .await
    }

    /// The last answer of [`Catalog::collection`].
    pub fn last_collection(&self, id: &ItemId) -> Option<CollectionDetails> {
        self.last(PageKey::Collection(id.clone()))
    }

    /// Fetches the biography and portrait of a person.
    pub async fn person(&self, id: &ItemId) -> Result<PersonDetails> {
        self.pages
            .keep(PageKey::Person(id.clone()), async {
                let dto = self.session.item(id).await?;
                Ok(PersonDetails::from_dto(&dto))
            })
            .await
    }

    /// The last answer of [`Catalog::person`].
    pub fn last_person(&self, id: &ItemId) -> Option<PersonDetails> {
        self.last(PageKey::Person(id.clone()))
    }

    /// Lists the movies and series of the library crediting the person `id`, newest first.
    pub async fn person_titles(&self, id: &ItemId) -> Result<Vec<PersonTitle>> {
        self.pages
            .keep(PageKey::PersonTitles(id.clone()), async {
                let query = ItemsQuery {
                    person_ids: vec![id.clone()],
                    include_item_types: vec![BaseItemKind::Movie, BaseItemKind::Series],
                    recursive: true,
                    sort_by: vec![ItemSortBy::PremiereDate],
                    sort_order: Some(SortOrder::Descending),
                    fields: vec![ItemFields::People],
                    ..ItemsQuery::default()
                };
                let result = self.session.items(&query).await?;
                Ok(result
                    .items
                    .iter()
                    .filter_map(|dto| PersonTitle::from_dto(dto, id))
                    .collect())
            })
            .await
    }

    /// The last answer of [`Catalog::person_titles`].
    pub fn last_person_titles(&self, id: &ItemId) -> Option<Vec<PersonTitle>> {
        self.last(PageKey::PersonTitles(id.clone()))
    }

    /// Fetches what a hovered card shows about an item, in one request whatever its kind.
    pub async fn item_brief(&self, id: &ItemId) -> Result<ItemBrief> {
        let asked = self.pages.revision();
        let dto = self.session.item(id).await?;
        // The movie page opens on it.
        if let Some(movie) = MovieDetails::from_dto(&dto) {
            self.pages.store(asked, PageKey::Movie(id.clone()), &movie);
        }
        ItemBrief::from_dto(&dto)
            .ok_or_else(|| CoreError::Decode(format!("item {id} cannot be shown")))
    }

    /// Fetches the details of a movie.
    pub async fn movie(&self, id: &ItemId) -> Result<MovieDetails> {
        self.pages
            .keep(PageKey::Movie(id.clone()), async {
                let dto = self.session.item(id).await?;
                MovieDetails::from_dto(&dto)
                    .ok_or_else(|| CoreError::Decode(format!("item {id} is not a movie")))
            })
            .await
    }

    /// The last answer of [`Catalog::movie`].
    pub fn last_movie(&self, id: &ItemId) -> Option<MovieDetails> {
        self.last(PageKey::Movie(id.clone()))
    }

    /// Fetches the details of a series with its seasons and next episode.
    ///
    /// A failed next-episode lookup leaves `next_episode` empty instead of
    /// failing the page, except when the session expired.
    pub async fn series(&self, id: &ItemId) -> Result<SeriesDetails> {
        self.pages
            .keep(PageKey::Series(id.clone()), async {
                let dto = self.session.item(id).await?;
                let (seasons, next) =
                    tokio::join!(self.session.seasons(id), self.next_episode_to_watch(id));
                let seasons = seasons?;
                let seasons: Vec<Season> =
                    seasons.items.iter().filter_map(Season::from_dto).collect();
                let next_episode = match next {
                    Ok(episode) => episode,
                    Err(CoreError::SessionExpired) => return Err(CoreError::SessionExpired),
                    Err(_) => None,
                };
                SeriesDetails::from_dto(&dto, seasons, next_episode)
                    .ok_or_else(|| CoreError::Decode(format!("item {id} is not a series")))
            })
            .await
    }

    /// The last answer of [`Catalog::series`].
    pub fn last_series(&self, id: &ItemId) -> Option<SeriesDetails> {
        self.last(PageKey::Series(id.clone()))
    }

    /// Lists the episodes of one season.
    pub async fn episodes(&self, series: &ItemId, season: &ItemId) -> Result<Vec<Episode>> {
        let key = PageKey::Episodes(series.clone(), Some(season.clone()));
        self.pages
            .keep(key, async {
                let result = self.session.episodes(series, Some(season)).await?;
                Ok(episodes_of(&result.items))
            })
            .await
    }

    /// Lists every episode of a series, in the order of the server.
    pub async fn all_episodes(&self, series: &ItemId) -> Result<Vec<Episode>> {
        self.pages
            .keep(PageKey::Episodes(series.clone(), None), async {
                let result = self.session.episodes(series, None).await?;
                Ok(episodes_of(&result.items))
            })
            .await
    }

    /// The last answer of [`Catalog::episodes`] for a season, or of [`Catalog::all_episodes`].
    pub fn last_episodes(&self, series: &ItemId, season: Option<&ItemId>) -> Option<Vec<Episode>> {
        self.last(PageKey::Episodes(series.clone(), season.cloned()))
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
        self.change(self.session.set_played(id, played)).await
    }

    /// Marks an item favorite or not.
    pub async fn set_favorite(&self, id: &ItemId, favorite: bool) -> Result<UserState> {
        self.change(self.session.set_favorite(id, favorite)).await
    }

    /// Takes an item out of Continue Watching, keeping its played and favorite state.
    pub async fn hide_from_resume(&self, id: &ItemId) -> Result<UserState> {
        self.change(self.session.hide_from_resume(id)).await
    }

    /// Puts an item back in Continue Watching at `position`, undoing [`Self::hide_from_resume`].
    pub async fn restore_resume(&self, id: &ItemId, position: Duration) -> Result<UserState> {
        let position = Ticks::from_duration(position);
        self.change(self.session.set_resume_position(id, position))
            .await
    }

    /// Fetches what the context menu of an item acts on; `None` for kinds it has no menu for.
    pub async fn menu_target(&self, id: &ItemId) -> Result<Option<MenuTarget>> {
        let dto = self.session.item(id).await?;
        let target = match dto.r#type {
            BaseItemKind::Movie | BaseItemKind::Video => {
                ItemSummary::from_dto(&dto).map(MenuTarget::Movie)
            }
            BaseItemKind::Episode => Episode::from_dto(&dto).map(MenuTarget::Episode),
            BaseItemKind::Series => match ItemSummary::from_dto(&dto) {
                Some(summary) => Some(MenuTarget::Series {
                    next_episode: self.next_episode_to_watch(id).await?.map(Box::new),
                    summary,
                }),
                None => None,
            },
            BaseItemKind::Season => match (Season::from_dto(&dto), dto.series_id.clone()) {
                (Some(season), Some(series_id)) => Some(MenuTarget::Season {
                    season,
                    series_id,
                    series_name: dto.series_name.clone(),
                }),
                _ => None,
            },
            _ => None,
        };
        Ok(target)
    }

    /// Finds the episode that playing a season starts: its first unplayed one, resumed when it
    /// was started, else its first.
    pub async fn season_episode_to_play(
        &self,
        series: &ItemId,
        season: &ItemId,
    ) -> Result<Option<Episode>> {
        let episodes = self.episodes(series, season).await?;
        let unplayed = episodes.iter().position(|episode| !episode.user.played);
        Ok(episodes.into_iter().nth(unplayed.unwrap_or(0)))
    }

    /// Signs another device in as this user with the Quick Connect code it shows, typed with
    /// or without spaces or dashes.
    ///
    /// Returns `Ok(false)` for an empty, wrong or expired code, and
    /// [`CoreError::QuickConnectDisabled`] when Quick Connect is turned off on the server.
    pub async fn authorize_device(&self, code: &str) -> Result<bool> {
        let code = crate::quick_connect_code(code);
        if code.is_empty() {
            return Ok(false);
        }
        Ok(self.session.quick_connect_authorize(&code).await?)
    }

    /// Whether the user may search and download subtitles, as the server's policy for them says.
    pub async fn can_manage_subtitles(&self) -> Result<bool> {
        Ok(self.session.me().await?.policy.can_manage_subtitles())
    }

    /// Asks the server's subtitle providers for subtitles of `item` in `language`, given in
    /// any form [`subtitle_language`] reads; a code it does not know is sent as given.
    ///
    /// [`subtitle_language`]: crate::subtitle_language
    pub async fn search_subtitles(
        &self,
        item: &ItemId,
        language: &str,
    ) -> Result<Vec<RemoteSubtitleInfo>> {
        let code = crate::subtitle_language(language).unwrap_or(language);
        Ok(self.session.search_subtitles(item, code).await?)
    }

    /// Makes the server download a subtitle found by [`Catalog::search_subtitles`] and keep
    /// it with `item`. [`PlaybackCommand::ReloadSubtitles`] then shows it in the playing item.
    ///
    /// [`PlaybackCommand::ReloadSubtitles`]: crate::PlaybackCommand::ReloadSubtitles
    pub async fn download_subtitle(&self, item: &ItemId, id: &str) -> Result<()> {
        Ok(self.session.download_subtitle(item, id).await?)
    }
}
