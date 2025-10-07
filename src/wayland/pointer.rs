use smithay_client_toolkit::delegate_pointer;
use smithay_client_toolkit::seat::pointer::PointerEvent;
use smithay_client_toolkit::seat::pointer::PointerHandler;
use wayland_client::Connection;
use wayland_client::QueueHandle;
use wayland_client::protocol::wl_pointer::WlPointer;

impl PointerHandler for super::WaylandState {
  fn pointer_frame(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _pointer: &WlPointer,
    events: &[PointerEvent],
  ) {
    for event in events {
      log::info!("Pointer event: {:#?}", event);
    }
  }
}

delegate_pointer!(super::WaylandState);
