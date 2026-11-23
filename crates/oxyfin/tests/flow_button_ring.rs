//! A button that becomes disabled loses its focus ring.

mod common;

use page::RingPage;
use slint::ComponentHandle;

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { Button } from "ui/components/button.slint";

        export component RingPage inherits Window {
            in property <bool> enabled <=> button.enabled;
            out property <bool> ring: button.ring;

            button := Button { label: "Clear"; }
        }
    }
}

#[test]
fn a_disabled_button_shows_no_focus_ring() {
    i_slint_backend_testing::init_no_event_loop();
    let page = RingPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    let button = i_slint_backend_testing::ElementHandle::find_by_accessible_label(&page, "Clear")
        .next()
        .unwrap_or_else(|| panic!("no button"));
    common::click(&page, &button);
    assert!(page.get_ring(), "the clicked button has the focus");
    page.set_enabled(false);
    assert!(!page.get_ring(), "the disabled button keeps its ring");
}
