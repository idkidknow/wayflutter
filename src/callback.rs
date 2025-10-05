use std::ffi::c_void;
use std::time::Duration;

use anyhow::Context;
use glutin::prelude::GlDisplay;

use crate::error::FFIFlutterEngineResultExt;
use crate::error_in_callback;
use crate::ffi;

// `let state = unsafe { ... }` SAFETY: none of these callbacks borrows a mutable reference to the state

pub extern "C" fn make_current(user_data: *mut c_void) -> bool {
  let state = unsafe { &*(user_data as *const super::FlutterEngineState) };
  error_in_callback!(state, state.opengl_state.make_current_no_surface());
  true
}

pub extern "C" fn clear_current(user_data: *mut c_void) -> bool {
  let state = unsafe { &*(user_data as *const super::FlutterEngineState) };
  error_in_callback!(state, state.opengl_state.make_not_current());
  true
}

pub extern "C" fn make_resource_current(user_data: *mut c_void) -> bool {
  let state = unsafe { &*(user_data as *const super::FlutterEngineState) };
  let context = &state.opengl_state.resource_context;
  error_in_callback!(
    state,
    context
      .make_current_surfaceless()
      .context("Failed to make resource context current.")
  );
  true
}

pub extern "C" fn gl_proc_resolver(user_data: *mut c_void, name: *const i8) -> *mut c_void {
  let state = unsafe { &*(user_data as *const super::FlutterEngineState) };
  let name = unsafe { std::ffi::CStr::from_ptr(name) };
  state.opengl_state.egl_display.get_proc_address(name) as *mut c_void
}

pub extern "C" fn present_with_info(
  _user_data: *mut c_void,
  _info: *const ffi::FlutterPresentInfo,
) -> bool {
  // We have provided FlutterCompositor, so the engine shouldn't call this.
  false
}

pub extern "C" fn fbo_with_frame_info_callback(
  _state: *mut c_void,
  _info: *const ffi::FlutterFrameInfo,
) -> u32 {
  0
}

pub extern "C" fn log_message_callback(
  tag: *const i8,
  message: *const i8,
  _user_data: *mut c_void,
) {
  let tag = unsafe { std::ffi::CStr::from_ptr(tag) };
  let message = unsafe { std::ffi::CStr::from_ptr(message) };
  log::info!(
    "[{}] {}",
    tag.to_str().unwrap_or("<invalid utf8>"),
    message.to_str().unwrap_or("<invalid utf8>")
  );
}

pub extern "C" fn runs_task_on_current_thread_callback(user_data: *mut c_void) -> bool {
  let state = unsafe { &*(user_data as *const super::FlutterEngineState) };
  state.platform_thread_id == std::thread::current().id()
}

pub extern "C" fn post_task_callback(
  task: ffi::FlutterTask,
  target_time_nanos: u64,
  user_data: *mut c_void,
) {
  struct TaskWrapper(ffi::FlutterTask);
  unsafe impl Send for TaskWrapper {}

  let state = unsafe { &*(user_data as *const super::FlutterEngineState) };
  let now = unsafe { ffi::FlutterEngineGetCurrentTime() };
  let delay = target_time_nanos.saturating_sub(now);
  let delay = Duration::from_nanos(delay);
  let task_wrapped = TaskWrapper(task);
  let ret = state.task_runner_handle.post_task_after(
    move |engine| {
      let task = task_wrapped;
      unsafe {
        let ret = ffi::FlutterEngineRunTask(engine.engine, &task.0).into_flutter_engine_result();
        if let Err(e) = ret {
          log::error!("failed to run the task posted by the engine: {}", e);
        }
      }
    },
    delay,
  );
  error_in_callback!(state, ret, return ());
}
