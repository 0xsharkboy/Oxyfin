//! Lines up the durations of episode rows whatever the length of their title.

use i_slint_backend_testing::ElementHandle;
use page::EpisodesPage;
use slint::ComponentHandle;

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { EpisodeRow } from "ui/components/episode_row.slint";

        export component EpisodesPage inherits Window {
            VerticalLayout {
                EpisodeRow { number: "1"; title: "Short"; minutes: 24; }
                EpisodeRow { number: "2"; title: "A somewhat longer episode title"; overview: "A short overview."; minutes: 25; }
                EpisodeRow {
                    number: "3";
                    title: "An episode title so long that it cannot fit in the row and has to be cut before the duration that follows it, however wide the window is";
                    overview: "An overview long enough to wrap on more than one line of the row, so that it is cut after two lines like the reference design asks for it to be.";
                    minutes: 23;
                }
            }
        }
    }
}

fn by_row(page: &EpisodesPage, id: &str) -> Vec<ElementHandle> {
    let mut elements: Vec<_> = ElementHandle::find_by_element_id(page, id)
        .filter(|element| element.size().width > 0.)
        .collect();
    elements.sort_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y));
    elements
}

#[test]
fn durations_share_the_right_edge_of_the_rows() {
    i_slint_backend_testing::init_no_event_loop();
    let page = EpisodesPage::new().unwrap();
    page.window().set_size(slint::LogicalSize::new(1200., 600.));
    page.show().unwrap();

    let titles = by_row(&page, "EpisodeRow::title-text");
    let durations = by_row(&page, "EpisodeRow::duration");
    assert_eq!(titles.len(), 3, "titles");
    assert_eq!(durations.len(), 3, "durations");

    // The labels have the same length, so a label wider than its text shows as a shifted start.
    let edges: Vec<(f32, f32)> = durations
        .iter()
        .map(|duration| {
            let x = duration.absolute_position().x;
            (x, x + duration.size().width)
        })
        .collect();
    for (left, right) in &edges {
        assert!(
            (left - edges[0].0).abs() < 0.5 && (right - edges[0].1).abs() < 0.5,
            "the durations do not line up: {edges:?}"
        );
    }
    let rights: Vec<f32> = edges.iter().map(|edge| edge.1).collect();
    // The row is as wide as the window; its padding is the only space after the duration.
    assert!(
        rights[0] > 1200. - 40.,
        "the duration ends at {} instead of the end of the row",
        rights[0]
    );

    for (title, duration) in titles.iter().zip(&durations) {
        let title_end = title.absolute_position().x + title.size().width;
        assert!(
            title_end <= duration.absolute_position().x,
            "a title ending at {title_end} overlaps the duration starting at {}",
            duration.absolute_position().x
        );
    }
}
