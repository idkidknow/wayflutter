use std::{collections::HashMap, num::NonZero, ptr::NonNull};

use anyhow::{Context, Result};
use glutin::{
    api::egl,
    prelude::GlDisplay,
    surface::{SurfaceAttributesBuilder, WindowSurface},
};
use parking_lot::Mutex;
use raw_window_handle::{RawWindowHandle, WaylandWindowHandle};
use smithay_client_toolkit::reexports::protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::Layer,
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity},
};
use wayland_client::Proxy;

use crate::{
    error::FFIFlutterEngineResultExt,
    error_in_callback, ffi,
    opengl::OpenGLState,
    wayland::{
        WaylandClient,
        layer_shell::{CreateLayerSurfaceProp, LayerSurface, WaylandClientLayerSurfaceExt},
    },
};
use egl::surface::Surface;

pub mod callback;

#[derive(Debug, Clone, Copy)]
pub struct ViewId {
    raw: ffi::FlutterViewId,
}

impl ViewId {
    pub fn new(raw: ffi::FlutterViewId) -> Self {
        Self { raw }
    }

    pub fn raw(&self) -> ffi::FlutterViewId {
        self.raw
    }
}

impl PartialEq for ViewId {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl Eq for ViewId {}

impl std::hash::Hash for ViewId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl std::fmt::Display for ViewId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "view#{}", self.raw)
    }
}

pub struct Compositor {
    views: HashMap<ViewId, FlutterView>,
}

impl Compositor {
    pub fn init(wayland_client: &WaylandClient<'_>, opengl_state: &OpenGLState) -> Result<Self> {
        let mut map = HashMap::with_capacity(1);

        // create implicit view
        let layer_prop = CreateLayerSurfaceProp::builder()
            .layer(Layer::Background)
            .namespace("aaaaa")
            .anchor(Anchor::Left | Anchor::Right | Anchor::Top | Anchor::Bottom)
            .keyboard_interactivity(KeyboardInteractivity::OnDemand)
            .user_data(ViewId::new(0))
            .event_listener(|engine, event, id| {
                let state = unsafe { engine.get_state() };
                let result = || {
                    let this = state.compositor.get_view(*id).with_context(|| {
                        format!("Inconsistent: event from {}, which is not registered in the compositor", id)
                    })?;
                    let FlutterViewKind::LayerSurface(layer_surface) = &this.kind;

                    match event {
                        zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                            match (NonZero::new(width), NonZero::new(height)) {
                                (Some(width), Some(height)) => {
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
                                        view_id: id.raw(),
                                    };
                                    unsafe {
                                        ffi::FlutterEngineSendWindowMetricsEvent(engine.engine, &event)
                                            .into_flutter_engine_result()?;
                                    }
                                    layer_surface.layer_surface.wlr_layer_surface().ack_configure(serial);
                                    {
                                        let mut guard = this.size.lock();

                                        guard.0.width = width;
                                        guard.0.height = height;
                                        guard.1 = true;
                                    }
                                    
                                },
                                _ => {},
                            }
                        },
                        _ => {},
                    }

                    anyhow::Ok(())
                };
                error_in_callback!(state, result(), return ());
            })
            .build();
        let layer_surface = wayland_client.create_layer_surface(layer_prop)?;
        let implicit_view = FlutterView {
            view_id: ViewId::new(0),
            kind: FlutterViewKind::LayerSurface(LayerSurfaceView::new(
                layer_surface,
                opengl_state,
            )?),
            size: Mutex::new((
                NonZeroSize {
                    width: NonZero::new(1600).unwrap(),
                    height: NonZero::new(900).unwrap(),
                },
                false,
            )),
        };
        map.insert(implicit_view.view_id, implicit_view);

        Ok(Self { views: map })
    }

    pub fn get_view(&self, view_id: ViewId) -> Option<&FlutterView> {
        self.views.get(&view_id)
    }
}

pub struct FlutterView {
    pub view_id: ViewId,
    pub kind: FlutterViewKind,
    pub size: Mutex<(NonZeroSize, /*should resize*/ bool)>,
}



pub enum FlutterViewKind {
    LayerSurface(LayerSurfaceView),
    // Popup,
}

pub struct LayerSurfaceView {
    layer_surface: LayerSurface,
    egl_surface: Mutex<Surface<WindowSurface>>,
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
            egl_surface: Mutex::new(egl_window_surface),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NonZeroSize {
    pub width: NonZero<u32>,
    pub height: NonZero<u32>,
}
