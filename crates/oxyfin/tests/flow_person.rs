//! Cast and crew: the row of a movie page opens the person page, whose biography and filters
//! work, and Escape goes back to the movie page.

mod common;

use common::person::{
    back_returns_to_detail, cast_card_opens_person, person_bio_read_more, person_filters_counts,
    person_full, start_person, three_titles,
};
use common::run;
use slint::ComponentHandle;

#[test]
fn cast_card_opens_person_with_bio_filters_and_back() {
    let flow = start_person(person_full(), three_titles());
    let window = flow.window.as_weak();
    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        cast_card_opens_person(&window).await?;
        person_bio_read_more(&window).await?;
        person_filters_counts(&window).await?;
        back_returns_to_detail(&window).await
    });
}
