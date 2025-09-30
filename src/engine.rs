mod callback;
mod error;
mod view;

use std::{
    ffi::CString, os::unix::ffi::OsStrExt, path::Path, ptr::NonNull, rc::Rc, time::Duration,
};

use anyhow::{Context, Result};
use error::FFIFlutterEngineResultExt;
use futures::FutureExt;
use glutin::api::egl;
use raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle};
use smol::LocalExecutor;
use wayland_client::Proxy;

use crate::{
    engine::view::ViewState,
    wayland::{WaylandConnection, layer_shell::Size},
};

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
    executor: &LocalExecutor<'_>,
) -> Result<()> {
    let egl_display = get_egl_display(&conn)?;

    let (task_runner_tx, task_runner_rx) = smol::channel::unbounded::<(ffi::FlutterTask, u64)>();
    let task_runner_data = TaskRunnerData::new_on_current_thread(task_runner_tx);

    let (terminate_tx, terminate_rx) = smol::channel::unbounded();

    let state = FlutterEngineState::new(FlutterEngineStateInner {
        terminate: terminate_tx,
        implicit_view_state: ViewState::new_layer_surface(&conn, &egl_display)?,
        _wayland_connection: conn,
        egl_display: egl_display,
        task_runner_data,
    });

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

    let platform_task_runner = ffi::FlutterTaskRunnerDescription {
        struct_size: size_of::<ffi::FlutterTaskRunnerDescription>(),
        user_data: unsafe { &*state.0 } as *const _ as _,
        runs_task_on_current_thread_callback: Some(callback::runs_task_on_current_thread_callback),
        post_task_callback: Some(callback::post_task_callback),
        identifier: 1,
        destruction_callback: None,
    };

    let custom_task_runners = ffi::FlutterCustomTaskRunners {
        struct_size: size_of::<ffi::FlutterCustomTaskRunners>(),
        platform_task_runner: &platform_task_runner as _,
        render_task_runner: std::ptr::null(),
        thread_priority_setter: None,
        ui_task_runner: std::ptr::null(),
    };

    let project_args = unsafe {
        ffi::FlutterProjectArgs {
            struct_size: size_of::<ffi::FlutterProjectArgs>(),
            assets_path: asset_path.as_ptr(),
            icu_data_path: icu_data_path.as_ptr(),
            log_message_callback: Some(callback::log_message_callback),
            custom_task_runners: &custom_task_runners as _,
            ..core::mem::zeroed()
        }
    };

    log::info!("init flutter engine");
    let engine = FlutterEngine::init(state, &renderer_config, &project_args)?;

    let (configure_tx, configure_rx) = smol::channel::bounded::<Size>(1);
    {
        let state = unsafe { &*engine.state.0 };
        state
            .implicit_view_state
            .layer()
            .set_on_configure(move |size| {
                let _ = configure_tx.force_send(size);
            });
    }
    let send_window_metrics_event = async move {
        loop {
            let Size { width, height } = configure_rx
                .recv()
                .await
                .context("implicit view's configure event channel closed")?;
            let event = ffi::FlutterWindowMetricsEvent {
                struct_size: size_of::<ffi::FlutterWindowMetricsEvent>(),
                width: width as usize,
                height: height as usize,
                pixel_ratio: 1.0,
                left: 0,
                top: 0,
                physical_view_inset_top: 0.0,
                physical_view_inset_right: 0.0,
                physical_view_inset_bottom: 0.0,
                physical_view_inset_left: 0.0,
                display_id: 0,
                view_id: 0,
            };
            unsafe {
                ffi::FlutterEngineSendWindowMetricsEvent(engine.engine, &event as _)
                    .into_flutter_engine_result()?
            }
        }
        #[allow(unused)]
        anyhow::Ok(())
    };
    executor.spawn(send_window_metrics_event).detach();

    engine.run()?;

    let catch_fatal_errors = async move {
        terminate_rx.recv().await?.context("fatal error")?;
        anyhow::Ok(())
    };

    let task_runner = async move {
        loop {
            let (task, target_time) = task_runner_rx.recv().await?;
            let now = unsafe { ffi::FlutterEngineGetCurrentTime() };
            let delay = target_time.saturating_sub(now);
            let engine_ptr = engine.engine;
            if delay == 0 {
                unsafe {
                    ffi::FlutterEngineRunTask(engine_ptr, &task as _)
                        .into_flutter_engine_result()?;
                }
            } else {
                let future = async move {
                    smol::Timer::after(Duration::from_nanos(delay)).await;
                    unsafe {
                        ffi::FlutterEngineRunTask(engine_ptr, &task as _)
                            .into_flutter_engine_result()
                    }
                };
                executor.spawn(future).detach();
            }
        }
        #[allow(unused)]
        anyhow::Ok(())
    };

    futures::select! {
        result = task_runner.fuse() => result?,
        result = catch_fatal_errors.fuse() => result?,
    }

    anyhow::Ok(())
}

struct FlutterEngine {
    engine: ffi::FlutterEngine,
    state: FlutterEngineState,
}

impl FlutterEngine {
    fn init(
        state: FlutterEngineState,
        renderer_config: &ffi::FlutterRendererConfig,
        project_args: &ffi::FlutterProjectArgs,
    ) -> Result<Self> {
        let engine = unsafe {
            let mut engine: ffi::FlutterEngine = std::ptr::null_mut();
            let engine_out: *mut ffi::FlutterEngine = &mut engine as *mut _;
            ffi::FlutterEngineInitialize(
                ffi::FLUTTER_ENGINE_VERSION as usize,
                renderer_config as _,
                project_args as _,
                state.0 as _,
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
        }
    }
}

struct FlutterEngineState(*const FlutterEngineStateInner);

impl FlutterEngineState {
    fn new(inner: FlutterEngineStateInner) -> Self {
        Self(Box::into_raw(Box::new(inner)))
    }
}

impl Drop for FlutterEngineState {
    fn drop(&mut self) {
        let _ = unsafe { Box::from_raw(self.0 as *mut FlutterEngineStateInner) };
    }
}

/// Read only. Need interior mutability if necessary.
struct FlutterEngineStateInner {
    terminate: smol::channel::Sender<anyhow::Result<()>>,
    _wayland_connection: Rc<WaylandConnection>,
    egl_display: egl::display::Display,
    implicit_view_state: ViewState,
    task_runner_data: TaskRunnerData,
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

struct TaskRunnerData {
    tx: smol::channel::Sender<(ffi::FlutterTask, u64)>,
    main_thread: std::thread::ThreadId,
}

impl TaskRunnerData {
    fn new_on_current_thread(tx: smol::channel::Sender<(ffi::FlutterTask, u64)>) -> Self {
        Self {
            tx,
            main_thread: std::thread::current().id(),
        }
    }
}
