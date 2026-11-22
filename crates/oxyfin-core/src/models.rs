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
    /// A collection of movies or series, also called a box set.
    BoxSet,
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
    /// Index among the images of that kind, for a chapter image; `None` otherwise.
    pub index: Option<u32>,
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
        index: None,
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

/// Image standing for an item on its own, as in a media control: its primary image, else
/// its series poster, its thumbnail or a backdrop. Only images with a tag are picked.
pub(crate) fn artwork(dto: &BaseItemDto) -> Option<ImageRef> {
    own_image(dto, ImageType::Primary)
        .or_else(|| series_poster(dto))
        .or_else(|| own_image(dto, ImageType::Thumb))
        .or_else(|| backdrop(dto))
}

fn duration(dto: &BaseItemDto) -> Option<Duration> {
    dto.run_time_ticks.map(|ticks| ticks.to_duration())
}

/// Compact description of a movie, video, series, episode or box set, for lists and cards.
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
    /// Synopsis, when the list asked for it.
    pub overview: Option<String>,
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
            BaseItemKind::BoxSet => ItemKind::BoxSet,
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
            overview: non_empty(dto.overview.as_deref()),
            user: UserState::from_dto(dto.user_data.as_ref()),
        })
    }
}

/// Credit of a person on an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersonKind {
    /// An actor, a guest star or a voice.
    Actor,
    /// A director.
    Director,
    /// A writer.
    Writer,
    /// A composer.
    Composer,
    /// A producer.
    Producer,
    /// Any other credit.
    Other,
}

impl PersonKind {
    fn from_type(kind: Option<&str>) -> Self {
        match kind.unwrap_or_default() {
            "Actor" | "GuestStar" => Self::Actor,
            "Director" => Self::Director,
            "Writer" => Self::Writer,
            "Composer" => Self::Composer,
            "Producer" => Self::Producer,
            _ => Self::Other,
        }
    }
}

/// A person credited on an item, with all the credits of that person joined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    /// Identifier of the person, when the server knows one.
    pub id: Option<ItemId>,
    /// Name of the person.
    pub name: String,
    /// Characters played, for actors, joined with " / ".
    pub role: Option<String>,
    /// Main credit: acting first, then directing, writing, composing, producing.
    pub kind: PersonKind,
    /// Crew credit of an actor who also directs, writes, composes or produces the item.
    pub also: Option<PersonKind>,
    /// Portrait of the person.
    pub image: Option<ImageRef>,
}

impl Person {
    fn from_dto(dto: &BaseItemPerson) -> Self {
        Self {
            id: dto.id.clone(),
            name: dto.name.clone(),
            role: non_empty(dto.role.as_deref()),
            kind: PersonKind::from_type(dto.r#type.as_deref()),
            also: None,
            image: dto
                .id
                .as_ref()
                .and_then(|id| image(id, ImageType::Primary, dto.primary_image_tag.as_ref())),
        }
    }

    /// Adds `credit`, another credit of the same person.
    fn join(&mut self, credit: Person) {
        let (kind, role) = (credit.kind, credit.role);
        if (kind as u8) < (self.kind as u8) {
            let previous = std::mem::replace(&mut self.kind, kind);
            self.also = (kind == PersonKind::Actor).then_some(previous);
            self.role = role;
        } else if kind == self.kind {
            self.role = match (self.role.take(), role) {
                (Some(kept), Some(added)) if kept != added => Some(format!("{kept} / {added}")),
                (kept, added) => kept.or(added),
            };
        } else if self.kind == PersonKind::Actor && kind != PersonKind::Other {
            self.also = Some(self.also.map_or(kind, |also| {
                if (kind as u8) < (also as u8) {
                    kind
                } else {
                    also
                }
            }));
        }
        if self.image.is_none() {
            self.image = credit.image;
        }
    }
}

/// Number of people listed on a detail page.
const CAST_LIMIT: usize = 20;

/// Places kept for the crew when the actors alone would fill the list.
const CREW_PLACES: usize = 5;

fn non_empty(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// One card per person, actors in the order of the server, then directors, writers, composers,
/// producers and the other credits. A long cast leaves up to five places to the crew.
fn cast_and_crew(dto: &BaseItemDto) -> Vec<Person> {
    let mut people: Vec<Person> = Vec::new();
    for credit in dto.people.iter().map(Person::from_dto) {
        let same = credit.id.as_ref().and_then(|id| {
            people
                .iter()
                .position(|person| person.id.as_ref() == Some(id))
        });
        match same {
            Some(index) => people[index].join(credit),
            None => people.push(credit),
        }
    }
    people.sort_by_key(|person| person.kind as u8);
    let actors = people
        .iter()
        .filter(|person| person.kind == PersonKind::Actor)
        .count();
    let crew = people.len() - actors;
    let kept_actors = actors.min(CAST_LIMIT - crew.min(CREW_PLACES));
    let kept_crew = crew.min(CAST_LIMIT - kept_actors);
    let mut crew_people = people.split_off(actors);
    people.truncate(kept_actors);
    crew_people.truncate(kept_crew);
    people.extend(crew_people);
    people
}

/// Biography and portrait of a person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonDetails {
    /// Identifier of the person.
    pub id: ItemId,
    /// Name of the person.
    pub name: String,
    /// Biography.
    pub overview: Option<String>,
    /// Date of birth, as `YYYY-MM-DD`.
    pub birth: Option<String>,
    /// Place of birth.
    pub birth_place: Option<String>,
    /// Portrait of the person.
    pub image: Option<ImageRef>,
}

impl PersonDetails {
    /// Converts the DTO of a person.
    pub fn from_dto(dto: &BaseItemDto) -> Self {
        Self {
            id: dto.id.clone(),
            name: dto.name.clone().unwrap_or_default(),
            overview: non_empty(dto.overview.as_deref()),
            birth: dto
                .premiere_date
                .as_deref()
                .and_then(|date| date.get(..10))
                .filter(|date| date.len() == 10)
                .map(str::to_owned),
            birth_place: non_empty(dto.production_locations.first().map(String::as_str)),
            image: own_image(dto, ImageType::Primary),
        }
    }
}

/// A title crediting a person, with the role the person plays in it.
#[derive(Clone, Debug, PartialEq)]
pub struct PersonTitle {
    /// The title.
    pub summary: ItemSummary,
    /// Character played by the person, when known.
    pub role: Option<String>,
}

impl PersonTitle {
    /// Converts a DTO listed with its people; `None` for kinds the UI does not display.
    pub fn from_dto(dto: &BaseItemDto, person: &ItemId) -> Option<Self> {
        let summary =
            ItemSummary::from_dto(dto).filter(|summary| summary.kind != ItemKind::BoxSet)?;
        let role = dto
            .people
            .iter()
            .filter(|credit| credit.id.as_ref() == Some(person))
            .find_map(|credit| non_empty(credit.role.as_deref()));
        Some(Self { summary, role })
    }
}

/// What a card shows about an item when it is hovered.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemBrief {
    /// Summary, with the overview.
    pub summary: ItemSummary,
    /// Genre names.
    pub genres: Vec<String>,
    /// Average community rating, out of 10.
    pub community_rating: Option<f32>,
    /// Number of seasons of a series, when the server counted any.
    pub season_count: Option<u32>,
}

impl ItemBrief {
    /// Converts a DTO; returns `None` for kinds the UI does not display.
    pub fn from_dto(dto: &BaseItemDto) -> Option<Self> {
        let summary = ItemSummary::from_dto(dto)?;
        let season_count = dto
            .child_count
            .filter(|count| summary.kind == ItemKind::Series && *count > 0);
        Some(Self {
            summary,
            genres: dto.genres.clone(),
            community_rating: dto.community_rating,
            season_count,
        })
    }
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
    /// Average community rating, out of 10.
    pub community_rating: Option<f32>,
    /// Content rating such as `PG-13`.
    pub official_rating: Option<String>,
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
            cast: cast_and_crew(dto),
            can_resume,
            community_rating: dto.community_rating,
            official_rating: dto.official_rating.clone(),
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
    /// Average community rating, out of 10.
    pub community_rating: Option<f32>,
    /// Content rating such as `TV-14`.
    pub official_rating: Option<String>,
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
            cast: cast_and_crew(dto),
            seasons,
            next_episode,
            community_rating: dto.community_rating,
            official_rating: dto.official_rating.clone(),
        })
    }
}

/// A box set an item belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionLink {
    /// Identifier of the box set.
    pub id: ItemId,
    /// Name of the box set.
    pub name: String,
}

/// Full description of a box set with its items.
#[derive(Clone, Debug, PartialEq)]
pub struct CollectionDetails {
    /// Summary of the box set.
    pub summary: ItemSummary,
    /// Synopsis.
    pub overview: Option<String>,
    /// Items of the box set, oldest premiere first, undated ones last.
    pub items: Vec<ItemSummary>,
    /// Number of items played.
    pub watched: usize,
    /// First item not played yet, `None` when every item was played.
    pub first_unwatched: Option<ItemId>,
}

impl CollectionDetails {
    /// Converts the DTO of a box set and the DTOs of its items, whatever their order.
    ///
    /// Returns `None` when `dto` is not a box set.
    pub fn from_dto(dto: &BaseItemDto, children: &[BaseItemDto]) -> Option<Self> {
        let summary = ItemSummary::from_dto(dto).filter(|s| s.kind == ItemKind::BoxSet)?;
        let mut ordered: Vec<&BaseItemDto> = children.iter().collect();
        ordered.sort_by_key(|child| (child.premiere_date.is_none(), child.premiere_date.clone()));
        let items: Vec<ItemSummary> = ordered
            .into_iter()
            .filter_map(ItemSummary::from_dto)
            .collect();
        let watched = items.iter().filter(|item| item.user.played).count();
        let first_unwatched = items
            .iter()
            .find(|item| !item.user.played)
            .map(|item| item.id.clone());
        Some(Self {
            summary,
            overview: non_empty(dto.overview.as_deref()),
            items,
            watched,
            first_unwatched,
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

    fn rated(json: &str) -> BaseItemDto {
        let mut value: serde_json::Value = serde_json::from_str(json).unwrap();
        value["CommunityRating"] = serde_json::json!(8.4);
        value["OfficialRating"] = serde_json::json!("TV-14");
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn movie_details_carry_ratings() {
        let dto = rated(include_str!("../tests/fixtures/item_movie.json"));
        let details = MovieDetails::from_dto(&dto).unwrap();
        assert_eq!(details.community_rating, Some(8.4));
        assert_eq!(details.official_rating.as_deref(), Some("TV-14"));
    }

    #[test]
    fn series_details_carry_ratings() {
        let dto = rated(include_str!("../tests/fixtures/item_series.json"));
        let details = SeriesDetails::from_dto(&dto, Vec::new(), None).unwrap();
        assert_eq!(details.community_rating, Some(8.4));
        assert_eq!(details.official_rating.as_deref(), Some("TV-14"));
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
    fn episode_artwork_falls_back_to_the_series_poster() {
        let mut dto = item(include_str!("../tests/fixtures/item_episode.json"));
        dto.image_tags.clear();
        dto.backdrop_image_tags.clear();
        dto.series_primary_image_tag = None;
        dto.parent_backdrop_item_id = None;
        dto.parent_backdrop_image_tags.clear();
        assert_eq!(artwork(&dto), None);

        dto.parent_backdrop_item_id = dto.series_id.clone();
        dto.parent_backdrop_image_tags = vec!["backdroptag".to_owned()];
        assert_eq!(artwork(&dto).unwrap().image_type, ImageType::Backdrop);

        dto.series_primary_image_tag = Some("seriestag".to_owned());
        let art = artwork(&dto).unwrap();
        assert_eq!(art.item_id, dto.series_id.clone().unwrap());
        assert_eq!(
            (art.image_type, art.tag.as_str()),
            (ImageType::Primary, "seriestag")
        );

        dto.image_tags
            .insert(ImageType::Primary, "episodetag".to_owned());
        let art = artwork(&dto).unwrap();
        assert_eq!((art.item_id, art.tag.as_str()), (dto.id, "episodetag"));
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
            r#type: BaseItemKind::Folder,
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
