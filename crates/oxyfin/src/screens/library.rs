//! Library pages (movies, shows, favorites): filters, sort, genre and paged loading.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use oxyfin_api::ItemId;
use oxyfin_core::{
    BrowseQuery, BrowseSource, ContentKind, CoreError, ItemSummary, Page, SortField, UserState,
    WatchFilter,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::images::{PageLoads, load_into};
use crate::present::{card, core_error_message, detail_target, image_size};
use crate::rows;
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

/// The first page of `tab` with the settings it opens with.
pub(crate) fn first_page(tab: LibraryTab) -> BrowseQuery {
    query(tab, LibrarySettings::default(), 0, PAGE_SIZE)
}

fn query(tab: LibraryTab, settings: LibrarySettings, start: u32, limit: u32) -> BrowseQuery {
    BrowseQuery {
        source: source_of(tab),
        sort: settings.sort,
        descending: settings.descending,
        genre: settings.genre,
        watch: settings.watch,
        start,
        limit,
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
    // An answer waits until nothing points at what it moves.
    held_back: Cell<bool>,
    // The end of the grid came near while a request was on its way.
    more_wanted: Cell<bool>,
    // First item and number of items of the last request.
    asked: Cell<(u32, u32)>,
    ready: Cell<bool>,
    more_failed: Cell<bool>,
    loads: PageLoads,
}

impl LibraryController {
    /// A screen was shown: a library page loads when it is not the one already loaded.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: &Screen) {
        let Screen::Library(tab) = screen else {
            // A page opened over the grid gets the download slots its posters would wait for.
            if self.tab.get().is_some() && *screen != Screen::Player {
                self.loads.abort();
            }
            return;
        };
        if self.tab.get() == Some(*tab) {
            self.load_missing_posters(app);
            if self.ready.get() {
                app.prefetch.idle(app);
            }
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
        self.open_first_page(app);
    }

    /// Shows the first page of the settings as the server last answered it, if it did, and asks
    /// for it anew.
    fn open_first_page(&self, app: &Rc<App>) {
        let (Some(tab), Some((id, catalog, _)), Some(window)) =
            (self.tab.get(), app.services(), app.window())
        else {
            return;
        };
        let last = catalog.last_browse(&self.query(tab, 0, PAGE_SIZE));
        let Some(page) = last else {
            return self.request(app, 0, PAGE_SIZE, Ask::First);
        };
        self.loads.abort();
        self.show_page(app, id, &window, 0, page);
        self.request(app, 0, PAGE_SIZE, Ask::Behind);
    }

    /// Loads again the posters of the cards still without one, after their loads were aborted.
    fn load_missing_posters(&self, app: &Rc<App>) {
        let (Some((id, ..)), Some(window)) = (app.services(), app.window()) else {
            return;
        };
        let missing: Vec<ItemSummary> = self
            .items
            .borrow()
            .iter()
            .filter(|item| {
                !self
                    .cards
                    .iter()
                    .any(|card| card.has_image && card.id == item.id.as_str())
            })
            .cloned()
            .collect();
        load_posters(app, id, &window, &self.cards, &missing, &self.loads);
    }

    /// The page came back from the player or holds stale data: every page loaded is asked again
    /// in one request, and the grid keeps its place; an answer that moves cards waits for the
    /// pointer, and a failure keeps the grid.
    pub(crate) fn refresh(&self, app: &Rc<App>) {
        let shown = !self.items.borrow().is_empty();
        self.ask_loaded(app, if shown { Ask::Behind } else { Ask::Again });
    }

    /// Asks again for every page loaded, in one request.
    fn ask_loaded(&self, app: &Rc<App>, ask: Ask) {
        if self.tab.get().is_some() {
            let loaded = u32::try_from(self.items.borrow().len()).unwrap_or(u32::MAX);
            self.request(app, 0, loaded.max(PAGE_SIZE), ask);
        }
    }

    /// Item `id` changed to `user`: its card shows it at once, and the pages are asked again only
    /// when the change can take the card out of the grid.
    pub(crate) fn patch(&self, app: &Rc<App>, id: &ItemId, user: &UserState) {
        let Some(tab) = self.tab.get() else { return };
        patch_card(&self.items, &self.cards, id, user);
        if self.settings(tab).watch != WatchFilter::All || tab == LibraryTab::Favorites {
            // The user's own change: the card leaves at once.
            self.ask_loaded(app, Ask::Again);
        } else if self.in_flight.get() {
            // The answer on its way may hold the item as it was: it is asked for again.
            let (start, limit) = self.asked.get();
            self.request(app, start, limit, Ask::Again);
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

    /// The card of item `id` was clicked.
    pub(crate) fn open(&self, app: &Rc<App>, id: &str) {
        let target = target_of(&self.items.borrow(), id);
        if let Some(screen) = target {
            open_detail(app, screen);
        }
    }

    /// The end of the grid is near: asks for the next page when there is one.
    pub(crate) fn near_end(&self, app: &Rc<App>) {
        let loaded = self.items.borrow().len();
        if self.held_back.get() && loaded < self.total.get() as usize {
            // Scrolling moves the cards anyway, and the next page follows that answer.
            self.more_wanted.set(true);
            return app.run_waiting();
        }
        self.more_wanted.set(self.in_flight.get());
        if self.tab.get().is_some()
            && !self.more_failed.get()
            && should_load_more(
                loaded,
                self.total.get(),
                self.in_flight.get(),
                self.ready.get(),
            )
        {
            self.request(app, loaded as u32, PAGE_SIZE, Ask::Again);
        }
    }

    /// Asks again after the first page failed.
    pub(crate) fn retry(&self, app: &Rc<App>) {
        if self.tab.get().is_some() {
            self.load_genres(app);
            self.request(app, 0, PAGE_SIZE, Ask::First);
        }
    }

    /// Asks again for the page after the last one loaded, which failed.
    pub(crate) fn retry_more(&self, app: &Rc<App>) {
        if self.tab.get().is_some() {
            let loaded = self.items.borrow().len() as u32;
            self.request(app, loaded, PAGE_SIZE, Ask::Again);
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
        self.open_first_page(app);
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

    /// The interface language changed: the page drops its text, and loads again once shown.
    pub(crate) fn forget(&self, app: &App) {
        self.clear(app);
    }

    fn clear(&self, app: &App) {
        self.load.set(self.load.get() + 1);
        self.epoch.set(self.epoch.get() + 1);
        self.tab.set(None);
        self.in_flight.set(false);
        self.more_wanted.set(false);
        self.held_back.set(false);
        self.more_failed.set(false);
        self.ready.set(false);
        self.total.set(0);
        self.loads.abort();
        self.items.borrow_mut().clear();
        self.cards.set_vec(Vec::new());
        self.genres.borrow_mut().clear();
        self.options.borrow_mut().clear();
        self.genre_names.set_vec(Vec::new());
        if let Some(window) = app.window() {
            window.set_library_viewport_y(0.0);
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
        if let Some(names) = catalog.last_genres(&source) {
            self.genres_loaded(app, names);
        }
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
        if *self.genres.borrow() == names {
            return;
        }
        *self.genres.borrow_mut() = names;
        self.publish(&window);
    }

    fn query(&self, tab: LibraryTab, start: u32, limit: u32) -> BrowseQuery {
        query(tab, self.settings(tab), start, limit)
    }

    /// Requests `limit` items starting at `start`. A first page replaces the content.
    fn request(&self, app: &Rc<App>, start: u32, limit: u32, ask: Ask) {
        let (Some(tab), Some((id, catalog, _))) = (self.tab.get(), app.services()) else {
            return;
        };
        self.load.set(self.load.get() + 1);
        let load = self.load.get();
        self.in_flight.set(true);
        self.held_back.set(false);
        self.asked.set((start, limit));
        self.more_failed.set(false);
        if start == 0 && ask == Ask::First {
            self.ready.set(false);
            self.total.set(0);
            self.loads.abort();
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
        let query = self.query(tab, start, limit);
        app.spawn(
            async move { catalog.browse(&query).await },
            move |app, result| app.library.answered(app, load, id, start, ask, result),
        );
    }

    fn answered(
        &self,
        app: &Rc<App>,
        load: u64,
        id: GenerationId,
        start: u32,
        ask: Ask,
        result: Result<Page<ItemSummary>, CoreError>,
    ) {
        if !self.current(app, load, id) {
            return;
        }
        let Some(window) = app.window() else { return };
        if let Ok(page) = &result
            && ask == Ask::Behind
            && !self.holds_items(&page.items)
        {
            let page = page.clone();
            self.held_back.set(true);
            // Still in flight: a change made meanwhile asks for it again.
            return app.when_free(move |app| {
                let controller = &app.library;
                if let Some(window) = app.window()
                    && controller.current(app, load, id)
                {
                    controller.held_back.set(false);
                    controller.in_flight.set(false);
                    controller.show_page(app, id, &window, start, page);
                    controller.resume_more(app);
                }
            });
        }
        self.in_flight.set(false);
        window.set_library_loading(false);
        window.set_library_loading_more(false);
        let page = match result {
            Ok(page) => page,
            Err(CoreError::SessionExpired) => return app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot load the library page: {error}");
                if ask == Ask::Behind {
                    app.notify(core_error_message(&error).0);
                    self.resume_more(app);
                } else if start == 0 {
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
        self.show_page(app, id, &window, start, page);
        self.resume_more(app);
    }

    fn current(&self, app: &App, load: u64, id: GenerationId) -> bool {
        self.load.get() == load && app.services().is_some_and(|(current, ..)| current == id)
    }

    /// Whether the grid shows `items`, in that order.
    fn holds_items(&self, items: &[ItemSummary]) -> bool {
        let shown = self.items.borrow();
        shown.len() == items.len() && shown.iter().zip(items).all(|(old, new)| old.id == new.id)
    }

    fn show_page(
        &self,
        app: &Rc<App>,
        id: GenerationId,
        window: &AppWindow,
        start: u32,
        page: Page<ItemSummary>,
    ) {
        self.ready.set(true);
        self.total.set(page.total);
        window.set_library_total(i32::try_from(page.total).unwrap_or(i32::MAX));
        let missing = add_page(&self.items, &self.cards, start, page.items);
        load_posters(app, id, window, &self.cards, &missing, &self.loads);
    }

    /// Asks for the next page when the end of the grid came near during a request.
    fn resume_more(&self, app: &Rc<App>) {
        app.prefetch.idle(app);
        if self.more_wanted.take() {
            self.near_end(app);
        }
    }
}

/// How a page of a library is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    /// The grid is emptied at once.
    First,
    /// The grid stays until the answer comes.
    Again,
    /// The grid shows the last answer; a new one waits until nothing points at what it moves.
    Behind,
}

/// Adds the items of a page starting at `start` to a grid; a first page replaces the grid,
/// keeping the cards that stay the same. Returns the items whose poster has to be loaded.
pub(crate) fn add_page(
    items: &RefCell<Vec<ItemSummary>>,
    cards: &VecModel<CardData>,
    start: u32,
    page: Vec<ItemSummary>,
) -> Vec<ItemSummary> {
    if start != 0 {
        let (new_cards, missing) = cards_with_posters(&page, &[]);
        items.borrow_mut().extend(page);
        for data in new_cards {
            cards.push(data);
        }
        return missing;
    }
    let previous: Vec<CardData> = cards.iter().collect();
    let (new_cards, missing) = cards_with_posters(&page, &previous);
    *items.borrow_mut() = page;
    rows::update(cards, new_cards);
    missing
}

/// Shows `user` on the card of item `id`, keeping its poster.
pub(crate) fn patch_card(
    items: &RefCell<Vec<ItemSummary>>,
    cards: &VecModel<CardData>,
    id: &ItemId,
    user: &UserState,
) {
    let mut items = items.borrow_mut();
    let Some(item) = items.iter_mut().find(|item| item.id == *id) else {
        return;
    };
    item.user = user.clone();
    let Some(index) = cards.iter().position(|card| card.id == item.id.as_str()) else {
        return;
    };
    if let Some(old) = cards.row_data(index) {
        let mut data = card(item);
        data.image = old.image;
        data.has_image = old.has_image;
        cards.set_row_data(index, data);
    }
}

/// Detail page opened by the item `id` among `items`.
pub(crate) fn target_of(items: &[ItemSummary], id: &str) -> Option<Screen> {
    items
        .iter()
        .find(|item| item.id.as_str() == id)
        .and_then(detail_target)
}

/// Opens `screen` on top of the current one.
pub(crate) fn open_detail(app: &Rc<App>, screen: Screen) {
    app.dispatch(Msg::OpenItem(screen));
}

/// Cards for `items`, with the poster already loaded in `previous` for the same item.
/// Also returns the items whose poster still has to be loaded.
pub(crate) fn cards_with_posters(
    items: &[ItemSummary],
    previous: &[CardData],
) -> (Vec<CardData>, Vec<ItemSummary>) {
    let loaded: HashMap<&SharedString, &CardData> = previous
        .iter()
        .filter(|card| card.has_image)
        .map(|card| (&card.id, card))
        .collect();
    let mut missing = Vec::new();
    let cards = items
        .iter()
        .map(|item| {
            let mut data = card(item);
            match loaded.get(&data.id) {
                Some(kept) => {
                    data.image = kept.image.clone();
                    data.has_image = true;
                }
                None => missing.push(item.clone()),
            }
            data
        })
        .collect();
    (cards, missing)
}

/// Loads the posters of `items` into the matching cards of `cards`, as loads of the page `loads`.
pub(crate) fn load_posters(
    app: &Rc<App>,
    id: GenerationId,
    window: &AppWindow,
    cards: &Rc<VecModel<CardData>>,
    items: &[ItemSummary],
    loads: &PageLoads,
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
        loads.once(format!("{item_id} {}", poster.tag), || {
            load_into(app, id, &images, poster, size, move |image| {
                let Some(index) = cards.iter().position(|card| card.id == item_id) else {
                    return;
                };
                if let Some(mut data) = cards.row_data(index) {
                    data.image = image;
                    data.has_image = true;
                    cards.set_row_data(index, data);
                }
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyfin_core::{ItemKind, UserState};

    fn item(id: &str) -> ItemSummary {
        ItemSummary {
            id: id.into(),
            kind: ItemKind::Movie,
            title: id.into(),
            year: None,
            series_id: None,
            series_name: None,
            episode_label: None,
            poster: None,
            backdrop: None,
            thumb: None,
            duration: None,
            overview: None,
            user: UserState::default(),
        }
    }

    #[test]
    fn a_reload_keeps_the_posters_already_loaded() {
        let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(2, 2);
        let mut loaded = card(&item("a"));
        loaded.image = slint::Image::from_rgba8(pixels);
        loaded.has_image = true;
        let previous = [loaded, card(&item("b"))];
        let items = [item("a"), item("b"), item("c")];
        let (cards, missing) = cards_with_posters(&items, &previous);
        assert_eq!(
            cards.iter().map(|c| c.has_image).collect::<Vec<_>>(),
            [true, false, false]
        );
        assert_eq!(
            missing.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["b", "c"]
        );
    }

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
