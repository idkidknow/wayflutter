use std::{num::NonZero, ptr::NonNull};

use anyhow::{Context, Result};
use glutin::{
    api::egl,
    prelude::GlDisplay,
    surface::{SurfaceAttributesBuilder, WindowSurface},
};
use raw_window_handle::{RawWindowHandle, WaylandWindowHandle};
use smithay_client_toolkit::reexports::protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer,
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity},
};
use wayland_client::{Proxy, protocol::wl_surface::WlSurface};

use crate::{
    engine::opengl::OpenGLState,
    wayland::{
        WaylandConnection,
        layer_shell::{CreateLayerSurfaceProp, LayerSurface, Size},
    },
};

pub struct ViewState {
    pub layer: LayerSurface,
    pub surface: egl::surface::Surface<WindowSurface>,
}

impl ViewState {
    pub fn new_layer_surface(conn: &WaylandConnection, opengl_state: &OpenGLState) -> Result<Self> {
        let layer_prop = CreateLayerSurfaceProp::builder()
            .layer(Layer::Background)
            .namespace("aaaaa")
            .size(Size {
                width: 800,
                height: 600,
            })
            .anchor(Anchor::Right)
            .keyboard_interactivity(KeyboardInteractivity::OnDemand)
            .build();
        let layer = conn.create_layer_surface(layer_prop)?;
        let wl_surface = layer.wl_surface();

        let rwh = get_raw_window_handle(wl_surface)?;

        let egl_display = &opengl_state.egl_display;
        let egl_config = &opengl_state.egl_config;

        let egl_window_surface = {
            let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
                rwh,
                NonZero::new(800).unwrap(),
                NonZero::new(600).unwrap(),
            );
            unsafe { egl_display.create_window_surface(&egl_config, &surface_attributes)? }
        };

        Ok(Self {
            layer,
            surface: egl_window_surface,
        })
    }
}

fn get_raw_window_handle(wl_surface: &WlSurface) -> Result<RawWindowHandle> {
    let rwh = RawWindowHandle::Wayland(WaylandWindowHandle::new(
        NonNull::new(wl_surface.id().as_ptr() as _).context("null wl_surface pointer")?,
    ));
    Ok(rwh)
}
