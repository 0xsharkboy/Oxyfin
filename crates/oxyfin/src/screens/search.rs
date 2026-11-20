//! Search page: the text field, debounced requests and paged results.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use oxyfin_core::{CoreError, ItemSummary, Page};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app::App;
use crate::images::PageLoads;
use crate::present::core_error_message;
use crate::screens::library::{
    PAGE_SIZE, cards_with_posters, load_posters, open_detail, should_load_more, target_of,
};
use crate::state::{GenerationId, Msg, Screen};
use crate::{CardData, Messages};

/// Shortest text that is searched for.
const MIN_CHARS: usize = 2;

/// Pause after the last keystroke before the search starts.
const TYPING_DELAY: Duration = Duration::from_millis(300);

/// The text to search for, or `None` when it is too short.
fn searchable(text: &str) -> Option<String> {
    let text = text.trim();
    (text.chars().count() >= MIN_CHARS).then(|| text.to_owned())
}

/// State of the search page: the text searched for and its paged results.
#[derive(Default)]
pub(crate) struct SearchController {
    load: Cell<u64>,
    active: Cell<bool>,
    query: RefCell<String>,
    items: RefCell<Vec<ItemSummary>>,
    cards: Rc<VecModel<CardData>>,
    total: Cell<u32>,
    in_flight: Cell<bool>,
    ready: Cell<bool>,
    more_failed: Cell<bool>,
    loads: PageLoads,
    timer: Timer,
}

impl SearchController {
    /// A screen was shown: the search page starts empty the first time.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: &Screen) {
        if *screen != Screen::Search || self.active.get() {
            return;
        }
        self.active.set(true);
        if let Some(window) = app.window() {
            window.set_search_cards(ModelRc::from(Rc::clone(&self.cards)));
        }
    }

    /// The page came back from the player: the first page of the current text loads again.
    pub(crate) fn refresh(&self, app: &Rc<App>) {
        let query = self.query.borrow().clone();
        if self.active.get() && self.ready.get() && !query.is_empty() {
            self.request(app, query, 0, false);
        }
    }

    /// A screen left the stack: the search page forgets its text and results.
    pub(crate) fn discard(&self, app: &App, screen: &Screen) {
        if *screen != Screen::Search {
            return;
        }
        self.active.set(false);
        self.reset(app);
        if let Some(window) = app.window() {
            window.set_search_text(SharedString::default());
        }
    }

    /// The text of the field changed.
    pub(crate) fn edited(&self, app: &Rc<App>, text: &str) {
        if !self.active.get() {
            return;
        }
        self.timer.stop();
        let Some(text) = searchable(text) else {
            return self.reset(app);
        };
        if *self.query.borrow() == text && self.ready.get() {
            return self.settle(app);
        }
        // An answer for the previous text must not land on the new one.
        self.load.set(self.load.get() + 1);
        self.in_flight.set(false);
        // Loading until the debounced request answers, so no stale "No results" shows.
        if let Some(window) = app.window() {
            window.set_search_loading(true);
            window.set_search_loading_more(false);
        }
        let weak = Rc::downgrade(app);
        self.timer
            .start(TimerMode::SingleShot, TYPING_DELAY, move || {
                if let Some(app) = weak.upgrade() {
                    app.search.run(&app, text.clone());
                }
            });
    }

    /// Enter was pressed in the field: the search starts at once.
    pub(crate) fn accepted(&self, app: &Rc<App>, text: &str) {
        if !self.active.get() {
            return;
        }
        self.timer.stop();
        match searchable(text) {
            Some(text) => self.run(app, text),
            None => self.reset(app),
        }
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
        if self.active.get()
            && !self.more_failed.get()
            && should_load_more(
                loaded,
                self.total.get(),
                self.in_flight.get(),
                self.ready.get(),
            )
        {
            let query = self.query.borrow().clone();
            self.request(app, query, loaded as u32, false);
        }
    }

    /// Asks again after the first page failed.
    pub(crate) fn retry(&self, app: &Rc<App>) {
        let query = self.query.borrow().clone();
        if self.active.get() && !query.is_empty() {
            self.request(app, query, 0, true);
        }
    }

    /// Asks again for the page after the last one loaded, which failed.
    pub(crate) fn retry_more(&self, app: &Rc<App>) {
        let query = self.query.borrow().clone();
        if self.active.get() && !query.is_empty() {
            self.request(app, query, self.items.borrow().len() as u32, false);
        }
    }

    fn run(&self, app: &Rc<App>, text: String) {
        let busy = self.ready.get() || self.in_flight.get();
        if *self.query.borrow() == text && busy {
            if self.ready.get() {
                self.settle(app);
            }
            return;
        }
        self.request(app, text, 0, true);
    }

    /// Nothing is sent for the text: the results on screen stay and no loading is shown.
    fn settle(&self, app: &App) {
        if let Some(window) = app.window() {
            window.set_search_loading(false);
            window.set_search_loading_more(false);
        }
    }

    /// Empties the results and the invite state.
    fn reset(&self, app: &App) {
        self.load.set(self.load.get() + 1);
        self.timer.stop();
        self.query.borrow_mut().clear();
        self.in_flight.set(false);
        self.ready.set(false);
        self.more_failed.set(false);
        self.total.set(0);
        self.loads.abort();
        self.items.borrow_mut().clear();
        self.cards.set_vec(Vec::new());
        if let Some(window) = app.window() {
            window.set_search_viewport_y(0.0);
            window.set_search_query(SharedString::default());
            window.set_search_total(0);
            window.set_search_loading(false);
            window.set_search_loading_more(false);
            window.set_search_more_failed(false);
            window.set_search_error(SharedString::default());
        }
    }

    /// Requests the page of `text` starting at `start`. A first page replaces the
    /// results; `clear` empties the grid at once instead of when the answer arrives.
    fn request(&self, app: &Rc<App>, text: String, start: u32, clear: bool) {
        let Some((id, catalog, _)) = app.services() else {
            return;
        };
        self.load.set(self.load.get() + 1);
        let load = self.load.get();
        self.in_flight.set(true);
        self.more_failed.set(false);
        if start == 0 && clear {
            *self.query.borrow_mut() = text.clone();
            self.ready.set(false);
            self.total.set(0);
            self.loads.abort();
            self.items.borrow_mut().clear();
            self.cards.set_vec(Vec::new());
            if let Some(window) = app.window() {
                window.set_search_viewport_y(0.0);
            }
        }
        if let Some(window) = app.window() {
            window.set_search_query(self.query.borrow().as_str().into());
            window.set_search_error(SharedString::default());
            window.set_search_more_failed(false);
            window.set_search_loading(start == 0 && self.items.borrow().is_empty());
            window.set_search_loading_more(start > 0);
        }
        app.spawn(
            async move { catalog.search(&text, start, PAGE_SIZE).await },
            move |app, result| app.search.answered(app, load, id, start, result),
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
        window.set_search_loading(false);
        window.set_search_loading_more(false);
        let page = match result {
            Ok(page) => page,
            Err(CoreError::SessionExpired) => return app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot search: {error}");
                if start == 0 {
                    let (message, detail) = core_error_message(&error);
                    let text = window
                        .global::<Messages>()
                        .invoke_message_text(message, detail);
                    window.set_search_error(text);
                } else {
                    self.more_failed.set(true);
                    window.set_search_more_failed(true);
                }
                return;
            }
        };
        let previous: Vec<CardData> = if start == 0 {
            self.cards.iter().collect()
        } else {
            Vec::new()
        };
        if start == 0 {
            self.items.borrow_mut().clear();
            self.cards.set_vec(Vec::new());
        }
        self.ready.set(true);
        self.total.set(page.total);
        window.set_search_total(i32::try_from(page.total).unwrap_or(i32::MAX));
        let (new_cards, missing) = cards_with_posters(&page.items, &previous);
        self.items.borrow_mut().extend(page.items.iter().cloned());
        for data in new_cards {
            self.cards.push(data);
        }
        load_posters(app, id, &window, &self.cards, &missing, &self.loads);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_or_blank_text_is_not_searched() {
        assert_eq!(searchable(""), None);
        assert_eq!(searchable("x"), None);
        assert_eq!(searchable("  x  "), None);
        assert_eq!(searchable("  te "), Some("te".into()));
        assert_eq!(searchable("tes"), Some("tes".into()));
        assert_eq!(searchable("éé"), Some("éé".into()));
    }
}
