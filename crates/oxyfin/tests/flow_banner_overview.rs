//! The overview of the home banner is cut after two lines.

use i_slint_backend_testing::ElementHandle;
use page::BannerPage;
use slint::ComponentHandle;

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { Banner } from "ui/components/banner.slint";
        import { Theme } from "ui/theme.slint";

        export component BannerPage inherits Window {
            in property <string> overview;
            out property <length> line: Theme.body-line-height;

            width: 1200px;
            height: 520px;

            Banner {
                title: "Title";
                overview: root.overview;
            }
        }
    }
}

#[test]
fn a_long_overview_takes_two_lines() {
    i_slint_backend_testing::init_no_event_loop();
    let page = BannerPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    let overview = "A long overview that says far more than fits. ".repeat(20);
    page.set_overview(overview.as_str().into());
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    let text = ElementHandle::find_by_accessible_label(&page, &overview)
        .next()
        .unwrap_or_else(|| panic!("no overview"));
    assert_eq!(text.size().height, 2. * page.get_line());
}
