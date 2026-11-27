//! Home screen: banner and rows, each loaded on its own, with their images.

use oxyfin_api::ItemId;
use oxyfin_core::{Catalog, CoreError, ImageRef, ItemSummary, Library};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::app::App;
use crate::images::{PageLoads, load_artwork, load_into};
use crate::present::{card, core_error_message, detail_target, home_banner, image_size};
use crate::rows;
use crate::screens::library::open_detail;
use crate::state::{GenerationId, Msg, Screen};
use crate::{AppWindow, CardData, HomeBanner, HomeRow, HomeRowKind, HomeTitles, Messages};

/// Items asked for the "Continue Watching" row.
pub const CONTINUE_WATCHING_LIMIT: u32 = 12;
/// Items asked for the "Next Up" row.
pub const NEXT_UP_LIMIT: u32 = 12;
/// Items asked for each "Recently Added" row.
pub const LATEST_LIMIT: u32 = 16;

// Logical sizes of the artworks, as drawn by `LandscapeCard`, `PosterCard` and `Banner`.
const LANDSCAPE_ARTWORK: (f32, f32) = (288.0, 162.0);
const POSTER_ARTWORK: (f32, f32) = (160.0, 240.0);
const BANNER_HEIGHT: f32 = 520.0;
const BACKDROP_ASPECT: f32 = 16.0 / 9.0;

type CoreResult<T> = Result<T, CoreError>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum RowKey {
    ContinueWatching,
    NextUp,
    Library(ItemId),
}

impl RowKey {
    /// What tells the row from the others in the interface, whatever the language.
    fn name(&self) -> SharedString {
        match self {
            Self::ContinueWatching => "continue-watching".into(),
            Self::NextUp => "next-up".into(),
            Self::Library(id) => id.as_str().into(),
        }
    }
}

struct Slot {
    key: RowKey,
    /// Items of the row, none when it shows an error.
    items: Vec<ItemSummary>,
    cards: Rc<VecModel<CardData>>,
}

/// What a row shows.
struct RowContent {
    title: SharedString,
    kind: HomeRowKind,
    resume: bool,
    error: SharedString,
    items: Vec<ItemSummary>,
    cards: Vec<CardData>,
}

/// A load of the home screen.
#[derive(Clone, Copy)]
struct Load {
    number: u64,
    id: GenerationId,
    /// The screen shows the last answers: what moves them waits until nothing points at them.
    behind: bool,
}

/// What the banner is picked from, once every source has answered.
#[derive(Default)]
struct Featured {
    pending: usize,
    continue_watching: Vec<ItemSummary>,
    latest: Vec<Vec<ItemSummary>>,
}

/// State of the home screen.
#[derive(Default)]
pub(crate) struct HomeController {
    load: Cell<u64>,
    loaded: Cell<bool>,
    shown_generation: Cell<Option<GenerationId>>,
    rows: Rc<VecModel<HomeRow>>,
    slots: RefCell<Vec<Slot>>,
    layout: RefCell<Vec<RowKey>>,
    featured: RefCell<Featured>,
    targets: RefCell<HashMap<String, Screen>>,
    loads: PageLoads,
}

impl HomeController {
    /// A screen was shown: the home screen loads the first time it is.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: &Screen) {
        if *screen != Screen::Home {
            return;
        }
        if !self.loaded.get() {
            self.reload(app, false);
        } else if app.window().is_some_and(|window| window.get_home_settled()) {
            app.prefetch.idle(app);
        }
    }

    /// The home screen came back from the player or holds stale data: what it shows stays
    /// until the new answers come, and those that move rows wait for the pointer.
    pub(crate) fn refresh(&self, app: &Rc<App>) {
        self.reload(app, false);
    }

    /// The user changed an item from the home screen: the new answers show at once.
    pub(crate) fn changed(&self, app: &Rc<App>) {
        self.reload(app, true);
    }

    /// The poster or banner of item `id` was opened.
    pub(crate) fn open(&self, app: &Rc<App>, id: &str) {
        let target = self.targets.borrow().get(id).cloned();
        if let Some(screen) = target {
            open_detail(app, screen);
        }
    }

    /// A screen left the stack: the home screen frees its content.
    pub(crate) fn discard(&self, app: &Rc<App>, screen: &Screen) {
        if *screen == Screen::Home {
            self.clear(app);
        }
    }

    fn clear(&self, app: &App) {
        self.loads.abort();
        self.load.set(self.load.get() + 1);
        self.loaded.set(false);
        self.shown_generation.set(None);
        self.slots.borrow_mut().clear();
        self.rows.set_vec(Vec::new());
        *self.layout.borrow_mut() = Vec::new();
        self.targets.borrow_mut().clear();
        *self.featured.borrow_mut() = Featured::default();
        if let Some(window) = app.window() {
            window.set_home_banner(HomeBanner::default());
            window.set_home_settled(false);
            window.set_home_loading(false);
        }
    }

    fn reload(&self, app: &Rc<App>, at_once: bool) {
        let Some((id, catalog, _)) = app.services() else {
            return;
        };
        self.load.set(self.load.get() + 1);
        if self.shown_generation.get() != Some(id) {
            self.clear(app);
            self.shown_generation.set(Some(id));
        }
        let first = !self.loaded.replace(true);
        if let Some(window) = app.window() {
            window.set_home_rows(ModelRc::from(Rc::clone(&self.rows)));
            window.set_home_settled(false);
        }
        let behind = if first {
            self.show_last(app, id, &catalog)
        } else {
            !at_once
        };
        if let Some(window) = app.window()
            && first
        {
            window.set_home_loading(!behind);
        }
        let load = Load {
            number: self.load.get(),
            id,
            behind,
        };
        *self.featured.borrow_mut() = Featured {
            pending: 2,
            ..Featured::default()
        };

        let source = catalog.clone();
        app.spawn(
            async move { source.continue_watching(CONTINUE_WATCHING_LIMIT).await },
            move |app, result| {
                if !app.home.current(app, load) {
                    return;
                }
                let key = RowKey::ContinueWatching;
                let items = app.home.answered_items(&key, &result);
                app.home.row_answered(app, load, key, "", result);
                app.home
                    .settle(app, load, |featured| featured.continue_watching = items);
            },
        );

        let source = catalog.clone();
        app.spawn(
            async move { source.next_up(NEXT_UP_LIMIT).await },
            move |app, result| {
                if app.home.current(app, load) {
                    app.home.row_answered(app, load, RowKey::NextUp, "", result);
                }
            },
        );

        let source = catalog;
        app.spawn(
            async move {
                let libraries = source.libraries().await;
                (source, libraries)
            },
            move |app, (catalog, result)| {
                if app.home.current(app, load) {
                    app.home.libraries_loaded(app, load, catalog, result);
                }
            },
        );
    }

    /// Shows the last answers of the server, when they make the whole screen; tells whether they
    /// do.
    fn show_last(&self, app: &Rc<App>, id: GenerationId, catalog: &Catalog) -> bool {
        let (Some(resume), Some(next_up), Some(libraries)) = (
            catalog.last_continue_watching(CONTINUE_WATCHING_LIMIT),
            catalog.last_next_up(NEXT_UP_LIMIT),
            catalog.last_libraries(),
        ) else {
            return false;
        };
        let latest: Option<Vec<Vec<ItemSummary>>> = libraries
            .iter()
            .map(|library| catalog.last_latest(&library.id, LATEST_LIMIT))
            .collect();
        let Some(latest) = latest else { return false };
        self.set_layout(&libraries);
        *self.featured.borrow_mut() = Featured {
            pending: 0,
            continue_watching: resume.clone(),
            latest: latest.clone(),
        };
        self.show_banner(app);
        self.show_row(app, id, RowKey::ContinueWatching, "", Ok(resume));
        self.show_row(app, id, RowKey::NextUp, "", Ok(next_up));
        for (library, items) in libraries.into_iter().zip(latest) {
            let key = RowKey::Library(library.id);
            self.show_row(app, id, key, &library.name, Ok(items));
        }
        true
    }

    fn set_layout(&self, libraries: &[Library]) {
        let mut layout = vec![RowKey::ContinueWatching, RowKey::NextUp];
        layout.extend(
            libraries
                .iter()
                .map(|library| RowKey::Library(library.id.clone())),
        );
        *self.layout.borrow_mut() = layout;
    }

    fn current(&self, app: &App, load: Load) -> bool {
        self.load.get() == load.number
            && app
                .services()
                .is_some_and(|(current, ..)| current == load.id)
    }

    /// Items of the row `key`, none without that row.
    fn items(&self, key: &RowKey) -> Vec<ItemSummary> {
        self.slots
            .borrow()
            .iter()
            .find(|slot| slot.key == *key)
            .map(|slot| slot.items.clone())
            .unwrap_or_default()
    }

    /// Items the banner picks from: those answered, or still shown when the answer failed.
    fn answered_items(
        &self,
        key: &RowKey,
        result: &CoreResult<Vec<ItemSummary>>,
    ) -> Vec<ItemSummary> {
        match result {
            Ok(items) => items.clone(),
            Err(_) => self.items(key),
        }
    }

    /// Shows the answer for a row. Behind the last answers, an error keeps the row and is told,
    /// and other items wait until nothing points at what they move.
    fn row_answered(
        &self,
        app: &Rc<App>,
        load: Load,
        key: RowKey,
        library_name: &str,
        result: CoreResult<Vec<ItemSummary>>,
    ) {
        if !load.behind {
            return self.show_row(app, load.id, key, library_name, result);
        }
        let items = match result {
            Ok(items) => items,
            Err(CoreError::SessionExpired) => return app.dispatch(Msg::Expired(load.id)),
            Err(error) => {
                log::warn!("cannot load the home row {key:?}: {error}");
                return app.notify(core_error_message(&error).0);
            }
        };
        let shown = self.items(&key);
        if shown == items {
            return;
        }
        let same = shown.len() == items.len()
            && shown.iter().zip(&items).all(|(old, new)| old.id == new.id);
        if same {
            return self.show_row(app, load.id, key, library_name, Ok(items));
        }
        let library_name = library_name.to_owned();
        app.when_free(move |app| {
            if app.home.current(app, load) {
                app.home
                    .show_row(app, load.id, key, &library_name, Ok(items));
            }
        });
    }

    fn libraries_loaded(
        &self,
        app: &Rc<App>,
        load: Load,
        catalog: Catalog,
        result: CoreResult<Vec<Library>>,
    ) {
        let libraries = match result {
            Ok(libraries) => libraries,
            Err(CoreError::SessionExpired) => return app.dispatch(Msg::Expired(load.id)),
            Err(error) => {
                log::warn!("cannot list the libraries: {error}");
                if load.behind {
                    app.notify(core_error_message(&error).0);
                    let layout = self.layout.borrow().clone();
                    let latest = layout
                        .iter()
                        .filter(|key| matches!(key, RowKey::Library(_)))
                        .map(|key| self.items(key))
                        .collect();
                    return self.settle(app, load, |featured| featured.latest = latest);
                }
                Vec::new()
            }
        };
        self.set_layout(&libraries);
        let stale: Vec<RowKey> = {
            let layout = self.layout.borrow();
            self.slots
                .borrow()
                .iter()
                .map(|slot| slot.key.clone())
                .filter(|key| !layout.contains(key))
                .collect()
        };
        if !stale.is_empty() {
            let remove = move |app: &Rc<App>| {
                if app.home.current(app, load) {
                    for key in stale {
                        app.home.upsert(key, None);
                    }
                }
            };
            if load.behind {
                app.when_free(remove);
            } else {
                remove(app);
            }
        }

        self.settle(app, load, |featured| {
            featured.pending += libraries.len();
            featured.latest = vec![Vec::new(); libraries.len()];
        });
        for (index, library) in libraries.into_iter().enumerate() {
            let source = catalog.clone();
            let library_id = library.id.clone();
            app.spawn(
                async move { source.latest(&library_id, LATEST_LIMIT).await },
                move |app, result| {
                    if !app.home.current(app, load) {
                        return;
                    }
                    let key = RowKey::Library(library.id.clone());
                    let items = app.home.answered_items(&key, &result);
                    app.home.row_answered(app, load, key, &library.name, result);
                    app.home.settle(app, load, |featured| {
                        if let Some(slot) = featured.latest.get_mut(index) {
                            *slot = items;
                        }
                    });
                },
            );
        }
    }

    fn show_row(
        &self,
        app: &Rc<App>,
        id: GenerationId,
        key: RowKey,
        library_name: &str,
        result: CoreResult<Vec<ItemSummary>>,
    ) {
        if let Err(CoreError::SessionExpired) = result {
            return app.dispatch(Msg::Expired(id));
        }
        let Some(window) = app.window() else { return };
        let titles = window.global::<HomeTitles>();
        let resume = key == RowKey::ContinueWatching;
        let (title, kind) = match &key {
            RowKey::ContinueWatching => (titles.invoke_continue_watching(), HomeRowKind::Landscape),
            RowKey::NextUp => (titles.invoke_next_up(), HomeRowKind::Landscape),
            RowKey::Library(_) => (
                titles.invoke_recently_added(library_name.into()),
                HomeRowKind::Poster,
            ),
        };
        let items = match result {
            Ok(items) if items.is_empty() => return self.upsert(key, None),
            Ok(items) => items,
            Err(error) => {
                log::warn!("cannot load the home row {key:?}: {error}");
                let (message, detail) = core_error_message(&error);
                let text = window
                    .global::<Messages>()
                    .invoke_message_text(message, detail);
                let row = RowContent {
                    title,
                    kind,
                    resume,
                    error: text,
                    items: Vec::new(),
                    cards: Vec::new(),
                };
                return self.upsert(key, Some(row));
            }
        };

        if matches!(key, RowKey::Library(_)) {
            let mut targets = self.targets.borrow_mut();
            for item in &items {
                if let Some(screen) = detail_target(item) {
                    targets.insert(item.id.as_str().to_owned(), screen);
                }
            }
        }
        let kept = self.loaded_images(&key);
        let mut missing = Vec::new();
        let cards: Vec<CardData> = items
            .iter()
            .map(|item| {
                let mut data = card(item);
                match kept.get(&data.id) {
                    Some(image) => {
                        data.image = image.clone();
                        data.has_image = true;
                    }
                    None => missing.push(item.clone()),
                }
                data
            })
            .collect();
        let row = RowContent {
            title: title.clone(),
            kind,
            resume,
            error: SharedString::default(),
            items,
            cards,
        };
        self.upsert(key, Some(row));

        let Some((generation, _, images)) = app.services() else {
            return;
        };
        let (width, height) = match kind {
            HomeRowKind::Landscape => LANDSCAPE_ARTWORK,
            HomeRowKind::Poster => POSTER_ARTWORK,
        };
        let size = image_size(width, height, window.window().scale_factor());
        for item in missing {
            let artwork = match kind {
                HomeRowKind::Landscape => item.thumb.as_ref().or(item.backdrop.as_ref()),
                HomeRowKind::Poster => item.poster.as_ref(),
            };
            let Some(artwork) = artwork.cloned() else {
                continue;
            };
            let target = window.as_weak();
            let row_title = title.clone();
            let item_id: SharedString = item.id.as_str().into();
            let load = format!("{row_title} {item_id} {}", artwork.tag);
            self.loads.once(load, || {
                load_into(app, generation, &images, artwork, size, move |image| {
                    if let Some(window) = target.upgrade() {
                        set_card_image(&window, &row_title, &item_id, image);
                    }
                })
            });
        }
    }

    fn loaded_images(&self, key: &RowKey) -> HashMap<SharedString, slint::Image> {
        let slots = self.slots.borrow();
        let Some(slot) = slots.iter().find(|slot| slot.key == *key) else {
            return HashMap::new();
        };
        slot.cards
            .iter()
            .filter(|card| card.has_image)
            .map(|card| (card.id, card.image))
            .collect()
    }

    fn rank(&self, key: &RowKey) -> usize {
        self.layout
            .borrow()
            .iter()
            .position(|known| known == key)
            .or(match key {
                RowKey::ContinueWatching => Some(0),
                RowKey::NextUp => Some(1),
                RowKey::Library(_) => None,
            })
            .unwrap_or(usize::MAX)
    }

    fn upsert(&self, key: RowKey, row: Option<RowContent>) {
        let mut slots = self.slots.borrow_mut();
        let existing = slots.iter().position(|slot| slot.key == key);
        match (existing, row) {
            (Some(index), Some(content)) => {
                let slot = &mut slots[index];
                slot.items = content.items;
                rows::update(&slot.cards, content.cards);
                let row = HomeRow {
                    key: key.name(),
                    title: content.title,
                    kind: content.kind,
                    items: ModelRc::from(Rc::clone(&slot.cards)),
                    error: content.error,
                    resume: content.resume,
                };
                // The row keeps its cards unless what stands around them changes.
                if self.rows.row_data(index).as_ref() != Some(&row) {
                    self.rows.set_row_data(index, row);
                }
            }
            (Some(index), None) => {
                slots.remove(index);
                self.rows.remove(index);
            }
            (None, Some(content)) => {
                let cards = Rc::new(VecModel::from(content.cards));
                let row = HomeRow {
                    key: key.name(),
                    title: content.title,
                    kind: content.kind,
                    items: ModelRc::from(Rc::clone(&cards)),
                    error: content.error,
                    resume: content.resume,
                };
                let rank = self.rank(&key);
                let index = slots
                    .iter()
                    .take_while(|slot| self.rank(&slot.key) <= rank)
                    .count();
                let slot = Slot {
                    key,
                    items: content.items,
                    cards,
                };
                slots.insert(index, slot);
                self.rows.insert(index, row);
            }
            (None, None) => {}
        }
    }

    fn settle(&self, app: &Rc<App>, load: Load, update: impl FnOnce(&mut Featured)) {
        if !self.current(app, load) {
            return;
        }
        let done = {
            let mut featured = self.featured.borrow_mut();
            update(&mut featured);
            featured.pending = featured.pending.saturating_sub(1);
            featured.pending == 0
        };
        if done {
            let shown = app.window().map(|window| window.get_home_banner());
            let picked = self.picked().map(|(item, _)| item.id);
            let same = shown.is_some_and(|shown| {
                picked.as_ref().map(ItemId::as_str) == shown.has.then_some(shown.card.id.as_str())
            });
            if load.behind && !same {
                app.when_free(move |app| {
                    if app.home.current(app, load) {
                        app.home.show_banner(app);
                    }
                });
            } else {
                self.show_banner(app);
            }
            if let Some(window) = app.window() {
                window.set_home_settled(true);
                window.set_home_loading(false);
            }
            app.prefetch.idle(app);
        }
    }

    /// The item of the banner, and whether it is the first one to resume.
    fn picked(&self) -> Option<(ItemSummary, bool)> {
        let featured = self.featured.borrow();
        let latest: Vec<ItemSummary> = featured.latest.iter().flatten().cloned().collect();
        let item = Catalog::featured(&featured.continue_watching, &latest)?;
        let resuming = featured
            .continue_watching
            .first()
            .is_some_and(|first| first.id == item.id);
        Some((item, resuming))
    }

    fn show_banner(&self, app: &Rc<App>) {
        let Some(window) = app.window() else { return };
        let Some((item, resuming)) = self.picked() else {
            window.set_home_banner(HomeBanner::default());
            return;
        };
        let target = detail_target(&item);
        let has_detail = target.is_some();
        if let Some(screen) = target {
            self.targets
                .borrow_mut()
                .insert(item.id.as_str().to_owned(), screen);
        }
        let previous = window.get_home_banner();
        let mut banner = home_banner(&item, resuming, has_detail);
        let kept = previous.has && previous.card.id == banner.card.id && previous.card.has_image;
        if kept {
            banner.card.image = previous.card.image.clone();
            banner.card.has_image = true;
        }
        if banner != previous {
            window.set_home_banner(banner);
        }
        if !kept && let Some(backdrop) = banner_artwork(&item) {
            self.load_banner_image(app, &window, backdrop, item.id.as_str().into());
        }
    }

    fn load_banner_image(
        &self,
        app: &App,
        window: &AppWindow,
        artwork: ImageRef,
        item_id: SharedString,
    ) {
        let Some((generation, _, images)) = app.services() else {
            return;
        };
        let scale = window.window().scale_factor();
        let width = window.window().size().width as f32 / scale;
        let (width, height) = banner_box(width);
        let size = image_size(width, height, scale);
        let target = window.as_weak();
        let load = format!("banner {item_id} {}", artwork.tag);
        self.loads.once(load, || {
            load_artwork(app, generation, &images, artwork, size, move |image| {
                let Some(window) = target.upgrade() else {
                    return;
                };
                let mut banner = window.get_home_banner();
                if banner.has && banner.card.id == item_id {
                    banner.card.image = image;
                    banner.card.has_image = true;
                    window.set_home_banner(banner);
                }
            })
        });
    }
}

// Jellyfin fits images inside the box, so ask for a 16:9 box covering the banner.
pub(crate) fn banner_box(width: f32) -> (f32, f32) {
    (
        width.max(BANNER_HEIGHT * BACKDROP_ASPECT),
        BANNER_HEIGHT.max(width / BACKDROP_ASPECT),
    )
}

pub(crate) fn banner_artwork(item: &ItemSummary) -> Option<ImageRef> {
    item.backdrop.clone().or_else(|| item.thumb.clone())
}

fn set_card_image(window: &AppWindow, row_title: &str, item_id: &str, image: slint::Image) {
    let rows = window.get_home_rows();
    let Some(row) = rows.iter().find(|row| row.title == row_title) else {
        return;
    };
    let Some(index) = row.items.iter().position(|card| card.id == item_id) else {
        return;
    };
    if let Some(mut data) = row.items.row_data(index) {
        data.image = image;
        data.has_image = true;
        row.items.set_row_data(index, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_box_covers_a_wide_window() {
        assert_eq!(banner_box(1440.0), (1440.0, 810.0));
    }

    #[test]
    fn banner_box_covers_a_narrow_window() {
        let (width, height) = banner_box(800.0);
        assert!((width - 520.0 * 16.0 / 9.0).abs() < 0.01, "{width}");
        assert_eq!(height, 520.0);
    }
}
