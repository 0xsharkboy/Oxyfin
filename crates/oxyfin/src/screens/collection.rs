//! Collection page: a box set, its playback and toggles, and the row of its items.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use oxyfin_api::ItemId;
use oxyfin_core::{CollectionDetails, CoreError, ItemKind, ItemSummary, PlayMode, UserState};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::images::load_into;
use crate::present::{core_error_message, image_size};
use crate::screens::home::{banner_artwork, banner_box};
use crate::screens::library::{cards_with_posters, load_posters, open_detail, target_of};
use crate::state::{GenerationId, Msg, Screen};
use crate::{AppWindow, CardData, CollectionData, Messages};

// Logical size of the poster, as drawn by `DetailHeader`.
const POSTER_ARTWORK: (f32, f32) = (240.0, 360.0);

#[derive(Clone, Copy)]
enum Flag {
    Watched,
    Favorite,
}

/// The item Play starts: the first one not watched yet, else the first one.
fn play_target(details: &CollectionDetails) -> Option<&ItemSummary> {
    details
        .first_unwatched
        .as_ref()
        .and_then(|id| details.items.iter().find(|item| item.id == *id))
        .or_else(|| details.items.first())
}

/// What the collection page shows for `details`.
fn collection_data(details: &CollectionDetails) -> CollectionData {
    let summary = &details.summary;
    let years: Vec<i32> = details.items.iter().filter_map(|item| item.year).collect();
    let year = |found: Option<&i32>| found.map(i32::to_string).unwrap_or_default();
    CollectionData {
        title: summary.title.as_str().into(),
        overview: details.overview.clone().unwrap_or_default().into(),
        count: i32::try_from(details.items.len()).unwrap_or(i32::MAX),
        all_movies: details
            .items
            .iter()
            .all(|item| item.kind == ItemKind::Movie),
        first_year: year(years.iter().min()).into(),
        last_year: year(years.iter().max()).into(),
        watched_count: i32::try_from(details.watched).unwrap_or(i32::MAX),
        watched: summary.user.played,
        favorite: summary.user.favorite,
        play_title: play_target(details)
            .map(|item| item.title.as_str())
            .unwrap_or_default()
            .into(),
    }
}

/// State of the collection page: the box set shown and the answers still expected.
#[derive(Default)]
pub(crate) struct CollectionController {
    instance: Rc<Cell<u64>>,
    load: Cell<u64>,
    current: RefCell<Option<String>>,
    details: RefCell<Option<CollectionDetails>>,
    cards: Rc<VecModel<CardData>>,
    images_requested: Cell<bool>,
}

impl CollectionController {
    /// A screen was shown: a collection page loads when it is not the box set already shown.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: &Screen) {
        let Screen::Collection(id) = screen else {
            return;
        };
        if self.current.borrow().as_deref() == Some(id.as_str()) {
            return;
        }
        self.clear(app);
        *self.current.borrow_mut() = Some(id.clone());
        self.request(app);
    }

    /// A screen left the stack: the collection page frees its content.
    pub(crate) fn discard(&self, app: &App, screen: &Screen) {
        if matches!(screen, Screen::Collection(id) if self.current.borrow().as_deref() == Some(id.as_str()))
        {
            self.clear(app);
        }
    }

    /// The page came back from the player or from a changed item: the watched state may differ.
    pub(crate) fn refresh(&self, app: &Rc<App>) {
        if self.current.borrow().is_some() {
            self.request(app);
        }
    }

    /// Asks again after the box set failed to load.
    pub(crate) fn retry(&self, app: &Rc<App>) {
        self.refresh(app);
    }

    /// Plays the first item not watched yet, resuming it when it was started.
    pub(crate) fn play(&self, app: &Rc<App>) {
        let id = self
            .details
            .borrow()
            .as_ref()
            .and_then(play_target)
            .map(|item| item.id.clone());
        if let Some(id) = id {
            app.play(id, PlayMode::Auto);
        }
    }

    /// Opens the detail page of the item `id` of the row.
    pub(crate) fn open(&self, app: &Rc<App>, id: &str) {
        let target = self
            .details
            .borrow()
            .as_ref()
            .and_then(|details| target_of(&details.items, id));
        if let Some(screen) = target {
            open_detail(app, screen);
        }
    }

    /// Marks the box set watched or unwatched.
    pub(crate) fn toggle_watched(&self, app: &Rc<App>) {
        self.toggle(app, Flag::Watched);
    }

    /// Adds the box set to the favorites or removes it.
    pub(crate) fn toggle_favorite(&self, app: &Rc<App>) {
        self.toggle(app, Flag::Favorite);
    }

    fn clear(&self, app: &App) {
        self.instance.set(self.instance.get() + 1);
        self.load.set(self.load.get() + 1);
        self.current.borrow_mut().take();
        self.details.borrow_mut().take();
        self.cards.set_vec(Vec::new());
        self.images_requested.set(false);
        if let Some(window) = app.window() {
            window.set_collection_loaded(false);
            window.set_collection_loading(false);
            window.set_collection_error(SharedString::default());
            window.set_collection_busy(false);
            window.set_collection_data(CollectionData::default());
            window.set_collection_items(ModelRc::default());
            window.set_collection_poster(Default::default());
            window.set_collection_backdrop(Default::default());
        }
    }

    fn request(&self, app: &Rc<App>) {
        let (Some(collection), Some((id, catalog, _))) =
            (self.current.borrow().clone(), app.services())
        else {
            return;
        };
        self.load.set(self.load.get() + 1);
        let load = self.load.get();
        if let Some(window) = app.window() {
            window.set_collection_error(SharedString::default());
            window.set_collection_loading(self.details.borrow().is_none());
        }
        app.spawn(
            async move { catalog.collection(&ItemId::from(collection.as_str())).await },
            move |app, result| app.collection.answered(app, load, id, result),
        );
    }

    fn answered(
        &self,
        app: &Rc<App>,
        load: u64,
        id: GenerationId,
        result: Result<CollectionDetails, CoreError>,
    ) {
        if self.load.get() != load || app.services().is_none_or(|(current, ..)| current != id) {
            return;
        }
        let Some(window) = app.window() else { return };
        window.set_collection_loading(false);
        match result {
            Ok(details) => {
                *self.details.borrow_mut() = Some(details);
                self.publish(app, id, &window);
            }
            Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot load the collection: {error}");
                if self.details.borrow().is_none() {
                    let (message, detail) = core_error_message(&error);
                    let text = window
                        .global::<Messages>()
                        .invoke_message_text(message, detail);
                    window.set_collection_error(text);
                }
            }
        }
    }

    fn publish(&self, app: &Rc<App>, id: GenerationId, window: &AppWindow) {
        let details = self.details.borrow();
        let Some(details) = details.as_ref() else {
            return;
        };
        window.set_collection_data(collection_data(details));
        let previous: Vec<CardData> = self.cards.iter().collect();
        let (cards, missing) = cards_with_posters(&details.items, &previous);
        self.cards.set_vec(cards);
        window.set_collection_items(ModelRc::from(Rc::clone(&self.cards)));
        window.set_collection_loaded(true);
        load_posters(app, id, window, &self.cards, &missing);
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
                    window.set_collection_poster(image);
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
                    window.set_collection_backdrop(image);
                }
            });
        }
    }

    fn toggle(&self, app: &Rc<App>, flag: Flag) {
        let Some(window) = app.window() else { return };
        let Some((id, catalog, _)) = app.services() else {
            return;
        };
        if window.get_collection_busy() {
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
        window.set_collection_busy(true);
        let instance = self.instance.get();
        app.spawn(
            async move {
                match flag {
                    Flag::Watched => catalog.set_played(&item, target).await,
                    Flag::Favorite => catalog.set_favorite(&item, target).await,
                }
            },
            move |app, result| app.collection.toggled(app, instance, id, result),
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
        window.set_collection_busy(false);
        match result {
            Ok(state) => {
                if let Some(details) = self.details.borrow_mut().as_mut() {
                    details.summary.user = state;
                }
                self.publish(app, id, &window);
                // Marking a box set watched changes its items on the server.
                self.request(app);
                if let Some(collection) = self.current.borrow().clone() {
                    app.dispatch(Msg::ItemChanged(Screen::Collection(collection)));
                }
            }
            Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot change the collection: {error}");
                app.notify(core_error_message(&error).0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, year: Option<i32>, played: bool) -> ItemSummary {
        ItemSummary {
            id: id.into(),
            kind: ItemKind::Movie,
            title: format!("Title {id}"),
            year,
            series_id: None,
            series_name: None,
            episode_label: None,
            poster: None,
            backdrop: None,
            thumb: None,
            duration: None,
            user: UserState {
                played,
                ..UserState::default()
            },
        }
    }

    fn details(items: Vec<ItemSummary>) -> CollectionDetails {
        let watched = items.iter().filter(|item| item.user.played).count();
        let first_unwatched = items
            .iter()
            .find(|item| !item.user.played)
            .map(|item| item.id.clone());
        CollectionDetails {
            summary: ItemSummary {
                kind: ItemKind::BoxSet,
                title: "Northbound Collection".into(),
                ..item("box", None, false)
            },
            overview: None,
            items,
            watched,
            first_unwatched,
        }
    }

    #[test]
    fn play_starts_the_first_unwatched_item() {
        let details = details(vec![
            item("a", Some(2019), true),
            item("b", Some(2023), false),
            item("c", Some(2024), false),
        ]);
        assert_eq!(
            play_target(&details).map(|item| item.id.as_str()),
            Some("b")
        );
        let data = collection_data(&details);
        assert_eq!(data.play_title, "Title b");
        assert_eq!(data.count, 3);
        assert!(data.all_movies);
        assert_eq!(
            (data.first_year.as_str(), data.last_year.as_str()),
            ("2019", "2024")
        );
        assert_eq!(data.watched_count, 1);
    }

    #[test]
    fn play_falls_back_to_the_first_item_once_all_are_watched() {
        let watched = details(vec![item("a", None, true), item("b", None, true)]);
        assert_eq!(
            play_target(&watched).map(|item| item.id.as_str()),
            Some("a")
        );
        let empty = details(Vec::new());
        assert!(play_target(&empty).is_none());
        let data = collection_data(&empty);
        assert_eq!(data.play_title, "");
        assert_eq!(data.first_year, "");
    }

    #[test]
    fn mixed_items_are_not_all_movies() {
        let mut series = item("s", Some(2020), false);
        series.kind = ItemKind::Series;
        let data = collection_data(&details(vec![item("a", Some(2020), false), series]));
        assert!(!data.all_movies);
        assert_eq!(data.first_year, data.last_year);
    }
}
