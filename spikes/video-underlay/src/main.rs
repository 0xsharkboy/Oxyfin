//! Throwaway prototype: mpv renders video underneath a Slint scene.

use std::ffi::{CStr, CString, c_void};
use std::path::PathBuf;
use std::process::ExitCode;

use glow::HasContext;
use libmpv2::Mpv;
use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
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

    let weak = window.as_weak();
    window.on_toggle_pause(move || {
        let Some(window) = weak.upgrade() else { return };
        let paused = mpv.get_property::<bool>("pause").unwrap_or(false);
        match mpv.set_property("pause", !paused) {
            Ok(()) => window.set_paused(!paused),
            Err(err) => eprintln!("error: pause: {err}"),
        }
    });
    window.on_seek(move |value| {
        let percent = (value * 100.0).to_string();
        if let Err(err) = mpv.command("seek", &[&percent, "absolute-percent"]) {
            eprintln!("error: seek: {err}");
        }
    });

    let weak = window.as_weak();
    let sync_timer = slint::Timer::default();
    sync_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(250),
        move || {
            let Some(window) = weak.upgrade() else { return };
            if let Ok(percent) = mpv.get_property::<f64>("percent-pos") {
                window.set_position((percent / 100.0) as f32);
            }
            if let Ok(paused) = mpv.get_property::<bool>("pause") {
                window.set_paused(paused);
            }
        },
    );

    let weak = window.as_weak();
    let stats_timer = slint::Timer::default();
    stats_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(2),
        move || {
            let Some(window) = weak.upgrade() else { return };
            let line = stats_line(mpv, &window);
            println!("{line}");
            window.set_stats(line.into());
        },
    );

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

fn stats_line(mpv: &Mpv, window: &MainWindow) -> String {
    fn show<T: std::fmt::Display>(value: Option<T>) -> String {
        value.map_or_else(|| "-".to_owned(), |v| v.to_string())
    }
    let hwdec = mpv
        .get_property::<String>("hwdec-current")
        .ok()
        .filter(|h| !h.is_empty());
    let video = match (
        mpv.get_property::<i64>("video-params/w").ok(),
        mpv.get_property::<i64>("video-params/h").ok(),
    ) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        _ => "-".to_owned(),
    };
    let drop = mpv.get_property::<i64>("frame-drop-count").ok();
    let delayed = mpv.get_property::<i64>("vo-delayed-frame-count").ok();
    let fps = mpv
        .get_property::<f64>("estimated-vf-fps")
        .ok()
        .map(|f| format!("{f:.2}"));
    let size = window.window().size();
    let scale = window.window().scale_factor();
    format!(
        "hwdec={} video={video} drop={} delayed={} fps={} window={}x{} scale={scale:.2}",
        show(hwdec),
        show(drop),
        show(delayed),
        show(fps),
        size.width,
        size.height,
    )
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
    let mut params = vec![
        RenderParam::ApiType(RenderParamApiType::OpenGl),
        RenderParam::InitParams(OpenGLInitParams {
            get_proc_address,
            ctx: loader,
        }),
    ];
    match weak.upgrade().and_then(|window| native_display(&window)) {
        Some(display) => params.push(display),
        None => eprintln!("warning: no Wayland or Xlib display, direct hwdec is unavailable"),
    }
    let mut ctx = mpv.create_render_context(params)?;

    let weak = weak.clone();
    ctx.set_update_callback(move || {
        let _ = weak.upgrade_in_event_loop(|window| window.window().request_redraw());
    });

    mpv.command("loadfile", &[path])?;
    Ok((gl, ctx))
}

fn native_display(window: &MainWindow) -> Option<RenderParam<GlLoader>> {
    let handle = window.window().window_handle();
    match handle.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(h) => Some(RenderParam::WaylandDisplay(
            h.display.as_ptr() as *const c_void
        )),
        RawDisplayHandle::Xlib(h) => h
            .display
            .map(|d| RenderParam::X11Display(d.as_ptr() as *const c_void)),
        _ => None,
    }
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
