//! Home screen: banner and rows, each loaded on its own, with their images.

use oxyfin_api::ItemId;
use oxyfin_core::{Catalog, CoreError, ImageRef, ItemSummary, Library};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::app::App;
use crate::images::load_into;
use crate::present::{card, core_error_message, image_size};
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

type CoreResult<T> = Result<T, CoreError>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum RowKey {
    ContinueWatching,
    NextUp,
    Library(ItemId),
}

struct Slot {
    key: RowKey,
    cards: Rc<VecModel<CardData>>,
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
    shown_generation: Cell<Option<GenerationId>>,
    rows: Rc<VecModel<HomeRow>>,
    slots: RefCell<Vec<Slot>>,
    layout: RefCell<Vec<RowKey>>,
    featured: RefCell<Featured>,
}

impl HomeController {
    /// A screen was shown: load the home screen when it is the one.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: Screen) {
        self.load.set(self.load.get() + 1);
        match screen {
            Screen::Home => self.reload(app),
            Screen::SignIn | Screen::QuickConnect => self.clear(app),
            _ => {}
        }
    }

    fn clear(&self, app: &App) {
        self.shown_generation.set(None);
        self.slots.borrow_mut().clear();
        self.rows.set_vec(Vec::new());
        *self.layout.borrow_mut() = Vec::new();
        *self.featured.borrow_mut() = Featured::default();
        if let Some(window) = app.window() {
            window.set_home_banner(HomeBanner::default());
        }
    }

    fn reload(&self, app: &Rc<App>) {
        let Some((id, catalog, _)) = app.services() else {
            return;
        };
        if self.shown_generation.get() != Some(id) {
            self.clear(app);
            self.shown_generation.set(Some(id));
        }
        if let Some(window) = app.window() {
            window.set_home_rows(ModelRc::from(Rc::clone(&self.rows)));
        }
        let load = self.load.get();
        *self.featured.borrow_mut() = Featured {
            pending: 2,
            ..Featured::default()
        };

        let source = catalog.clone();
        app.spawn(
            async move { source.continue_watching(CONTINUE_WATCHING_LIMIT).await },
            move |app, result| {
                if !app.home.current(app, load, id) {
                    return;
                }
                let items = result.as_ref().ok().cloned().unwrap_or_default();
                app.home
                    .row_loaded(app, id, RowKey::ContinueWatching, result);
                app.home
                    .settle(app, load, id, |featured| featured.continue_watching = items);
            },
        );

        let source = catalog.clone();
        app.spawn(
            async move { source.next_up(NEXT_UP_LIMIT).await },
            move |app, result| {
                if app.home.current(app, load, id) {
                    app.home.row_loaded(app, id, RowKey::NextUp, result);
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
                if app.home.current(app, load, id) {
                    app.home.libraries_loaded(app, load, id, catalog, result);
                }
            },
        );
    }

    fn current(&self, app: &App, load: u64, id: GenerationId) -> bool {
        self.load.get() == load && app.services().is_some_and(|(current, ..)| current == id)
    }

    fn libraries_loaded(
        &self,
        app: &Rc<App>,
        load: u64,
        id: GenerationId,
        catalog: Catalog,
        result: CoreResult<Vec<Library>>,
    ) {
        let libraries = match result {
            Ok(libraries) => libraries,
            Err(CoreError::SessionExpired) => return app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot list the libraries: {error}");
                Vec::new()
            }
        };
        let mut layout = vec![RowKey::ContinueWatching, RowKey::NextUp];
        layout.extend(
            libraries
                .iter()
                .map(|library| RowKey::Library(library.id.clone())),
        );
        let stale: Vec<RowKey> = self
            .slots
            .borrow()
            .iter()
            .map(|slot| slot.key.clone())
            .filter(|key| !layout.contains(key))
            .collect();
        *self.layout.borrow_mut() = layout;
        for key in stale {
            self.upsert(key, None);
        }

        self.settle(app, load, id, |featured| {
            featured.pending += libraries.len();
            featured.latest = vec![Vec::new(); libraries.len()];
        });
        for (index, library) in libraries.into_iter().enumerate() {
            let source = catalog.clone();
            let library_id = library.id.clone();
            app.spawn(
                async move { source.latest(&library_id, LATEST_LIMIT).await },
                move |app, result| {
                    if !app.home.current(app, load, id) {
                        return;
                    }
                    let items = result.as_ref().ok().cloned().unwrap_or_default();
                    let key = RowKey::Library(library.id.clone());
                    app.home
                        .row_loaded_titled(app, id, key, &library.name, result);
                    app.home.settle(app, load, id, |featured| {
                        if let Some(slot) = featured.latest.get_mut(index) {
                            *slot = items;
                        }
                    });
                },
            );
        }
    }

    fn row_loaded(
        &self,
        app: &Rc<App>,
        id: GenerationId,
        key: RowKey,
        result: CoreResult<Vec<ItemSummary>>,
    ) {
        self.row_loaded_titled(app, id, key, "", result);
    }

    fn row_loaded_titled(
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
                let row = HomeRow {
                    title,
                    kind,
                    items: ModelRc::default(),
                    error: text,
                };
                return self.upsert(key, Some((row, Rc::new(VecModel::default()))));
            }
        };

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
                    None => missing.push(item),
                }
                data
            })
            .collect();
        let cards = Rc::new(VecModel::from(cards));
        let row = HomeRow {
            title: title.clone(),
            kind,
            items: ModelRc::from(Rc::clone(&cards)),
            error: SharedString::default(),
        };
        self.upsert(key, Some((row, cards)));

        let Some((_, _, images)) = app.services() else {
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
            load_into(&images, app.runtime(), artwork, size, move |image| {
                if let Some(window) = target.upgrade() {
                    set_card_image(&window, &row_title, &item_id, image);
                }
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

    fn upsert(&self, key: RowKey, row: Option<(HomeRow, Rc<VecModel<CardData>>)>) {
        let mut slots = self.slots.borrow_mut();
        let existing = slots.iter().position(|slot| slot.key == key);
        match (existing, row) {
            (Some(index), Some((row, cards))) => {
                slots[index].cards = cards;
                self.rows.set_row_data(index, row);
            }
            (Some(index), None) => {
                slots.remove(index);
                self.rows.remove(index);
            }
            (None, Some((row, cards))) => {
                let rank = self.rank(&key);
                let index = slots
                    .iter()
                    .take_while(|slot| self.rank(&slot.key) <= rank)
                    .count();
                slots.insert(index, Slot { key, cards });
                self.rows.insert(index, row);
            }
            (None, None) => {}
        }
    }

    fn settle(
        &self,
        app: &Rc<App>,
        load: u64,
        id: GenerationId,
        update: impl FnOnce(&mut Featured),
    ) {
        if !self.current(app, load, id) {
            return;
        }
        let done = {
            let mut featured = self.featured.borrow_mut();
            update(&mut featured);
            featured.pending = featured.pending.saturating_sub(1);
            featured.pending == 0
        };
        if done {
            self.show_banner(app);
        }
    }

    fn show_banner(&self, app: &Rc<App>) {
        let Some(window) = app.window() else { return };
        let item = {
            let featured = self.featured.borrow();
            let latest: Vec<ItemSummary> = featured.latest.iter().flatten().cloned().collect();
            Catalog::featured(&featured.continue_watching, &latest)
        };
        let Some(item) = item else {
            window.set_home_banner(HomeBanner::default());
            return;
        };
        let previous = window.get_home_banner();
        let mut data = card(&item);
        let kept = previous.has && previous.card.id == data.id && previous.card.has_image;
        if kept {
            data.image = previous.card.image;
            data.has_image = true;
        }
        window.set_home_banner(HomeBanner {
            has: true,
            title: item.title.as_str().into(),
            year: item.year.unwrap_or(0),
            duration_minutes: item.duration.map_or(0, |duration| {
                i32::try_from(duration.as_secs() / 60).unwrap_or(0)
            }),
            card: data,
        });
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
        let Some((_, _, images)) = app.services() else {
            return;
        };
        let scale = window.window().scale_factor();
        let width = window.window().size().width as f32 / scale;
        let size = image_size(width, BANNER_HEIGHT, scale);
        let target = window.as_weak();
        load_into(&images, app.runtime(), artwork, size, move |image| {
            let Some(window) = target.upgrade() else {
                return;
            };
            let mut banner = window.get_home_banner();
            if banner.has && banner.card.id == item_id {
                banner.card.image = image;
                banner.card.has_image = true;
                window.set_home_banner(banner);
            }
        });
    }
}

fn banner_artwork(item: &ItemSummary) -> Option<ImageRef> {
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
