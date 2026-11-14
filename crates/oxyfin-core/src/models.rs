//! Domain models displayed by the UI, converted from API DTOs.

use oxyfin_api::{BaseItemDto, BaseItemKind, BaseItemPerson, ImageType, ItemId, UserItemDataDto};
use std::time::Duration;

/// Kind of item the UI can display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A movie.
    Movie,
    /// A standalone video: a home video or a loose file of a mixed library.
    Video,
    /// A TV series.
    Series,
    /// An episode of a series.
    Episode,
}

/// Reference to an image of an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef {
    /// Item owning the image.
    pub item_id: ItemId,
    /// Kind of image.
    pub image_type: ImageType,
    /// Cache tag of the image.
    pub tag: String,
}

/// State of an item for the signed-in user.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserState {
    /// Whether the item was played.
    pub played: bool,
    /// Whether the item is a favorite.
    pub favorite: bool,
    /// Resume position.
    pub position: Duration,
    /// Fraction played, within `0.0..=1.0`.
    pub progress: Option<f32>,
    /// Number of unplayed children, for folders.
    pub unplayed_count: Option<u32>,
}

impl UserState {
    /// Whether playback can resume from a saved position: started and not finished.
    pub fn can_resume(&self) -> bool {
        !self.position.is_zero() && !self.played
    }

    pub(crate) fn from_dto(data: Option<&UserItemDataDto>) -> Self {
        let Some(data) = data else {
            return Self::default();
        };
        Self {
            played: data.played,
            favorite: data.is_favorite,
            position: data.playback_position_ticks.to_duration(),
            progress: data
                .played_percentage
                .map(|percent| (percent / 100.0).clamp(0.0, 1.0) as f32),
            unplayed_count: data.unplayed_item_count,
        }
    }
}

fn image(item_id: &ItemId, image_type: ImageType, tag: Option<&String>) -> Option<ImageRef> {
    tag.map(|tag| ImageRef {
        item_id: item_id.clone(),
        image_type,
        tag: tag.clone(),
    })
}

fn own_image(dto: &BaseItemDto, image_type: ImageType) -> Option<ImageRef> {
    let tag = dto.image_tags.get(&image_type);
    image(&dto.id, image_type, tag)
}

fn backdrop(dto: &BaseItemDto) -> Option<ImageRef> {
    if let Some(tag) = dto.backdrop_image_tags.first() {
        return image(&dto.id, ImageType::Backdrop, Some(tag));
    }
    let parent = dto.parent_backdrop_item_id.as_ref()?;
    image(
        parent,
        ImageType::Backdrop,
        dto.parent_backdrop_image_tags.first(),
    )
}

fn episode_label(dto: &BaseItemDto) -> Option<String> {
    let season = dto.parent_index_number?;
    let episode = dto.index_number?;
    Some(format!("S{season:02}E{episode:02}"))
}

fn series_poster(dto: &BaseItemDto) -> Option<ImageRef> {
    let series_id = dto.series_id.as_ref()?;
    image(
        series_id,
        ImageType::Primary,
        dto.series_primary_image_tag.as_ref(),
    )
}

fn episode_thumb(dto: &BaseItemDto) -> Option<ImageRef> {
    own_image(dto, ImageType::Primary)
        .or_else(|| own_image(dto, ImageType::Thumb))
        .or_else(|| backdrop(dto))
}

fn duration(dto: &BaseItemDto) -> Option<Duration> {
    dto.run_time_ticks.map(|ticks| ticks.to_duration())
}

/// Compact description of a movie, video, series or episode, for lists and cards.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemSummary {
    /// Identifier of the item.
    pub id: ItemId,
    /// Kind of item.
    pub kind: ItemKind,
    /// Display title.
    pub title: String,
    /// Production year.
    pub year: Option<i32>,
    /// Series the item belongs to, for episodes.
    pub series_id: Option<ItemId>,
    /// Name of the series, for episodes.
    pub series_name: Option<String>,
    /// Label such as `S01E02`, for episodes.
    pub episode_label: Option<String>,
    /// Poster image.
    pub poster: Option<ImageRef>,
    /// Backdrop image.
    pub backdrop: Option<ImageRef>,
    /// Landscape thumbnail.
    pub thumb: Option<ImageRef>,
    /// Duration.
    pub duration: Option<Duration>,
    /// State for the signed-in user.
    pub user: UserState,
}

impl ItemSummary {
    /// Converts a DTO; returns `None` for kinds the UI does not display.
    pub fn from_dto(dto: &BaseItemDto) -> Option<ItemSummary> {
        let kind = match dto.r#type {
            BaseItemKind::Movie => ItemKind::Movie,
            BaseItemKind::Video => ItemKind::Video,
            BaseItemKind::Series => ItemKind::Series,
            BaseItemKind::Episode => ItemKind::Episode,
            _ => return None,
        };
        let is_episode = kind == ItemKind::Episode;
        let poster = if is_episode {
            series_poster(dto).or_else(|| own_image(dto, ImageType::Primary))
        } else {
            own_image(dto, ImageType::Primary)
        };
        let thumb = if is_episode {
            episode_thumb(dto)
        } else {
            own_image(dto, ImageType::Thumb)
        };
        Some(ItemSummary {
            id: dto.id.clone(),
            kind,
            title: dto.name.clone().unwrap_or_default(),
            year: dto.production_year,
            series_id: dto.series_id.clone(),
            series_name: dto.series_name.clone(),
            episode_label: if is_episode { episode_label(dto) } else { None },
            poster,
            backdrop: backdrop(dto),
            thumb,
            duration: duration(dto),
            user: UserState::from_dto(dto.user_data.as_ref()),
        })
    }
}

/// A person credited on an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    /// Name of the person.
    pub name: String,
    /// Character played, for actors.
    pub role: Option<String>,
    /// Portrait of the person.
    pub image: Option<ImageRef>,
}

impl Person {
    fn from_dto(dto: &BaseItemPerson) -> Self {
        Self {
            name: dto.name.clone(),
            role: dto.role.clone().filter(|role| !role.is_empty()),
            image: dto
                .id
                .as_ref()
                .and_then(|id| image(id, ImageType::Primary, dto.primary_image_tag.as_ref())),
        }
    }
}

fn cast(dto: &BaseItemDto) -> Vec<Person> {
    dto.people.iter().map(Person::from_dto).collect()
}

/// Full description of a movie or a standalone video.
#[derive(Clone, Debug, PartialEq)]
pub struct MovieDetails {
    /// Summary shown in lists.
    pub summary: ItemSummary,
    /// Synopsis.
    pub overview: Option<String>,
    /// Genre names.
    pub genres: Vec<String>,
    /// Cast and crew.
    pub cast: Vec<Person>,
    /// Whether playback can resume from a saved position.
    pub can_resume: bool,
}

impl MovieDetails {
    /// Converts a DTO; returns `None` when it is neither a movie nor a standalone video.
    pub fn from_dto(dto: &BaseItemDto) -> Option<Self> {
        let summary = ItemSummary::from_dto(dto)
            .filter(|s| matches!(s.kind, ItemKind::Movie | ItemKind::Video))?;
        let can_resume = summary.user.can_resume();
        Some(Self {
            summary,
            overview: dto.overview.clone(),
            genres: dto.genres.clone(),
            cast: cast(dto),
            can_resume,
        })
    }
}

/// A season of a series.
#[derive(Clone, Debug, PartialEq)]
pub struct Season {
    /// Identifier of the season.
    pub id: ItemId,
    /// Display name.
    pub name: String,
    /// Season number.
    pub number: Option<u32>,
    /// Poster image.
    pub poster: Option<ImageRef>,
    /// State for the signed-in user.
    pub user: UserState,
}

impl Season {
    /// Converts a DTO; returns `None` when it is not a season.
    pub fn from_dto(dto: &BaseItemDto) -> Option<Self> {
        if dto.r#type != BaseItemKind::Season {
            return None;
        }
        Some(Self {
            id: dto.id.clone(),
            name: dto.name.clone().unwrap_or_default(),
            number: dto.index_number,
            poster: own_image(dto, ImageType::Primary),
            user: UserState::from_dto(dto.user_data.as_ref()),
        })
    }
}

/// An episode of a series.
#[derive(Clone, Debug, PartialEq)]
pub struct Episode {
    /// Identifier of the episode.
    pub id: ItemId,
    /// Series the episode belongs to.
    pub series_id: ItemId,
    /// Name of the series, when the server reports it.
    pub series_name: Option<String>,
    /// Season the episode belongs to.
    pub season_id: Option<ItemId>,
    /// Display title.
    pub title: String,
    /// Season number.
    pub season_number: Option<u32>,
    /// Episode number.
    pub number: Option<u32>,
    /// Label such as `S01E02`.
    pub label: Option<String>,
    /// Synopsis.
    pub overview: Option<String>,
    /// Landscape thumbnail.
    pub thumb: Option<ImageRef>,
    /// Duration.
    pub duration: Option<Duration>,
    /// State for the signed-in user.
    pub user: UserState,
}

impl Episode {
    /// Converts a DTO; returns `None` when it is not an episode or lacks its series id.
    pub fn from_dto(dto: &BaseItemDto) -> Option<Self> {
        if dto.r#type != BaseItemKind::Episode {
            return None;
        }
        Some(Self {
            id: dto.id.clone(),
            series_id: dto.series_id.clone()?,
            series_name: dto.series_name.clone(),
            season_id: dto.season_id.clone(),
            title: dto.name.clone().unwrap_or_default(),
            season_number: dto.parent_index_number,
            number: dto.index_number,
            label: episode_label(dto),
            overview: dto.overview.clone(),
            thumb: episode_thumb(dto),
            duration: duration(dto),
            user: UserState::from_dto(dto.user_data.as_ref()),
        })
    }

    /// Whether playback can resume from a saved position.
    pub fn can_resume(&self) -> bool {
        self.user.can_resume()
    }
}

/// Full description of a series with its seasons.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesDetails {
    /// Summary shown in lists.
    pub summary: ItemSummary,
    /// Synopsis.
    pub overview: Option<String>,
    /// Genre names.
    pub genres: Vec<String>,
    /// Cast and crew.
    pub cast: Vec<Person>,
    /// Seasons of the series.
    pub seasons: Vec<Season>,
    /// Episode to play next, `None` when there is none or the lookup failed.
    pub next_episode: Option<Episode>,
}

impl SeriesDetails {
    /// Converts a DTO; returns `None` when it is not a series.
    pub fn from_dto(
        dto: &BaseItemDto,
        seasons: Vec<Season>,
        next_episode: Option<Episode>,
    ) -> Option<Self> {
        let summary = ItemSummary::from_dto(dto).filter(|s| s.kind == ItemKind::Series)?;
        Some(Self {
            summary,
            overview: dto.overview.clone(),
            genres: dto.genres.clone(),
            cast: cast(dto),
            seasons,
            next_episode,
        })
    }
}

/// Kind of content a library holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryKind {
    /// Movies.
    Movies,
    /// TV shows.
    Shows,
    /// Movies, shows and videos together: home videos and libraries without a declared type.
    Mixed,
    /// Any other content, such as music.
    Other,
}

/// A library of the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    /// Identifier of the library.
    pub id: ItemId,
    /// Display name.
    pub name: String,
    /// Kind of content.
    pub kind: LibraryKind,
}

impl Library {
    /// Converts a library view DTO.
    pub fn from_dto(dto: &BaseItemDto) -> Self {
        let kind = match dto.collection_type.as_deref() {
            Some("movies") => LibraryKind::Movies,
            Some("tvshows") => LibraryKind::Shows,
            None | Some("mixed" | "homevideos") => LibraryKind::Mixed,
            _ => LibraryKind::Other,
        };
        Self {
            id: dto.id.clone(),
            name: dto.name.clone().unwrap_or_default(),
            kind,
        }
    }
}

/// A page of results.
#[derive(Clone, Debug, PartialEq)]
pub struct Page<T> {
    /// Items of this page.
    pub items: Vec<T>,
    /// Number of items across all pages.
    pub total: u32,
    /// Index of the first item of this page.
    pub start: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyfin_api::QueryResult;

    fn item(json: &str) -> BaseItemDto {
        serde_json::from_str(json).unwrap()
    }

    fn list(json: &str) -> Vec<BaseItemDto> {
        serde_json::from_str::<QueryResult<BaseItemDto>>(json)
            .unwrap()
            .items
    }

    #[test]
    fn movie_summary_from_fixture() {
        let dto = item(include_str!("../tests/fixtures/item_movie.json"));
        let summary = ItemSummary::from_dto(&dto).unwrap();
        assert_eq!(summary.kind, ItemKind::Movie);
        assert_eq!(summary.title, "Test Movie (2020)");
        assert_eq!(summary.year, Some(2020));
        assert_eq!(summary.duration, Some(Duration::from_micros(10_023_000)));
        let poster = summary.poster.unwrap();
        assert_eq!(poster.tag, "30340af04bf3a0495109fff75a0b7cd0");
        assert_eq!(poster.item_id, dto.id);
        let progress = summary.user.progress.unwrap();
        assert!((progress - 0.2993).abs() < 0.001);
        let details = MovieDetails::from_dto(&dto).unwrap();
        assert_eq!(details.genres, ["Drama", "Comedy"]);
    }

    #[test]
    fn episode_has_label_and_series_poster() {
        let mut dto = item(include_str!("../tests/fixtures/item_episode.json"));
        dto.series_primary_image_tag = Some("seriestag".to_owned());
        dto.image_tags
            .insert(ImageType::Primary, "thumbtag".to_owned());
        let summary = ItemSummary::from_dto(&dto).unwrap();
        assert_eq!(summary.episode_label.as_deref(), Some("S01E01"));
        let poster = summary.poster.unwrap();
        assert_eq!(poster.item_id, dto.series_id.clone().unwrap());
        assert_eq!(poster.tag, "seriestag");
        assert_eq!(summary.thumb.unwrap().tag, "thumbtag");
    }

    #[test]
    fn video_is_a_summary_and_has_movie_details() {
        let dto = BaseItemDto {
            r#type: BaseItemKind::Video,
            name: Some("Holiday".to_owned()),
            ..BaseItemDto::default()
        };
        let summary = ItemSummary::from_dto(&dto).unwrap();
        assert_eq!(summary.kind, ItemKind::Video);
        assert_eq!(
            MovieDetails::from_dto(&dto).unwrap().summary.title,
            "Holiday"
        );
    }

    #[test]
    fn collection_types_map_to_library_kinds() {
        let kind = |collection: Option<&str>| {
            Library::from_dto(&BaseItemDto {
                collection_type: collection.map(str::to_owned),
                ..BaseItemDto::default()
            })
            .kind
        };
        assert_eq!(kind(None), LibraryKind::Mixed);
        assert_eq!(kind(Some("mixed")), LibraryKind::Mixed);
        assert_eq!(kind(Some("homevideos")), LibraryKind::Mixed);
        assert_eq!(kind(Some("music")), LibraryKind::Other);
    }

    #[test]
    fn unknown_kind_is_skipped() {
        let dto = BaseItemDto {
            r#type: BaseItemKind::BoxSet,
            ..BaseItemDto::default()
        };
        assert!(ItemSummary::from_dto(&dto).is_none());
    }

    #[test]
    fn backdrop_falls_back_to_parent() {
        let mut dto = item(include_str!("../tests/fixtures/item_series.json"));
        assert!(ItemSummary::from_dto(&dto).unwrap().backdrop.is_none());
        dto.parent_backdrop_item_id = Some(ItemId::from("parent"));
        dto.parent_backdrop_image_tags = vec!["ptag".to_owned()];
        let backdrop = ItemSummary::from_dto(&dto).unwrap().backdrop.unwrap();
        assert_eq!(backdrop.item_id, ItemId::from("parent"));
        assert_eq!(backdrop.tag, "ptag");
        dto.backdrop_image_tags = vec!["own".to_owned()];
        let backdrop = ItemSummary::from_dto(&dto).unwrap().backdrop.unwrap();
        assert_eq!(backdrop.item_id, dto.id);
        assert_eq!(backdrop.tag, "own");
    }

    #[test]
    fn user_views_map_to_library_kinds() {
        let libraries: Vec<Library> = list(include_str!("../tests/fixtures/user_views.json"))
            .iter()
            .map(Library::from_dto)
            .collect();
        let kinds: Vec<_> = libraries
            .iter()
            .map(|l| (l.name.as_str(), l.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("Movies", LibraryKind::Movies),
                ("Shows", LibraryKind::Shows)
            ]
        );
    }

    #[test]
    fn seasons_and_episodes_from_fixtures() {
        let seasons: Vec<Season> = list(include_str!("../tests/fixtures/shows_seasons.json"))
            .iter()
            .filter_map(Season::from_dto)
            .collect();
        assert_eq!(seasons.len(), 2);
        assert_eq!(seasons[0].name, "Season 1");
        assert_eq!(seasons[0].number, Some(1));
        assert_eq!(seasons[0].user.progress, Some(0.5));
        assert_eq!(seasons[0].user.unplayed_count, Some(1));

        let series = item(include_str!("../tests/fixtures/item_series.json"));
        let details = SeriesDetails::from_dto(&series, seasons, None).unwrap();
        assert_eq!(details.summary.title, "Test Show");
        assert_eq!(details.genres, ["Animation"]);
        assert_eq!(details.seasons.len(), 2);

        let episodes: Vec<Episode> = list(include_str!("../tests/fixtures/shows_episodes.json"))
            .iter()
            .filter_map(Episode::from_dto)
            .collect();
        assert_eq!(episodes.len(), 4);
        assert_eq!(episodes[3].label.as_deref(), Some("S02E02"));
        assert_eq!(episodes[0].title, "Test Show S01E01");
    }

    #[test]
    fn episode_without_series_name_is_kept() {
        let mut dto = item(include_str!("../tests/fixtures/item_episode.json"));
        dto.series_name = None;
        let episode = Episode::from_dto(&dto).unwrap();
        assert_eq!(episode.series_name, None);
    }

    #[test]
    fn episode_without_series_id_is_skipped() {
        let mut dto = item(include_str!("../tests/fixtures/item_episode.json"));
        dto.series_id = None;
        assert!(Episode::from_dto(&dto).is_none());
    }

    #[test]
    fn episode_images_fall_back_in_order() {
        let mut dto = item(include_str!("../tests/fixtures/item_episode.json"));
        // The captured fixture carries no image tags at all.
        assert!(Episode::from_dto(&dto).unwrap().thumb.is_none());
        assert!(ItemSummary::from_dto(&dto).unwrap().poster.is_none());

        dto.parent_backdrop_item_id = Some(ItemId::from("parent"));
        dto.parent_backdrop_image_tags = vec!["bd".to_owned()];
        let thumb = Episode::from_dto(&dto).unwrap().thumb.unwrap();
        assert_eq!(
            (thumb.image_type, thumb.tag.as_str()),
            (ImageType::Backdrop, "bd")
        );

        dto.image_tags.insert(ImageType::Thumb, "th".to_owned());
        let thumb = Episode::from_dto(&dto).unwrap().thumb.unwrap();
        assert_eq!(thumb.tag, "th");

        dto.image_tags.insert(ImageType::Primary, "pr".to_owned());
        let thumb = Episode::from_dto(&dto).unwrap().thumb.unwrap();
        assert_eq!(thumb.tag, "pr");
        let poster = ItemSummary::from_dto(&dto).unwrap().poster.unwrap();
        assert_eq!(
            (poster.item_id, poster.tag.as_str()),
            (dto.id.clone(), "pr")
        );
    }

    #[test]
    fn progress_is_clamped_and_resume_needs_position() {
        let mut dto = item(include_str!("../tests/fixtures/item_movie.json"));
        let data = dto.user_data.as_mut().unwrap();
        data.played_percentage = Some(140.0);
        data.playback_position_ticks = oxyfin_api::Ticks(600_000_000);
        let details = MovieDetails::from_dto(&dto).unwrap();
        assert_eq!(details.summary.user.progress, Some(1.0));
        assert!(details.can_resume);
        dto.user_data.as_mut().unwrap().played = true;
        assert!(!MovieDetails::from_dto(&dto).unwrap().can_resume);
    }

    #[test]
    fn episode_can_resume_needs_position_and_unplayed() {
        let mut episode =
            Episode::from_dto(&item(include_str!("../tests/fixtures/item_episode.json"))).unwrap();
        episode.user.position = Duration::ZERO;
        episode.user.played = false;
        assert!(!episode.can_resume());
        episode.user.position = Duration::from_secs(60);
        assert!(episode.can_resume());
        episode.user.played = true;
        assert!(!episode.can_resume());
    }
}
