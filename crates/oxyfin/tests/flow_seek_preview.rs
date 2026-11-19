//! Hovering the seek bar shows the time under the pointer and, when the item
//! has trickplay thumbnails, the tile of that time cropped from its sheet.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::screens::player;
use oxyfin::{AppWindow, Screen};
use oxyfin_api::{ItemId, MediaSourceId};
use oxyfin_core::{PlaybackEvent, Trickplay, TrickplaySheet};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize};

const WIDTH: f32 = 1280.;
const HEIGHT: f32 = 720.;
const ITEM: &str = "76282b1c408f374beeee50519e9157a4";

fn trickplay() -> Trickplay {
    Trickplay {
        item: ItemId::from(ITEM),
        media_source: MediaSourceId::from(ITEM),
        resolution: 320,
        width: 320,
        height: 180,
        columns: 10,
        rows: 10,
        count: 1000,
        interval: Duration::from_secs(1),
    }
}

fn find(window: &AppWindow, label: &str, role: AccessibleRole) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label).find(|element| {
        let size = element.size();
        element.accessible_role() == Some(role) && size.width > 0. && size.height > 0.
    })
}

fn seek_bar(window: &AppWindow) -> ElementHandle {
    find(window, "Seek", AccessibleRole::Slider).unwrap_or_else(|| panic!("no seek bar"))
}

fn preview(window: &AppWindow) -> Option<ElementHandle> {
    find(window, "Seek preview", AccessibleRole::Groupbox)
}

/// Moves the pointer over the seek bar, at `ratio` of its width.
fn hover(window: &AppWindow, ratio: f32) -> LogicalPosition {
    let bar = seek_bar(window);
    let position = LogicalPosition::new(
        bar.absolute_position().x + bar.size().width * ratio,
        bar.absolute_position().y + bar.size().height / 2.,
    );
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    position
}

struct Setup {
    window: AppWindow,
    requests: Rc<RefCell<Vec<TrickplaySheet>>>,
}

#[allow(clippy::unwrap_used)]
fn setup(trickplay: Option<Trickplay>) -> Setup {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    window.window().set_size(LogicalSize::new(WIDTH, HEIGHT));
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    player::bind(&window, Rc::new(|_| {}));
    let requests = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&requests);
    player::bind_preview(
        &window,
        Rc::new(move |sheet| recorder.borrow_mut().push(sheet)),
    );
    player::reset(&window);
    window.set_screen(Screen::Player);
    if let Some(trickplay) = trickplay {
        player::apply(&window, PlaybackEvent::Trickplay(trickplay));
    }
    player::apply(
        &window,
        PlaybackEvent::Ready {
            duration: Some(Duration::from_secs(1000)),
            audio: Vec::new(),
            subtitles: Vec::new(),
            has_previous: true,
            has_next: false,
        },
    );
    Setup { window, requests }
}

#[test]
fn hovering_the_seek_bar_previews_the_tile_of_the_time() {
    let Setup { window, requests } = setup(Some(trickplay()));
    assert!(preview(&window).is_none());

    let pointer = hover(&window, 0.2505);
    let popup = preview(&window).unwrap_or_else(|| panic!("no preview on hover"));
    let state = window.get_player_preview();
    assert_eq!(state.time, "4:10");
    assert!(
        ElementHandle::find_by_accessible_label(&window, "4:10")
            .next()
            .is_some()
    );
    assert_eq!(
        *requests.borrow(),
        vec![trickplay().tile_at(Duration::from_secs(250)).sheet]
    );
    assert!(!state.has_image, "no image before the sheet arrives");

    let bar = seek_bar(&window);
    let (at, size) = (popup.absolute_position(), popup.size());
    assert!(
        (at.x + size.width / 2. - pointer.x).abs() < 1.,
        "centered on the pointer: {at:?} {size:?} {pointer:?}"
    );
    assert!(
        at.y + size.height <= bar.absolute_position().y,
        "above the bar"
    );

    let sheet = trickplay().tile_at(Duration::from_secs(250)).sheet;
    let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(3200, 1800);
    player::show_sheet(&window, &sheet, slint::Image::from_rgba8(pixels));
    let state = window.get_player_preview();
    assert!(state.has_image);
    // Thumbnail 250 is the 51st of sheet 2: row 5, column 0.
    assert_eq!(
        (state.x, state.y, state.width, state.height),
        (0, 900, 320, 180)
    );
    let thumb = preview(&window).unwrap_or_else(|| panic!("preview gone"));
    assert!(
        thumb.size().height > size.height,
        "the thumbnail adds height"
    );

    hover(&window, 0.2515);
    assert_eq!(requests.borrow().len(), 1, "same sheet: no new request");
    let state = window.get_player_preview();
    assert!(state.has_image);

    hover(&window, 0.0125);
    let state = window.get_player_preview();
    assert_eq!(state.time, "0:12");
    assert_eq!((state.x, state.y), (640, 180));
    assert_eq!(requests.borrow().len(), 2);
    assert!(!state.has_image, "the other sheet is not loaded yet");
    player::show_sheet(&window, &sheet, slint::Image::default());
    assert!(
        !window.get_player_preview().has_image,
        "a sheet that is no longer wanted is dropped"
    );

    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(WIDTH / 2., HEIGHT / 2.),
    });
    assert!(preview(&window).is_none(), "the preview follows the hover");
}

#[test]
fn the_preview_stays_inside_the_window_at_both_ends() {
    let Setup { window, .. } = setup(Some(trickplay()));
    player::show_sheet(
        &window,
        &trickplay().sheet(0),
        slint::Image::from_rgba8(slint::SharedPixelBuffer::new(3200, 1800)),
    );
    for ratio in [0.0, 0.001, 0.999] {
        hover(&window, ratio);
        let popup = preview(&window).unwrap_or_else(|| panic!("no preview at {ratio}"));
        let (at, size) = (popup.absolute_position(), popup.size());
        assert!(
            at.x >= 0. && at.x + size.width <= WIDTH,
            "{ratio}: {at:?} {size:?}"
        );
    }
}

#[test]
fn dragging_previews_the_dragged_time() {
    let Setup { window, requests } = setup(Some(trickplay()));
    let start = hover(&window, 0.1);
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    let bar = seek_bar(&window);
    let end = LogicalPosition::new(
        bar.absolute_position().x + bar.size().width * 0.6005,
        start.y - 40.,
    );
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: end });
    assert!(
        preview(&window).is_some(),
        "shown while dragging off the bar"
    );
    assert_eq!(window.get_player_preview().time, "10:00");
    assert_eq!(
        requests.borrow().last(),
        Some(&trickplay().tile_at(Duration::from_secs(600)).sheet)
    );
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Left,
        });
}

#[test]
fn without_trickplay_the_preview_shows_only_the_time() {
    let Setup { window, requests } = setup(None);
    hover(&window, 0.5005);
    let popup = preview(&window).unwrap_or_else(|| panic!("no preview"));
    assert_eq!(window.get_player_preview().time, "8:20");
    assert!(requests.borrow().is_empty());
    assert!(!window.get_player_preview().has_image);
    assert!(
        popup.size().height < 60.,
        "a time label only: {:?}",
        popup.size()
    );
}

fn controls_visible(window: &AppWindow) -> bool {
    find(window, "Seek", AccessibleRole::Slider).is_some()
}

#[test]
fn controls_stay_while_the_pointer_rests_on_the_seek_bar() {
    let Setup { window, .. } = setup(Some(trickplay()));
    hover(&window, 0.5005);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(5));
    assert!(
        controls_visible(&window),
        "the controls hid under the pointer"
    );
    assert!(
        preview(&window).is_some(),
        "the preview hid under the pointer"
    );
}

#[test]
fn controls_hide_after_leaving_the_seek_bar() {
    let Setup { window, .. } = setup(Some(trickplay()));
    hover(&window, 0.5005);
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(5));
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(WIDTH / 2., HEIGHT / 3.),
    });
    i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(4));
    assert!(!controls_visible(&window), "the controls stayed up");
    assert!(preview(&window).is_none());
}
