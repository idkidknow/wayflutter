use crate::engine::ffi;
use std::ffi::c_void;

use anyhow::Context;
use glutin::prelude::GlDisplay;

// `let state = unsafe { ... }` SAFETY: none of these callbacks borrows a mutable reference to the state

pub extern "C" fn make_current(user_data: *mut c_void) -> bool {
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    let ret = state.implicit_view_state.make_current();
    match ret {
        Ok(_) => true,
        Err(_) => {
            let _ = state
                .terminate
                .send_blocking(ret.context("Failed to make context current."));
            false
        }
    }
}

pub extern "C" fn clear_current(user_data: *mut c_void) -> bool {
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    let ret = state.implicit_view_state.clear_current();
    match ret {
        Ok(_) => true,
        Err(_) => {
            let _ = state
                .terminate
                .send_blocking(ret.context("Failed to clear context."));
            false
        }
    }
}

pub extern "C" fn make_resource_current(user_data: *mut c_void) -> bool {
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    let ret = state.implicit_view_state.make_resource_current();
    match ret {
        Ok(_) => true,
        Err(_) => {
            let _ = state
                .terminate
                .send_blocking(ret.context("Failed to make resource context current."));
            false
        }
    }
}

pub extern "C" fn gl_proc_resolver(user_data: *mut c_void, name: *const i8) -> *mut c_void {
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    let name = unsafe { std::ffi::CStr::from_ptr(name) };
    state.egl_display.get_proc_address(name) as *mut c_void
}

pub extern "C" fn present_with_info(
    user_data: *mut c_void,
    info: *const ffi::FlutterPresentInfo,
) -> bool {
    let state = unsafe { &*(user_data as *const super::FlutterEngineStateInner) };
    let info = unsafe { &*info };
    let damage = info.frame_damage;
    let rects = Vec::with_capacity(damage.num_rects);
    // for i in 0..damage.num_rects {
    //     let rect = unsafe { &*damage.damage.offset(i as isize) };
    //     // let rect = Rect::new(x, y, width, height); // TODO: Convert to Rect
    // }
    let ret = state.implicit_view_state.swap_buffers_with_damage(&rects);
    match ret {
        Ok(_) => true,
        Err(_) => {
            let _ = state
                .terminate
                .send_blocking(ret.context("Failed to swap buffers."));
            false
        }
    }
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
    let _ = state
        .task_runner_data
        .tx
        .send_blocking((task, target_time_nanos)); // unbound channel
}
