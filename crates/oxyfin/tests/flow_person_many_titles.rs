//! A person credited on hundreds of titles: the grid lays them out in rows and the page
//! scrolls down to the last one.

mod common;

use std::time::Duration;

use common::person::{many_titles, open_person, person_full, scroll, start_person, window_height};
use common::{exists, pause, run, wait_role, wait_visible};
use i_slint_backend_testing::AccessibleRole;
use slint::{ComponentHandle, Model};

const LAST: &str = "Title 299";

#[test]
fn person_many_titles() {
    let flow = start_person(person_full(), many_titles(300));
    let window = flow.window.as_weak();
    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        open_person(&window).await?;
        wait_visible(
            &window,
            "Born 12 March 1984 in Bergen, Norway \u{b7} 300 titles in your library",
        )
        .await?;
        if window.get_person_titles().row_count() != 300 {
            return Err("the grid does not hold the 300 titles".into());
        }
        let first = wait_role(&window, "Title 000", AccessibleRole::Button).await?;
        let second = wait_role(&window, "Title 001", AccessibleRole::Button).await?;
        if second.absolute_position().x <= first.absolute_position().x {
            return Err("the titles are not laid out side by side".into());
        }
        let height = window_height(&window);
        for _ in 0..400 {
            if exists(&window, LAST) {
                let last = wait_role(&window, LAST, AccessibleRole::Button).await?;
                if last.absolute_position().y + last.size().height <= height {
                    break;
                }
            }
            scroll(&window, -400.);
            pause(Duration::from_millis(5)).await;
        }
        let last = wait_role(&window, LAST, AccessibleRole::Button).await?;
        let (top, bottom) = (
            last.absolute_position().y,
            last.absolute_position().y + last.size().height,
        );
        if top < 0. || bottom > height {
            return Err(format!(
                "the last title is not in the window: {top}..{bottom}"
            ));
        }
        if exists(&window, "Title 000") && first.absolute_position().y >= 0. {
            return Err("the page did not scroll".into());
        }
        Ok(())
    });
}
