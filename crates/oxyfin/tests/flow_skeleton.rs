//! Bones stand in for a page while it loads: only after a short wait, in the place of the real
//! elements, swept by a band of light.

mod common;

use std::time::Duration;

use i_slint_backend_testing::ElementHandle;
use page::{GridPage, PageCard, SkeletonPage};
use slint::{ComponentHandle, ModelRc, VecModel};

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { Skeleton, BannerBones, DetailHeaderBones, EpisodeRowBones } from "ui/components/skeleton.slint";
        import { EpisodeRow } from "ui/components/episode_row.slint";
        import { PosterGrid } from "ui/components/poster_grid.slint";
        import { CardData, Theme } from "ui/theme.slint";

        export { CardData as PageCard }

        export component SkeletonPage inherits Window {
            in property <bool> loading;
            out property <duration> delay: Theme.skeleton-delay;
            out property <length> poster-width: Theme.poster-width * 1.5;
            out property <length> poster-height: Theme.poster-height * 1.5;
            out property <length> button-height: Theme.button-height;
            out property <length> banner-height: Theme.banner-height;

            width: 1200px;
            height: 1600px;

            VerticalLayout {
                alignment: start;

                if root.loading: Skeleton {
                    VerticalLayout {
                        DetailHeaderBones { }
                        EpisodeRowBones { }
                    }
                }
                EpisodeRow {
                    number: "1";
                    title: "Pilot";
                    overview: "The first one.";
                    minutes: 42;
                }
                if root.loading: Skeleton {
                    BannerBones { }
                }
            }
        }

        export component GridPage inherits Window {
            in property <[CardData]> cards;
            in property <bool> loading;
            out property <length> margin: Theme.space-3xl;

            width: 1000px;
            height: 700px;

            PosterGrid {
                cards: root.cards;
                loading: root.loading;
            }
        }
    }
}

/// Starts a load; looking at the window builds the skeleton, as a frame would.
fn start_loading(page: &impl LoadingPage) {
    page.set_loading(true);
    let _ = common::bones(page);
}

trait LoadingPage: ComponentHandle {
    fn set_loading(&self, loading: bool);
}

impl LoadingPage for SkeletonPage {
    fn set_loading(&self, loading: bool) {
        SkeletonPage::set_loading(self, loading);
    }
}

impl LoadingPage for GridPage {
    fn set_loading(&self, loading: bool) {
        GridPage::set_loading(self, loading);
    }
}

fn elapse(ms: u64) {
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
}

fn skeleton_page() -> SkeletonPage {
    i_slint_backend_testing::init_no_event_loop();
    let page = SkeletonPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    page
}

fn skeleton(page: &impl ComponentHandle) -> ElementHandle {
    ElementHandle::find_by_element_type_name(page, "Skeleton")
        .next()
        .unwrap_or_else(|| panic!("no skeleton"))
}

fn by_id(page: &impl ComponentHandle, id: &str) -> ElementHandle {
    ElementHandle::find_by_element_id(page, id)
        .next()
        .unwrap_or_else(|| panic!("no {id}"))
}

fn delay(page: &SkeletonPage) -> u64 {
    u64::try_from(page.get_delay()).unwrap_or_else(|err| panic!("bad delay: {err}"))
}

#[test]
fn the_bones_show_only_after_the_delay() {
    let page = skeleton_page();
    start_loading(&page);
    assert!(!common::bones(&page).is_empty(), "the skeleton is in place");
    assert!(!common::skeleton_shown(&page), "nothing shows at once");
    elapse(delay(&page) - 10);
    assert!(
        !common::skeleton_shown(&page),
        "nothing shows before the delay"
    );
    elapse(20);
    assert!(
        common::skeleton_shown(&page),
        "the bones show after the delay"
    );
}

#[test]
fn a_fast_answer_never_shows_the_bones() {
    let page = skeleton_page();
    start_loading(&page);
    elapse(delay(&page) / 2);
    page.set_loading(false);
    assert!(
        common::bones(&page).is_empty(),
        "the skeleton left with the answer"
    );
    elapse(delay(&page));
    assert!(common::bones(&page).is_empty(), "nothing shows afterwards");

    start_loading(&page);
    elapse(delay(&page) / 2);
    assert!(!common::skeleton_shown(&page), "the wait starts again");
    elapse(delay(&page));
    assert!(common::skeleton_shown(&page));
}

#[test]
fn a_band_of_light_sweeps_across() {
    let page = skeleton_page();
    start_loading(&page);
    elapse(delay(&page) + 10);
    let skeleton = skeleton(&page);
    let band = by_id(&page, "Skeleton::shimmer");
    assert_eq!(band.size().width, skeleton.size().width);
    assert_eq!(band.size().height, skeleton.size().height);
    let before = band.absolute_position().x;
    elapse(400);
    let after = band.absolute_position().x;
    assert!(
        (after - before).abs() > 1.,
        "the band moved from {before} to {after}"
    );
}

#[test]
fn the_bones_sit_where_the_real_elements_go() {
    let page = skeleton_page();
    start_loading(&page);
    elapse(delay(&page) + 10);

    let poster = by_id(&page, "DetailHeaderBones::poster");
    assert_eq!(poster.size().width, page.get_poster_width());
    assert_eq!(poster.size().height, page.get_poster_height());
    let play = by_id(&page, "DetailHeaderBones::play");
    assert_eq!(play.size().height, page.get_button_height());

    let skeleton = skeleton(&page);
    assert_eq!(
        poster.absolute_position().x,
        skeleton.absolute_position().x,
        "the poster starts the header, as in DetailHeader"
    );

    let row = ElementHandle::find_by_element_type_name(&page, "EpisodeRow")
        .next()
        .unwrap_or_else(|| panic!("no episode row"));
    assert!(skeleton.size().height > page.get_poster_height());
    assert_eq!(
        row.absolute_position().y,
        skeleton.absolute_position().y + skeleton.size().height,
        "the skeleton takes its room in the layout"
    );

    let thumb = by_id(&page, "EpisodeRow::thumb");
    let bone = by_id(&page, "EpisodeRowBones::thumb");
    assert_eq!(bone.size(), thumb.size());
    assert_eq!(bone.absolute_position().x, thumb.absolute_position().x);

    let banner = ElementHandle::find_by_element_type_name(&page, "BannerBones")
        .next()
        .unwrap_or_else(|| panic!("no banner bones"));
    assert_eq!(banner.size().height, page.get_banner_height());
    assert_eq!(banner.size().width, 1200.);
    assert_eq!(banner.absolute_position().x, 0.);
}

fn card(index: usize) -> PageCard {
    PageCard {
        id: format!("id{index}").into(),
        title: format!("Poster {index}").into(),
        subtitle: "2024".into(),
        ..PageCard::default()
    }
}

fn grid_page(cards: usize) -> GridPage {
    i_slint_backend_testing::init_no_event_loop();
    let page = GridPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.set_cards(ModelRc::new(VecModel::from(
        (0..cards).map(card).collect::<Vec<_>>(),
    )));
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    page
}

fn artworks(page: &GridPage) -> Vec<ElementHandle> {
    let mut found: Vec<_> =
        ElementHandle::find_by_element_id(page, "PosterCardBones::artwork").collect();
    found.sort_by(|a, b| {
        let (a, b) = (a.absolute_position(), b.absolute_position());
        (a.y, a.x)
            .partial_cmp(&(b.y, b.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    found
}

#[test]
fn an_empty_grid_fills_with_poster_bones() {
    let page = grid_page(0);
    start_loading(&page);
    elapse(1000);
    let bones = artworks(&page);
    assert!(bones.len() >= 10, "a few rows of posters: {}", bones.len());
    let first = &bones[0];
    assert_eq!(first.absolute_position().x, page.get_margin());
    assert!(common::skeleton_shown(&page));
    page.set_loading(false);
    assert!(common::bones(&page).is_empty());
}

#[test]
fn more_pages_load_after_the_last_poster() {
    let page = grid_page(2);
    start_loading(&page);
    elapse(1000);
    let posters: Vec<_> = ElementHandle::find_by_element_type_name(&page, "PosterCard")
        .filter(|card| {
            card.accessible_label()
                .is_some_and(|label| label.starts_with("Poster"))
        })
        .collect();
    assert_eq!(posters.len(), 2);
    let (first, second) = (&posters[0], &posters[1]);
    let step = second.absolute_position().x - first.absolute_position().x;
    let bones = artworks(&page);
    assert!(!bones.is_empty(), "bones follow the posters");
    let next = &bones[0];
    assert_eq!(next.absolute_position().y, first.absolute_position().y);
    assert_eq!(
        next.absolute_position().x,
        second.absolute_position().x + step
    );
    assert_eq!(next.size().width, first.size().width);
}

#[test]
fn no_screen_says_loading() {
    let ui = common::manifest_dir().join("ui");
    let mut files = vec![ui.join("app.slint")];
    for folder in ["screens", "components"] {
        let entries = std::fs::read_dir(ui.join(folder))
            .unwrap_or_else(|err| panic!("cannot list {folder}: {err}"));
        files.extend(entries.filter_map(|entry| entry.ok().map(|entry| entry.path())));
    }
    for file in files {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("cannot read {}: {err}", file.display()));
        assert!(
            !text.contains("@tr(\"Loading\")"),
            "{} still says Loading",
            file.display()
        );
    }
    let catalog = std::fs::read_to_string(common::manifest_dir().join("po").join("oxyfin.pot"))
        .unwrap_or_else(|err| panic!("cannot read the catalog: {err}"));
    assert!(
        !catalog.lines().any(|line| line == "msgid \"Loading\""),
        "the catalog still has Loading"
    );
}
