//! What the user is likely to open next, asked for in the background once the page shown has
//! loaded: the home screen, the first pages of the libraries, the series whose card is hovered.
//!
//! Everything goes one request at a time and stops when another page shows; nothing runs while a
//! video plays.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use oxyfin_api::ItemId;
use oxyfin_core::{
    BACKGROUND_FETCHES, BrowseQuery, Catalog, CoreError, ImageCache, ImagePriority, ImageSize,
    ItemSummary,
};
use slint::ComponentHandle;
use tokio::task::{AbortHandle, JoinSet};

use crate::app::App;
use crate::present::image_size;
use crate::screens::home::{CONTINUE_WATCHING_LIMIT, LATEST_LIMIT, NEXT_UP_LIMIT};
use crate::screens::library::{POSTER_ARTWORK, first_page};
use crate::state::{LibraryTab, Screen};

/// How long the page shown has been loaded before anything else is asked for.
pub const IDLE_DELAY: Duration = Duration::from_millis(500);

/// How long an answer is not asked for again in the background.
pub const FRESH: Duration = Duration::from_secs(5 * 60);

/// Posters of a first page loaded in the background: those a large window shows at once.
pub const POSTERS: usize = 32;

/// The background requests running.
pub(crate) struct Prefetcher {
    enabled: Cell<bool>,
    delay: Cell<Duration>,
    idle: RefCell<Option<AbortHandle>>,
    hovered: RefCell<Option<AbortHandle>>,
}

impl Default for Prefetcher {
    fn default() -> Self {
        Self {
            enabled: Cell::new(true),
            delay: Cell::new(IDLE_DELAY),
            idle: RefCell::default(),
            hovered: RefCell::default(),
        }
    }
}

impl Prefetcher {
    /// Turns the background requests on or off.
    pub(crate) fn enable(&self, on: bool) {
        self.enabled.set(on);
    }

    /// Sets how long the page shown has been loaded before anything else is asked for.
    pub(crate) fn set_delay(&self, delay: Duration) {
        self.delay.set(delay);
    }

    /// Whether something is asked for in the background.
    pub(crate) fn busy(&self) -> bool {
        [&self.idle, &self.hovered].iter().any(|task| {
            task.borrow()
                .as_ref()
                .is_some_and(|task| !task.is_finished())
        })
    }

    /// Another page shows: everything stops.
    pub(crate) fn stop(&self) {
        for task in [&self.idle, &self.hovered] {
            if let Some(task) = task.borrow_mut().take() {
                task.abort();
            }
        }
    }

    /// The page shown has loaded: asks for what is likely to be opened next, unless that is
    /// running or was answered a short while ago.
    pub(crate) fn idle(&self, app: &Rc<App>) {
        let mut running = self.idle.borrow_mut();
        if running.as_ref().is_some_and(|task| !task.is_finished()) || !self.allowed(app) {
            return;
        }
        let (Some((_, catalog, images)), Some(window)) = (app.services(), app.window()) else {
            return;
        };
        let (width, height) = POSTER_ARTWORK;
        let size = image_size(width, height, window.window().scale_factor());
        let delay = self.delay.get();
        *running = Some(app.spawn(
            async move {
                tokio::time::sleep(delay).await;
                if let Err(error) = likely_next(&catalog, &images, size).await {
                    log::debug!("background loading stopped: {error}");
                }
            },
            |_, ()| {},
        ));
    }

    /// The card of series `id` is hovered: asks for its page.
    pub(crate) fn series_hovered(&self, app: &Rc<App>, id: &ItemId) {
        let Some((_, catalog, _)) = app.services() else {
            return;
        };
        if !self.allowed(app) || catalog.last_series(id).is_some() {
            return;
        }
        let id = id.clone();
        let task = app.spawn(async move { catalog.series(&id).await }, |_, result| {
            if let Err(error) = result {
                log::debug!("cannot load the hovered series: {error}");
            }
        });
        if let Some(previous) = self.hovered.borrow_mut().replace(task) {
            previous.abort();
        }
    }

    fn allowed(&self, app: &App) -> bool {
        self.enabled.get() && !matches!(app.screen(), Screen::Player | Screen::Settings)
    }
}

async fn likely_next(
    catalog: &Catalog,
    images: &Arc<ImageCache>,
    size: ImageSize,
) -> Result<(), CoreError> {
    if !catalog.home_within(FRESH, CONTINUE_WATCHING_LIMIT, NEXT_UP_LIMIT, LATEST_LIMIT) {
        catalog.continue_watching(CONTINUE_WATCHING_LIMIT).await?;
        catalog.next_up(NEXT_UP_LIMIT).await?;
        for library in catalog.libraries().await? {
            catalog.latest(&library.id, LATEST_LIMIT).await?;
        }
    }
    for tab in [LibraryTab::Movies, LibraryTab::Shows] {
        let query: BrowseQuery = first_page(tab);
        if catalog.browsed_within(&query, FRESH) {
            continue;
        }
        let page = catalog.browse(&query).await?;
        catalog.genres(&query.source).await?;
        load_posters(images, size, &page.items).await;
    }
    Ok(())
}

/// Puts the first posters of `items` in the image cache, a few at a time.
async fn load_posters(images: &Arc<ImageCache>, size: ImageSize, items: &[ItemSummary]) {
    let mut loads = JoinSet::new();
    for poster in items
        .iter()
        .take(POSTERS)
        .filter_map(|item| item.poster.clone())
    {
        if loads.len() >= BACKGROUND_FETCHES {
            loads.join_next().await;
        }
        let images = Arc::clone(images);
        loads.spawn(async move {
            let _ = images.get(&poster, size, ImagePriority::BACKGROUND).await;
        });
    }
    while loads.join_next().await.is_some() {}
}
