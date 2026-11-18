//! The Linux desktop files under `data/` agree with the application ID the window advertises.

use std::path::PathBuf;

fn data_file(extension: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .join(format!("{}.{extension}", oxyfin::APP_ID));
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn desktop_entry(key: &str) -> Option<String> {
    data_file("desktop")
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&format!("{key}=")).map(str::to_owned))
}

#[test]
fn desktop_entry_matches_the_app_id() {
    assert_eq!(desktop_entry("Icon").as_deref(), Some(oxyfin::APP_ID));
    assert_eq!(
        desktop_entry("StartupWMClass").as_deref(),
        Some(oxyfin::APP_ID)
    );
    assert_eq!(desktop_entry("Exec").as_deref(), Some("oxyfin"));
}

#[test]
fn metainfo_matches_the_app_id() {
    let metainfo = data_file("metainfo.xml");
    assert!(metainfo.contains(&format!("<id>{}</id>", oxyfin::APP_ID)));
    assert!(metainfo.contains(&format!(
        "<launchable type=\"desktop-id\">{}.desktop</launchable>",
        oxyfin::APP_ID
    )));
}

#[test]
fn icon_is_named_after_the_app_id() {
    assert!(data_file("svg").starts_with("<svg"));
}
