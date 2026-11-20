//! Person page: portrait, birth and biography of a person of the cast and crew, and the
//! titles of the library crediting them.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use oxyfin_api::ItemId;
use oxyfin_core::{CoreError, ItemKind, PersonDetails, PersonTitle};
use slint::{ComponentHandle, FilterModel, Model, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::images::load_into;
use crate::present::{core_error_message, date_parts, image_size, initials, person_title_card};
use crate::screens::library::{load_posters, open_detail, target_of};
use crate::state::{GenerationId, Msg, Screen};
use crate::{AppWindow, CardData, Messages, PersonData, PersonFilter};

// Logical size of the photo, as drawn by `PersonScreen`.
const PHOTO: f32 = 200.0;

type Loaded = (PersonDetails, Vec<PersonTitle>);

/// The cards of the titles kept by the chosen filter.
type Shown = FilterModel<Rc<VecModel<CardData>>, Box<dyn Fn(&CardData) -> bool>>;

fn is_movie(kind: ItemKind) -> bool {
    matches!(kind, ItemKind::Movie | ItemKind::Video)
}

/// Whether a title of kind `kind` is listed under `filter`.
fn accepts(filter: PersonFilter, kind: Option<ItemKind>) -> bool {
    match filter {
        PersonFilter::All => true,
        PersonFilter::Movies => kind.is_some_and(is_movie),
        PersonFilter::Shows => kind == Some(ItemKind::Series),
    }
}

/// What the person page shows for `details` and its `titles`.
fn person_data(details: &PersonDetails, titles: &[PersonTitle]) -> PersonData {
    let count = |keep: fn(ItemKind) -> bool| {
        let found = titles
            .iter()
            .filter(|title| keep(title.summary.kind))
            .count();
        i32::try_from(found).unwrap_or(i32::MAX)
    };
    let (year, month, day) = details
        .birth
        .as_deref()
        .and_then(date_parts)
        .unwrap_or_default();
    PersonData {
        name: details.name.as_str().into(),
        initials: initials(&details.name).into(),
        overview: details.overview.clone().unwrap_or_default().into(),
        birth_year: year,
        birth_month: month,
        birth_day: day,
        birth_place: details.birth_place.clone().unwrap_or_default().into(),
        title_count: count(|_| true),
        movie_count: count(is_movie),
        show_count: count(|kind| kind == ItemKind::Series),
    }
}

/// State of the person page: the person shown and the answers still expected.
#[derive(Default)]
pub(crate) struct PersonController {
    instance: Rc<Cell<u64>>,
    load: Cell<u64>,
    current: RefCell<Option<String>>,
    loaded: RefCell<Option<Loaded>>,
    filter: Rc<Cell<PersonFilter>>,
    kinds: Rc<RefCell<HashMap<SharedString, ItemKind>>>,
    cards: Rc<VecModel<CardData>>,
    shown_cards: RefCell<Option<Rc<Shown>>>,
}

impl PersonController {
    /// A screen was shown: a person page loads when it is not the person already shown.
    pub(crate) fn shown(&self, app: &Rc<App>, screen: &Screen) {
        let Screen::Person(id) = screen else { return };
        if self.current.borrow().as_deref() == Some(id.as_str()) {
            return;
        }
        self.clear(app);
        *self.current.borrow_mut() = Some(id.clone());
        self.request(app);
    }

    /// A screen left the stack: the person page frees its content.
    pub(crate) fn discard(&self, app: &App, screen: &Screen) {
        if matches!(screen, Screen::Person(id) if self.current.borrow().as_deref() == Some(id.as_str()))
        {
            self.clear(app);
        }
    }

    /// A title changed below: the watched badges may be stale.
    pub(crate) fn refresh(&self, app: &Rc<App>) {
        if self.current.borrow().is_some() {
            self.request(app);
        }
    }

    /// Asks again after the person failed to load.
    pub(crate) fn retry(&self, app: &Rc<App>) {
        self.refresh(app);
    }

    /// Lists only the titles of `filter`.
    pub(crate) fn select_filter(&self, app: &App, filter: PersonFilter) {
        self.filter.set(filter);
        if let Some(window) = app.window() {
            window.set_person_filter(filter);
        }
        if let Some(shown) = self.shown_cards.borrow().as_ref() {
            shown.reset();
        }
    }

    /// Opens the detail page of the title `id`.
    pub(crate) fn open(&self, app: &Rc<App>, id: &str) {
        let target = self.loaded.borrow().as_ref().and_then(|(_, titles)| {
            let summaries: Vec<_> = titles.iter().map(|title| title.summary.clone()).collect();
            target_of(&summaries, id)
        });
        if let Some(screen) = target {
            open_detail(app, screen);
        }
    }

    fn clear(&self, app: &App) {
        self.instance.set(self.instance.get() + 1);
        self.load.set(self.load.get() + 1);
        self.current.borrow_mut().take();
        self.loaded.borrow_mut().take();
        self.filter.set(PersonFilter::All);
        self.kinds.borrow_mut().clear();
        self.cards.set_vec(Vec::new());
        self.shown_cards.borrow_mut().take();
        if let Some(window) = app.window() {
            window.set_person_loaded(false);
            window.set_person_loading(false);
            window.set_person_error(SharedString::default());
            window.set_person_data(PersonData::default());
            window.set_person_photo(Default::default());
            window.set_person_has_photo(false);
            window.set_person_titles(ModelRc::default());
            window.set_person_filter(PersonFilter::All);
            window.set_person_expanded(false);
            window.set_person_viewport_y(0.0);
        }
    }

    fn request(&self, app: &Rc<App>) {
        let (Some(person), Some((id, catalog, _))) =
            (self.current.borrow().clone(), app.services())
        else {
            return;
        };
        self.load.set(self.load.get() + 1);
        let load = self.load.get();
        if let Some(window) = app.window() {
            window.set_person_error(SharedString::default());
            window.set_person_loading(self.loaded.borrow().is_none());
        }
        app.spawn(
            async move {
                let person = ItemId::from(person.as_str());
                let (details, titles) =
                    tokio::join!(catalog.person(&person), catalog.person_titles(&person));
                Ok::<_, CoreError>((details?, titles?))
            },
            move |app, result| app.person.answered(app, load, id, result),
        );
    }

    fn answered(
        &self,
        app: &Rc<App>,
        load: u64,
        id: GenerationId,
        result: Result<Loaded, CoreError>,
    ) {
        if self.load.get() != load || app.services().is_none_or(|(current, ..)| current != id) {
            return;
        }
        let Some(window) = app.window() else { return };
        window.set_person_loading(false);
        match result {
            Ok(loaded) => {
                let first = self.loaded.borrow().is_none();
                self.publish(app, id, &window, &loaded, first);
                *self.loaded.borrow_mut() = Some(loaded);
            }
            Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(id)),
            Err(error) => {
                log::warn!("cannot load the person: {error}");
                if self.loaded.borrow().is_none() {
                    let (message, detail) = core_error_message(&error);
                    let text = window
                        .global::<Messages>()
                        .invoke_message_text(message, detail);
                    window.set_person_error(text);
                }
            }
        }
    }

    fn publish(
        &self,
        app: &Rc<App>,
        id: GenerationId,
        window: &AppWindow,
        loaded: &Loaded,
        first: bool,
    ) {
        let (details, titles) = loaded;
        window.set_person_data(person_data(details, titles));
        let previous: HashMap<SharedString, CardData> = self
            .cards
            .iter()
            .filter(|card| card.has_image)
            .map(|card| (card.id.clone(), card))
            .collect();
        let mut missing = Vec::new();
        let cards: Vec<CardData> = titles
            .iter()
            .map(|title| {
                let mut card = person_title_card(title);
                match previous.get(&card.id) {
                    Some(kept) => {
                        card.image = kept.image.clone();
                        card.has_image = true;
                    }
                    None => missing.push(title.summary.clone()),
                }
                card
            })
            .collect();
        *self.kinds.borrow_mut() = titles
            .iter()
            .map(|title| (title.summary.id.as_str().into(), title.summary.kind))
            .collect();
        self.cards.set_vec(cards);
        let shown = Rc::clone(self.shown_cards.borrow_mut().get_or_insert_with(|| {
            let (filter, kinds) = (Rc::clone(&self.filter), Rc::clone(&self.kinds));
            let keep: Box<dyn Fn(&CardData) -> bool> =
                Box::new(move |card| accepts(filter.get(), kinds.borrow().get(&card.id).copied()));
            Rc::new(FilterModel::new(Rc::clone(&self.cards), keep))
        }));
        shown.reset();
        window.set_person_titles(ModelRc::from(shown));
        window.set_person_loaded(true);
        load_posters(app, id, window, &self.cards, &missing);
        if !first {
            return;
        }
        let Some((_, _, images)) = app.services() else {
            return;
        };
        if let Some(photo) = details.image.clone() {
            let size = image_size(PHOTO, PHOTO, window.window().scale_factor());
            let (target, current, instance) = (
                window.as_weak(),
                Rc::clone(&self.instance),
                self.instance.get(),
            );
            load_into(app, id, &images, photo, size, move |image| {
                if let (Some(window), true) = (target.upgrade(), current.get() == instance) {
                    window.set_person_photo(image);
                    window.set_person_has_photo(true);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_keep_their_kinds() {
        assert!(accepts(PersonFilter::All, None));
        assert!(accepts(PersonFilter::Movies, Some(ItemKind::Movie)));
        assert!(!accepts(PersonFilter::Movies, Some(ItemKind::Series)));
        assert!(accepts(PersonFilter::Shows, Some(ItemKind::Series)));
        assert!(!accepts(PersonFilter::Shows, Some(ItemKind::Movie)));
    }
}
