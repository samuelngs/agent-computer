use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSCursor, NSEvent, NSEventType};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect};

use crate::display;
use crate::input;

use super::{
    DISPLAY_SCALE, LAST_DISPLAYED_SURFACE, PRESSED_KEYS, ca_transaction_begin,
    ca_transaction_commit, ca_transaction_set_disable_actions,
};

fn track_key_press(code: u16) -> u32 {
    let mut guard = PRESSED_KEYS.lock().unwrap();
    let set = guard.get_or_insert_with(std::collections::HashSet::new);
    if set.insert(code) { 1 } else { 2 }
}

fn track_key_release(code: u16) {
    if let Ok(mut guard) = PRESSED_KEYS.lock() {
        if let Some(set) = guard.as_mut() {
            set.remove(&code);
        }
    }
}

pub struct FramebufferViewIvars;

fn set_native_arrow_cursor() {
    NSCursor::setHiddenUntilMouseMoves(false);
    NSCursor::arrowCursor().set();
}

fn force_native_cursor_visible() {
    set_native_arrow_cursor();
    NSCursor::unhide();
}

define_class!(
    #[unsafe(super(objc2_app_kit::NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "FramebufferView"]
    #[ivars = FramebufferViewIvars]
    pub struct FramebufferView;

    unsafe impl NSObjectProtocol for FramebufferView {}

    #[allow(non_snake_case)]
    impl FramebufferView {
        #[unsafe(method(acceptsFirstResponder))]
        fn acceptsFirstResponder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn acceptsFirstMouse(&self, _event: Option<&NSEvent>) -> bool {
            self.focus_host_window();
            true
        }

        #[unsafe(method(resetCursorRects))]
        fn resetCursorRects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::arrowCursor());
            force_native_cursor_visible();
        }

        #[unsafe(method(cursorUpdate:))]
        fn cursorUpdate(&self, _event: &NSEvent) {
            force_native_cursor_visible();
        }

        #[unsafe(method(keyDown:))]
        fn keyDown(&self, event: &NSEvent) {
            self.forward_key_down(event);
        }

        #[unsafe(method(keyUp:))]
        fn keyUp(&self, event: &NSEvent) {
            self.forward_key_up(event);
        }

        #[unsafe(method(flagsChanged:))]
        fn flagsChanged(&self, event: &NSEvent) {
            self.forward_flags_changed(event);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouseEntered(&self, _event: &NSEvent) {
            // The guest compositor intentionally does not render a software cursor in
            // GUI mode; keep the native cursor visible for smooth pointer feedback.
            self.focus_host_window();
            force_native_cursor_visible();
        }

        #[unsafe(method(mouseExited:))]
        fn mouseExited(&self, _event: &NSEvent) {
            force_native_cursor_visible();
        }

        #[unsafe(method(resignFirstResponder))]
        fn resignFirstResponder_(&self) -> bool {
            input::release_all_modifiers();
            force_native_cursor_visible();
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouseDown(&self, event: &NSEvent) {
            self.focus_host_window();
            self.forward_mouse_event(event);
            self.forward_mouse_button_event(event, true);
        }

        #[unsafe(method(mouseUp:))]
        fn mouseUp(&self, event: &NSEvent) {
            self.forward_mouse_event(event);
            self.forward_mouse_button_event(event, false);
        }

        #[unsafe(method(rightMouseDown:))]
        fn rightMouseDown(&self, event: &NSEvent) {
            self.focus_host_window();
            self.forward_mouse_event(event);
            self.forward_mouse_button_event(event, true);
        }

        #[unsafe(method(rightMouseUp:))]
        fn rightMouseUp(&self, event: &NSEvent) {
            self.forward_mouse_event(event);
            self.forward_mouse_button_event(event, false);
        }

        #[unsafe(method(otherMouseDown:))]
        fn otherMouseDown(&self, event: &NSEvent) {
            self.focus_host_window();
            self.forward_mouse_event(event);
            self.forward_mouse_button_event(event, true);
        }

        #[unsafe(method(otherMouseUp:))]
        fn otherMouseUp(&self, event: &NSEvent) {
            self.forward_mouse_event(event);
            self.forward_mouse_button_event(event, false);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouseMoved(&self, event: &NSEvent) {
            set_native_arrow_cursor();
            self.forward_mouse_event(event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouseDragged(&self, event: &NSEvent) {
            set_native_arrow_cursor();
            self.forward_mouse_event(event);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn rightMouseDragged(&self, event: &NSEvent) {
            set_native_arrow_cursor();
            self.forward_mouse_event(event);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn otherMouseDragged(&self, event: &NSEvent) {
            set_native_arrow_cursor();
            self.forward_mouse_event(event);
        }

        #[unsafe(method(scrollWheel:))]
        fn scrollWheel(&self, event: &NSEvent) {
            let raw_dy = -event.scrollingDeltaY();
            let raw_dx = -event.scrollingDeltaX();
            let dy = if raw_dy > 0.0 {
                raw_dy.ceil() as i32
            } else if raw_dy < 0.0 {
                raw_dy.floor() as i32
            } else {
                0
            };
            let dx = if raw_dx > 0.0 {
                raw_dx.ceil() as i32
            } else if raw_dx < 0.0 {
                raw_dx.floor() as i32
            } else {
                0
            };
            if dx != 0 || dy != 0 {
                input::send_mouse_scroll(dx, dy);
            }
        }
    }
);

impl FramebufferView {
    pub(crate) fn focus_host_window(&self) {
        if let Some(window) = self.window() {
            window.makeKeyAndOrderFront(None);
            window.makeKeyWindow();
            window.makeMainWindow();
            let _ = window.makeFirstResponder(Some(self));
        }
    }

    pub(crate) fn forward_mouse_button_event(&self, event: &NSEvent, pressed: bool) {
        let button = match event.r#type() {
            NSEventType::LeftMouseDown | NSEventType::LeftMouseUp
                if event
                    .modifierFlags()
                    .contains(objc2_app_kit::NSEventModifierFlags::Control) =>
            {
                1
            }
            NSEventType::RightMouseDown | NSEventType::RightMouseUp => 1,
            _ => event.buttonNumber() as u16,
        };
        input::send_mouse_button(input::macos_mouse_button_to_linux(button), pressed);
    }

    pub(crate) fn forward_key_down(&self, event: &NSEvent) {
        let code = input::macos_keycode_to_linux(event.keyCode());
        if code != 0 {
            let value = track_key_press(code);
            input::send_key_event(code, value);
        }
    }

    pub(crate) fn forward_key_up(&self, event: &NSEvent) {
        let code = input::macos_keycode_to_linux(event.keyCode());
        if code != 0 {
            track_key_release(code);
            input::send_key_event(code, 0);
        }
    }

    pub(crate) fn forward_flags_changed(&self, event: &NSEvent) {
        if event.keyCode() == 57 {
            input::send_capslock_toggle();
            return;
        }
        input::sync_modifiers(event.modifierFlags());
    }

    pub(crate) fn forward_mouse_event(&self, event: &NSEvent) {
        self.forward_mouse_window_point(event.locationInWindow());
    }

    fn forward_mouse_window_point(&self, window_point: NSPoint) {
        let local = self.convertPoint_fromView(window_point, None);
        self.forward_mouse_local_point(local);
    }

    fn forward_mouse_local_point(&self, local: NSPoint) {
        let bounds = self.bounds();
        let vw = bounds.size.width;
        let vh = bounds.size.height;
        if vw <= 0.0 || vh <= 0.0 {
            return;
        }

        let ds = display::global_display();
        let vm_w = ds.vm_width() as f64;
        let vm_h = ds.vm_height() as f64;
        if vm_w <= 0.0 || vm_h <= 0.0 {
            return;
        }

        let scale = (vw / vm_w).min(vh / vm_h);
        let rendered_w = vm_w * scale;
        let rendered_h = vm_h * scale;
        let pad_x = (vw - rendered_w) / 2.0;
        let pad_y = (vh - rendered_h) / 2.0;

        let nx = ((local.x - pad_x) / rendered_w).clamp(0.0, 1.0);
        let ny = (1.0 - (local.y - pad_y) / rendered_h).clamp(0.0, 1.0);
        let abs_x = (nx * 32767.0) as u32;
        let abs_y = (ny * 32767.0) as u32;
        input::send_mouse_move_abs(abs_x, abs_y);
    }

    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(FramebufferViewIvars);
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            let scale = DISPLAY_SCALE.load(std::sync::atomic::Ordering::Relaxed);
            layer.setContentsScale(scale.max(1) as f64);
            layer.setContentsGravity(unsafe { objc2_quartz_core::kCAGravityResizeAspect });
            layer.setOpaque(true);
        }
        unsafe {
            use objc2_app_kit::NSTrackingAreaOptions;
            let options = NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::CursorUpdate
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::InVisibleRect;
            let tracking_area = objc2_app_kit::NSTrackingArea::initWithRect_options_owner_userInfo(
                mtm.alloc::<objc2_app_kit::NSTrackingArea>(),
                NSRect::ZERO,
                options,
                Some(&view),
                None,
            );
            view.addTrackingArea(&tracking_area);
        }
        force_native_cursor_visible();
        view.addCursorRect_cursor(view.bounds(), &NSCursor::arrowCursor());
        view
    }

    pub fn update_framebuffer(&self) {
        let state = display::global_display();
        let Some(surface) = state.get_front_surface() else {
            return;
        };
        let surface_usize = surface as usize;
        let prev = LAST_DISPLAYED_SURFACE.swap(surface_usize, std::sync::atomic::Ordering::Relaxed);
        if surface_usize == prev {
            return;
        }
        if let Some(layer) = self.layer() {
            ca_transaction_begin();
            ca_transaction_set_disable_actions(true);
            unsafe {
                let contents = &*(surface as *const objc2::runtime::AnyObject);
                layer.setContents(Some(contents));
            }
            ca_transaction_commit();
        }
    }
}
