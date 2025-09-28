mod callback;
mod error;
mod view;

use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path, ptr::NonNull, rc::Rc};

use anyhow::{Context, Result};
use error::FFIFlutterEngineResultExt;
use glutin::api::egl;
use raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle};
use wayland_client::Proxy;

use crate::{engine::view::ViewState, wayland::WaylandConnection};

mod ffi {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(dead_code)]

    include!(concat!(env!("OUT_DIR"), "/embedder_bindings.rs"));
}

pub async fn run_flutter(
    conn: Rc<WaylandConnection>,
    asset_path: &Path,
    icu_data_path: &Path,
) -> Result<()> {
    let renderer_config = ffi::FlutterRendererConfig {
        type_: ffi::FlutterRendererType_kOpenGL,
        __bindgen_anon_1: ffi::FlutterRendererConfig__bindgen_ty_1 {
            open_gl: ffi::FlutterOpenGLRendererConfig {
                struct_size: size_of::<ffi::FlutterOpenGLRendererConfig>(),
                make_current: Some(callback::make_current),
                clear_current: Some(callback::clear_current),
                present: None,
                fbo_callback: None,
                make_resource_current: Some(callback::make_resource_current),
                fbo_reset_after_present: false,
                surface_transformation: None,
                gl_proc_resolver: Some(callback::gl_proc_resolver),
                gl_external_texture_frame_callback: None,
                fbo_with_frame_info_callback: Some(callback::fbo_with_frame_info_callback),
                present_with_info: Some(callback::present_with_info),
                populate_existing_damage: None,
            },
        },
    };

    let asset_path = CString::new(asset_path.as_os_str().as_bytes())?;
    let icu_data_path = CString::new(icu_data_path.as_os_str().as_bytes())?;

    let project_args = unsafe {
        ffi::FlutterProjectArgs {
            struct_size: size_of::<ffi::FlutterProjectArgs>(),
            assets_path: asset_path.as_ptr(),
            icu_data_path: icu_data_path.as_ptr(),
            log_message_callback: Some(callback::log_message_callback),
            ..core::mem::zeroed()
        }
    };

    let egl_display = get_egl_display(&conn)?;

    let state = FlutterEngineState {
        implicit_view_state: ViewState::new_layer_surface(&conn, &egl_display)?,
        _wayland_connection: conn,
        egl_display: egl_display,
    };

    log::info!("init flutter engine");
    let engine = FlutterEngine::init(state, &renderer_config, &project_args)?;
    engine.run()?;

    smol::Timer::never().await;

    Ok(())
}

struct FlutterEngine {
    engine: ffi::FlutterEngine,
    /// do not borrow &mut
    state: *const FlutterEngineState,
}

impl FlutterEngine {
    fn init(
        state: FlutterEngineState,
        renderer_config: &ffi::FlutterRendererConfig,
        project_args: &ffi::FlutterProjectArgs,
    ) -> Result<Self> {
        let state = Box::into_raw(Box::new(state));

        let engine = unsafe {
            let mut engine: ffi::FlutterEngine = std::ptr::null_mut();
            let engine_out: *mut ffi::FlutterEngine = &mut engine as *mut _;
            ffi::FlutterEngineInitialize(
                ffi::FLUTTER_ENGINE_VERSION as usize,
                renderer_config as _,
                project_args as _,
                state as _,
                engine_out,
            )
            .into_flutter_engine_result()?;
            engine
        };

        Ok(FlutterEngine { engine, state })
    }

    fn run(&self) -> Result<()> {
        unsafe {
            log::info!("run flutter engine");
            ffi::FlutterEngineRunInitialized(self.engine).into_flutter_engine_result()?;
        }
        Ok(())
    }
}

impl Drop for FlutterEngine {
    fn drop(&mut self) {
        unsafe {
            ffi::FlutterEngineDeinitialize(self.engine);
            let _ = Box::from_raw(self.state as *mut FlutterEngineState);
        }
    }
}

struct FlutterEngineState {
    _wayland_connection: Rc<WaylandConnection>,
    egl_display: egl::display::Display,
    implicit_view_state: ViewState,
}

fn get_egl_display(conn: &WaylandConnection) -> Result<egl::display::Display> {
    // SAFETY: trust `wayland-client` crate and `libwayland`...
    let display = unsafe {
        let display = NonNull::new(conn.wl_display().id().as_ptr() as _)
            .context("null wl_display pointer")?;
        egl::display::Display::new(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            display,
        )))
        .context("failed to create EGL display")?
    };
    Ok(display)
}
