#![allow(dead_code)]

#[cfg(target_os = "macos")]
mod keyboard;
#[cfg(target_os = "macos")]
mod keymap;
#[cfg(target_os = "macos")]
mod mouse;
#[cfg(target_os = "macos")]
pub mod types;

#[cfg(target_os = "macos")]
pub use keymap::{macos_keycode_to_linux, macos_mouse_button_to_linux};
#[cfg(target_os = "macos")]
pub use types::{KrunInputConfig, KrunInputEventProvider, KrunInputEvent};

#[cfg(target_os = "macos")]
use types::*;

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_os = "macos")]
pub fn create_keyboard_backend() -> (KrunInputConfig, KrunInputEventProvider) {
    keyboard::create_backend()
}

#[cfg(target_os = "macos")]
pub fn create_mouse_backend() -> (KrunInputConfig, KrunInputEventProvider) {
    mouse::create_backend()
}

#[cfg(target_os = "macos")]
pub fn send_key_event(code: u16, value: u32) {
    keyboard_queue().push_batch(&[
        KrunInputEvent { r#type: EV_KEY, code, value },
        KrunInputEvent { r#type: EV_SYN, code: SYN_REPORT, value: 0 },
    ]);
}

#[cfg(target_os = "macos")]
pub fn send_mouse_move_abs(x: u32, y: u32) {
    mouse_queue().push_batch(&[
        KrunInputEvent { r#type: EV_ABS, code: ABS_X, value: x },
        KrunInputEvent { r#type: EV_ABS, code: ABS_Y, value: y },
        KrunInputEvent { r#type: EV_SYN, code: SYN_REPORT, value: 0 },
    ]);
}

#[cfg(target_os = "macos")]
pub fn send_mouse_button(button: u16, pressed: bool) {
    mouse_queue().push_batch(&[
        KrunInputEvent {
            r#type: EV_KEY,
            code: button,
            value: if pressed { 1 } else { 0 },
        },
        KrunInputEvent { r#type: EV_SYN, code: SYN_REPORT, value: 0 },
    ]);
}

#[cfg(target_os = "macos")]
pub fn send_mouse_scroll(dx: i32, dy: i32) {
    let mut batch = [KrunInputEvent { r#type: 0, code: 0, value: 0 }; 3];
    let mut n = 0;
    if dy != 0 {
        batch[n] = KrunInputEvent { r#type: EV_REL, code: REL_WHEEL, value: dy as u32 };
        n += 1;
    }
    if dx != 0 {
        batch[n] = KrunInputEvent { r#type: EV_REL, code: REL_HWHEEL, value: dx as u32 };
        n += 1;
    }
    batch[n] = KrunInputEvent { r#type: EV_SYN, code: SYN_REPORT, value: 0 };
    n += 1;
    mouse_queue().push_batch(&batch[..n]);
}

#[cfg(target_os = "macos")]
static LAST_MODIFIER_FLAGS: AtomicUsize = AtomicUsize::new(0);

#[cfg(target_os = "macos")]
struct ModifierMapping {
    flag: usize,
    linux_code: u16,
}

#[cfg(target_os = "macos")]
const MODIFIER_MAP: &[ModifierMapping] = &[
    ModifierMapping { flag: 1 << 17, linux_code: 42 },  // Shift
    ModifierMapping { flag: 1 << 18, linux_code: 29 },  // Control
    ModifierMapping { flag: 1 << 19, linux_code: 56 },  // Option
    ModifierMapping { flag: 1 << 20, linux_code: 125 }, // Command
];

#[cfg(target_os = "macos")]
pub fn sync_modifiers(new_flags: objc2_app_kit::NSEventModifierFlags) {
    let new_raw = new_flags.bits();
    let old_raw = LAST_MODIFIER_FLAGS.swap(new_raw, Ordering::SeqCst);
    for m in MODIFIER_MAP {
        let was = old_raw & m.flag != 0;
        let is = new_raw & m.flag != 0;
        if was && !is {
            send_key_event(m.linux_code, 0);
        } else if !was && is {
            send_key_event(m.linux_code, 1);
        }
    }
}

#[cfg(target_os = "macos")]
pub fn release_all_modifiers() {
    LAST_MODIFIER_FLAGS.store(0, Ordering::SeqCst);
    for m in MODIFIER_MAP {
        send_key_event(m.linux_code, 0);
    }
}

#[cfg(target_os = "macos")]
pub fn send_capslock_toggle() {
    keyboard_queue().push_batch(&[
        KrunInputEvent { r#type: EV_KEY, code: 58, value: 1 },
        KrunInputEvent { r#type: EV_SYN, code: SYN_REPORT, value: 0 },
        KrunInputEvent { r#type: EV_KEY, code: 58, value: 0 },
        KrunInputEvent { r#type: EV_SYN, code: SYN_REPORT, value: 0 },
    ]);
}
