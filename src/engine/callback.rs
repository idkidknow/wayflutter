use crate::engine::{ffi, task_runner::PendingTask};
use std::ffi::c_void;

use anyhow::Context;
use glutin::prelude::{GlDisplay, PossiblyCurrentGlContext};

/// Sends termination signal to the main event loop and returns false if $result is an error.
macro_rules! error_in_callback {
    ($state:ident, $result:expr) => {
        match $result {
            Ok(v) => v,
            Err(e) => {
                let _ = $state
                    .terminate
                    .unbounded_send(::anyhow::Result::Err(::anyhow::Error::from(e)));
                return false;
            }
        }
    };
}

// `let state = unsafe { ... }` SAFETY: none of these callbacks borrows a mutable reference to the state

pub extern "C" fn make_current(user_data: *mut c_void) -> bool {
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    let context = &state.opengl_state.render_context;
    let surface = &state.implicit_view_state.surface;
    error_in_callback!(
        state,
        context
            .make_current(surface)
            .context("Failed to make context current.")
    );
    true
}

pub extern "C" fn clear_current(user_data: *mut c_void) -> bool {
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    let context = &state.opengl_state.render_context;
    error_in_callback!(
        state,
        context
            .make_not_current_in_place()
            .context("Failed to clear context.")
    );
    true
}

pub extern "C" fn make_resource_current(user_data: *mut c_void) -> bool {
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
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
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    let name = unsafe { std::ffi::CStr::from_ptr(name) };
    state.opengl_state.egl_display.get_proc_address(name) as *mut c_void
}

pub extern "C" fn present_with_info(
    user_data: *mut c_void,
    info: *const ffi::FlutterPresentInfo,
) -> bool {
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    let context = &state.opengl_state.render_context;
    let surface = &state.implicit_view_state.surface;

    let info = unsafe { &*info };
    let damage = info.frame_damage;
    let rects = Vec::with_capacity(damage.num_rects);
    // for i in 0..damage.num_rects {
    //     let rect = unsafe { &*damage.damage.offset(i as isize) };
    //     // let rect = Rect::new(x, y, width, height); // TODO: Convert to Rect
    // }
    error_in_callback!(
        state,
        surface
            .swap_buffers_with_damage(&context, &rects)
            .context("Failed to swap buffers.")
    );
    true
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
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    state.task_runner_data.main_thread == std::thread::current().id()
}

pub extern "C" fn post_task_callback(
    task: ffi::FlutterTask,
    target_time_nanos: u64,
    user_data: *mut c_void,
) {
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    let _ = state.task_runner_data.tx.unbounded_send(PendingTask {
        task,
        target_nanos: target_time_nanos,
    });
}
