//! A person without photo, biography, birth nor titles: the page shows the initials and the
//! name, and no empty label.

mod common;

use std::time::Duration;

use common::person::{open_person, person_bare, start_person, titles};
use common::{exists, pause, run, wait_visible};
use i_slint_backend_testing::ElementHandle;
use slint::ComponentHandle;

#[test]
fn person_without_data() {
    let flow = start_person(person_bare(), titles(&[]));
    let window = flow.window.as_weak();
    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        open_person(&window).await?;
        wait_visible(&window, "Maren Holt").await?;
        wait_visible(&window, "MH").await?;
        pause(Duration::from_millis(200)).await;
        let texts: Vec<String> = [
            "Text",
            "DisplayText",
            "HeadingText",
            "BodyText",
            "LabelText",
        ]
        .into_iter()
        .flat_map(|kind| ElementHandle::find_by_element_type_name(&window, kind))
        .filter(|text| text.size().width > 0. && text.size().height > 0.)
        .filter_map(|text| text.accessible_label().map(|label| label.to_string()))
        .collect();
        if let Some(empty) = texts.iter().find(|text| text.trim().is_empty()) {
            return Err(format!("an empty label is shown: {empty:?} in {texts:?}"));
        }
        for absent in ["Read more", "All (", "Born", "titles in your library"] {
            if texts.iter().any(|text| text.contains(absent)) || exists(&window, absent) {
                return Err(format!("{absent} is shown for a person without data"));
            }
        }
        if !window.get_person_error().is_empty() || window.get_person_loading() {
            return Err("the page is not loaded".into());
        }
        Ok(())
    });
}
