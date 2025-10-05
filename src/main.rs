mod callback;
mod compositor;
mod error;
mod opengl;
mod task_runner;
mod wayland;
#[macro_use]
mod macros;

use crate::{
    compositor::Compositor,
    opengl::OpenGLState,
    task_runner::{TaskRunnerHandle, make_task_runner},
    wayland::WaylandClient,
};
use anyhow::{Context, Result};
use error::FFIFlutterEngineResultExt;
use futures::{FutureExt, StreamExt, channel::mpsc::UnboundedSender};
use std::{cell::Cell, ffi::CString, os::unix::ffi::OsStrExt, path::Path, thread::ThreadId};
use std::{ffi::c_void, mem::MaybeUninit, path::PathBuf};

mod ffi {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(dead_code)]

    include!(concat!(env!("OUT_DIR"), "/embedder_bindings.rs"));
}

fn main() -> Result<()> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .try_init()?;

    let args = std::env::args().collect::<Vec<_>>();
    let asset_path = PathBuf::from(args.get(1).expect("no asset path given"));
    let icu_data_path = PathBuf::from(args.get(2).expect("no icu data path given"));

    smol::block_on(async { run_flutter(&asset_path, &icu_data_path).await })
}

pub async fn run_flutter(asset_path: &Path, icu_data_path: &Path) -> Result<()> {
    log::info!("init flutter engine");
    let engine = FlutterEngine::init(asset_path, icu_data_path)?;

    let conn = wayland_client::Connection::connect_to_env()?;

    let (terminate_tx, mut terminate_rx) = futures::channel::mpsc::unbounded();

    let opengl_state = OpenGLState::init(&conn)?;

    let wayland_client = WaylandClient::new(&conn, &engine)?;

    let compositor = Compositor::init(&wayland_client, &opengl_state)?;

    let (task_runner, task_runner_handle) = make_task_runner(&engine);

    unsafe {
        engine.init_state(FlutterEngineState {
            terminate: terminate_tx,
            compositor,
            opengl_state,
            task_runner_handle,
            platform_thread_id: std::thread::current().id(),
        });

        engine.run()?;
    }

    let catch_fatal_errors = async move {
        terminate_rx
            .next()
            .await
            .context("terminate event channel closed")?
            .context("fatal error")?;
        anyhow::Ok(())
    };

    futures::select! {
        result = wayland_client.run().fuse() => { result?; },
        result = catch_fatal_errors.fuse() => result?,
        result = task_runner.fuse() => { result?; },
    }

    anyhow::Ok(())
}

struct FlutterEngine {
    engine: *mut ffi::_FlutterEngine,
    state: *mut FlutterEngineState,
    state_initialized: Cell<bool>,
}

impl Drop for FlutterEngine {
    fn drop(&mut self) {
        unsafe {
            let _ = ffi::FlutterEngineDeinitialize(self.engine);
            let state = Box::from_raw(self.state as *mut MaybeUninit<FlutterEngineState>);
            if self.state_initialized.get() {
                drop(state.assume_init());
            }
        }
    }
}

impl FlutterEngine {
    /// setup config and project args and initialize the engine
    fn init(asset_path: &Path, icu_data_path: &Path) -> Result<Self> {
        let state = Box::<FlutterEngineState>::new_uninit();
        let mut ret = Self {
            engine: std::ptr::null_mut(),
            state: Box::into_raw(state) as _,
            state_initialized: Cell::new(false),
        };

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

        let flutter_compositor = ffi::FlutterCompositor {
            struct_size: size_of::<ffi::FlutterCompositor>(),
            user_data: ret.state as *mut c_void,
            create_backing_store_callback: Some(
                compositor::callback::create_backing_store_callback,
            ),
            collect_backing_store_callback: Some(
                compositor::callback::collect_backing_store_callback,
            ),
            present_layers_callback: None,
            avoid_backing_store_cache: false,
            present_view_callback: Some(compositor::callback::present_view_callback),
        };

        let asset_path = CString::new(asset_path.as_os_str().as_bytes())?;
        let icu_data_path = CString::new(icu_data_path.as_os_str().as_bytes())?;

        let platform_task_runner = ffi::FlutterTaskRunnerDescription {
            struct_size: size_of::<ffi::FlutterTaskRunnerDescription>(),
            user_data: ret.state as *mut c_void,
            runs_task_on_current_thread_callback: Some(
                callback::runs_task_on_current_thread_callback,
            ),
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
                compositor: &flutter_compositor as _,
                ..core::mem::zeroed()
            }
        };

        log::info!("init flutter engine");
        let engine = flutter_engine_init(ret.state as _, &renderer_config, &project_args)?;
        ret.engine = engine;
        Ok(ret)
    }

    /// Must not call twice
    unsafe fn init_state(&self, state: FlutterEngineState) {
        unsafe {
            self.state.write(state);
        }
        self.state_initialized.set(true);
    }

    /// Must have called `init_state`
    unsafe fn get_state(&self) -> &FlutterEngineState {
        unsafe { &*self.state }
    }

    unsafe fn run(&self) -> Result<()> {
        log::info!("run flutter engine");
        unsafe {
            ffi::FlutterEngineRunInitialized(self.engine).into_flutter_engine_result()?;
        }
        Ok(())
    }

    fn schedule_frame(&self) -> Result<()> {
        unsafe {
            ffi::FlutterEngineScheduleFrame(self.engine).into_flutter_engine_result()?;
        }
        Ok(())
    }
}

fn flutter_engine_init(
    user_data: *const c_void,
    renderer_config: &ffi::FlutterRendererConfig,
    project_args: &ffi::FlutterProjectArgs,
) -> Result<ffi::FlutterEngine> {
    unsafe {
        let mut engine: ffi::FlutterEngine = std::ptr::null_mut();
        let engine_out: *mut ffi::FlutterEngine = &mut engine as *mut _;
        ffi::FlutterEngineInitialize(
            ffi::FLUTTER_ENGINE_VERSION as usize,
            renderer_config as _,
            project_args as _,
            user_data as _,
            engine_out,
        )
        .into_flutter_engine_result()?;
        Ok(engine)
    }
}

/// Read only. Need interior mutability if necessary.
struct FlutterEngineState
where
    Self: Sync,
{
    terminate: UnboundedSender<anyhow::Result<()>>,
    opengl_state: OpenGLState,
    compositor: Compositor,
    task_runner_handle: TaskRunnerHandle,
    platform_thread_id: ThreadId,
}
