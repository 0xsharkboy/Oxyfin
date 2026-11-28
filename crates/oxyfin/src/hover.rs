//! Details of the hovered card, fetched once per item and kept for the session.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use oxyfin_api::ItemId;
use oxyfin_core::{CoreError, ItemBrief, ItemKind};
use slint::ComponentHandle;

use crate::app::App;
use crate::present::hover_details;
use crate::state::{GenerationId, Msg};
use crate::{AppWindow, HoverCard};

enum Entry {
    Pending,
    Ready(Box<ItemBrief>),
    // The server will not describe this item: it is not asked again.
    Unavailable,
}

/// Answers the hover card's requests, from the cache or with one fetch per item.
#[derive(Default)]
pub(crate) struct HoverController {
    cache: RefCell<Option<(GenerationId, HashMap<String, Entry>)>>,
}

impl HoverController {
    /// Hands the hover card the details of `id`, now when they are known, else once fetched.
    pub(crate) fn request(&self, app: &Rc<App>, id: &str, progress: f32) {
        let Some(window) = app.window() else { return };
        let Some((generation, catalog, _)) = app.services() else {
            return;
        };
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|(kept, _)| *kept != generation) {
            *cache = Some((generation, HashMap::new()));
        }
        let Some((_, entries)) = cache.as_mut() else {
            return;
        };
        match entries.get(id) {
            Some(Entry::Ready(brief)) => {
                show(&window, brief, progress);
                return;
            }
            Some(Entry::Pending | Entry::Unavailable) => return,
            None => {}
        }
        entries.insert(id.to_owned(), Entry::Pending);
        drop(cache);
        let item = ItemId::from(id);
        let asked = item.clone();
        app.spawn(
            async move { catalog.item_brief(&asked).await },
            move |app, result| app.hover.answered(app, generation, &item, result),
        );
    }

    fn answered(
        &self,
        app: &Rc<App>,
        generation: GenerationId,
        item: &ItemId,
        result: Result<ItemBrief, CoreError>,
    ) {
        let mut cache = self.cache.borrow_mut();
        let Some((_, entries)) = cache.as_mut().filter(|(kept, _)| *kept == generation) else {
            return;
        };
        let id = item.as_str();
        match result {
            Ok(brief) => {
                if brief.summary.kind == ItemKind::Series {
                    app.prefetch.series_hovered(app, item);
                }
                if let Some(window) = app.window() {
                    let card = window.global::<HoverCard>();
                    if card.get_id() == id {
                        show(&window, &brief, card.get_progress());
                    }
                }
                entries.insert(id.to_owned(), Entry::Ready(Box::new(brief)));
            }
            Err(CoreError::SessionExpired) => {
                entries.remove(id);
                drop(cache);
                app.dispatch(Msg::Expired(generation));
            }
            Err(error @ CoreError::Decode(_)) => {
                log::warn!("cannot describe item {id}: {error}");
                entries.insert(id.to_owned(), Entry::Unavailable);
            }
            Err(error) => {
                log::warn!("cannot fetch the details of item {id}: {error}");
                // A later hover tries again.
                entries.remove(id);
            }
        }
    }
}

fn show(window: &AppWindow, brief: &ItemBrief, progress: f32) {
    window
        .global::<HoverCard>()
        .set_details(hover_details(brief, progress));
}
