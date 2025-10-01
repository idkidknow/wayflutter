mod callback;
mod error;
mod opengl;
mod task_runner;
mod view;

use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path, rc::Rc};

use anyhow::{Context, Result};
use error::FFIFlutterEngineResultExt;
use futures::{channel::mpsc::UnboundedSender, FutureExt, StreamExt};

use crate::{
    engine::{
        opengl::OpenGLState,
        task_runner::{TaskRunnerData, run_task_runner},
        view::ViewState,
    },
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
) -> Result<()> {
    let (task_runner_tx, task_runner_rx) = futures::channel::mpsc::unbounded();
    let task_runner_data = TaskRunnerData::new_on_current_thread(task_runner_tx);

    let (terminate_tx, mut terminate_rx) = futures::channel::mpsc::unbounded();

    let opengl_state = OpenGLState::init(&conn)?;

    let state = FlutterEngineState::new(FlutterEngineStateInner {
        terminate: terminate_tx,
        implicit_view_state: ViewState::new_layer_surface(&conn, &opengl_state)?,
        _wayland_connection: conn,
        opengl_state,
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

    let (configure_tx, mut configure_rx) = futures::channel::mpsc::channel::<Size>(1);
    {
        let state = unsafe { &*engine.state.0 };
        state
            .implicit_view_state
            .layer
            .set_on_configure(move |size| {
                let mut configure_tx = configure_tx.clone();
                let _ = configure_tx.try_send(size);
            });
    }
    let send_window_metrics_event = async move {
        loop {
            let Size { width, height } = configure_rx
                .next()
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

    engine.run()?;

    let catch_fatal_errors = async move {
        terminate_rx
            .next()
            .await
            .context("terminate event channel closed")?
            .context("fatal error")?;
        anyhow::Ok(())
    };

    let task_runner = run_task_runner(&engine, task_runner_rx);

    futures::select! {
        result = task_runner.fuse() => result?,
        result = catch_fatal_errors.fuse() => result?,
        result = send_window_metrics_event.fuse() => result?,
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
    terminate: UnboundedSender<anyhow::Result<()>>,
    _wayland_connection: Rc<WaylandConnection>,
    opengl_state: OpenGLState,
    implicit_view_state: ViewState,
    task_runner_data: TaskRunnerData,
}
