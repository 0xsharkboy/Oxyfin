//! Settings > Diagnostics shows the size of the image cache, and "Clear cache" empties it and
//! removes the last pages;
//! Escape still leaves the settings afterwards.

mod common;

use common::playback_settings::{WIDTH, open_settings_prepared};
use common::{click, wait_role, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize};

fn shown(window: &AppWindow, text: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, text).any(|element| element.size().height > 0.)
}

fn clear_enabled(window: &AppWindow) -> bool {
    ElementHandle::find_by_accessible_label(window, "Clear cache")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .and_then(|element| element.accessible_enabled())
        .unwrap_or(false)
}

// One app per test binary: the testing backend can be set up only once per process.
#[test]
fn the_image_cache_size_is_shown_and_cleared() {
    open_settings_prepared(
        Vec::new(),
        |paths| {
            let images = paths.cache_dir.join("images");
            std::fs::create_dir_all(&images).unwrap();
            std::fs::write(
                images.join("a-Primary-t-300x450"),
                vec![0u8; 2 * 1024 * 1024],
            )
            .unwrap();
            std::fs::write(images.join("b-Primary-t-300x450"), vec![0u8; 1024 * 1024]).unwrap();
        },
        |_| {},
        async |window, paths| {
            window.window().set_size(LogicalSize::new(WIDTH, 2600.));
            let images = paths
                .lock()
                .map_err(|_| "paths poisoned")?
                .clone()
                .ok_or("no paths")?
                .cache_dir
                .join("images");
            wait_until("the image cache row", || shown(window, "Image cache")).await?;
            let pages = paths
                .lock()
                .map_err(|_| "paths poisoned")?
                .clone()
                .ok_or("no paths")?
                .cache_dir
                .join("pages.json");
            wait_until("the last pages on disk", || pages.exists()).await?;
            wait_until("the cache size", || shown(window, "3 MB")).await?;
            if !clear_enabled(window) {
                return Err("Clear cache is disabled with images stored".into());
            }

            let clear = wait_role(window, "Clear cache", AccessibleRole::Button).await?;
            click(window, &clear);
            wait_until("the emptied cache", || shown(window, "0 KB")).await?;
            let left = std::fs::read_dir(&images).map_or(0, |entries| entries.count());
            if left != 0 {
                return Err(format!("{left} files left in the image cache"));
            }
            if pages.exists() {
                return Err("the last pages are left on disk".into());
            }
            wait_until("Clear cache disabled", || !clear_enabled(window)).await?;

            // The focus stays on the disabled button; Escape still leaves the settings.
            let escape = slint::SharedString::from(Key::Escape);
            window.window().dispatch_event(WindowEvent::KeyPressed {
                text: escape.clone(),
            });
            window
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text: escape });
            wait_until("the settings closed", || {
                window.get_screen() != Screen::Settings
            })
            .await
        },
    );
}
