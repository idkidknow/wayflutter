use smithay_client_toolkit::shell::wlr_layer::LayerSurface as SctkLayerSurface;
use smithay_client_toolkit::shell::{
    WaylandSurface,
    wlr_layer::{Anchor, KeyboardInteractivity, Layer},
};
use wayland_client::protocol::{wl_output::WlOutput, wl_surface::WlSurface};

#[derive(Debug, Clone)]
pub struct CreateLayerSurfaceProp {
    pub layer: Layer,
    pub namespace: Option<String>,
    pub output: Option<WlOutput>,
    pub size: Option<Size>,
    pub anchor: Option<Anchor>,
    pub exclusive_zone: Option<i32>,
    pub margin: Option<Margin>,
    pub keyboard_interactivity: Option<KeyboardInteractivity>,
}

#[derive(Debug, Clone, Copy)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Margin {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

pub struct LayerSurface {
    pub(super) inner: SctkLayerSurface,
}

impl LayerSurface {
    pub fn wl_surface(&self) -> &WlSurface {
        self.inner.wl_surface()
    }
}
