//! Movie page: details, playback buttons and the watched and favorite toggles.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use oxyfin_api::ItemId;
use oxyfin_core::{CollectionLink, CoreError, MovieDetails, PlayMode, UserState};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::images::load_into;
use crate::present::{clock, core_error_message, image_size};
use crate::screens::cast::cast_cards;
use crate::screens::home::{banner_artwork, banner_box};
use crate::screens::library::open_detail;
use crate::screens::similar::SimilarRow;
use crate::state::{GenerationId, Msg, Screen};
use crate::{AppWindow, Messages, MovieData};

// Logical size of the poster, as drawn by `DetailHeader`.
const POSTER_ARTWORK: (f32, f32) = (240.0, 360.0);

#[derive(Clone, Copy)]
enum Flag {
    Watched,
    Favorite,
}

/// State of the movie page: the movie shown and the answers still expected.
#[derive(Default)]
pub(crate) struct MovieController {
    instance: Rc<Cell<u64>>,
    load: Cell<u64>,
    current: RefCell<Option<String>>,
    details: RefCell<Option<MovieDetails>>,
    images_requested: Cell<bool>,
    similar: SimilarRow,
    collection: Rc<RefCell<Option<CollectionLink>>>,
}

impl MovieController {
    /// A screen was shown: a movie page loads when it is not the movie already shown.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: &Screen) {
        let Screen::Movie(id) = screen else { return };
        if self.current.borrow().as_deref() == Some(id.as_str()) {
            return;
        }
        self.clear(app);
        *self.current.borrow_mut() = Some(id.clone());
        self.request(app);
    }

    /// A screen left the stack: the movie page frees its content.
    pub(crate) fn discard(&self, app: &App, screen: &Screen) {
        if matches!(screen, Screen::Movie(id) if self.current.borrow().as_deref() == Some(id.as_str()))
        {
            self.clear(app);
        }
    }

    /// The page came back from the player: the position may have changed.
    pub(crate) fn refresh(&self, app: &Rc<App>) {
        if self.current.borrow().is_some() {
            self.request(app);
        }
    }

    /// Asks again after the movie failed to load.
    pub(crate) fn retry(&self, app: &Rc<App>) {
        self.refresh(app);
    }

    /// Plays the movie, resuming it or starting over.
    pub(crate) fn play(&self, app: &Rc<App>, mode: PlayMode) {
        let id = self
            .details
            .borrow()
            .as_ref()
            .map(|details| details.summary.id.clone());
        if let Some(id) = id {
            app.play(id, mode);
        }
    }

    /// Opens the item of the card `id` in the "More like this" row.
    pub(crate) fn open_similar(&self, app: &Rc<App>, id: &str) {
        self.similar.open(app, id);
    }

    /// Opens the page of the box set the movie belongs to.
    pub(crate) fn open_collection(&self, app: &Rc<App>) {
        let link = self.collection.borrow().clone();
        if let Some(link) = link {
            open_detail(app, Screen::Collection(link.id.as_str().to_owned()));
        }
    }

    /// Marks the movie watched or unwatched.
    pub(crate) fn toggle_watched(&self, app: &Rc<App>) {
        self.toggle(app, Flag::Watched);
    }

    /// Adds the movie to the favorites or removes it.
    pub(crate) fn toggle_favorite(&self, app: &Rc<App>) {
        self.toggle(app, Flag::Favorite);
    }

    fn clear(&self, app: &App) {
        self.instance.set(self.instance.get() + 1);
        self.load.set(self.load.get() + 1);
        self.current.borrow_mut().take();
        self.details.borrow_mut().take();
        self.images_requested.set(false);
        self.similar.clear(app, AppWindow::set_movie_similar);
        self.collection.borrow_mut().take();
        if let Some(window) = app.window() {
            window.set_movie_people(ModelRc::default());
            window.set_movie_collection(SharedString::default());
            window.set_movie_loaded(false);
            window.set_movie_loading(false);
            window.set_movie_error(SharedString::default());
            window.set_movie_busy(false);
            window.set_movie_data(MovieData::default());
            window.set_movie_poster(Default::default());
            window.set_movie_backdrop(Default::default());
        }
    }

    fn request(&self, app: &Rc<App>) {
        let (Some(movie), Some((id, catalog, _))) = (self.current.borrow().clone(), app.services())
        else {
            return;
        };
        self.load.set(self.load.get() + 1);
        let load = self.load.get();
        if let Some(window) = app.window() {
            window.set_movie_error(SharedString::default());
            window.set_movie_loading(self.details.borrow().is_none());
        }
        app.spawn(
            async move { catalog.movie(&ItemId::from(movie.as_str())).await },
            move |app, result| app.movie.answered(app, load, id, result),
        );
    }

    fn answered(
        &self,
        app: &Rc<App>,
        load: u64,
        id: GenerationId,
        result: Result<MovieDetails, CoreError>,
    ) {
        if self.load.get() != load || app.services().is_none_or(|(current, ..)| current != id) {
            return;
        }
        let Some(window) = app.window() else { return };
        window.set_movie_loading(false);
        match result {
            Ok(details) => {
                *self.details.borrow_mut() = Some(details);
                self.publish(app, id, &window);
                if let Some(movie) = self.current.borrow().clone() {
                    self.similar
                        .request(app, &movie, AppWindow::set_movie_similar);
                }
            }
            Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot load the movie: {error}");
                if self.details.borrow().is_none() {
                    let (message, detail) = core_error_message(&error);
                    let text = window
                        .global::<Messages>()
                        .invoke_message_text(message, detail);
                    window.set_movie_error(text);
                }
            }
        }
    }

    fn publish(&self, app: &Rc<App>, id: GenerationId, window: &AppWindow) {
        let details = self.details.borrow();
        let Some(details) = details.as_ref() else {
            return;
        };
        window.set_movie_data(movie_data(details));
        window.set_movie_loaded(true);
        if self.images_requested.replace(true) {
            return;
        }
        window.set_movie_people(cast_cards(app, id, window, &details.cast));
        self.request_collection(app, id, &details.summary.id);
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
                    window.set_movie_poster(image);
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
                    window.set_movie_backdrop(image);
                }
            });
        }
    }

    /// Asks for the box sets of `movie`; the first one is linked, a failure hides the link.
    fn request_collection(&self, app: &Rc<App>, id: GenerationId, movie: &ItemId) {
        let Some((_, catalog, _)) = app.services() else {
            return;
        };
        let (movie, instance) = (movie.clone(), self.instance.get());
        let (current, slot) = (Rc::clone(&self.instance), Rc::clone(&self.collection));
        app.spawn(
            async move { catalog.collections_of(&movie).await },
            move |app, result| {
                if current.get() != instance || app.services().is_none_or(|(now, ..)| now != id) {
                    return;
                }
                match result {
                    Ok(links) => {
                        let Some(link) = links.into_iter().next() else {
                            return;
                        };
                        if let Some(window) = app.window() {
                            window.set_movie_collection(link.name.as_str().into());
                        }
                        *slot.borrow_mut() = Some(link);
                    }
                    Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
                    Err(error) => log::warn!("cannot load the collections of the movie: {error}"),
                }
            },
        );
    }

    fn toggle(&self, app: &Rc<App>, flag: Flag) {
        let Some(window) = app.window() else { return };
        let Some((id, catalog, _)) = app.services() else {
            return;
        };
        if window.get_movie_busy() {
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
        window.set_movie_busy(true);
        let instance = self.instance.get();
        app.spawn(
            async move {
                match flag {
                    Flag::Watched => catalog.set_played(&item, target).await,
                    Flag::Favorite => catalog.set_favorite(&item, target).await,
                }
            },
            move |app, result| app.movie.toggled(app, instance, id, result),
        );
    }

    fn toggled(
        &self,
        app: &Rc<App>,
        instance: u64,
        id: GenerationId,
        result: Result<UserState, CoreError>,
    ) {
        if self.instance.get() != instance
            || app.services().is_none_or(|(current, ..)| current != id)
        {
            return;
        }
        let Some(window) = app.window() else { return };
        window.set_movie_busy(false);
        match result {
            Ok(state) => {
                if let Some(details) = self.details.borrow_mut().as_mut() {
                    details.can_resume = state.can_resume();
                    details.summary.user = state;
                }
                // A reload that started before the change would bring the old state back.
                self.load.set(self.load.get() + 1);
                self.publish(app, id, &window);
                if let Some(movie) = self.current.borrow().clone() {
                    app.dispatch(Msg::ItemChanged(Screen::Movie(movie)));
                }
            }
            Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot change the movie: {error}");
                app.notify(core_error_message(&error).0);
            }
        }
    }
}

/// What the movie page shows for `details`.
fn movie_data(details: &MovieDetails) -> MovieData {
    let summary = &details.summary;
    let genres: Vec<SharedString> = details
        .genres
        .iter()
        .map(|genre| genre.as_str().into())
        .collect();
    MovieData {
        title: summary.title.as_str().into(),
        year: summary
            .year
            .map(|year| year.to_string())
            .unwrap_or_default()
            .into(),
        minutes: summary.duration.map_or(0, |duration| {
            i32::try_from(duration.as_secs() / 60).unwrap_or(0)
        }),
        rating: details
            .community_rating
            .map(|rating| format!("{rating:.1}"))
            .unwrap_or_default()
            .into(),
        official_rating: details.official_rating.clone().unwrap_or_default().into(),
        genres: ModelRc::new(VecModel::from(genres)),
        overview: details.overview.clone().unwrap_or_default().into(),
        watched: summary.user.played,
        favorite: summary.user.favorite,
        can_resume: details.can_resume,
        resume_from: clock(summary.user.position),
    }
}
