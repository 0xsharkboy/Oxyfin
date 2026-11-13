//! Video rendering through the mpv OpenGL render API.

use std::ffi::{CStr, CString, c_void};
use std::marker::PhantomData;
use std::sync::Arc;

use glow::HasContext;
use libmpv2::Mpv;
use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};
use raw_window_handle::{DisplayHandle, RawDisplayHandle};

use crate::error::describe;
use crate::{Player, PlayerError, VideoOutput};

type GetProcAddress<'a> = &'a dyn Fn(&CStr) -> *const c_void;

/// Points at the `get_proc_address` given to [`VideoRenderer::new`], which
/// only lives during that call.
struct GlLoader(*const c_void);

fn load_gl_symbol(loader: &GlLoader, name: &str) -> *mut c_void {
    let Ok(name) = CString::new(name) else {
        return std::ptr::null_mut();
    };
    // SAFETY: `loader.0` points at the `get_proc_address` argument of
    // `VideoRenderer::new`, alive for the whole call. mpv resolves every GL
    // symbol inside `mpv_render_context_create`, and libmpv2 frees the loader
    // as soon as that returns, so this never runs once `new` has returned.
    let get_proc_address = unsafe { *loader.0.cast::<GetProcAddress<'_>>() };
    get_proc_address(&name).cast_mut()
}

/// Draws the video of a [`Player`] into the current OpenGL framebuffer.
///
/// A `VideoRenderer` must be created, used and dropped on the thread where
/// the OpenGL context is current, while it is current. With Slint, that means
/// creating it in `RenderingState::RenderingSetup`, calling [`render`] in
/// `BeforeRendering`, [`report_swap`] in `AfterRendering`, and dropping it in
/// `RenderingTeardown`. The `get_proc_address` function Slint provides is only
/// valid inside those callbacks, and the type is not `Send`, which keeps it on
/// that thread.
///
/// ```compile_fail,E0277
/// fn assert_send<T: Send>() {}
/// assert_send::<oxyfin_player::VideoRenderer>();
/// ```
///
/// The renderer keeps the mpv core alive, and dropping it frees its render
/// context before the core can be destroyed. Rendering and loading are
/// independent: a renderer can be dropped and created again (for example when
/// the window is hidden and shown) without reloading the media.
///
/// [`render`]: VideoRenderer::render
/// [`report_swap`]: VideoRenderer::report_swap
pub struct VideoRenderer {
    // Fields drop in declaration order: the render context must go before the
    // `Arc` that keeps the core it borrows alive.
    ctx: RenderContext<'static>,
    gl: glow::Context,
    _mpv: Arc<Mpv>,
    _not_send: PhantomData<*const ()>,
}

impl VideoRenderer {
    /// Creates a renderer for `player`.
    ///
    /// Must be called on the thread where the OpenGL context is current,
    /// while it is current (see the type documentation).
    ///
    /// `get_proc_address` resolves OpenGL symbols of the current context; it
    /// is only used during this call. `display` is the native display of the
    /// window: a Wayland or Xlib display lets mpv decode straight into GPU
    /// memory, any other display is ignored. mpv keeps using it after this
    /// call, so the display must outlive the renderer, which holds for the
    /// display of the window being rendered to.
    ///
    /// If a media is already loaded with no video track selected, its default
    /// (or first) video track is selected again: mpv disables video when it
    /// has no render context, while loading or when the previous renderer was
    /// dropped during playback. The selection lasts for that media only.
    ///
    /// Fails with [`PlayerError::Render`] when the player was created with
    /// [`VideoOutput::None`] or when mpv cannot create the render context, and
    /// with the error of the track list or of the `vid` property when the
    /// video track cannot be selected again; the renderer is then dropped.
    pub fn new(
        player: &Player,
        get_proc_address: &dyn Fn(&CStr) -> *const c_void,
        display: Option<DisplayHandle<'_>>,
    ) -> Result<Self, PlayerError> {
        if player.video_output() == VideoOutput::None {
            return Err(PlayerError::Render(
                "the player has no embedded video output".to_owned(),
            ));
        }
        let mpv = Arc::clone(player.mpv());

        // SAFETY: the caller made the OpenGL context current on this thread
        // (type contract), and `get_proc_address` resolves its symbols.
        let gl = unsafe { glow::Context::from_loader_function_cstr(get_proc_address) };

        let loader = GlLoader((&raw const get_proc_address).cast());
        let mut params = vec![
            RenderParam::ApiType(RenderParamApiType::OpenGl),
            RenderParam::InitParams(OpenGLInitParams {
                get_proc_address: load_gl_symbol,
                ctx: loader,
            }),
        ];
        if let Some(display) = display.and_then(|display| native_display(display.as_raw())) {
            params.push(display);
        }

        // SAFETY: the `Mpv` lives in the heap allocation of the `Arc`, which
        // never moves and is kept alive by the `_mpv` field of the returned
        // renderer. `ctx` is declared before `_mpv`, so the context is freed
        // before that `Arc` is released, and the borrow never outlives the core.
        let core: &'static Mpv = unsafe { &*Arc::as_ptr(&mpv) };
        let ctx = core
            .create_render_context(params)
            .map_err(|e| PlayerError::Render(describe(&e)))?;

        let renderer = Self {
            ctx,
            gl,
            _mpv: mpv,
            _not_send: PhantomData,
        };
        player.restore_video()?;
        Ok(renderer)
    }

    /// Draws the latest video frame into the framebuffer currently bound for
    /// drawing, at `width` x `height` physical pixels.
    ///
    /// The previous frame is drawn again when there is no new one, so this
    /// can be called on every redraw of the window.
    pub fn render(&self, width: u32, height: u32) -> Result<(), PlayerError> {
        let width = i32::try_from(width).map_err(|e| PlayerError::Render(e.to_string()))?;
        let height = i32::try_from(height).map_err(|e| PlayerError::Render(e.to_string()))?;
        self.ctx
            .update()
            .map_err(|e| PlayerError::Render(describe(&e)))?;
        // SAFETY: the OpenGL context `gl` was loaded from is current on this
        // thread (type contract), and reading an integer state has no other
        // requirement.
        let fbo = unsafe { self.gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING) };
        self.ctx
            .render::<()>(fbo, width, height, true)
            .map_err(|e| PlayerError::Render(describe(&e)))
    }

    /// Tells mpv that the frame was presented. Call it after the buffers are
    /// swapped.
    pub fn report_swap(&self) {
        self.ctx.report_swap();
    }

    /// Sets the function called when mpv has a new frame to draw, typically to
    /// request a redraw of the window.
    ///
    /// It runs on an mpv thread and must not call into the player. It must not
    /// panic either: it runs inside an `extern "C"` wrapper, so a panic aborts
    /// the process.
    pub fn set_update_callback(&mut self, callback: impl Fn() + Send + 'static) {
        self.ctx.set_update_callback(callback);
    }
}

impl Drop for VideoRenderer {
    fn drop(&mut self) {
        // libmpv2 frees the update callback before the render context, so mpv
        // could still call it; a capture-free callback has nothing to free.
        self.ctx.set_update_callback(|| {});
    }
}

fn native_display(display: RawDisplayHandle) -> Option<RenderParam<GlLoader>> {
    match display {
        RawDisplayHandle::Wayland(handle) => Some(RenderParam::WaylandDisplay(
            handle.display.as_ptr().cast_const(),
        )),
        RawDisplayHandle::Xlib(handle) => handle
            .display
            .map(|display| RenderParam::X11Display(display.as_ptr().cast_const())),
        _ => None,
    }
}
