//! Library pages (movies, shows, favorites): filters, sort, genre and paged loading.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use oxyfin_core::{
    BrowseQuery, BrowseSource, ContentKind, CoreError, ItemSummary, Page, SortField, WatchFilter,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::images::load_into;
use crate::present::{card, core_error_message, image_size};
use crate::state::{GenerationId, LibraryTab, Msg, Screen};
use crate::{AppWindow, CardData, LibraryTitles, Messages};

/// Items asked for each page of a library.
pub const PAGE_SIZE: u32 = 60;

// Logical size of the poster artwork, as drawn by `PosterCard`.
pub(crate) const POSTER_ARTWORK: (f32, f32) = (160.0, 240.0);

const WATCH_FILTERS: [WatchFilter; 4] = [
    WatchFilter::All,
    WatchFilter::Unplayed,
    WatchFilter::InProgress,
    WatchFilter::Played,
];
const SORT_FIELDS: [SortField; 3] = [SortField::DateAdded, SortField::Title, SortField::Year];

/// Sort, filters and genre of one library tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LibrarySettings {
    pub(crate) sort: SortField,
    pub(crate) descending: bool,
    pub(crate) watch: WatchFilter,
    pub(crate) genre: Option<String>,
}

impl Default for LibrarySettings {
    fn default() -> Self {
        Self {
            sort: SortField::DateAdded,
            descending: true,
            watch: WatchFilter::All,
            genre: None,
        }
    }
}

impl LibrarySettings {
    fn filtered(&self) -> bool {
        self.watch != WatchFilter::All || self.genre.is_some()
    }

    fn watch_index(&self) -> i32 {
        position(&WATCH_FILTERS, &self.watch)
    }

    fn sort_index(&self) -> i32 {
        position(&SORT_FIELDS, &self.sort)
    }
}

fn position<T: PartialEq>(all: &[T], value: &T) -> i32 {
    all.iter()
        .position(|candidate| candidate == value)
        .map_or(0, |index| index as i32)
}

fn source_of(tab: LibraryTab) -> BrowseSource {
    match tab {
        LibraryTab::Movies => BrowseSource::All(ContentKind::Movies),
        LibraryTab::Shows => BrowseSource::All(ContentKind::Shows),
        LibraryTab::Favorites => BrowseSource::Favorites,
    }
}

/// Whether the next page must be requested now.
pub(crate) fn should_load_more(loaded: usize, total: u32, in_flight: bool, ready: bool) -> bool {
    ready && !in_flight && loaded < total as usize
}

/// The settings after `update`, or `None` when it changed nothing.
fn updated(
    current: &LibrarySettings,
    update: impl FnOnce(&mut LibrarySettings),
) -> Option<LibrarySettings> {
    let mut settings = current.clone();
    update(&mut settings);
    (settings != *current).then_some(settings)
}

/// Genres offered by the dropdown: the server list, plus the saved genre when it is missing.
fn genre_options(server: &[String], saved: Option<&str>) -> Vec<String> {
    let mut options = server.to_vec();
    if let Some(saved) = saved
        && !options.iter().any(|name| name == saved)
    {
        options.push(saved.to_owned());
    }
    options
}

/// State of the library page of the current tab.
#[derive(Default)]
pub(crate) struct LibraryController {
    load: Cell<u64>,
    epoch: Cell<u64>,
    tab: Cell<Option<LibraryTab>>,
    settings: RefCell<HashMap<LibraryTab, LibrarySettings>>,
    items: RefCell<Vec<ItemSummary>>,
    cards: Rc<VecModel<CardData>>,
    genres: RefCell<Vec<String>>,
    options: RefCell<Vec<String>>,
    genre_names: Rc<VecModel<SharedString>>,
    total: Cell<u32>,
    in_flight: Cell<bool>,
    ready: Cell<bool>,
    more_failed: Cell<bool>,
}

impl LibraryController {
    /// A screen was shown: a library page loads when it is not the one already loaded.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: &Screen) {
        let Screen::Library(tab) = screen else { return };
        if self.tab.get() == Some(*tab) {
            return;
        }
        self.clear(app);
        self.tab.set(Some(*tab));
        if let Some(window) = app.window() {
            window.set_library_cards(ModelRc::from(Rc::clone(&self.cards)));
            window.set_library_genres(ModelRc::from(Rc::clone(&self.genre_names)));
            self.publish(&window);
        }
        self.load_genres(app);
        self.request(app, 0, true);
    }

    /// The page came back from the player.
    pub(crate) fn refresh(&self, app: &Rc<App>) {
        if self.tab.get().is_some() {
            self.request(app, 0, false);
        }
    }

    /// A screen left the stack: the library page frees its content.
    pub(crate) fn discard(&self, app: &Rc<App>, screen: &Screen) {
        if matches!(screen, Screen::Library(tab) if self.tab.get() == Some(*tab)) {
            self.clear(app);
        }
    }

    /// The generation ended: the settings of every tab are forgotten.
    pub(crate) fn end_generation(&self) {
        self.settings.borrow_mut().clear();
    }

    /// The watch filter pill at `index` was chosen.
    pub(crate) fn set_watch(&self, app: &Rc<App>, index: i32) {
        let Some(watch) = usize::try_from(index)
            .ok()
            .and_then(|index| WATCH_FILTERS.get(index))
        else {
            return;
        };
        self.change(app, |settings| settings.watch = *watch);
    }

    /// The genre at `index` of the genre list was chosen; 0 is all genres.
    pub(crate) fn set_genre(&self, app: &Rc<App>, index: i32) {
        let genre = match usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_sub(1))
        {
            None => None,
            Some(position) => match self.options.borrow().get(position) {
                Some(name) => Some(name.clone()),
                None => return,
            },
        };
        self.change(app, |settings| settings.genre = genre);
    }

    /// The sort field at `index` was chosen.
    pub(crate) fn set_sort(&self, app: &Rc<App>, index: i32) {
        let Some(sort) = usize::try_from(index)
            .ok()
            .and_then(|index| SORT_FIELDS.get(index))
        else {
            return;
        };
        self.change(app, |settings| settings.sort = *sort);
    }

    /// The sort direction was flipped.
    pub(crate) fn toggle_direction(&self, app: &Rc<App>) {
        self.change(app, |settings| settings.descending = !settings.descending);
    }

    /// The end of the grid is near: asks for the next page when there is one.
    pub(crate) fn near_end(&self, app: &Rc<App>) {
        let loaded = self.items.borrow().len();
        if self.tab.get().is_some()
            && !self.more_failed.get()
            && should_load_more(
                loaded,
                self.total.get(),
                self.in_flight.get(),
                self.ready.get(),
            )
        {
            self.request(app, loaded as u32, false);
        }
    }

    /// Asks again after the first page failed.
    pub(crate) fn retry(&self, app: &Rc<App>) {
        if self.tab.get().is_some() {
            self.load_genres(app);
            self.request(app, 0, true);
        }
    }

    /// Asks again for the page after the last one loaded, which failed.
    pub(crate) fn retry_more(&self, app: &Rc<App>) {
        if self.tab.get().is_some() {
            self.request(app, self.items.borrow().len() as u32, false);
        }
    }

    fn settings(&self, tab: LibraryTab) -> LibrarySettings {
        self.settings
            .borrow()
            .get(&tab)
            .cloned()
            .unwrap_or_default()
    }

    fn change(&self, app: &Rc<App>, update: impl FnOnce(&mut LibrarySettings)) {
        let Some(tab) = self.tab.get() else { return };
        let Some(settings) = updated(&self.settings(tab), update) else {
            return;
        };
        self.settings.borrow_mut().insert(tab, settings);
        if let Some(window) = app.window() {
            self.publish(&window);
        }
        self.request(app, 0, true);
    }

    fn publish(&self, window: &AppWindow) {
        let Some(tab) = self.tab.get() else { return };
        let settings = self.settings(tab);
        window.set_library_watch(settings.watch_index());
        window.set_library_sort_field(settings.sort_index());
        window.set_library_descending(settings.descending);
        window.set_library_filtered(settings.filtered());
        let options = genre_options(&self.genres.borrow(), settings.genre.as_deref());
        let genre = settings.genre.as_ref().and_then(|genre| {
            options
                .iter()
                .position(|name| name == genre)
                .map(|position| position as i32 + 1)
        });
        let all = window.global::<LibraryTitles>().invoke_all_genres();
        let shown: Vec<SharedString> = if options.is_empty() {
            Vec::new()
        } else {
            std::iter::once(all)
                .chain(options.iter().map(|name| name.as_str().into()))
                .collect()
        };
        *self.options.borrow_mut() = options;
        self.genre_names.set_vec(shown);
        window.set_library_genre(genre.unwrap_or(0));
    }

    fn clear(&self, app: &App) {
        self.load.set(self.load.get() + 1);
        self.epoch.set(self.epoch.get() + 1);
        self.tab.set(None);
        self.in_flight.set(false);
        self.more_failed.set(false);
        self.ready.set(false);
        self.total.set(0);
        self.items.borrow_mut().clear();
        self.cards.set_vec(Vec::new());
        self.genres.borrow_mut().clear();
        self.options.borrow_mut().clear();
        self.genre_names.set_vec(Vec::new());
        if let Some(window) = app.window() {
            window.set_library_more_failed(false);
            window.set_library_total(0);
            window.set_library_loading(false);
            window.set_library_loading_more(false);
            window.set_library_error(SharedString::default());
        }
    }

    fn load_genres(&self, app: &Rc<App>) {
        let (Some(tab), Some((id, catalog, _))) = (self.tab.get(), app.services()) else {
            return;
        };
        let epoch = self.epoch.get();
        let source = source_of(tab);
        app.spawn(
            async move { catalog.genres(&source).await },
            move |app, result| {
                let controller = &app.library;
                if controller.epoch.get() != epoch {
                    return;
                }
                match result {
                    Ok(names) => controller.genres_loaded(app, names),
                    Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
                    Err(error) => log::warn!("cannot list the genres: {error}"),
                }
            },
        );
    }

    fn genres_loaded(&self, app: &App, names: Vec<String>) {
        let Some(window) = app.window() else { return };
        *self.genres.borrow_mut() = names;
        self.publish(&window);
    }

    /// Requests the page starting at `start`. A first page replaces the content;
    /// `clear` empties the grid at once instead of when the answer arrives.
    fn request(&self, app: &Rc<App>, start: u32, clear: bool) {
        let (Some(tab), Some((id, catalog, _))) = (self.tab.get(), app.services()) else {
            return;
        };
        self.load.set(self.load.get() + 1);
        let load = self.load.get();
        self.in_flight.set(true);
        self.more_failed.set(false);
        if start == 0 && clear {
            self.ready.set(false);
            self.total.set(0);
            self.items.borrow_mut().clear();
            self.cards.set_vec(Vec::new());
        }
        if let Some(window) = app.window() {
            window.set_library_error(SharedString::default());
            window.set_library_more_failed(false);
            if start == 0 {
                window.set_library_loading(self.items.borrow().is_empty());
            } else {
                window.set_library_loading_more(true);
            }
        }
        let settings = self.settings(tab);
        let query = BrowseQuery {
            source: source_of(tab),
            sort: settings.sort,
            descending: settings.descending,
            genre: settings.genre,
            watch: settings.watch,
            start,
            limit: PAGE_SIZE,
        };
        app.spawn(
            async move { catalog.browse(&query).await },
            move |app, result| app.library.answered(app, load, id, start, result),
        );
    }

    fn answered(
        &self,
        app: &Rc<App>,
        load: u64,
        id: GenerationId,
        start: u32,
        result: Result<Page<ItemSummary>, CoreError>,
    ) {
        if self.load.get() != load || app.services().is_none_or(|(current, ..)| current != id) {
            return;
        }
        self.in_flight.set(false);
        let Some(window) = app.window() else { return };
        window.set_library_loading(false);
        window.set_library_loading_more(false);
        let page = match result {
            Ok(page) => page,
            Err(CoreError::SessionExpired) => return app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot load the library page: {error}");
                if start == 0 {
                    let (message, detail) = core_error_message(&error);
                    let text = window
                        .global::<Messages>()
                        .invoke_message_text(message, detail);
                    window.set_library_error(text);
                } else {
                    self.more_failed.set(true);
                    window.set_library_more_failed(true);
                }
                return;
            }
        };
        if start == 0 {
            self.items.borrow_mut().clear();
            self.cards.set_vec(Vec::new());
        }
        self.ready.set(true);
        self.total.set(page.total);
        window.set_library_total(i32::try_from(page.total).unwrap_or(i32::MAX));
        let new_cards: Vec<CardData> = page.items.iter().map(card).collect();
        self.items.borrow_mut().extend(page.items.iter().cloned());
        for data in new_cards {
            self.cards.push(data);
        }
        load_posters(app, id, &window, &self.cards, &page.items);
    }
}

/// Loads the posters of `items` into the matching cards of `cards`.
pub(crate) fn load_posters(
    app: &Rc<App>,
    id: GenerationId,
    window: &AppWindow,
    cards: &Rc<VecModel<CardData>>,
    items: &[ItemSummary],
) {
    let Some((_, _, images)) = app.services() else {
        return;
    };
    let size = image_size(
        POSTER_ARTWORK.0,
        POSTER_ARTWORK.1,
        window.window().scale_factor(),
    );
    for item in items {
        let Some(poster) = item.poster.clone() else {
            continue;
        };
        let cards = Rc::clone(cards);
        let item_id: SharedString = item.id.as_str().into();
        load_into(app, id, &images, poster, size, move |image| {
            let Some(index) = cards.iter().position(|card| card.id == item_id) else {
                return;
            };
            if let Some(mut data) = cards.row_data(index) {
                data.image = image;
                data.has_image = true;
                cards.set_row_data(index, data);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_page_is_asked_only_when_there_is_more_and_nothing_is_in_flight() {
        assert!(should_load_more(60, 130, false, true));
        assert!(!should_load_more(130, 130, false, true));
        assert!(!should_load_more(60, 130, true, true));
        assert!(!should_load_more(0, 130, false, false));
    }

    #[test]
    fn choosing_the_current_setting_changes_nothing() {
        let current = LibrarySettings::default();
        assert_eq!(updated(&current, |s| s.watch = WatchFilter::All), None);
        assert_eq!(updated(&current, |s| s.sort = SortField::DateAdded), None);
        assert_eq!(updated(&current, |s| s.genre = None), None);
        let changed = updated(&current, |s| s.watch = WatchFilter::Played);
        assert_eq!(changed.map(|s| s.watch), Some(WatchFilter::Played));
        let flipped = updated(&current, |s| s.descending = !s.descending);
        assert_eq!(flipped.map(|s| s.descending), Some(false));
    }

    #[test]
    fn a_saved_genre_stays_listed_when_the_server_list_lacks_it() {
        let server = vec!["Comedy".to_owned()];
        assert_eq!(genre_options(&server, None), ["Comedy"]);
        assert_eq!(genre_options(&server, Some("Comedy")), ["Comedy"]);
        assert_eq!(genre_options(&server, Some("Drama")), ["Comedy", "Drama"]);
        assert_eq!(genre_options(&[], Some("Drama")), ["Drama"]);
        assert!(genre_options(&[], None).is_empty());
    }

    #[test]
    fn default_settings_sort_by_date_added_descending() {
        let settings = LibrarySettings::default();
        assert_eq!(settings.sort, SortField::DateAdded);
        assert!(settings.descending);
        assert_eq!(settings.watch, WatchFilter::All);
        assert!(!settings.filtered());
    }

    #[test]
    fn indexes_follow_the_option_order_of_the_screen() {
        let settings = LibrarySettings {
            sort: SortField::Year,
            descending: false,
            watch: WatchFilter::Played,
            genre: Some("Drama".into()),
        };
        assert_eq!((settings.watch_index(), settings.sort_index()), (3, 2));
        assert!(settings.filtered());
    }

    #[test]
    fn each_tab_browses_its_own_source() {
        assert_eq!(
            source_of(LibraryTab::Movies),
            BrowseSource::All(ContentKind::Movies)
        );
        assert_eq!(
            source_of(LibraryTab::Shows),
            BrowseSource::All(ContentKind::Shows)
        );
        assert_eq!(source_of(LibraryTab::Favorites), BrowseSource::Favorites);
    }
}
