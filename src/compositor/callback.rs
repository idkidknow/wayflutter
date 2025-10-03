use glutin::surface::GlSurface;

use crate::compositor::FlutterViewKind;
use crate::{FlutterEngineStateInner, ffi};
use crate::error_in_callback;
use std::ffi::c_void;

pub extern "C" fn create_backing_store_callback(
    config: *const ffi::FlutterBackingStoreConfig,
    backing_store_out: *mut ffi::FlutterBackingStore,
    user_data: *mut c_void,
) -> bool {
    let state = unsafe { &*(user_data as *const FlutterEngineStateInner) };

    let backing_store = unsafe { &mut *backing_store_out };
    if backing_store.struct_size < size_of::<ffi::FlutterBackingStore>() {
        let ret = anyhow::Result::<()>::Err(anyhow::anyhow!("Invalid backing store ABI"));
        error_in_callback!(state, ret);
    }

    let config = unsafe { &*config };
    let width = unsafe { config.size.width.to_int_unchecked() };
    let height = unsafe { config.size.height.to_int_unchecked() };

    error_in_callback!(state, state.opengl_state.make_current_no_surface());

    let (framebuffer, texture, renderbuffer) = unsafe {
        use gl::{types::*, *};

        let mut framebuffer: GLuint = 0;
        GenFramebuffers(1, &mut framebuffer);
        BindFramebuffer(FRAMEBUFFER, framebuffer);

        let mut texture: GLuint = 0;
        GenTextures(1, &mut texture);
        BindTexture(TEXTURE_2D, texture);
        TexParameteri(TEXTURE_2D, TEXTURE_WRAP_S, CLAMP_TO_EDGE as _);
        TexParameteri(TEXTURE_2D, TEXTURE_WRAP_T, CLAMP_TO_EDGE as _);
        TexParameteri(TEXTURE_2D, TEXTURE_MIN_FILTER, NEAREST as _);
        TexParameteri(TEXTURE_2D, TEXTURE_MAG_FILTER, NEAREST as _);
        TexImage2D(
            TEXTURE_2D,
            0,
            RGBA8 as _,
            width,
            height,
            0,
            RGBA,
            UNSIGNED_BYTE,
            std::ptr::null_mut(),
        );
        BindTexture(TEXTURE_2D, 0);
        FramebufferTexture2D(FRAMEBUFFER, COLOR_ATTACHMENT0, TEXTURE_2D, texture, 0);

        let mut renderbuffer: GLuint = 0;
        GenRenderbuffers(1, &mut renderbuffer);
        BindRenderbuffer(RENDERBUFFER, renderbuffer);
        RenderbufferStorage(RENDERBUFFER, DEPTH24_STENCIL8, width, height);
        BindRenderbuffer(RENDERBUFFER, 0);
        FramebufferRenderbuffer(
            FRAMEBUFFER,
            DEPTH_STENCIL_ATTACHMENT,
            RENDERBUFFER,
            renderbuffer,
        );

        (framebuffer, texture, renderbuffer)
    };

    error_in_callback!(state, state.opengl_state.make_not_current());

    extern "C" fn destruction_callback(_: *mut c_void) {} // destruct in collect_backing_store_callback

    backing_store.user_data = user_data;
    backing_store.type_ = ffi::FlutterBackingStoreType_kFlutterBackingStoreTypeOpenGL;
    backing_store.did_update = false;
    backing_store.__bindgen_anon_1 = ffi::FlutterBackingStore__bindgen_ty_1 {
        open_gl: ffi::FlutterOpenGLBackingStore {
            type_: ffi::FlutterOpenGLTargetType_kFlutterOpenGLTargetTypeFramebuffer,
            __bindgen_anon_1: ffi::FlutterOpenGLBackingStore__bindgen_ty_1 {
                framebuffer: ffi::FlutterOpenGLFramebuffer {
                    target: gl::RGBA8,
                    name: framebuffer,
                    user_data: Box::into_raw(Box::new((framebuffer, texture, renderbuffer))) as _,
                    destruction_callback: Some(destruction_callback),
                },
            },
        },
    };

    true
}

pub extern "C" fn collect_backing_store_callback(
    backing_store: *const ffi::FlutterBackingStore,
    user_data: *mut c_void,
) -> bool {
    let backing_store = unsafe { &*backing_store };
    let state = unsafe { &*(user_data as *const FlutterEngineStateInner) };
    error_in_callback!(state, state.opengl_state.make_current_no_surface());

    unsafe {
        use gl::{types::*, *};
        let user_data = backing_store
            .__bindgen_anon_1
            .open_gl
            .__bindgen_anon_1
            .framebuffer
            .user_data as *mut (GLuint, GLuint, GLuint);
        let (framebuffer, texture, renderbuffer) = *Box::from_raw(user_data);
        DeleteFramebuffers(1, &framebuffer);
        DeleteTextures(1, &texture);
        DeleteRenderbuffers(1, &renderbuffer);
    };

    error_in_callback!(state, state.opengl_state.make_not_current());

    true
}

pub extern "C" fn present_view_callback(present_info: *const ffi::FlutterPresentViewInfo) -> bool {
    let present_info = unsafe { &*present_info };
    let view_id = present_info.view_id;
    let state = unsafe { &*(present_info.user_data as *const FlutterEngineStateInner) };
    let view = match state.compositor.get_view(view_id) {
        Some(view) => view,
        None => {
            log::warn!("View #{} not found", view_id);
            return false;
        }
    };

    match &view.kind {
        FlutterViewKind::LayerSurface(layer_surface_view) => {
            let opengl_state = &state.opengl_state;
            let egl_surface = &layer_surface_view.egl_surface;

            let (view_width, view_height) = {
                let guard = view.size.read_blocking();
                (guard.width, guard.height)
            };
            egl_surface.resize(&opengl_state.render_context, view_width, view_height);

            error_in_callback!(state, opengl_state.make_current(egl_surface));

            let layers = unsafe { *present_info.layers };
            let layers = unsafe { std::slice::from_raw_parts(layers, present_info.layers_count) };

            for layer in layers {
                let ffi::FlutterPoint {
                    x: offset_x,
                    y: offset_y,
                } = layer.offset;
                let offset_x: i32 = unsafe { offset_x.to_int_unchecked() };
                let offset_y: i32 = unsafe { offset_y.to_int_unchecked() };
                let ffi::FlutterSize { width, height } = layer.size;
                let width: i32 = unsafe { width.to_int_unchecked() };
                let height: i32 = unsafe { height.to_int_unchecked() };
                let paint_region = unsafe { &*(*layer.backing_store_present_info).paint_region };
                let paint_region = unsafe {
                    std::slice::from_raw_parts(paint_region.rects, paint_region.rects_count)
                };
                let presentation_time = layer.presentation_time;

                log::info!(
                    "offset: ({}, {}), size: ({}, {}), presentation_time: {}",
                    offset_x,
                    offset_y,
                    width,
                    height,
                    presentation_time
                );
                log::info!("paint_region: {:?}", paint_region);

                match layer.type_ {
                    ffi::FlutterLayerContentType_kFlutterLayerContentTypeBackingStore => {
                        let backing_store = unsafe { &*layer.__bindgen_anon_1.backing_store };

                        unsafe {
                            use gl::{types::*, *};

                            let (_, texture, _) = *(backing_store
                                .__bindgen_anon_1
                                .open_gl
                                .__bindgen_anon_1
                                .framebuffer
                                .user_data
                                as *mut (GLuint, GLuint, GLuint));

                            // save
                            let mut prev_array_buffer = 0;
                            GetIntegerv(ARRAY_BUFFER_BINDING, &mut prev_array_buffer);
                            let mut prev_vertex_array = 0;
                            GetIntegerv(VERTEX_ARRAY_BINDING, &mut prev_vertex_array);
                            let mut prev_draw_framebuffer = 0;
                            GetIntegerv(DRAW_FRAMEBUFFER_BINDING, &mut prev_draw_framebuffer);
                            let mut prev_texture = 0;
                            GetIntegerv(TEXTURE_BINDING_2D, &mut prev_texture);

                            log::info!("prev: {}, {}, {}, {}", prev_array_buffer, prev_vertex_array, prev_draw_framebuffer, prev_texture);

                            BindFramebuffer(DRAW_FRAMEBUFFER, 0);

                            // https://github.com/NVIDIA/egl-wayland/issues/48
                            // THANK YOU AMBIGUOUS BIG STATE MACHINE. THANK YOU EGL and OpenGL.
                            DrawBuffer(BACK);

                            // TODO: offset, size, paint_region, presentation_time
                            BindVertexArray(opengl_state.vertex_array);
                            BindBuffer(ARRAY_BUFFER, opengl_state.vertex_buffer);
                            BindTexture(TEXTURE_2D, texture);
                            UseProgram(opengl_state.program);
                            DrawArrays(TRIANGLES, 0, 6);
                            error_in_callback!(
                                state,
                                layer_surface_view
                                    .egl_surface
                                    .swap_buffers(&opengl_state.render_context)
                            );

                            // restore
                            BindBuffer(ARRAY_BUFFER, prev_array_buffer as u32);
                            BindVertexArray(prev_vertex_array as u32);
                            BindFramebuffer(DRAW_FRAMEBUFFER, prev_draw_framebuffer as u32);
                            BindTexture(TEXTURE_2D, prev_texture as u32);
                        }
                    }
                    ffi::FlutterLayerContentType_kFlutterLayerContentTypePlatformView => {
                        let platform_view = unsafe { &*layer.__bindgen_anon_1.platform_view };
                        log::warn!(
                            "There's no platform views now. Ignored. (id: {})",
                            platform_view.identifier
                        );
                    }
                    _ => unreachable!(),
                }
            }

            error_in_callback!(state, opengl_state.make_not_current());

            true
        }
    }
}
