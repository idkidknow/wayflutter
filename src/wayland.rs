use std::{cell::UnsafeCell, convert::Infallible, future::poll_fn, task::ready};

use anyhow::Result;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Surface},
    delegate_compositor, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1::ZwlrLayerShellV1, zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1}},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, globals::registry_queue_init, protocol::wl_display::WlDisplay,
};

use crate::wayland::layer_shell::{CreateLayerSurfaceProp, LayerShell, LayerSurface, LayerSurfaceData, Size};

pub mod layer_shell;

pub struct WaylandConnection {
    conn: Connection,
    queue: UnsafeCell<EventQueue<State>>,
    state: UnsafeCell<State>,
}

impl WaylandConnection {
    pub fn new(conn: Connection) -> Result<Self> {
        let (globals, event_queue) = registry_queue_init::<State>(&conn)?;
        let qh = event_queue.handle();

        let output_state = OutputState::new(&globals, &qh);

        let compositor_state = CompositorState::bind(&globals, &qh)?;

        let layer_shell = LayerShell::bind(&globals, &qh)?;

        let state = State {
            registry_state: RegistryState::new(&globals),
            output_state,
            compositor_state,
            layer_shell,
        };

        Ok(Self {
            conn,
            queue: UnsafeCell::new(event_queue),
            state: UnsafeCell::new(state),
        })
    }

    pub async fn run(&self) -> Result<Infallible> {
        loop {
            // SAFETY: `Self: !Sync`, only one &mut inside brace, no reentrancy
            // and references are dropped before await point
            {
                let queue = unsafe { &mut *self.queue.get() };
                queue.flush()?;
                let state = unsafe { &mut *self.state.get() };
                queue.dispatch_pending(state)?;
            }

            let backend = self.conn.backend();
            let fd = smol::Async::new_nonblocking(backend.poll_fd())?;

            // try read
            poll_fn(|cx| {
                let guard = self.conn.prepare_read();
                match guard {
                    None => {
                        // we need to dispatch pending (next loop)
                        std::task::Poll::Ready(Ok(()))
                    }
                    Some(guard) => {
                        ready!(fd.poll_readable(cx))?;
                        guard.read()?;
                        std::task::Poll::Ready(anyhow::Ok(()))
                    }
                }
            })
            .await?;
        }
    }

    pub fn wl_display(&self) -> WlDisplay {
        self.conn.display()
    }

    pub fn create_layer_surface(&self, prop: CreateLayerSurfaceProp) -> Result<LayerSurface> {
        let state = unsafe { &mut *self.state.get() };
        let queue = unsafe { &*self.queue.get() };
        let qh = queue.handle();

        let surface = Surface::new(&state.compositor_state, &qh)?;

        let layer_surface = state.layer_shell.create_layer_surface(
            &qh,
            surface,
            prop.output.as_ref(),
            prop.layer,
            prop.namespace.unwrap_or_default(),
        );

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
        if let Some(exclusive_edge) = prop .exclusive_edge {
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

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: LayerShell,
}

impl State {}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut smithay_client_toolkit::registry::RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

delegate_registry!(State);

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &wayland_client::QueueHandle<Self>,
        _output: wayland_client::protocol::wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &wayland_client::QueueHandle<Self>,
        _output: wayland_client::protocol::wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &wayland_client::QueueHandle<Self>,
        _output: wayland_client::protocol::wl_output::WlOutput,
    ) {
    }
}

delegate_output!(State);

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &wayland_client::QueueHandle<Self>,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &wayland_client::QueueHandle<Self>,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
        _new_transform: wayland_client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &wayland_client::QueueHandle<Self>,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &wayland_client::QueueHandle<Self>,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
        _output: &wayland_client::protocol::wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &wayland_client::QueueHandle<Self>,
        _surface: &wayland_client::protocol::wl_surface::WlSurface,
        _output: &wayland_client::protocol::wl_output::WlOutput,
    ) {
    }
}

delegate_compositor!(State);

impl Dispatch<ZwlrLayerShellV1, ()> for State {
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

impl Dispatch<ZwlrLayerSurfaceV1, LayerSurfaceData> for State {
    fn event(
        _state: &mut Self,
        proxy: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        data: &LayerSurfaceData,
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                proxy.ack_configure(serial);
                data.inner.upgrade().map(|inner| {
                    if let Some(cb) = inner.on_configure.get() {
                        cb(Size { width, height });
                    }
                });
            },
            zwlr_layer_surface_v1::Event::Closed => {},
            _ => {},
        }
    }
}
