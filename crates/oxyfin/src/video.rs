//! Video drawn under the interface by the window's rendering notifier.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use oxyfin_player::{Player, VideoRenderer};
use raw_window_handle::HasDisplayHandle;
use slint::{ComponentHandle, GraphicsAPI, RenderingState, SetRenderingNotifierError};

use crate::state::GenerationId;
use crate::{AppWindow, Screen};

/// The player of the current generation, published by the application for the notifier.
pub type VideoSource = Rc<RefCell<Option<(GenerationId, Arc<Player>)>>>;

/// A renderer and the player it draws.
struct Active {
    id: GenerationId,
    // Fields drop in declaration order: the renderer goes before its player.
    renderer: VideoRenderer,
    _player: Arc<Player>,
}

/// What the notifier does with its renderer before a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Keep,
    Drop,
    Replace,
}

fn step(held: Option<GenerationId>, wanted: Option<GenerationId>) -> Step {
    match (held, wanted) {
        (held, wanted) if held == wanted => Step::Keep,
        (_, None) => Step::Drop,
        _ => Step::Replace,
    }
}

/// Draws the video of `current` under the interface of `window`.
///
/// `VideoRenderer::new` needs the OpenGL context current, so the renderer is
/// created in the notifier, before the first frame that follows a change of
/// generation. Nothing is drawn outside the player screen.
pub fn install(window: &AppWindow, current: VideoSource) -> Result<(), SetRenderingNotifierError> {
    let weak = window.as_weak();
    let mut active: Option<Active> = None;
    let mut failed: Option<GenerationId> = None;
    let mut rendered = false;
    window
        .window()
        .set_rendering_notifier(move |state, graphics_api| match state {
            RenderingState::BeforeRendering => {
                rendered = false;
                let wanted = current.borrow().clone();
                let wanted_id = wanted.as_ref().map(|(id, _)| *id);
                match step(active.as_ref().map(|active| active.id), wanted_id) {
                    Step::Keep => {}
                    Step::Drop => active = None,
                    Step::Replace => {
                        active = None;
                        if let Some((id, player)) = wanted
                            && failed != Some(id)
                        {
                            match create(&weak, &player, graphics_api) {
                                Ok(renderer) => {
                                    active = Some(Active {
                                        id,
                                        renderer,
                                        _player: player,
                                    });
                                }
                                Err(error) => {
                                    log::error!("cannot create the video renderer: {error}");
                                    failed = Some(id);
                                }
                            }
                        }
                    }
                }
                let (Some(active), Some(window)) = (&active, weak.upgrade()) else {
                    return;
                };
                if window.get_screen() != Screen::Player {
                    return;
                }
                let size = window.window().size();
                match active.renderer.render(size.width, size.height) {
                    Ok(()) => rendered = true,
                    Err(error) => log::warn!("cannot render the video: {error}"),
                }
            }
            RenderingState::AfterRendering => {
                if rendered && let Some(active) = &active {
                    active.renderer.report_swap();
                }
            }
            RenderingState::RenderingTeardown => {
                active = None;
                failed = None;
            }
            _ => {}
        })
}

fn create(
    weak: &slint::Weak<AppWindow>,
    player: &Player,
    graphics_api: &GraphicsAPI<'_>,
) -> Result<VideoRenderer, Box<dyn std::error::Error>> {
    let GraphicsAPI::NativeOpenGL { get_proc_address } = graphics_api else {
        return Err("the Slint renderer does not use OpenGL".into());
    };
    let window = weak.upgrade().ok_or("the window is gone")?;
    let handle = window.window().window_handle();
    let display = handle.display_handle().ok();
    if display.is_none() {
        log::warn!("no native display, direct hardware decoding is unavailable");
    }
    let mut renderer = VideoRenderer::new(player, *get_proc_address, display)?;
    let weak = weak.clone();
    renderer.set_update_callback(move || {
        let _ = weak.upgrade_in_event_loop(|window| window.window().request_redraw());
    });
    Ok(renderer)
}

#[cfg(test)]
mod tests {
    use super::*;

    const G1: GenerationId = GenerationId(1);
    const G2: GenerationId = GenerationId(2);

    #[test]
    fn renderer_follows_the_published_generation() {
        assert_eq!(step(None, None), Step::Keep);
        assert_eq!(step(Some(G1), Some(G1)), Step::Keep);
        assert_eq!(step(None, Some(G1)), Step::Replace);
        assert_eq!(step(Some(G1), Some(G2)), Step::Replace);
        assert_eq!(step(Some(G1), None), Step::Drop);
    }
}
