//! A movie in two box sets links to both, one link under the other.

mod common;

use common::collection::{LINK, open_movie, start_sets};
use common::{click, run, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;

#[test]
fn movie_links_every_box_set() {
    let (flow, _) = start_sets(true, true);
    let window = flow.window.as_weak();
    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        open_movie(&window).await?;
        let first = wait_role(&window, LINK, AccessibleRole::Button).await?;
        let second = wait_role(
            &window,
            "Part of the Other Collection",
            AccessibleRole::Button,
        )
        .await?;
        let (top, bottom) = (first.absolute_position(), second.absolute_position());
        if bottom.y < top.y + first.size().height || (bottom.x - top.x).abs() > 0.5 {
            return Err(format!(
                "the links are not stacked: {top:?} {:?} then {bottom:?}",
                first.size()
            ));
        }
        click(&window, &second);
        wait_until("the other collection", || {
            window.get_screen() == Screen::Collection
                && window.get_collection_data().title == "Other Collection"
        })
        .await?;
        Ok(())
    });
}
