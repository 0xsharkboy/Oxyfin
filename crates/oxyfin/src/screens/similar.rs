//! The "More like this" row of the detail pages.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use oxyfin_api::ItemId;
use oxyfin_core::{CoreError, ItemSummary};
use slint::{ModelRc, VecModel};

use crate::app::App;
use crate::screens::library::{cards_with_posters, load_posters, open_detail, target_of};
use crate::state::Msg;
use crate::{AppWindow, CardData};

/// Number of similar items asked for.
const LIMIT: u32 = 12;

/// Window property a page shows its row in.
pub(crate) type Publish = fn(&AppWindow, ModelRc<CardData>);

/// Items similar to the item of a detail page, asked for once per page.
#[derive(Default)]
pub(crate) struct SimilarRow {
    instance: Rc<Cell<u64>>,
    requested: Cell<bool>,
    items: Rc<RefCell<Vec<ItemSummary>>>,
}

impl SimilarRow {
    /// Forgets the row of the previous item.
    pub(crate) fn clear(&self, app: &App, publish: Publish) {
        self.instance.set(self.instance.get() + 1);
        self.requested.set(false);
        self.items.borrow_mut().clear();
        if let Some(window) = app.window() {
            publish(&window, ModelRc::default());
        }
    }

    /// Asks for the items similar to `item` unless already asked; a failure leaves the row hidden.
    pub(crate) fn request(&self, app: &Rc<App>, item: &str, publish: Publish) {
        if self.requested.replace(true) {
            return;
        }
        let Some((id, catalog, _)) = app.services() else {
            return;
        };
        let item = ItemId::from(item);
        let instance = self.instance.get();
        let (current, items) = (Rc::clone(&self.instance), Rc::clone(&self.items));
        app.spawn(
            async move { catalog.similar(&item, LIMIT).await },
            move |app, result| {
                if current.get() != instance || app.services().is_none_or(|(now, ..)| now != id) {
                    return;
                }
                match result {
                    Ok(found) => {
                        let Some(window) = app.window() else { return };
                        let (cards, missing) = cards_with_posters(&found, &[]);
                        let cards = Rc::new(VecModel::from(cards));
                        *items.borrow_mut() = found;
                        publish(&window, ModelRc::from(Rc::clone(&cards)));
                        load_posters(app, id, &window, &cards, &missing);
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
