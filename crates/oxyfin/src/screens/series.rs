//! Series page: details, seasons, episodes and the watched and favorite toggles.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use oxyfin_api::ItemId;
use oxyfin_core::{CoreError, Episode, PlayMode, SeriesDetails, UserState};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::images::load_into;
use crate::present::{
    SeriesPlay, cast_line, core_error_message, episode_row, image_size, initial_season,
    kept_thumbnails, series_play, thumbnail_tags,
};
use crate::screens::home::{banner_artwork, banner_box};
use crate::state::{GenerationId, Msg, Screen};
use crate::{AppWindow, EpisodeData, Messages, SeriesData, SeriesPlayKind};

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
    episodes_load: Rc<Cell<u64>>,
    current: RefCell<Option<String>>,
    details: RefCell<Option<SeriesDetails>>,
    season: Cell<Option<usize>>,
    rows_season: RefCell<Option<Option<String>>>,
    reloading: Cell<bool>,
    episodes: Rc<VecModel<EpisodeData>>,
    thumbnail_tags: RefCell<HashMap<String, String>>,
    images_requested: Cell<bool>,
}

impl SeriesController {
    /// A screen was shown: a series page loads when it is not the series already shown.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: &Screen) {
        let Screen::Series(id) = screen else { return };
        if self.current.borrow().as_deref() == Some(id.as_str()) {
            return;
        }
        self.clear(app);
        *self.current.borrow_mut() = Some(id.clone());
        if let Some(window) = app.window() {
            window.set_series_episodes(ModelRc::from(Rc::clone(&self.episodes)));
        }
        self.request(app);
    }

    /// A screen left the stack: the series page frees its content.
    pub(crate) fn discard(&self, app: &App, screen: &Screen) {
        if matches!(screen, Screen::Series(id) if self.current.borrow().as_deref() == Some(id.as_str()))
        {
            self.clear(app);
        }
    }

    /// The page came back from the player: the progress may have changed.
    pub(crate) fn refresh(&self, app: &Rc<App>) {
        if self.current.borrow().is_some() {
            self.request(app);
        }
    }

    /// Asks again after the series failed to load.
    pub(crate) fn retry(&self, app: &Rc<App>) {
        self.refresh(app);
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

    /// Marks the series watched or unwatched.
    pub(crate) fn toggle_watched(&self, app: &Rc<App>) {
        self.toggle(app, Flag::Watched);
    }

    /// Adds the series to the favorites or removes it.
    pub(crate) fn toggle_favorite(&self, app: &Rc<App>) {
        self.toggle(app, Flag::Favorite);
    }

    fn clear(&self, app: &App) {
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
        self.images_requested.set(false);
        if let Some(window) = app.window() {
            window.set_series_loaded(false);
            window.set_series_loading(false);
            window.set_series_error(SharedString::default());
            window.set_series_busy(false);
            window.set_series_data(SeriesData::default());
            window.set_series_poster(Default::default());
            window.set_series_backdrop(Default::default());
            window.set_series_seasons(ModelRc::default());
            window.set_series_season(-1);
            window.set_series_episodes_loading(false);
            window.set_series_episodes_stale(false);
            window.set_series_episodes_error(SharedString::default());
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
        if let Some(window) = app.window() {
            window.set_series_error(SharedString::default());
            window.set_series_loading(self.details.borrow().is_none());
        }
        app.spawn(
            async move { catalog.series(&ItemId::from(series.as_str())).await },
            move |app, result| app.series.answered(app, load, id, result),
        );
    }

    fn answered(
        &self,
        app: &Rc<App>,
        load: u64,
        id: GenerationId,
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
                // The chosen season is kept by id: the list may be reordered by a reload.
                let chosen = self
                    .chosen_season_id()
                    .and_then(|chosen| details.seasons.iter().position(|s| s.id.as_str() == chosen))
                    .or_else(|| initial_season(&details));
                self.season.set(chosen);
                *self.details.borrow_mut() = Some(details);
                self.publish(app, id, &window);
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
                }
            }
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
                window.set_series_episodes_stale(true);
            }
            window.set_series_episodes_loading(
                window.get_series_episodes_stale() || self.episodes.row_count() == 0,
            );
        }
        app.spawn(
            async move {
                let series = ItemId::from(series.as_str());
                match season {
                    Some(season) => catalog.episodes(&series, &season).await,
                    None => catalog.all_episodes(&series).await,
                }
            },
            move |app, result| app.series.episodes_answered(app, load, id, result),
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
                let (message, detail) = core_error_message(&error);
                let text = window
                    .global::<Messages>()
                    .invoke_message_text(message, detail);
                window.set_series_episodes_error(text);
                return;
            }
        };
        let rows: Vec<EpisodeData> = self.episodes.iter().collect();
        let kept = kept_thumbnails(&rows, &self.thumbnail_tags.borrow(), &episodes);
        *self.thumbnail_tags.borrow_mut() = thumbnail_tags(&episodes);
        self.episodes.set_vec(
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
        let Some((_, _, images)) = app.services() else {
            return;
        };
        let size = image_size(
            THUMB_ARTWORK.0,
            THUMB_ARTWORK.1,
            window.window().scale_factor(),
        );
        for episode in episodes {
            if kept.contains_key(episode.id.as_str()) {
                continue;
            }
            let Some(thumb) = episode.thumb.clone() else {
                continue;
            };
            let (target, current) = (window.as_weak(), Rc::clone(&self.episodes_load));
            let episode_id = episode.id.as_str().to_owned();
            load_into(app, id, &images, thumb, size, move |image| {
                let Some(window) = target.upgrade() else {
                    return;
                };
                if current.get() != load {
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
        window.set_series_season(self.season.get().map_or(-1, |index| index as i32));
        window.set_series_loaded(true);
        if self.images_requested.replace(true) {
            return;
        }
        let Some((_, _, images)) = app.services() else {
            return;
        };
        let scale = window.window().scale_factor();
        let instance = self.instance.get();
        if let Some(poster) = details.summary.poster.clone() {
            let size = image_size(POSTER_ARTWORK.0, POSTER_ARTWORK.1, scale);
            let (target, current) = (window.as_weak(), Rc::clone(&self.instance));
            load_into(app, id, &images, poster, size, move |image| {
                if let (Some(window), true) = (target.upgrade(), current.get() == instance) {
                    window.set_series_poster(image);
                }
            });
        }
        if let Some(backdrop) = banner_artwork(&details.summary) {
            let width = window.window().size().width as f32 / scale;
            let (width, height) = banner_box(width);
            let size = image_size(width, height, scale);
            let (target, current) = (window.as_weak(), Rc::clone(&self.instance));
            load_into(app, id, &images, backdrop, size, move |image| {
                if let (Some(window), true) = (target.upgrade(), current.get() == instance) {
                    window.set_series_backdrop(image);
                }
            });
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
        cast: cast_line(&details.cast).into(),
        watched: summary.user.played,
        favorite: summary.user.favorite,
        play_kind,
        play_label: play_label.into(),
    }
}
