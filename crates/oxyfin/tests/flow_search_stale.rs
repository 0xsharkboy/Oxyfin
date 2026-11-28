//! While a new search loads, the results of the previous text stay and dim after the skeleton
//! delay; a fast answer never dims them.

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, CardData, Screen};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, ModelRc, VecModel};

fn card_opacity(window: &AppWindow) -> f32 {
    ElementHandle::find_by_accessible_label(window, "Tesseract")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no result card"))
        .computed_opacity()
}

#[test]
fn previous_results_dim_while_new_ones_load() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(1280., 720.));
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    window.set_screen(Screen::Search);
    window.set_search_query("tes".into());
    window.set_search_total(1);
    window.set_search_cards(ModelRc::new(VecModel::from(vec![CardData {
        id: "a".into(),
        title: "Tesseract".into(),
        ..CardData::default()
    }])));
    i_slint_backend_testing::mock_elapsed_time(Duration::ZERO);
    assert_eq!(card_opacity(&window), 1.0);

    window.set_search_loading(true);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(100));
    assert_eq!(card_opacity(&window), 1.0, "a fast answer never dims them");
    window.set_search_loading(false);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(400));
    assert_eq!(card_opacity(&window), 1.0);

    window.set_search_loading(true);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(50));
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(500));
    assert!(
        (card_opacity(&window) - 0.5).abs() < 0.01,
        "dimmed to stale-opacity: {}",
        card_opacity(&window)
    );

    window.set_search_loading(false);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(10));
    assert_eq!(card_opacity(&window), 1.0, "the new results show at once");
}
