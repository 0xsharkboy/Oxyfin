//! The "More like this" row of the detail pages.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use oxyfin_api::ItemId;
use oxyfin_core::{CoreError, ItemSummary};
use slint::{Model, ModelRc, VecModel};

use crate::app::App;
use crate::images::PageLoads;
use crate::rows;
use crate::screens::library::{cards_with_posters, load_posters, open_detail, target_of};
use crate::state::{GenerationId, Msg};
use crate::{AppWindow, CardData};

/// Number of similar items asked for.
const LIMIT: u32 = 12;

/// Window property a page shows its row in.
pub(crate) type Publish = fn(&AppWindow, ModelRc<CardData>);

/// A row kept with its page for Back.
pub(crate) struct ParkedRow {
    items: Vec<ItemSummary>,
    cards: Rc<VecModel<CardData>>,
    answered: bool,
}

/// Items similar to the item of a detail page, asked for once per page and again when the
/// page is refreshed, keeping the order shown.
#[derive(Default)]
pub(crate) struct SimilarRow {
    instance: Rc<Cell<u64>>,
    requested: Cell<bool>,
    answered: Rc<Cell<bool>>,
    items: Rc<RefCell<Vec<ItemSummary>>>,
    cards: RefCell<Rc<VecModel<CardData>>>,
    loads: Rc<PageLoads>,
}

impl SimilarRow {
    /// Forgets the row of the previous item.
    pub(crate) fn clear(&self, app: &App, publish: Publish) {
        self.loads.abort();
        self.instance.set(self.instance.get() + 1);
        self.requested.set(false);
        self.answered.set(false);
        self.items.borrow_mut().clear();
        *self.cards.borrow_mut() = Rc::default();
        if let Some(window) = app.window() {
            publish(&window, ModelRc::default());
        }
    }

    /// Hands the row over to be kept with its page; `clear` must follow.
    pub(crate) fn park(&self) -> ParkedRow {
        ParkedRow {
            items: self.items.take(),
            cards: Rc::clone(&self.cards.borrow()),
            answered: self.answered.get(),
        }
    }

    /// Shows a row kept with its page again, loading the posters it still misses. A row whose
    /// answer had not come yet is asked for again by the next `request`.
    pub(crate) fn restore(
        &self,
        app: &Rc<App>,
        id: GenerationId,
        window: &AppWindow,
        row: ParkedRow,
        publish: Publish,
    ) {
        self.answered.set(row.answered);
        self.requested.set(row.answered);
        let missing: Vec<ItemSummary> = row
            .items
            .iter()
            .filter(|item| {
                !row.cards
                    .iter()
                    .any(|card| card.has_image && card.id == item.id.as_str())
            })
            .cloned()
            .collect();
        *self.items.borrow_mut() = row.items;
        publish(window, ModelRc::from(Rc::clone(&row.cards)));
        load_posters(app, id, window, &row.cards, &missing, &self.loads);
        *self.cards.borrow_mut() = row.cards;
    }

    /// Lets the next `request` ask again, keeping the titles shown and their order.
    pub(crate) fn reload(&self) {
        self.requested.set(false);
    }

    /// Asks for the items similar to `item` unless already asked; a failure leaves the row as
    /// it was.
    pub(crate) fn request(&self, app: &Rc<App>, item: &str, publish: Publish) {
        if self.requested.replace(true) {
            return;
        }
        let Some((id, catalog, _)) = app.services() else {
            return;
        };
        let item = ItemId::from(item);
        let instance = self.instance.get();
        let cards = Rc::clone(&self.cards.borrow());
        let last = catalog
            .last_similar(&item, LIMIT)
            .filter(|_| !self.answered.get());
        // The server can list the same titles in another order each time.
        let kept = last.is_some() || self.answered.get();
        if let (Some(found), Some(window)) = (last, app.window()) {
            let (shown, missing) = cards_with_posters(&found, &[]);
            cards.set_vec(shown);
            *self.items.borrow_mut() = found;
            self.answered.set(true);
            publish(&window, ModelRc::from(Rc::clone(&cards)));
            load_posters(app, id, &window, &cards, &missing, &self.loads);
        }
        let (current, items, loads, answered) = (
            Rc::clone(&self.instance),
            Rc::clone(&self.items),
            Rc::clone(&self.loads),
            Rc::clone(&self.answered),
        );
        app.spawn(
            async move { catalog.similar(&item, LIMIT).await },
            move |app, result| {
                if current.get() != instance || app.services().is_none_or(|(now, ..)| now != id) {
                    return;
                }
                match result {
                    Ok(mut found) => {
                        let Some(window) = app.window() else { return };
                        if kept {
                            // The row shown stays for this visit, with the new state of its titles.
                            found = items
                                .borrow()
                                .iter()
                                .map(|shown| {
                                    found
                                        .iter()
                                        .find(|new| new.id == shown.id)
                                        .unwrap_or(shown)
                                        .clone()
                                })
                                .collect();
                        }
                        let previous: Vec<CardData> = cards.iter().collect();
                        let (shown, missing) = cards_with_posters(&found, &previous);
                        rows::update(&cards, shown);
                        *items.borrow_mut() = found;
                        answered.set(true);
                        publish(&window, ModelRc::from(Rc::clone(&cards)));
                        load_posters(app, id, &window, &cards, &missing, &loads);
                    }
                    Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
                    Err(error) => log::warn!("cannot load similar items: {error}"),
                }
            },
        );
    }

    /// Opens the detail page of the card `id`.
    pub(crate) fn open(&self, app: &Rc<App>, id: &str) {
        let target = target_of(&self.items.borrow(), id);
        if let Some(screen) = target {
            open_detail(app, screen);
        }
    }
}
