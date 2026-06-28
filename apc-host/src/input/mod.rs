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
pub use types::{KrunInputConfig, KrunInputEvent, KrunInputEventProvider};

#[cfg(target_os = "macos")]
use types::*;

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[cfg(target_os = "macos")]
fn input_debug_enabled() -> bool {
    std::env::var_os("APC_INPUT_DEBUG").is_some()
}

#[cfg(target_os = "macos")]
fn should_log_input(counter: &AtomicU64) -> Option<u64> {
    if !input_debug_enabled() {
        return None;
    }

    let count = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if count <= 20 || count % 120 == 0 {
        Some(count)
    } else {
        None
    }
}

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
    static KEY_EVENTS_SENT: AtomicU64 = AtomicU64::new(0);
    if let Some(count) = should_log_input(&KEY_EVENTS_SENT) {
        tracing::info!(count, code, value, "host input: queue key event");
    }

    keyboard_queue().push_batch(&[
        KrunInputEvent {
            r#type: EV_KEY,
            code,
            value,
        },
        KrunInputEvent {
            r#type: EV_SYN,
            code: SYN_REPORT,
            value: 0,
        },
    ]);
}

#[cfg(target_os = "macos")]
pub fn send_mouse_move_abs(x: u32, y: u32) {
    static LAST_MOUSE_ABS: AtomicU64 = AtomicU64::new(u64::MAX);
    static MOUSE_MOVES_SENT: AtomicU64 = AtomicU64::new(0);
    static MOUSE_MOVES_DROPPED: AtomicU64 = AtomicU64::new(0);
    let packed = ((x as u64) << 32) | y as u64;
    let previous = LAST_MOUSE_ABS.swap(packed, Ordering::SeqCst);
    if previous != u64::MAX && previous == packed {
        if let Some(count) = should_log_input(&MOUSE_MOVES_DROPPED) {
            tracing::info!(count, x, y, "host input: drop duplicate mouse abs");
        }
        return;
    }

    if let Some(count) = should_log_input(&MOUSE_MOVES_SENT) {
        tracing::info!(count, x, y, "host input: queue mouse abs");
    }

    mouse_queue().push_batch(&[
        KrunInputEvent {
            r#type: EV_ABS,
            code: ABS_X,
            value: x,
        },
        KrunInputEvent {
            r#type: EV_ABS,
            code: ABS_Y,
            value: y,
        },
        KrunInputEvent {
            r#type: EV_SYN,
            code: SYN_REPORT,
            value: 0,
        },
    ]);
}

#[cfg(target_os = "macos")]
pub fn send_mouse_button(button: u16, pressed: bool) {
    static LAST_MOUSE_BUTTONS: AtomicUsize = AtomicUsize::new(0);
    static MOUSE_BUTTONS_SENT: AtomicU64 = AtomicU64::new(0);
    static MOUSE_BUTTONS_DROPPED: AtomicU64 = AtomicU64::new(0);
    let bit = 1usize << (button as usize).min(15);
    let old = if pressed {
        LAST_MOUSE_BUTTONS.fetch_or(bit, Ordering::SeqCst)
    } else {
        LAST_MOUSE_BUTTONS.fetch_and(!bit, Ordering::SeqCst)
    };
    if (old & bit != 0) == pressed {
        if let Some(count) = should_log_input(&MOUSE_BUTTONS_DROPPED) {
            tracing::info!(
                count,
                button,
                pressed,
                "host input: drop duplicate mouse button"
            );
        }
        return;
    }
    if let Some(count) = should_log_input(&MOUSE_BUTTONS_SENT) {
        tracing::info!(count, button, pressed, "host input: queue mouse button");
    }

    mouse_queue().push_batch(&[
        KrunInputEvent {
            r#type: EV_KEY,
            code: button,
            value: if pressed { 1 } else { 0 },
        },
        KrunInputEvent {
            r#type: EV_SYN,
            code: SYN_REPORT,
            value: 0,
        },
    ]);
}

#[cfg(target_os = "macos")]
pub fn send_mouse_scroll(dx: i32, dy: i32) {
    static MOUSE_SCROLLS_SENT: AtomicU64 = AtomicU64::new(0);
    if let Some(count) = should_log_input(&MOUSE_SCROLLS_SENT) {
        tracing::info!(count, dx, dy, "host input: queue mouse scroll");
    }

    let mut batch = [KrunInputEvent {
        r#type: 0,
        code: 0,
        value: 0,
    }; 3];
    let mut n = 0;
    if dy != 0 {
        batch[n] = KrunInputEvent {
            r#type: EV_REL,
            code: REL_WHEEL,
            value: dy as u32,
        };
        n += 1;
    }
    if dx != 0 {
        batch[n] = KrunInputEvent {
            r#type: EV_REL,
            code: REL_HWHEEL,
            value: dx as u32,
        };
        n += 1;
    }
    batch[n] = KrunInputEvent {
        r#type: EV_SYN,
        code: SYN_REPORT,
        value: 0,
    };
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
    ModifierMapping {
        flag: 1 << 17,
        linux_code: 42,
    }, // Shift
    ModifierMapping {
        flag: 1 << 18,
        linux_code: 29,
    }, // Control
    ModifierMapping {
        flag: 1 << 19,
        linux_code: 56,
    }, // Option
    ModifierMapping {
        flag: 1 << 20,
        linux_code: 125,
    }, // Command
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
        KrunInputEvent {
            r#type: EV_KEY,
            code: 58,
            value: 1,
        },
        KrunInputEvent {
            r#type: EV_SYN,
            code: SYN_REPORT,
            value: 0,
        },
        KrunInputEvent {
            r#type: EV_KEY,
            code: 58,
            value: 0,
        },
        KrunInputEvent {
            r#type: EV_SYN,
            code: SYN_REPORT,
            value: 0,
        },
    ]);
}
