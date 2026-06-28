#[cfg(target_os = "linux")]
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
        RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Size},
};

#[cfg(target_os = "linux")]
use super::render::queue_redraw;
#[cfg(target_os = "linux")]
use super::state::ApcCompositor;

#[cfg(target_os = "linux")]
pub(crate) struct MoveSurfaceGrab {
    pub(crate) window: Window,
    pub(crate) start_data: GrabStartData<ApcCompositor>,
    pub(crate) initial_loc: Point<i32, Logical>,
}

#[cfg(target_os = "linux")]
impl PointerGrab<ApcCompositor> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut ApcCompositor,
        handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, focus, event);
        let delta = event.location - self.start_data.location;
        let new_loc = self.initial_loc.to_f64() + delta;
        data.space
            .map_element(self.window.clone(), new_loc.to_i32_round(), true);
        queue_redraw(data);
    }

    fn relative_motion(
        &mut self,
        data: &mut ApcCompositor,
        handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut ApcCompositor,
        handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut ApcCompositor,
        handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(
        &mut self,
        data: &mut ApcCompositor,
        handle: &mut PointerInnerHandle<'_, ApcCompositor>,
    ) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GestureSwipeBeginEvent,
    ) {
    }
    fn gesture_swipe_update(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GestureSwipeUpdateEvent,
    ) {
    }
    fn gesture_swipe_end(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GestureSwipeEndEvent,
    ) {
    }
    fn gesture_pinch_begin(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GesturePinchBeginEvent,
    ) {
    }
    fn gesture_pinch_update(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GesturePinchUpdateEvent,
    ) {
    }
    fn gesture_pinch_end(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GesturePinchEndEvent,
    ) {
    }
    fn gesture_hold_begin(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GestureHoldBeginEvent,
    ) {
    }
    fn gesture_hold_end(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GestureHoldEndEvent,
    ) {
    }

    fn start_data(&self) -> &GrabStartData<ApcCompositor> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut ApcCompositor) {}
}

#[cfg(target_os = "linux")]
pub(crate) struct ResizeSurfaceGrab {
    pub(crate) window: Window,
    pub(crate) start_data: GrabStartData<ApcCompositor>,
    pub(crate) edges: u32,
    pub(crate) initial_size: Size<i32, Logical>,
    pub(crate) initial_loc: Point<i32, Logical>,
}

#[cfg(target_os = "linux")]
impl PointerGrab<ApcCompositor> for ResizeSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut ApcCompositor,
        handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, focus, event);
        let delta = event.location - self.start_data.location;

        let mut new_w = self.initial_size.w;
        let mut new_h = self.initial_size.h;
        let mut loc = self.initial_loc;

        if self.edges & 8 != 0 {
            new_w = (self.initial_size.w as f64 + delta.x) as i32;
        }
        if self.edges & 4 != 0 {
            new_w = (self.initial_size.w as f64 - delta.x) as i32;
            loc.x = (self.initial_loc.x as f64 + delta.x) as i32;
        }
        if self.edges & 2 != 0 {
            new_h = (self.initial_size.h as f64 + delta.y) as i32;
        }
        if self.edges & 1 != 0 {
            new_h = (self.initial_size.h as f64 - delta.y) as i32;
            loc.y = (self.initial_loc.y as f64 + delta.y) as i32;
        }

        new_w = new_w.max(100);
        new_h = new_h.max(100);

        if let Some(toplevel) = self.window.toplevel() {
            toplevel.with_pending_state(|s| {
                s.size = Some((new_w, new_h).into());
            });
            toplevel.send_configure();
        }
        data.space.map_element(self.window.clone(), loc, true);
    }

    fn relative_motion(
        &mut self,
        data: &mut ApcCompositor,
        handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut ApcCompositor,
        handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut ApcCompositor,
        handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(
        &mut self,
        data: &mut ApcCompositor,
        handle: &mut PointerInnerHandle<'_, ApcCompositor>,
    ) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GestureSwipeBeginEvent,
    ) {
    }
    fn gesture_swipe_update(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GestureSwipeUpdateEvent,
    ) {
    }
    fn gesture_swipe_end(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GestureSwipeEndEvent,
    ) {
    }
    fn gesture_pinch_begin(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GesturePinchBeginEvent,
    ) {
    }
    fn gesture_pinch_update(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GesturePinchUpdateEvent,
    ) {
    }
    fn gesture_pinch_end(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GesturePinchEndEvent,
    ) {
    }
    fn gesture_hold_begin(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GestureHoldBeginEvent,
    ) {
    }
    fn gesture_hold_end(
        &mut self,
        _data: &mut ApcCompositor,
        _handle: &mut PointerInnerHandle<'_, ApcCompositor>,
        _event: &smithay::input::pointer::GestureHoldEndEvent,
    ) {
    }

    fn start_data(&self) -> &GrabStartData<ApcCompositor> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut ApcCompositor) {}
}
