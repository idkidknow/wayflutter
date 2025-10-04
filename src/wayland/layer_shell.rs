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
    protocol::{wl_output::WlOutput, wl_pointer::{ButtonState, WlPointer}, wl_surface::WlSurface}, Connection, Dispatch
};

use crate::FlutterEngine;

type LayerSurfaceEventListener<T> = for<'a> fn(&'a FlutterEngine, zwlr_layer_surface_v1::Event, &T);

#[derive(Builder)]
pub struct CreateLayerSurfaceProp<T> {
    layer: Layer,
    #[builder(into)]
    namespace: Option<String>,
    output: Option<WlOutput>,
    size: Option<Size>,
    anchor: Option<Anchor>,
    exclusive_zone: Option<i32>,
    margin: Option<Margin>,
    keyboard_interactivity: Option<KeyboardInteractivity>,
    exclusive_edge: Option<Anchor>,

    event_listener: Option<LayerSurfaceEventListener<T>>,
    user_data: T,
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
    surface: Surface,
    wlr_layer_surface: ZwlrLayerSurfaceV1,
}

impl LayerSurface {
    pub fn wl_surface(&self) -> &WlSurface {
        &self.surface.wl_surface()
    }

    pub fn wlr_layer_surface(&self) -> &ZwlrLayerSurfaceV1 {
        &self.wlr_layer_surface
    }
}

pub trait WaylandClientLayerSurfaceExt {
    fn create_layer_surface<T: Send + Sync + 'static>(
        &self,
        prop: CreateLayerSurfaceProp<T>,
    ) -> Result<LayerSurface>;
}

impl WaylandClientLayerSurfaceExt for super::WaylandClient<'_> {
    fn create_layer_surface<T: Send + Sync + 'static>(
        &self,
        prop: CreateLayerSurfaceProp<T>,
    ) -> Result<LayerSurface> {
        let layer_surface = {
            let state = unsafe { &mut *self.state.get() };
            let qh = unsafe { (&*self.queue.get()).handle() };
            let surface = Surface::new(&state.compositor_state, &qh)?;
            let wlr_layer_surface = state.layer_shell.get_layer_surface(
                surface.wl_surface(),
                prop.output.as_ref(),
                prop.layer,
                prop.namespace.unwrap_or_default(),
                &qh,
                (prop.event_listener.unwrap_or(|_, _, _| {}), prop.user_data),
            );

            let ret = LayerSurface {
                surface,
                wlr_layer_surface,
            };

            for seat in state.seat_state.seats() {
                seat.get_pointer(&qh, ());
            }

            ret
        };

        let wlr_layer_surface = layer_surface.wlr_layer_surface();

        if let Some(anchor) = prop.anchor {
            wlr_layer_surface.set_anchor(anchor);
        }
        if let Some(exclusive_zone) = prop.exclusive_zone {
            wlr_layer_surface.set_exclusive_zone(exclusive_zone);
        }
        if let Some(margin) = prop.margin {
            wlr_layer_surface.set_margin(margin.top, margin.right, margin.bottom, margin.left);
        }
        if let Some(keyboard_interactivity) = prop.keyboard_interactivity {
            wlr_layer_surface.set_keyboard_interactivity(keyboard_interactivity);
        }
        if let Some(exclusive_edge) = prop.exclusive_edge {
            wlr_layer_surface.set_exclusive_edge(exclusive_edge);
        }

        let size = prop.size.unwrap_or(Size {
            width: 0,
            height: 0,
        });

        wlr_layer_surface.set_size(size.width, size.height);
        layer_surface.wl_surface().commit();

        Ok(layer_surface)
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for super::WaylandState {
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

impl<T> Dispatch<ZwlrLayerSurfaceV1, (LayerSurfaceEventListener<T>, T)> for super::WaylandState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        data: &(LayerSurfaceEventListener<T>, T),
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        let (event_listener, user_data) = data;
        event_listener(state.engine, event, user_data);
    }
}

impl Dispatch<WlPointer, ()> for super::WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &WlPointer,
        event: <WlPointer as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        match event {
            wayland_client::protocol::wl_pointer::Event::Button {
                serial: _,
                time: _,
                button: _,
                state: button_state,
            } => {
                if button_state.into_result().unwrap() == ButtonState::Pressed {
                    log::info!("Pressed");
                }
            }
            _ => {}
        }
    }
}
