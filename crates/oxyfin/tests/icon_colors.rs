//! Icons are drawn in the color of their file, never colorized.

mod common;

use std::path::{Path, PathBuf};

use page::ColorsPage;

#[allow(missing_docs)]
mod page {
    slint::slint! {
        import { Theme } from "ui/theme.slint";

        export component ColorsPage inherits Window {
            out property <color> primary: Theme.text-primary;
            out property <color> secondary: Theme.text-secondary;
            out property <color> muted: Theme.text-muted;
            out property <color> on-accent: Theme.text-on-accent;
            out property <color> accent: Theme.accent;
            out property <color> watched: Theme.watched;
        }
    }
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("cannot list {dir:?}: {err}"))
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files(&path));
        } else {
            found.push(path);
        }
    }
    found
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|err| panic!("cannot read {path:?}: {err}"))
}

#[test]
fn nothing_is_colorized() {
    for path in files(&common::manifest_dir().join("ui")) {
        if path
            .extension()
            .is_some_and(|extension| extension == "slint")
        {
            assert!(!read(&path).contains("colorize:"), "{path:?}");
        }
    }
}

#[test]
fn an_icon_has_the_color_its_name_tells() {
    i_slint_backend_testing::init_no_event_loop();
    let page = ColorsPage::new().unwrap_or_else(|err| panic!("cannot create the page: {err}"));
    let hex = |color: slint::Color| {
        format!(
            "#{:02X}{:02X}{:02X}",
            color.red(),
            color.green(),
            color.blue()
        )
    };
    let primary = hex(page.get_primary());
    // The longest suffix first: "-on-accent" ends with "-accent".
    let suffixes = [
        ("-on-accent", hex(page.get_on_accent())),
        ("-accent", hex(page.get_accent())),
        ("-secondary", hex(page.get_secondary())),
        ("-muted", hex(page.get_muted())),
        ("-watched", hex(page.get_watched())),
    ];
    let icons = common::manifest_dir().join("ui/icons");
    let mut colored = 0;
    for path in files(&icons) {
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        let svg = read(&path);
        // The logo keeps its own colors.
        if name == "logo" {
            continue;
        }
        let base = |suffix: &str| Some(icons.join(format!("{}.svg", name.strip_suffix(suffix)?)));
        let variant = suffixes.iter().find_map(|(suffix, color)| {
            let base = base(suffix).filter(|base| base.exists())?;
            Some((base, color))
        });
        match variant {
            Some((base, color)) if name != "volume-muted" => {
                colored += 1;
                assert_eq!(svg, read(&base).replace(&primary, color), "{name}");
            }
            _ => {
                for (at, _) in svg.match_indices('#') {
                    assert_eq!(svg[at..at + 7], primary, "{name}");
                }
            }
        }
    }
    assert!(colored > 0, "no colored icon found");
}
