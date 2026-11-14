//! Video drawn under the interface by the window's rendering notifier.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use oxyfin_player::{Player, VideoRenderer};
use raw_window_handle::HasDisplayHandle;
use slint::{ComponentHandle, GraphicsAPI, RenderingState, SetRenderingNotifierError};
use tokio::runtime::Handle;

use crate::state::GenerationId;
use crate::{AppWindow, Screen};

/// The player of the current generation, published by the application for the notifier.
pub type VideoSource = Rc<RefCell<Option<(GenerationId, Arc<Player>)>>>;

/// A renderer and the player it draws.
struct Active {
    id: GenerationId,
    renderer: VideoRenderer,
    player: Arc<Player>,
}

impl Active {
    /// Drops the renderer here, where the OpenGL context is current, then its player on `runtime`.
    fn release(self, runtime: &Handle) {
        let Active {
            renderer, player, ..
        } = self;
        drop(renderer);
        release_player(runtime, player);
    }
}

/// Drops `player` on `runtime`: dropping the last `Player` joins mpv's event
/// thread, which the Slint thread must never wait on.
pub fn release_player(runtime: &Handle, player: Arc<Player>) {
    runtime.spawn_blocking(move || drop(player));
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
/// generation. Nothing is drawn outside the player screen. The window's
/// `player-video-active` is set once the current renderer drew a frame, so the
/// player screen keeps an opaque backdrop until then. Players are released on
/// `runtime`.
pub fn install(
    window: &AppWindow,
    current: VideoSource,
    runtime: Handle,
) -> Result<(), SetRenderingNotifierError> {
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
                    Step::Drop => {
                        if let Some(old) = active.take() {
                            old.release(&runtime);
                        }
                        set_video_active(&weak, false);
                    }
                    Step::Replace => {
                        if let Some(old) = active.take() {
                            old.release(&runtime);
                        }
                        set_video_active(&weak, false);
                        if let Some((id, player)) = wanted
                            && failed != Some(id)
                        {
                            match create(&weak, &player, graphics_api) {
                                Ok(renderer) => {
                                    active = Some(Active {
                                        id,
                                        renderer,
                                        player,
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
                    Ok(()) => {
                        rendered = true;
                        if !window.get_player_video_active() {
                            window.set_player_video_active(true);
                        }
                    }
                    Err(error) => log::warn!("cannot render the video: {error}"),
                }
            }
            RenderingState::AfterRendering => {
                if rendered && let Some(active) = &active {
                    active.renderer.report_swap();
                }
            }
            RenderingState::RenderingTeardown => {
                if let Some(old) = active.take() {
                    old.release(&runtime);
                }
                failed = None;
            }
            _ => {}
        })
}

fn set_video_active(weak: &slint::Weak<AppWindow>, value: bool) {
    if let Some(window) = weak.upgrade() {
        window.set_player_video_active(value);
    }
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
