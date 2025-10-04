use std::{cell::UnsafeCell, convert::Infallible, future::poll_fn, task::ready};

use anyhow::Result;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use wayland_client::{Connection, EventQueue, globals::registry_queue_init};

use crate::FlutterEngine;

pub mod layer_shell;

pub struct WaylandClient<'a> {
    conn: &'a Connection,
    queue: UnsafeCell<EventQueue<WaylandState>>,
    state: UnsafeCell<WaylandState>,
}

impl<'a> WaylandClient<'a> {
    pub(super) fn new(conn: &'a Connection, engine: &'a FlutterEngine) -> Result<Self> {
        let (globals, queue) = registry_queue_init::<WaylandState>(conn)?;
        let qh = queue.handle();
        let output_state = OutputState::new(&globals, &qh);
        let compositor_state = CompositorState::bind(&globals, &qh)?;
        let layer_shell = globals.bind::<ZwlrLayerShellV1, _, _>(&qh, 1..=5, ())?;

        // `wayland-client` requires that the State struct should be 'static.
        //
        // SAFETY: `WaylandState` is only used in `queue.dispatch_pending()``.
        // `queue.dispatch_pending()` is only called from the `WaylandClient::run` method.
        // `'a` outlives the future returned by `WaylandClient::run(&'a self)`.
        let static_engine_ref: &'static FlutterEngine = unsafe { std::mem::transmute(engine) };

        let state = WaylandState {
            engine: static_engine_ref,
            registry_state: RegistryState::new(&globals),
            output_state,
            compositor_state,
            layer_shell,
        };

        Ok(Self {
            conn,
            queue: UnsafeCell::new(queue),
            state: UnsafeCell::new(state),
        })
    }

    pub async fn run(&self) -> Result<Infallible> {
        loop {
            // SAFETY: `Self: !Sync`, only one &mut per field inside brace,
            // no reentrancy (Maybe I can call this again in event handlers
            // in queue.dispatch_pending? I will never do that)
            // and references are dropped before await point
            {
                let queue = unsafe { &mut *self.queue.get() };
                let state = unsafe { &mut *self.state.get() };
                queue.flush()?;
                queue.dispatch_pending(state)?;
            }

            let backend = self.conn.backend();
            let fd = smol::Async::new_nonblocking(backend.poll_fd())?;

            // read from socket
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
            .await?
        }
    }
}

struct WaylandState {
    engine: &'static FlutterEngine,
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: ZwlrLayerShellV1,
}

impl ProvidesRegistryState for WaylandState {
    fn registry(&mut self) -> &mut smithay_client_toolkit::registry::RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

delegate_registry!(WaylandState);

impl OutputHandler for WaylandState {
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

delegate_output!(WaylandState);

impl CompositorHandler for WaylandState {
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

delegate_compositor!(WaylandState);
