//! Series page: details, seasons, episodes and the watched and favorite toggles.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use oxyfin_api::ItemId;
use oxyfin_core::{CoreError, Episode, ImageRef, PlayMode, SeriesDetails, UserState};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::back_cache::Parked;
use crate::images::{PageLoads, load_artwork, load_into};
use crate::present::{
    SeriesPlay, core_error_message, episode_row, image_size, initial_season, kept_thumbnails,
    series_play, thumbnail_tags,
};
use crate::rows;
use crate::screens::cast::{cast_cards, load_photos};
use crate::screens::home::{banner_artwork, banner_box};
use crate::screens::similar::{ParkedRow, SimilarRow};
use crate::state::{GenerationId, Msg, Screen};
use crate::{AppWindow, EpisodeData, Messages, PersonCardData, SeriesData, SeriesPlayKind};

// Logical sizes of the poster and of an episode thumbnail, as drawn by the page.
const POSTER_ARTWORK: (f32, f32) = (240.0, 360.0);
const THUMB_ARTWORK: (f32, f32) = (224.0, 126.0);

#[derive(Clone, Copy)]
enum Flag {
    Watched,
    Favorite,
}

/// State of the series page: the series shown, its chosen season and the answers still expected.
#[derive(Default)]
pub(crate) struct SeriesController {
    instance: Rc<Cell<u64>>,
    load: Cell<u64>,
    episodes_load: Cell<u64>,
    current: RefCell<Option<String>>,
    depth: Cell<usize>,
    details: RefCell<Option<SeriesDetails>>,
    season: Cell<Option<usize>>,
    rows_season: RefCell<Option<Option<String>>>,
    reloading: Cell<bool>,
    episodes: Rc<VecModel<EpisodeData>>,
    thumbnail_tags: RefCell<HashMap<String, String>>,
    // Thumbnail of each episode row, by episode id.
    thumbs: RefCell<Vec<(String, ImageRef)>>,
    // True once the episodes of the chosen season answered.
    episodes_ready: Cell<bool>,
    images_requested: Cell<bool>,
    people: RefCell<Rc<VecModel<PersonCardData>>>,
    similar: SimilarRow,
    loads: PageLoads,
    // Season to show once series `.0` loads, as (series id, season id).
    wanted_season: RefCell<Option<(String, String)>>,
}

/// A series page kept for Back.
pub(crate) struct ParkedSeries {
    details: SeriesDetails,
    season: Option<usize>,
    rows_season: Option<Option<String>>,
    episodes: Vec<EpisodeData>,
    thumbnail_tags: HashMap<String, String>,
    thumbs: Vec<(String, ImageRef)>,
    episodes_ready: bool,
    people: Rc<VecModel<PersonCardData>>,
    similar: ParkedRow,
    poster: Image,
    backdrop: Image,
    viewport_y: f32,
}

fn has_pixels(image: &Image) -> bool {
    image.size().width > 0
}

impl SeriesController {
    /// A screen was shown: a series page loads when it is not the page already shown, or comes
    /// back as it was left when Back returns to it.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: &Screen) {
        let Screen::Series(id) = screen else { return };
        let depth = app.top_depth();
        if self.current.borrow().as_deref() == Some(id.as_str()) && self.depth.get() == depth {
            return;
        }
        self.park(app);
        self.clear(app);
        *self.current.borrow_mut() = Some(id.clone());
        self.depth.set(depth);
        if let Some(window) = app.window() {
            window.set_series_episodes(ModelRc::from(Rc::clone(&self.episodes)));
        }
        match app.unpark(screen) {
            Some(Parked::Series(page)) => self.restore(app, *page),
            _ => self.request(app),
        }
    }

    /// A screen left the stack: the series page frees its content.
    pub(crate) fn discard(&self, app: &App, screen: &Screen) {
        // A season asked for the page that leaves must not apply to a later one.
        if matches!(screen, Screen::Series(_)) {
            self.wanted_season.borrow_mut().take();
        }
        let current = matches!(screen, Screen::Series(id) if self.current.borrow().as_deref() == Some(id.as_str()));
        if current && !app.holds(self.depth.get(), screen) {
            self.clear(app);
        }
    }

    /// Keeps the loaded page shown for Back, before another series replaces it.
    fn park(&self, app: &App) {
        let Some(series) = self.current.borrow().clone() else {
            return;
        };
        let (Some(details), Some(window)) = (self.details.borrow().clone(), app.window()) else {
            return;
        };
        let page = ParkedSeries {
            details,
            season: self.season.get(),
            rows_season: self.rows_season.borrow().clone(),
            episodes: self.episodes.iter().collect(),
            thumbnail_tags: self.thumbnail_tags.take(),
            thumbs: self.thumbs.take(),
            episodes_ready: self.episodes_ready.get(),
            people: Rc::clone(&self.people.borrow()),
            similar: self.similar.park(),
            poster: window.get_series_poster(),
            backdrop: window.get_series_backdrop(),
            viewport_y: window.get_series_viewport_y(),
        };
        app.park(
            self.depth.get(),
            Screen::Series(series),
            Parked::Series(Box::new(page)),
        );
    }

    /// Shows a kept page again without asking the server, loading only what it still misses.
    fn restore(&self, app: &Rc<App>, page: ParkedSeries) {
        let wanted = self.wanted_season.borrow_mut().take();
        let (Some((id, ..)), Some(window)) = (app.services(), app.window()) else {
            return;
        };
        let series = page.details.summary.id.as_str().to_owned();
        *self.details.borrow_mut() = Some(page.details);
        self.season.set(page.season);
        *self.rows_season.borrow_mut() = page.rows_season;
        self.episodes.set_vec(page.episodes);
        *self.thumbnail_tags.borrow_mut() = page.thumbnail_tags;
        *self.thumbs.borrow_mut() = page.thumbs;
        self.episodes_ready.set(page.episodes_ready);
        *self.people.borrow_mut() = Rc::clone(&page.people);
        window.set_series_people(ModelRc::from(page.people));
        window.set_series_poster(page.poster.clone());
        window.set_series_backdrop(page.backdrop.clone());
        self.images_requested.set(true);
        self.publish(app, id, &window);
        if let Some(details) = self.details.borrow().as_ref() {
            let people = Rc::clone(&self.people.borrow());
            load_photos(app, id, &window, &people, &details.cast, &self.loads);
        }
        self.load_images(
            app,
            id,
            &window,
            !has_pixels(&page.poster),
            !has_pixels(&page.backdrop),
        );
        self.similar.restore(
            app,
            id,
            &window,
            page.similar,
            AppWindow::set_series_similar,
        );
        self.similar
            .request(app, &series, AppWindow::set_series_similar);
        if self.episodes_ready.get() {
            let missing: Vec<(String, ImageRef)> = self
                .thumbs
                .borrow()
                .iter()
                .filter(|(episode, _)| {
                    !self
                        .episodes
                        .iter()
                        .any(|row| row.id == episode.as_str() && row.image.size().width > 0)
                })
                .cloned()
                .collect();
            self.load_thumbnails(app, id, &window, missing);
        } else {
            // The episodes had not come, or failed: they are asked again.
            self.request_episodes(app);
        }
        window.set_series_viewport_y(page.viewport_y);
        let wanted = wanted.and_then(|(series, season)| self.season_index(&series, &season));
        if let Some(index) = wanted {
            self.select_season(app, index);
        }
    }

    /// The page came back from the player or holds stale data: the progress may have changed.
    pub(crate) fn refresh(&self, app: &Rc<App>) {
        if self.current.borrow().is_some() {
            self.similar.reload();
            self.request(app);
        }
    }

    /// Asks again after the series failed to load.
    pub(crate) fn retry(&self, app: &Rc<App>) {
        if self.current.borrow().is_some() {
            self.request(app);
        }
    }

    /// Asks again after the episodes failed to load.
    pub(crate) fn retry_episodes(&self, app: &Rc<App>) {
        self.request_episodes(app);
    }

    /// Plays what the core picks for the series: the next episode to watch.
    pub(crate) fn play(&self, app: &Rc<App>) {
        let id = self
            .details
            .borrow()
            .as_ref()
            .map(|details| details.summary.id.clone());
        if let Some(id) = id {
            app.play(id, PlayMode::Auto);
        }
    }

    /// Plays episode `id`, resuming it when it was started.
    pub(crate) fn play_episode(&self, app: &Rc<App>, id: &str) {
        app.play(ItemId::from(id), PlayMode::Auto);
    }

    /// A season pill was clicked.
    pub(crate) fn select_season(&self, app: &Rc<App>, index: usize) {
        let count = self
            .details
            .borrow()
            .as_ref()
            .map_or(0, |details| details.seasons.len());
        if index >= count || self.season.get() == Some(index) {
            return;
        }
        self.season.set(Some(index));
        if let Some(window) = app.window() {
            window.set_series_season(index as i32);
        }
        self.request_episodes(app);
    }

    /// Shows season `season` of series `series`: at once when the page shows that series, or
    /// once the series page about to open has loaded.
    pub(crate) fn show_season(&self, app: &Rc<App>, series: &str, season: &str) {
        // A page of that series kept below another screen stays as it was left.
        let current = matches!(app.screen(), Screen::Series(id) if id == series);
        let index = self.season_index(series, season).filter(|_| current);
        match index {
            Some(index) => self.select_season(app, index),
            None => *self.wanted_season.borrow_mut() = Some((series.to_owned(), season.to_owned())),
        }
    }

    /// Place of season `season` when the page holds series `series`.
    fn season_index(&self, series: &str, season: &str) -> Option<usize> {
        let details = self.details.borrow();
        let details = details.as_ref()?;
        (details.summary.id.as_str() == series)
            .then(|| details.seasons.iter().position(|s| s.id.as_str() == season))
            .flatten()
    }

    /// Opens the item of the card `id` in the "More like this" row.
    pub(crate) fn open_similar(&self, app: &Rc<App>, id: &str) {
        self.similar.open(app, id);
    }

    /// Marks the series watched or unwatched.
    pub(crate) fn toggle_watched(&self, app: &Rc<App>) {
        self.toggle(app, Flag::Watched);
    }

    /// Adds the series to the favorites or removes it.
    pub(crate) fn toggle_favorite(&self, app: &Rc<App>) {
        self.toggle(app, Flag::Favorite);
    }

    fn clear(&self, app: &App) {
        self.loads.abort();
        self.instance.set(self.instance.get() + 1);
        self.load.set(self.load.get() + 1);
        self.episodes_load.set(self.episodes_load.get() + 1);
        self.current.borrow_mut().take();
        self.details.borrow_mut().take();
        self.season.set(None);
        self.rows_season.borrow_mut().take();
        self.reloading.set(false);
        self.episodes.set_vec(Vec::new());
        self.thumbnail_tags.borrow_mut().clear();
        self.thumbs.borrow_mut().clear();
        self.episodes_ready.set(false);
        self.images_requested.set(false);
        *self.people.borrow_mut() = Rc::default();
        self.similar.clear(app, AppWindow::set_series_similar);
        if let Some(window) = app.window() {
            window.set_series_people(ModelRc::default());
            window.set_series_loaded(false);
            window.set_series_loading(false);
            window.set_series_error(SharedString::default());
            window.set_series_busy(false);
            window.set_series_data(SeriesData::default());
            window.set_series_poster(Default::default());
            window.set_series_backdrop(Default::default());
            window.set_series_seasons(ModelRc::default());
            window.set_series_season_ids(ModelRc::default());
            window.set_series_season(-1);
            window.set_series_episodes_loading(false);
            window.set_series_episodes_stale(false);
            window.set_series_episodes_error(SharedString::default());
            window.set_series_viewport_y(0.0);
        }
    }

    fn request(&self, app: &Rc<App>) {
        let (Some(series), Some((id, catalog, _))) =
            (self.current.borrow().clone(), app.services())
        else {
            return;
        };
        self.load.set(self.load.get() + 1);
        let load = self.load.get();
        self.reloading.set(true);
        let last = match &*self.details.borrow() {
            Some(_) => None,
            None => catalog.last_series(&ItemId::from(series.as_str())),
        };
        let behind = last.is_some();
        if let Some(window) = app.window() {
            window.set_series_error(SharedString::default());
            if let Some(details) = last {
                self.show(app, id, &window, details);
                self.show_last_episodes(app, id, &window);
                // Those asked for after the series come then.
                window.set_series_episodes_loading(!self.episodes_ready.get());
            }
            window.set_series_loading(self.details.borrow().is_none());
        }
        app.spawn(
            async move { catalog.series(&ItemId::from(series.as_str())).await },
            move |app, result| app.series.answered(app, load, id, behind, result),
        );
    }

    fn answered(
        &self,
        app: &Rc<App>,
        load: u64,
        id: GenerationId,
        behind: bool,
        result: Result<SeriesDetails, CoreError>,
    ) {
        if self.load.get() != load || app.services().is_none_or(|(current, ..)| current != id) {
            return;
        }
        let Some(window) = app.window() else { return };
        self.reloading.set(false);
        window.set_series_loading(false);
        match result {
            Ok(details) => {
                self.show(app, id, &window, details);
                if let Some(series) = self.current.borrow().clone() {
                    app.dispatch(Msg::ItemChanged(Screen::Series(series)));
                }
                self.request_episodes(app);
            }
            Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot load the series: {error}");
                if self.details.borrow().is_none() {
                    let (message, detail) = core_error_message(&error);
                    let text = window
                        .global::<Messages>()
                        .invoke_message_text(message, detail);
                    window.set_series_error(text);
                } else if behind {
                    app.notify(core_error_message(&error).0);
                }
            }
        }
    }

    /// Shows `details` unless the page shows them already, with the season wanted or chosen, and
    /// asks for similar titles.
    fn show(&self, app: &Rc<App>, id: GenerationId, window: &AppWindow, details: SeriesDetails) {
        // The chosen season is kept by id: the list may be reordered by a reload.
        let wanted = self
            .wanted_season
            .take()
            .filter(|(series, _)| series == details.summary.id.as_str())
            .map(|(_, season)| season);
        let chosen = wanted
            .or_else(|| self.chosen_season_id())
            .and_then(|chosen| details.seasons.iter().position(|s| s.id.as_str() == chosen))
            .or_else(|| initial_season(&details));
        if self.details.borrow().as_ref() != Some(&details) || self.season.get() != chosen {
            self.season.set(chosen);
            *self.details.borrow_mut() = Some(details);
            self.publish(app, id, window);
        }
        if let Some(series) = self.current.borrow().clone() {
            self.similar
                .request(app, &series, AppWindow::set_series_similar);
        }
    }

    /// Shows the last answer for the episodes of the chosen season, if there is one.
    fn show_last_episodes(&self, app: &Rc<App>, id: GenerationId, window: &AppWindow) {
        let (Some(series), Some((_, catalog, _))) = (self.current.borrow().clone(), app.services())
        else {
            return;
        };
        let season = self.chosen_season_id();
        let last = catalog.last_episodes(
            &ItemId::from(series.as_str()),
            season.as_deref().map(ItemId::from).as_ref(),
        );
        if let Some(episodes) = last {
            *self.rows_season.borrow_mut() = Some(season);
            self.show_episodes(app, id, window, &episodes);
        }
    }

    fn request_episodes(&self, app: &Rc<App>) {
        let (Some(series), Some((id, catalog, _))) =
            (self.current.borrow().clone(), app.services())
        else {
            return;
        };
        let season = self.chosen_season_id();
        let wanted = Some(season.clone());
        let changed = self.rows_season.replace(wanted.clone()) != wanted;
        let season = season.map(ItemId::from);
        self.episodes_load.set(self.episodes_load.get() + 1);
        let load = self.episodes_load.get();
        if let Some(window) = app.window() {
            window.set_series_episodes_error(SharedString::default());
            // Rows of another season are hidden, not shown under the new pill.
            if changed {
                self.episodes_ready.set(false);
                window.set_series_episodes_stale(true);
                self.show_last_episodes(app, id, &window);
            }
            window.set_series_episodes_loading(
                window.get_series_episodes_stale() || self.episodes.row_count() == 0,
            );
        }
        let behind = self.episodes_ready.get();
        app.spawn(
            async move {
                let series = ItemId::from(series.as_str());
                match season {
                    Some(season) => catalog.episodes(&series, &season).await,
                    None => catalog.all_episodes(&series).await,
                }
            },
            move |app, result| app.series.episodes_answered(app, load, id, behind, result),
        );
    }

    fn chosen_season_id(&self) -> Option<String> {
        let details = self.details.borrow();
        self.season
            .get()
            .and_then(|index| details.as_ref()?.seasons.get(index))
            .map(|season| season.id.as_str().to_owned())
    }

    fn episodes_answered(
        &self,
        app: &Rc<App>,
        load: u64,
        id: GenerationId,
        behind: bool,
        result: Result<Vec<Episode>, CoreError>,
    ) {
        if self.episodes_load.get() != load
            || app.services().is_none_or(|(current, ..)| current != id)
        {
            return;
        }
        let Some(window) = app.window() else { return };
        window.set_series_episodes_loading(false);
        let episodes = match result {
            Ok(episodes) => episodes,
            Err(CoreError::SessionExpired) => return app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot load the episodes: {error}");
                if behind {
                    return app.notify(core_error_message(&error).0);
                }
                let (message, detail) = core_error_message(&error);
                let text = window
                    .global::<Messages>()
                    .invoke_message_text(message, detail);
                window.set_series_episodes_error(text);
                return;
            }
        };
        self.show_episodes(app, id, &window, &episodes);
    }

    fn show_episodes(
        &self,
        app: &Rc<App>,
        id: GenerationId,
        window: &AppWindow,
        episodes: &[Episode],
    ) {
        let rows: Vec<EpisodeData> = self.episodes.iter().collect();
        let kept = kept_thumbnails(&rows, &self.thumbnail_tags.borrow(), episodes);
        *self.thumbnail_tags.borrow_mut() = thumbnail_tags(episodes);
        rows::update(
            &self.episodes,
            episodes
                .iter()
                .map(|episode| {
                    let mut row = episode_row(episode);
                    if let Some(image) = kept.get(episode.id.as_str()) {
                        row.image = image.clone();
                    }
                    row
                })
                .collect::<Vec<_>>(),
        );
        window.set_series_episodes_stale(false);
        self.episodes_ready.set(true);
        *self.thumbs.borrow_mut() = episodes
            .iter()
            .filter_map(|episode| Some((episode.id.as_str().to_owned(), episode.thumb.clone()?)))
            .collect();
        let missing: Vec<(String, ImageRef)> = self
            .thumbs
            .borrow()
            .iter()
            .filter(|(episode, _)| !kept.contains_key(episode.as_str()))
            .cloned()
            .collect();
        self.load_thumbnails(app, id, window, missing);
    }

    /// Loads the thumbnails `wanted`, by episode id, into the rows of the current episodes.
    fn load_thumbnails(
        &self,
        app: &Rc<App>,
        id: GenerationId,
        window: &AppWindow,
        wanted: Vec<(String, ImageRef)>,
    ) {
        let Some((_, _, images)) = app.services() else {
            return;
        };
        let size = image_size(
            THUMB_ARTWORK.0,
            THUMB_ARTWORK.1,
            window.window().scale_factor(),
        );
        let instance = self.instance.get();
        for (episode_id, thumb) in wanted {
            let (target, current) = (window.as_weak(), Rc::clone(&self.instance));
            let key = format!("{episode_id} {}", thumb.tag);
            self.loads.once(key, || {
                load_into(app, id, &images, thumb, size, move |image| {
                    let Some(window) = target.upgrade() else {
                        return;
                    };
                    if current.get() != instance {
                        return;
                    }
                    let rows = window.get_series_episodes();
                    for index in 0..rows.row_count() {
                        if let Some(mut row) = rows.row_data(index)
                            && row.id == episode_id
                        {
                            row.image = image;
                            rows.set_row_data(index, row);
                            break;
                        }
                    }
                })
            });
        }
    }

    fn publish(&self, app: &Rc<App>, id: GenerationId, window: &AppWindow) {
        let details = self.details.borrow();
        let Some(details) = details.as_ref() else {
            return;
        };
        window.set_series_data(series_data(details));
        window.set_series_seasons(ModelRc::new(VecModel::from(
            details
                .seasons
                .iter()
                .map(|season| SharedString::from(season.name.as_str()))
                .collect::<Vec<_>>(),
        )));
        window.set_series_season_ids(ModelRc::new(VecModel::from(
            details
                .seasons
                .iter()
                .map(|season| SharedString::from(season.id.as_str()))
                .collect::<Vec<_>>(),
        )));
        window.set_series_season(self.season.get().map_or(-1, |index| index as i32));
        window.set_series_loaded(true);
        if self.images_requested.replace(true) {
            return;
        }
        let people = cast_cards(&details.cast);
        *self.people.borrow_mut() = Rc::clone(&people);
        window.set_series_people(ModelRc::from(Rc::clone(&people)));
        load_photos(app, id, window, &people, &details.cast, &self.loads);
        self.load_images(app, id, window, true, true);
    }

    /// Loads the poster and the backdrop of the series, as asked.
    fn load_images(
        &self,
        app: &Rc<App>,
        id: GenerationId,
        window: &AppWindow,
        want_poster: bool,
        want_backdrop: bool,
    ) {
        let details = self.details.borrow();
        let (Some(details), Some((_, _, images))) = (details.as_ref(), app.services()) else {
            return;
        };
        let scale = window.window().scale_factor();
        let instance = self.instance.get();
        if let Some(poster) = details.summary.poster.clone().filter(|_| want_poster) {
            let size = image_size(POSTER_ARTWORK.0, POSTER_ARTWORK.1, scale);
            let (target, current) = (window.as_weak(), Rc::clone(&self.instance));
            self.loads
                .track(load_artwork(app, id, &images, poster, size, move |image| {
                    if let (Some(window), true) = (target.upgrade(), current.get() == instance) {
                        window.set_series_poster(image);
                    }
                }));
        }
        if let Some(backdrop) = banner_artwork(&details.summary).filter(|_| want_backdrop) {
            let width = window.window().size().width as f32 / scale;
            let (width, height) = banner_box(width);
            let size = image_size(width, height, scale);
            let (target, current) = (window.as_weak(), Rc::clone(&self.instance));
            self.loads.track(load_artwork(
                app,
                id,
                &images,
                backdrop,
                size,
                move |image| {
                    if let (Some(window), true) = (target.upgrade(), current.get() == instance) {
                        window.set_series_backdrop(image);
                    }
                },
            ));
        }
    }

    fn toggle(&self, app: &Rc<App>, flag: Flag) {
        let Some(window) = app.window() else { return };
        let Some((id, catalog, _)) = app.services() else {
            return;
        };
        if window.get_series_busy() {
            return;
        }
        let (item, target) = {
            let details = self.details.borrow();
            let Some(details) = details.as_ref() else {
                return;
            };
            let user = &details.summary.user;
            let target = match flag {
                Flag::Watched => !user.played,
                Flag::Favorite => !user.favorite,
            };
            (details.summary.id.clone(), target)
        };
        window.set_series_busy(true);
        let instance = self.instance.get();
        app.spawn(
            async move {
                match flag {
                    Flag::Watched => catalog.set_played(&item, target).await,
                    Flag::Favorite => catalog.set_favorite(&item, target).await,
                }
            },
            move |app, result| app.series.toggled(app, instance, id, flag, result),
        );
    }

    fn toggled(
        &self,
        app: &Rc<App>,
        instance: u64,
        id: GenerationId,
        flag: Flag,
        result: Result<UserState, CoreError>,
    ) {
        if self.instance.get() != instance
            || app.services().is_none_or(|(current, ..)| current != id)
        {
            return;
        }
        let Some(window) = app.window() else { return };
        window.set_series_busy(false);
        match result {
            Ok(state) => {
                if let Some(details) = self.details.borrow_mut().as_mut() {
                    details.summary.user = state;
                }
                // A reload that started before the change would bring the old state back.
                let dropped = self.reloading.get();
                self.load.set(self.load.get() + 1);
                self.publish(app, id, &window);
                if matches!(flag, Flag::Watched) || dropped {
                    self.request(app);
                }
            }
            Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot change the series: {error}");
                app.notify(core_error_message(&error).0);
            }
        }
    }
}

/// What the series page shows for `details`.
fn series_data(details: &SeriesDetails) -> SeriesData {
    let summary = &details.summary;
    let genres: Vec<SharedString> = details
        .genres
        .iter()
        .map(|genre| genre.as_str().into())
        .collect();
    let (play_kind, play_label) = match series_play(details.next_episode.as_ref()) {
        SeriesPlay::Play => (SeriesPlayKind::Play, String::new()),
        SeriesPlay::PlayEpisode(label) => (SeriesPlayKind::PlayEpisode, label),
        SeriesPlay::ResumeEpisode(label) => (SeriesPlayKind::ResumeEpisode, label),
    };
    SeriesData {
        title: summary.title.as_str().into(),
        year: summary
            .year
            .map(|year| year.to_string())
            .unwrap_or_default()
            .into(),
        rating: details
            .community_rating
            .map(|rating| format!("{rating:.1}"))
            .unwrap_or_default()
            .into(),
        official_rating: details.official_rating.clone().unwrap_or_default().into(),
        season_count: i32::try_from(details.seasons.len()).unwrap_or(i32::MAX),
        genres: ModelRc::new(VecModel::from(genres)),
        overview: details.overview.clone().unwrap_or_default().into(),
        watched: summary.user.played,
        favorite: summary.user.favorite,
        play_kind,
        play_label: play_label.into(),
    }
}
