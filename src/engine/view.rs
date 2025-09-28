use std::{num::NonZero, ptr::NonNull};

use anyhow::{Context, Result};
use glutin::{
    api::egl::{self, context::PossiblyCurrentContext},
    config::ConfigTemplate,
    context::ContextAttributesBuilder,
    prelude::{GlDisplay, NotCurrentGlContext, PossiblyCurrentGlContext},
    surface::{Rect, SurfaceAttributesBuilder, WindowSurface},
};
use raw_window_handle::{RawWindowHandle, WaylandWindowHandle};
use smithay_client_toolkit::shell::wlr_layer::Anchor;
use wayland_client::{Proxy, protocol::wl_surface::WlSurface};

use crate::wayland::{
    WaylandConnection,
    layer_shell::{CreateLayerSurfaceProp, LayerSurface, Size},
};

pub struct ViewState {
    _layer: LayerSurface,
    surface: egl::surface::Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    resource_context: PossiblyCurrentContext,
}

impl ViewState {
    pub fn new_layer_surface(
        conn: &WaylandConnection,
        egl_display: &egl::display::Display,
    ) -> Result<Self> {
        let layer = conn.create_layer_surface(CreateLayerSurfaceProp {
            layer: smithay_client_toolkit::shell::wlr_layer::Layer::Background,
            namespace: Some("aaaaa".to_owned()),
            output: None,
            size: Some(Size {
                width: 800,
                height: 600,
            }),
            anchor: Some(Anchor::RIGHT),
            exclusive_zone: None,
            margin: None,
            keyboard_interactivity: Some(
                smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::OnDemand,
            ),
        })?;
        let wl_surface = layer.wl_surface();

        let egl_config = unsafe {
            egl_display
                .find_configs(ConfigTemplate::default())?
                .next()
                .context("no egl config found")?
        };

        let rwh = get_raw_window_handle(wl_surface)?;

        let egl_window_surface = {
            let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
                rwh,
                NonZero::new(800).unwrap(),
                NonZero::new(600).unwrap(),
            );
            unsafe { egl_display.create_window_surface(&egl_config, &surface_attributes)? }
        };

        let egl_context = unsafe {
            let context_attributes = ContextAttributesBuilder::new().build(Some(rwh));
            let context = egl_display.create_context(&egl_config, &context_attributes)?;
            context.treat_as_possibly_current()
        };

        let resource_context = unsafe {
            let context_attributes = ContextAttributesBuilder::new()
                .with_sharing(&egl_context)
                .build(Some(rwh));
            let context = egl_display.create_context(&egl_config, &context_attributes)?;
            context.treat_as_possibly_current()
        };

        Ok(Self {
            _layer: layer,
            surface: egl_window_surface,
            context: egl_context,
            resource_context: resource_context,
        })
    }

    pub fn make_current(&self) -> Result<()> {
        self.context.make_current(&self.surface)?;
        Ok(())
    }

    pub fn clear_current(&self) -> Result<()> {
        self.context.make_not_current_in_place()?;
        Ok(())
    }

    pub fn make_resource_current(&self) -> Result<()> {
        self.resource_context.make_current_surfaceless()?;
        Ok(())
    }

    pub fn swap_buffers_with_damage(&self, rects: &[Rect]) -> Result<()> {
        self.surface
            .swap_buffers_with_damage(&self.context, rects)?;
        Ok(())
    }
}

fn get_raw_window_handle(wl_surface: &WlSurface) -> Result<RawWindowHandle> {
    let rwh = RawWindowHandle::Wayland(WaylandWindowHandle::new(
        NonNull::new(wl_surface.id().as_ptr() as _).context("null wl_surface pointer")?,
    ));
    Ok(rwh)
}
