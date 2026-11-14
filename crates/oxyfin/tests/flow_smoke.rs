//! Proves the testing backend and the cross-thread round trip work.

use oxyfin::AppWindow;
use slint::ComponentHandle;

#[test]
fn event_loop_round_trips_between_threads() {
    i_slint_backend_testing::init_integration_test_with_system_time();
    let window = AppWindow::new().unwrap();
    window.show().unwrap();

    let thread = std::thread::spawn(|| {
        slint::invoke_from_event_loop(|| {
            slint::quit_event_loop().unwrap();
        })
        .unwrap();
    });
    slint::run_event_loop().unwrap();
    thread.join().unwrap();
}
