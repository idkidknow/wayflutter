use std::sync::{Arc, OnceLock, Weak};

use anyhow::Result;
use bon::Builder;
use smithay_client_toolkit::{
    compositor::Surface,
    reexports::protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
        zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
    },
};
use wayland_client::{
    globals::GlobalList, protocol::{wl_output::WlOutput, wl_surface::WlSurface}, Connection, Dispatch, QueueHandle
};

#[derive(Builder)]
pub struct CreateLayerSurfaceProp {
    pub(super) layer: Layer,
    #[builder(into)]
    pub(super) namespace: Option<String>,
    pub(super) output: Option<WlOutput>,
    pub(super) size: Option<Size>,
    pub(super) anchor: Option<Anchor>,
    pub(super) exclusive_zone: Option<i32>,
    pub(super) margin: Option<Margin>,
    pub(super) keyboard_interactivity: Option<KeyboardInteractivity>,
    pub(super) exclusive_edge: Option<Anchor>,
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

pub struct LayerShell {
    wlr_layer_shell: ZwlrLayerShellV1,
}

impl LayerShell {
    pub fn bind<S>(globals: &GlobalList, qh: &QueueHandle<S>) -> Result<Self>
    where
        S: Dispatch<ZwlrLayerShellV1, (), S> + 'static,
    {
        Ok(Self {
            wlr_layer_shell: globals.bind(qh, 1..=5, ())?,
        })
    }

    pub fn create_layer_surface<S>(
        &self,
        qh: &QueueHandle<S>,
        surface: Surface,
        output: Option<&WlOutput>,
        layer: Layer,
        namespace: String,
    ) -> LayerSurface
    where
        S: Dispatch<ZwlrLayerSurfaceV1, LayerSurfaceData, S> + 'static,
    {
        let layer_surface_inner: Arc<LayerSurfaceInner> = Arc::new_cyclic(|weak| {
            let layer_surface = self.wlr_layer_shell.get_layer_surface(
                surface.wl_surface(),
                output,
                layer,
                namespace,
                qh,
                LayerSurfaceData {
                    inner: weak.clone(),
                },
            );
            LayerSurfaceInner {
                surface: surface,
                layer_surface: layer_surface,
                on_configure: OnceLock::new(),
            }
        });

        LayerSurface(layer_surface_inner)
    }
}

impl Drop for LayerShell {
    fn drop(&mut self) {
        self.wlr_layer_shell.destroy();
    }
}

pub struct LayerSurface(Arc<LayerSurfaceInner>);

impl LayerSurface {
    pub fn wl_surface(&self) -> &WlSurface {
        self.0.surface.wl_surface()
    }

    pub fn wlr_layer_surface(&self) -> &ZwlrLayerSurfaceV1 {
        &self.0.layer_surface
    }

    pub fn set_on_configure(&self, on_configure: impl Fn(Size) + Send + Sync + 'static) {
        let _ = self.0.on_configure.set(Box::new(on_configure));
    }
}

pub(super) struct LayerSurfaceInner {
    surface: Surface,
    layer_surface: ZwlrLayerSurfaceV1,
    pub(super) on_configure: OnceLock<Box<dyn Fn(Size) + Send + Sync + 'static>>,
}

impl Drop for LayerSurfaceInner {
    fn drop(&mut self) {
        self.layer_surface.destroy();
    }
}

pub struct LayerSurfaceData {
    pub(super) inner: Weak<LayerSurfaceInner>,
}

impl Dispatch<ZwlrLayerShellV1, ()> for super::State {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrLayerShellV1,
        _event: <ZwlrLayerShellV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        unreachable!();
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, LayerSurfaceData> for super::State {
    fn event(
        _state: &mut Self,
        proxy: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        data: &LayerSurfaceData,
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                proxy.ack_configure(serial);
                data.inner.upgrade().map(|inner| {
                    if let Some(cb) = inner.on_configure.get() {
                        cb(Size { width, height });
                    }
                });
            }
            zwlr_layer_surface_v1::Event::Closed => {}
            _ => {}
        }
    }
}

