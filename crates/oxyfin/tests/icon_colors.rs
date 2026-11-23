//! Icons are drawn in the color of their file wherever that color is wanted.

mod common;

use page::TintPage;
use slint::ComponentHandle;

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { Icons } from "ui/icons.slint";
        import { Theme } from "ui/theme.slint";

        export component TintPage inherits Window {
            out property <color> primary: Theme.text-primary;
            out property <bool> primary-left-alone: Icons.tint(Theme.text-primary) == transparent;
            out property <bool> others-kept: Icons.tint(Theme.accent) == Theme.accent;
        }
    }
}

#[test]
fn icons_of_the_text_color_are_not_colorized() {
    i_slint_backend_testing::init_no_event_loop();
    let page = TintPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    page.show()
        .unwrap_or_else(|err| panic!("cannot show the page: {err}"));
    assert!(page.get_primary_left_alone());
    assert!(page.get_others_kept());

    let color = page.get_primary();
    let expected = format!(
        "#{:02X}{:02X}{:02X}",
        color.red(),
        color.green(),
        color.blue()
    );
    let icons = common::manifest_dir().join("ui/icons");
    for entry in std::fs::read_dir(&icons)
        .unwrap_or_else(|err| panic!("cannot list the icons: {err}"))
        .flatten()
    {
        // The logo keeps its own colors.
        if entry.file_name() == "logo.svg" {
            continue;
        }
        let svg = std::fs::read_to_string(entry.path())
            .unwrap_or_else(|err| panic!("cannot read {:?}: {err}", entry.path()));
        for (at, _) in svg.match_indices('#') {
            assert_eq!(&svg[at..at + 7], expected, "{:?}", entry.file_name());
        }
    }
}
