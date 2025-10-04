use std::{cell::UnsafeCell, future::poll_fn, task::ready};

use anyhow::Result;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Surface},
    delegate_compositor, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use smol::LocalExecutor;
use wayland_client::{Connection, EventQueue, globals::registry_queue_init};

use layer_shell::LayerShell;

use crate::wayland::layer_shell::{CreateLayerSurfaceProp, LayerSurface, Size};

pub mod layer_shell;

pub struct WaylandClient<'a> {
    executor: LocalExecutor<'a>,
    conn: &'a Connection,
    queue: UnsafeCell<EventQueue<WaylandState>>,
    state: UnsafeCell<WaylandState>,
}

impl<'a> WaylandClient<'a> {
    pub(super) fn new(conn: &'a Connection) -> Result<Self> {
        let (globals, queue) = registry_queue_init::<WaylandState>(conn)?;
        let qh = queue.handle();
        let output_state = OutputState::new(&globals, &qh);
        let compositor_state = CompositorState::bind(&globals, &qh)?;
        let layer_shell = LayerShell::bind(&globals, &qh)?;

        // `wayland-client` requires that the State struct should be 'static.
        //
        // SAFETY: `WaylandState` is only used in `queue.dispatch_pending()``.
        // `queue.dispatch_pending()` is only called from the `WaylandClient::run` method.
        // `'a` outlives the future returned by `WaylandClient::run(&'a self)`.
        // let static_engine_ref: &'static FlutterEngine = unsafe { std::mem::transmute(engine) };

        let state = WaylandState {
            // engine: static_engine_ref,
            registry_state: RegistryState::new(&globals),
            output_state,
            compositor_state,
            layer_shell,
        };

        Ok(Self {
            executor: LocalExecutor::new(),
            conn,
            queue: UnsafeCell::new(queue),
            state: UnsafeCell::new(state),
        })
    }

    pub async fn run(&self) -> Result<()> {
        let dispatching_loop = async {
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
        };
        self.executor.run(dispatching_loop).await
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

struct WaylandState {
    // engine: &'static FlutterEngine,
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    layer_shell: LayerShell,
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
