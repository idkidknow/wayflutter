use std::{collections::HashMap, num::NonZero, ptr::NonNull};

use anyhow::{Context, Result};
use futures::{StreamExt, channel::mpsc::UnboundedReceiver};
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
use smol::lock::RwLock;
use wayland_client::{Connection, Proxy};

use crate::{
    error::FFIFlutterEngineResultExt, ffi, opengl::OpenGLState, wayland::{layer_shell::{CreateLayerSurfaceProp, LayerSurface, Size}, WaylandClient}, FlutterEngine
};
use egl::surface::Surface;

pub mod callback;

pub struct Compositor {
    views: HashMap<ffi::FlutterViewId, FlutterView>,
}

impl Compositor {
    pub fn init(
        wayland_client: &WaylandClient<'_>,
        opengl_state: &OpenGLState,
    ) -> Result<(Self, CompositorCoroutine)> {
        let mut map = HashMap::with_capacity(1);
        let (resize_event_tx, resize_event_rx) = futures::channel::mpsc::unbounded();

        // create implicit view
        let layer_prop = CreateLayerSurfaceProp::builder()
            .layer(Layer::Background)
            .namespace("aaaaa")
            .anchor(Anchor::Left | Anchor::Right | Anchor::Top | Anchor::Bottom)
            .keyboard_interactivity(KeyboardInteractivity::OnDemand)
            .build();
        let layer_surface = wayland_client.create_layer_surface(layer_prop)?;
        layer_surface.set_on_configure(move |Size { width, height }| {
            match (NonZero::new(width), NonZero::new(height)) {
                (Some(width), Some(height)) => {
                    let _ = resize_event_tx.unbounded_send(ResizeEvent {
                        view_id: 0,
                        size: NonZeroSize { width, height },
                    });
                }
                _ => {}
            }
        });
        let implicit_view = FlutterView {
            view_id: 0,
            kind: FlutterViewKind::LayerSurface(LayerSurfaceView::new(
                layer_surface,
                opengl_state,
            )?),
            size: RwLock::new(NonZeroSize {
                width: NonZero::new(1600).unwrap(),
                height: NonZero::new(900).unwrap(),
            }),
        };
        map.insert(implicit_view.view_id, implicit_view);

        Ok((Self { views: map }, CompositorCoroutine { resize_event_rx }))
    }

    pub fn get_view(&self, view_id: ffi::FlutterViewId) -> Option<&FlutterView> {
        self.views.get(&view_id)
    }
}

pub struct CompositorCoroutine {
    resize_event_rx: UnboundedReceiver<ResizeEvent>,
}

impl CompositorCoroutine {
    /// use this after FlutterEngine::init_state()
    pub async fn with(self, engine: &FlutterEngine) -> Result<()> {
        let mut resize_event_rx = self.resize_event_rx;
        loop {
            let ResizeEvent {
                view_id,
                size: NonZeroSize { width, height },
            } = resize_event_rx
                .next()
                .await
                .context("all resize event sender dropped")?;

            let state = unsafe { engine.get_state() };
            let Some(view) = state.compositor.get_view(view_id) else {
                // The view has been removed
                continue;
            };

            let event = ffi::FlutterWindowMetricsEvent {
                struct_size: size_of::<ffi::FlutterWindowMetricsEvent>(),
                width: width.get() as usize,
                height: height.get() as usize,
                pixel_ratio: 1.0,
                left: 0,
                top: 0,
                physical_view_inset_top: 0.0,
                physical_view_inset_right: 0.0,
                physical_view_inset_bottom: 0.0,
                physical_view_inset_left: 0.0,
                display_id: 0,
                view_id,
            };
            unsafe {
                ffi::FlutterEngineSendWindowMetricsEvent(engine.engine, &event)
                    .into_flutter_engine_result()?;
            }
            {
                let mut guard = view.size.write().await;
                guard.width = width;
                guard.height = height;
            }
        }
    }
}

pub struct FlutterView {
    pub view_id: ffi::FlutterViewId,
    pub kind: FlutterViewKind,
    pub size: RwLock<NonZeroSize>,
}

pub enum FlutterViewKind {
    LayerSurface(LayerSurfaceView),
    // Popup,
}

pub struct LayerSurfaceView {
    layer_surface: LayerSurface,
    pub egl_surface: Surface<WindowSurface>,
}

impl LayerSurfaceView {
    fn new(layer_surface: LayerSurface, opengl_state: &OpenGLState) -> Result<Self> {
        let wl_surface = layer_surface.wl_surface();
        let rwh = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(wl_surface.id().as_ptr() as _).context("null wl_surface pointer")?,
        ));

        let egl_display = &opengl_state.egl_display;
        let egl_config = &opengl_state.egl_config;
        let egl_window_surface = {
            let surface_attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
                rwh,
                NonZero::new(1600).unwrap(),
                NonZero::new(900).unwrap(),
            );
            unsafe { egl_display.create_window_surface(&egl_config, &surface_attributes)? }
        };

        Ok(Self {
            layer_surface,
            egl_surface: egl_window_surface,
        })
    }
}

#[derive(Debug, Clone)]
struct ResizeEvent {
    view_id: ffi::FlutterViewId,
    size: NonZeroSize,
}

#[derive(Debug, Clone, Copy)]
pub struct NonZeroSize {
    pub width: NonZero<u32>,
    pub height: NonZero<u32>,
}
