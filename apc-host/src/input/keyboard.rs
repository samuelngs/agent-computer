use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

use super::types::*;

static NAME: &[u8] = b"APC Virtual Keyboard";
static SERIAL: &[u8] = b"apc-kbd-0";

const SUPPORTED_KEYS: &[u16] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, // ESC .. EQUAL
    14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, // BACKSPACE .. P
    26, 27, 28, // LEFTBRACE .. ENTER
    29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, // LEFTCTRL .. SEMICOLON
    40, 41, 42, 43, // APOSTROPHE .. BACKSLASH
    44, 45, 46, 47, 48, 49, 50, 51, 52, 53, // Z .. SLASH
    54, 56, 57, 58, // RIGHTSHIFT, LEFTALT, SPACE, CAPSLOCK
    59, 60, 61, 62, 63, 64, 65, 66, 67, 68, // F1 .. F10
    70, // SCROLLLOCK
    87, 88, // F11, F12
    97, 100, // RIGHTCTRL, RIGHTALT
    102, 103, 104, 105, 106, 107, 108, 109, // HOME .. PAGEDOWN
    110, 111, // INSERT, DELETE
    113, 114, 115, 116, // MUTE, VOLUMEDOWN, VOLUMEUP, POWER
    119, // PAUSE
    125, // LEFTMETA (Command)
    210, // F13 area
];

unsafe extern "C" fn config_create(
    instance: *mut *mut c_void,
    _userdata: *const c_void,
    _reserved: *const c_void,
) -> i32 {
    unsafe { *instance = std::ptr::null_mut() };
    0
}

unsafe extern "C" fn config_destroy(_instance: *mut c_void) -> i32 {
    0
}

unsafe extern "C" fn query_device_name(
    _instance: *mut c_void,
    name_buf: *mut u8,
    name_buf_len: usize,
) -> i32 {
    let n = NAME.len().min(name_buf_len);
    unsafe { std::ptr::copy_nonoverlapping(NAME.as_ptr(), name_buf, n) };
    n as i32
}

unsafe extern "C" fn query_serial_name(
    _instance: *mut c_void,
    name_buf: *mut u8,
    name_buf_len: usize,
) -> i32 {
    let n = SERIAL.len().min(name_buf_len);
    unsafe { std::ptr::copy_nonoverlapping(SERIAL.as_ptr(), name_buf, n) };
    n as i32
}

unsafe extern "C" fn query_device_ids(_instance: *mut c_void, ids: *mut KrunInputDeviceIds) -> i32 {
    unsafe {
        *ids = KrunInputDeviceIds {
            bustype: BUS_VIRTUAL,
            vendor: 0x0001,
            product: 0x0001,
            version: 0x0001,
        };
    }
    0
}

unsafe extern "C" fn query_event_capabilities(
    _instance: *mut c_void,
    ev_type: u8,
    bitmap: *mut u8,
    bitmap_len: usize,
) -> i32 {
    let max_bit: u16 = match ev_type as u16 {
        EV_SYN => {
            unsafe { write_bitmap(bitmap, bitmap_len, SYN_REPORT) };
            SYN_REPORT
        }
        EV_KEY => {
            let mut max = 0u16;
            for &key in SUPPORTED_KEYS {
                unsafe { write_bitmap(bitmap, bitmap_len, key) };
                if key > max {
                    max = key;
                }
            }
            max
        }
        _ => return 0,
    };
    (max_bit / 8 + 1) as i32
}

unsafe extern "C" fn query_abs_info(
    _instance: *mut c_void,
    _code: u8,
    _absinfo: *mut KrunInputAbsinfo,
) -> i32 {
    -1
}

unsafe extern "C" fn query_properties(
    _instance: *mut c_void,
    _bitmap: *mut u8,
    _bitmap_len: usize,
) -> i32 {
    0
}

unsafe extern "C" fn event_create(
    instance: *mut *mut c_void,
    _userdata: *const c_void,
    _reserved: *const c_void,
) -> i32 {
    keyboard_queue();
    unsafe { *instance = std::ptr::null_mut() };
    0
}

unsafe extern "C" fn event_destroy(_instance: *mut c_void) -> i32 {
    0
}

unsafe extern "C" fn get_ready_efd(_instance: *mut c_void) -> i32 {
    keyboard_queue().read_fd()
}

unsafe extern "C" fn next_event(_instance: *mut c_void, event: *mut KrunInputEvent) -> i32 {
    match keyboard_queue().pop() {
        Some(e) => {
            static KEY_EVENTS_DRAINED: AtomicU64 = AtomicU64::new(0);
            if std::env::var_os("APC_INPUT_DEBUG").is_some() {
                let count = KEY_EVENTS_DRAINED.fetch_add(1, Ordering::Relaxed) + 1;
                if count <= 20 || count % 120 == 0 {
                    tracing::info!(
                        count,
                        event_type = e.r#type,
                        code = e.code,
                        value = e.value,
                        "host input: libkrun drained key event"
                    );
                }
            }
            unsafe { *event = e };
            1
        }
        None => 0,
    }
}

pub fn create_backend() -> (KrunInputConfig, KrunInputEventProvider) {
    let config = KrunInputConfig {
        features: KRUN_INPUT_CONFIG_FEATURE_QUERY,
        create_userdata: std::ptr::null_mut(),
        create: Some(config_create),
        vtable: KrunInputConfigVtable {
            destroy: Some(config_destroy),
            query_device_name: Some(query_device_name),
            query_serial_name: Some(query_serial_name),
            query_device_ids: Some(query_device_ids),
            query_event_capabilities: Some(query_event_capabilities),
            query_abs_info: Some(query_abs_info),
            query_properties: Some(query_properties),
        },
    };

    let provider = KrunInputEventProvider {
        features: KRUN_INPUT_EVENT_PROVIDER_FEATURE_QUEUE,
        create_userdata: std::ptr::null_mut(),
        create: Some(event_create),
        vtable: KrunInputEventProviderVtable {
            destroy: Some(event_destroy),
            get_ready_efd: Some(get_ready_efd),
            next_event: Some(next_event),
        },
    };

    (config, provider)
}
