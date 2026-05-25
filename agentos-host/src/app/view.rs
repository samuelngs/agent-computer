use objc2::{define_class, msg_send, rc::Retained, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSEvent;
use objc2_foundation::{NSObjectProtocol, NSRect};

use crate::display;
use crate::input;

use super::{
    ca_transaction_begin, ca_transaction_commit, ca_transaction_set_disable_actions,
    DISPLAY_SCALE, LAST_DISPLAYED_SURFACE, PRESSED_KEYS,
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
        fn acceptsFirstResponder(&self) -> bool { true }

        #[unsafe(method(keyDown:))]
        fn keyDown(&self, event: &NSEvent) {
            let code = input::macos_keycode_to_linux(event.keyCode());
            if code != 0 {
                let value = track_key_press(code);
                input::send_key_event(code, value);
            }
        }

        #[unsafe(method(keyUp:))]
        fn keyUp(&self, event: &NSEvent) {
            let code = input::macos_keycode_to_linux(event.keyCode());
            if code != 0 {
                track_key_release(code);
                input::send_key_event(code, 0);
            }
        }

        #[unsafe(method(flagsChanged:))]
        fn flagsChanged(&self, event: &NSEvent) {
            if event.keyCode() == 57 {
                input::send_capslock_toggle();
                return;
            }
            input::sync_modifiers(event.modifierFlags());
        }

        #[unsafe(method(mouseEntered:))]
        fn mouseEntered(&self, _event: &NSEvent) {
            unsafe {
                let cls = objc2::runtime::AnyClass::get(c"NSCursor").unwrap();
                let _: () = msg_send![cls, hide];
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouseExited(&self, _event: &NSEvent) {
            unsafe {
                let cls = objc2::runtime::AnyClass::get(c"NSCursor").unwrap();
                let _: () = msg_send![cls, unhide];
            }
        }

        #[unsafe(method(resignFirstResponder))]
        fn resignFirstResponder_(&self) -> bool {
            input::release_all_modifiers();
            unsafe {
                let cls = objc2::runtime::AnyClass::get(c"NSCursor").unwrap();
                let _: () = msg_send![cls, unhide];
            }
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouseDown(&self, event: &NSEvent) {
            let btn = if event.modifierFlags().contains(objc2_app_kit::NSEventModifierFlags::Control) {
                input::macos_mouse_button_to_linux(1)
            } else {
                input::macos_mouse_button_to_linux(0)
            };
            input::send_mouse_button(btn, true);
        }

        #[unsafe(method(mouseUp:))]
        fn mouseUp(&self, event: &NSEvent) {
            let btn = if event.modifierFlags().contains(objc2_app_kit::NSEventModifierFlags::Control) {
                input::macos_mouse_button_to_linux(1)
            } else {
                input::macos_mouse_button_to_linux(0)
            };
            input::send_mouse_button(btn, false);
        }

        #[unsafe(method(rightMouseDown:))]
        fn rightMouseDown(&self, _event: &NSEvent) {
            input::send_mouse_button(input::macos_mouse_button_to_linux(1), true);
        }

        #[unsafe(method(rightMouseUp:))]
        fn rightMouseUp(&self, _event: &NSEvent) {
            input::send_mouse_button(input::macos_mouse_button_to_linux(1), false);
        }

        #[unsafe(method(otherMouseDown:))]
        fn otherMouseDown(&self, event: &NSEvent) {
            let btn = input::macos_mouse_button_to_linux(event.buttonNumber() as u16);
            input::send_mouse_button(btn, true);
        }

        #[unsafe(method(otherMouseUp:))]
        fn otherMouseUp(&self, event: &NSEvent) {
            let btn = input::macos_mouse_button_to_linux(event.buttonNumber() as u16);
            input::send_mouse_button(btn, false);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouseMoved(&self, event: &NSEvent) {
            self.send_abs_position(event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouseDragged(&self, event: &NSEvent) {
            self.send_abs_position(event);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn rightMouseDragged(&self, event: &NSEvent) {
            self.send_abs_position(event);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn otherMouseDragged(&self, event: &NSEvent) {
            self.send_abs_position(event);
        }

        #[unsafe(method(scrollWheel:))]
        fn scrollWheel(&self, event: &NSEvent) {
            let raw_dy = -event.scrollingDeltaY();
            let raw_dx = -event.scrollingDeltaX();
            let dy = if raw_dy > 0.0 { raw_dy.ceil() as i32 } else if raw_dy < 0.0 { raw_dy.floor() as i32 } else { 0 };
            let dx = if raw_dx > 0.0 { raw_dx.ceil() as i32 } else if raw_dx < 0.0 { raw_dx.floor() as i32 } else { 0 };
            if dx != 0 || dy != 0 { input::send_mouse_scroll(dx, dy); }
        }
    }
);

impl FramebufferView {
    fn send_abs_position(&self, event: &NSEvent) {
        let loc = event.locationInWindow();
        let local = self.convertPoint_fromView(loc, None);
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
