use std::{
    ffi::{CStr, CString},
    ptr::NonNull,
};

use anyhow::{Context, Result};
use glutin::{
    api::egl::{self, config::Config, context::PossiblyCurrentContext, display::Display, surface::Surface},
    config::ConfigTemplate,
    context::ContextAttributesBuilder,
    prelude::{GlDisplay, NotCurrentGlContext, PossiblyCurrentGlContext}, surface::WindowSurface,
};
use raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle};
use wayland_client::Proxy;

use crate::wayland::WaylandConnection;

pub struct OpenGLState {
    pub egl_display: Display,
    pub egl_config: Config,
    pub render_context: PossiblyCurrentContext,
    pub program: gl::types::GLuint,
    pub vertex_array: gl::types::GLuint,
    pub vertex_buffer: gl::types::GLuint,
    pub resource_context: PossiblyCurrentContext,
}

impl OpenGLState {
    pub fn init(conn: &WaylandConnection) -> Result<Self> {
        let display = get_egl_display(conn)?;

        gl::load_with(|symbol| {
            let Ok(address) = CString::new(symbol) else {
                log::warn!("Failed to convert symbol \"{}\" to CString.", symbol);
                return std::ptr::null();
            };
            display.get_proc_address(&address)
        });

        let config = unsafe {
            display
                .find_configs(ConfigTemplate::default())?
                .next()
                .context("no egl config found")?
        };

        let render_context = unsafe {
            let context_attributes = ContextAttributesBuilder::new().build(None);
            display
                .create_context(&config, &context_attributes)?
                .treat_as_possibly_current()
        };

        let resource_context = unsafe {
            let context_attributes = ContextAttributesBuilder::new()
                .with_sharing(&render_context)
                .build(None);
            display
                .create_context(&config, &context_attributes)?
                .treat_as_possibly_current()
        };

        let program = compile_shader_and_link_program(&render_context)?;
        let (vertex_array, vertex_buffer) = unsafe {
            use gl::{types::*, *};

            render_context.make_current_surfaceless()?;

            let vertices: [GLfloat; _] = [
                -1.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
                -1.0, -1.0, 0.0, 0.0, 1.0, -1.0, 1.0, 0.0,
            ]; // rectangle vertices with texture coords

            let mut vertex_array = 0;
            GenVertexArrays(1, &mut vertex_array);
            let mut vertex_buffer = 0;
            GenBuffers(1, &mut vertex_buffer);

            BindVertexArray(vertex_array);
            BindBuffer(ARRAY_BUFFER, vertex_buffer);

            BufferData(
                ARRAY_BUFFER,
                (vertices.len() * size_of::<GLfloat>()) as isize,
                vertices.as_ptr() as _,
                STATIC_DRAW,
            );

            let position_loc: GLuint = GetAttribLocation(program, c"position".as_ptr()) as _;
            EnableVertexAttribArray(position_loc);
            VertexAttribPointer(
                position_loc,
                2,
                FLOAT,
                FALSE,
                (4 * size_of::<GLfloat>()) as _,
                0 as _,
            );
            let texcoord_loc: GLuint = GetAttribLocation(program, c"in_texcoord".as_ptr()) as _;
            EnableVertexAttribArray(texcoord_loc);
            VertexAttribPointer(
                texcoord_loc,
                2,
                FLOAT,
                FALSE,
                (4 * size_of::<GLfloat>()) as _,
                (2 * size_of::<GLfloat>()) as _,
            );

            BindBuffer(ARRAY_BUFFER, 0);
            BindVertexArray(0);

            render_context.make_not_current_in_place()?;

            (vertex_array, vertex_buffer)
        };

        Ok(Self {
            egl_display: display,
            egl_config: config,
            render_context,
            program,
            vertex_array,
            vertex_buffer,
            resource_context,
        })
    }

    pub fn make_current_no_surface(&self) -> Result<()> {
        self.render_context.make_current_surfaceless()
            .context("failed to make context current with EGL_NO_SURFACE")?;
        Ok(())
    }

    pub fn make_current(&self, surface: &Surface<WindowSurface>) -> Result<()> {
        self.render_context.make_current(surface)
            .context("failed to make context current")?;
        Ok(())
    }

    pub fn make_not_current(&self) -> Result<()> {
        self.render_context.make_not_current_in_place()?;
        Ok(())
    }
}

fn get_egl_display(conn: &WaylandConnection) -> Result<Display> {
    // SAFETY: trust `wayland-client` crate and `libwayland`...
    let display = unsafe {
        let display = NonNull::new(conn.wl_display().id().as_ptr() as _)
            .context("null wl_display pointer")?;
        Display::new(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            display,
        )))
        .context("failed to create EGL display")?
    };
    Ok(display)
}

const VERTEX_SHADER_SRC: &CStr = c"
#version 330 core

in vec2 position;
in vec2 in_texcoord;
out vec2 texcoord;

void main() {
    gl_Position = vec4(position, 0.0, 1.0);
    texcoord = in_texcoord;
}
";

const FRAGMENT_SHADER_SRC: &CStr = c"
#version 330 core

out vec4 color;
in vec2 texcoord;
uniform sampler2D tex;

void main() {
    color = texture(tex, texcoord);
}
";

fn compile_shader_and_link_program(context: &PossiblyCurrentContext) -> Result<gl::types::GLuint> {
    context.make_current_surfaceless()?;

    use gl::{types::*, *};

    unsafe fn compile(type_: GLenum, src: &CStr) -> Result<GLuint> {
        unsafe {
            let shader = CreateShader(type_);
            ShaderSource(shader, 1, &src.as_ptr(), std::ptr::null());
            CompileShader(shader);
            let mut compile_status = 0;
            GetShaderiv(shader, COMPILE_STATUS, &mut compile_status);
            if compile_status == FALSE as i32 {
                let mut log = [0i8; 512];
                let mut log_len = 0;
                GetShaderInfoLog(shader, 512, &mut log_len, log.as_mut_ptr());
                let log: [u8; 512] = std::mem::transmute(log);
                let log = String::from_utf8_lossy(&log[..log_len as usize]);
                anyhow::bail!("Failed to compile shader: {}", log);
            }
            Ok(shader)
        }
    }

    let program = unsafe {
        let vertex_shader = compile(VERTEX_SHADER, VERTEX_SHADER_SRC)?;
        let fragment_shader = compile(FRAGMENT_SHADER, FRAGMENT_SHADER_SRC)?;
        let program = CreateProgram();
        AttachShader(program, vertex_shader);
        AttachShader(program, fragment_shader);
        LinkProgram(program);
        let mut link_status = 0;
        GetProgramiv(program, LINK_STATUS, &mut link_status);
        if link_status == FALSE as i32 {
            let mut log = [0i8; 512];
            let mut log_len = 0;
            GetProgramInfoLog(program, 512, &mut log_len, log.as_mut_ptr());
            let log: [u8; 512] = std::mem::transmute(log);
            let log = String::from_utf8_lossy(&log[..log_len as usize]);
            anyhow::bail!("Failed to link program: {}", log);
        }
        DeleteShader(vertex_shader);
        DeleteShader(fragment_shader);

        program
    };

    context.make_not_current_in_place()?;

    Ok(program)
}
