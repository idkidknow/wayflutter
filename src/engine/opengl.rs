use std::ptr::NonNull;

use anyhow::{Context, Result};
use glutin::{
    api::egl::{config::Config, context::PossiblyCurrentContext, display::Display},
    config::ConfigTemplate,
    context::ContextAttributesBuilder,
    prelude::{GlDisplay, NotCurrentGlContext},
};
use raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle};
use wayland_client::Proxy;

use crate::wayland::WaylandConnection;

pub struct OpenGLState {
    pub egl_display: Display,
    pub egl_config: Config,
    pub render_context: PossiblyCurrentContext,
    pub resource_context: PossiblyCurrentContext,
}

impl OpenGLState {
    pub fn init(conn: &WaylandConnection) -> Result<Self> {
        let display = get_egl_display(conn)?;

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

        Ok(Self {
            egl_display: display,
            egl_config: config,
            render_context,
            resource_context,
        })
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
