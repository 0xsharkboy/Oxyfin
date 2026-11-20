//! A movie outside any box set shows no collection link.

mod common;

use std::time::Duration;

use common::collection::{OTHER_BOX_SET, open_movie, requested, start};
use common::{exists, pause, run, wait_until};
use i_slint_backend_testing::ElementHandle;
use slint::ComponentHandle;

#[test]
fn movie_without_collection_has_no_link() {
    let (flow, log) = start(false);
    let window = flow.window.as_weak();
    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        open_movie(&window).await?;
        wait_until("the box set items", || {
            requested(&log, "parentId", OTHER_BOX_SET)
        })
        .await?;
        pause(Duration::from_millis(300)).await;
        let link = ElementHandle::find_by_element_type_name(&window, "CollectionLink")
            .any(|link| link.size().height > 0.);
        if link || exists(&window, "Part of the Other Collection") {
            return Err("a movie outside any collection shows a link".into());
        }
        Ok(())
    });
}
