//! Throwaway prototype: mpv renders video underneath a Slint scene.

use std::ffi::{CStr, CString, c_void};
use std::path::PathBuf;
use std::process::ExitCode;

use glow::HasContext;
use libmpv2::Mpv;
use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};
use slint::{GraphicsAPI, RenderingState};

slint::include_modules!();

type SlintGetProcAddress<'a> = &'a dyn Fn(&CStr) -> *const c_void;

/// Points at Slint's `get_proc_address`, which only lives during the notifier call.
struct GlLoader(*const c_void);

fn get_proc_address(loader: &GlLoader, name: &str) -> *mut c_void {
    let Ok(name) = CString::new(name) else {
        return std::ptr::null_mut();
    };
    // SAFETY: the pointer targets a `SlintGetProcAddress` on the stack of the
    // notifier call that creates the render context. mpv resolves every GL
    // symbol inside `mpv_render_context_create`, and libmpv2 frees this loader
    // right after, so it is never dereferenced once that frame is gone.
    let slint_gpa = unsafe { *(loader.0 as *const SlintGetProcAddress<'_>) };
    slint_gpa(&name) as *mut c_void
}

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: video-underlay <video-file>");
        return ExitCode::from(2);
    };
    if !path.is_file() {
        eprintln!("file not found: {}", path.display());
        return ExitCode::from(2);
    }

    match run(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg".into())
        .require_opengl()
        .select()?;

    let mpv: &'static Mpv = Box::leak(Box::new(Mpv::with_initializer(|init| {
        init.set_option("vo", "libmpv")?;
        init.set_option("hwdec", "auto-safe")?;
        init.set_option("keep-open", "yes")?;
        Ok(())
    })?));

    let window = MainWindow::new()?;

    let weak = window.as_weak();
    window.on_toggle_fullscreen(move || {
        if let Some(window) = weak.upgrade() {
            let w = window.window();
            w.set_fullscreen(!w.is_fullscreen());
        }
    });

    let path = path
        .to_str()
        .ok_or("the video path is not valid UTF-8")?
        .to_owned();
    let weak = window.as_weak();
    let mut gl: Option<glow::Context> = None;
    let mut render_ctx: Option<RenderContext<'static>> = None;

    window
        .window()
        .set_rendering_notifier(move |state, graphics_api| {
            let result = match state {
                RenderingState::RenderingSetup => {
                    setup(mpv, graphics_api, &weak, &path).map(|(g, ctx)| {
                        gl = Some(g);
                        render_ctx = Some(ctx);
                    })
                }
                RenderingState::BeforeRendering => match (&gl, &render_ctx, weak.upgrade()) {
                    (Some(gl), Some(ctx), Some(window)) => draw(gl, ctx, &window),
                    _ => Ok(()),
                },
                RenderingState::AfterRendering => {
                    if let Some(ctx) = &render_ctx {
                        ctx.report_swap();
                    }
                    Ok(())
                }
                RenderingState::RenderingTeardown => {
                    render_ctx = None;
                    gl = None;
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(err) = result {
                eprintln!("error: {err}");
                let _ = slint::quit_event_loop();
            }
        })?;

    window.run()?;
    Ok(())
}

fn setup(
    mpv: &'static Mpv,
    graphics_api: &GraphicsAPI<'_>,
    weak: &slint::Weak<MainWindow>,
    path: &str,
) -> Result<(glow::Context, RenderContext<'static>), Box<dyn std::error::Error>> {
    let GraphicsAPI::NativeOpenGL {
        get_proc_address: slint_gpa,
    } = graphics_api
    else {
        return Err("the Slint renderer does not use OpenGL".into());
    };
    let slint_gpa: SlintGetProcAddress<'_> = *slint_gpa;

    // SAFETY: Slint's loader is valid for the duration of this notifier call.
    let gl = unsafe { glow::Context::from_loader_function_cstr(|name| slint_gpa(name)) };

    let loader = GlLoader(&slint_gpa as *const SlintGetProcAddress<'_> as *const c_void);
    let mut ctx = mpv.create_render_context([
        RenderParam::ApiType(RenderParamApiType::OpenGl),
        RenderParam::InitParams(OpenGLInitParams {
            get_proc_address,
            ctx: loader,
        }),
    ])?;

    let weak = weak.clone();
    ctx.set_update_callback(move || {
        let _ = weak.upgrade_in_event_loop(|window| window.window().request_redraw());
    });

    mpv.command("loadfile", &[path])?;
    Ok((gl, ctx))
}

fn draw(
    gl: &glow::Context,
    ctx: &RenderContext<'static>,
    window: &MainWindow,
) -> Result<(), Box<dyn std::error::Error>> {
    ctx.update()?;
    // SAFETY: called from the notifier, where Slint's GL context is current.
    let fbo = unsafe { gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING) };
    let size = window.window().size();
    // Slint can redraw without a new video frame, so the last frame is always drawn again.
    ctx.render::<GlLoader>(fbo, size.width as i32, size.height as i32, true)?;
    Ok(())
}
