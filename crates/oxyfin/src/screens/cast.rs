//! The "Cast and crew" row of the detail pages.

use std::rc::Rc;

use oxyfin_core::Person;
use slint::{ComponentHandle, Model, VecModel};

use crate::app::App;
use crate::images::{PageLoads, load_into};
use crate::present::{image_size, person_card};
use crate::state::{GenerationId, Msg, Screen};
use crate::{AppWindow, PersonCardData};

// Logical size of the photo, as drawn by `PersonCard`.
const PHOTO: f32 = 112.0;

/// Cards of `people`, without their photos.
pub(crate) fn cast_cards(people: &[Person]) -> Rc<VecModel<PersonCardData>> {
    Rc::new(VecModel::from(
        people.iter().map(person_card).collect::<Vec<_>>(),
    ))
}

/// Loads the photos of `people` still missing from `cards`, their cards, as loads of the page
/// `loads`.
pub(crate) fn load_photos(
    app: &Rc<App>,
    id: GenerationId,
    window: &AppWindow,
    cards: &Rc<VecModel<PersonCardData>>,
    people: &[Person],
    loads: &PageLoads,
) {
    let Some((_, _, images)) = app.services() else {
        return;
    };
    let size = image_size(PHOTO, PHOTO, window.window().scale_factor());
    for (index, person) in people.iter().enumerate() {
        let loaded = cards.row_data(index).is_none_or(|card| card.has_image);
        let Some(photo) = person.image.clone().filter(|_| !loaded) else {
            continue;
        };
        let cards = Rc::clone(cards);
        loads.track(load_into(app, id, &images, photo, size, move |image| {
            if let Some(mut data) = cards.row_data(index) {
                data.image = image;
                data.has_image = true;
                cards.set_row_data(index, data);
            }
        }));
    }
}

/// Opens the page of the person `id`; people without an identifier open nothing.
pub(crate) fn open_person(app: &Rc<App>, id: &str) {
    if !id.is_empty() {
        app.dispatch(Msg::OpenItem(Screen::Person(id.to_owned())));
    }
}
