//! The "Cast and crew" row of the detail pages.

use std::rc::Rc;

use oxyfin_core::Person;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::app::App;
use crate::images::load_into;
use crate::present::{image_size, person_card};
use crate::state::{GenerationId, Msg, Screen};
use crate::{AppWindow, PersonCardData};

// Logical size of the photo, as drawn by `PersonCard`.
const PHOTO: f32 = 112.0;

/// Cards of `people`, whose photos load into the cards as they arrive.
pub(crate) fn cast_cards(
    app: &Rc<App>,
    id: GenerationId,
    window: &AppWindow,
    people: &[Person],
) -> ModelRc<PersonCardData> {
    let cards = Rc::new(VecModel::from(
        people.iter().map(person_card).collect::<Vec<_>>(),
    ));
    if let Some((_, _, images)) = app.services() {
        let size = image_size(PHOTO, PHOTO, window.window().scale_factor());
        for (index, person) in people.iter().enumerate() {
            let Some(photo) = person.image.clone() else {
                continue;
            };
            let cards = Rc::clone(&cards);
            load_into(app, id, &images, photo, size, move |image| {
                if let Some(mut data) = cards.row_data(index) {
                    data.image = image;
                    data.has_image = true;
                    cards.set_row_data(index, data);
                }
            });
        }
    }
    ModelRc::from(cards)
}

/// Opens the page of the person `id`; people without an identifier open nothing.
pub(crate) fn open_person(app: &Rc<App>, id: &str) {
    if !id.is_empty() {
        app.dispatch(Msg::OpenItem(Screen::Person(id.to_owned())));
    }
}
