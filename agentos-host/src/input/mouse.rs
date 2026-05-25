use std::ffi::c_void;

use super::types::*;

static NAME: &[u8] = b"AgentOS Virtual Tablet";
static SERIAL: &[u8] = b"agentos-tablet-0";

unsafe extern "C" fn config_create(
    instance: *mut *mut c_void,
    _userdata: *const c_void,
    _reserved: *const c_void,
) -> i32 {
    unsafe { *instance = std::ptr::null_mut() };
    0
}

unsafe extern "C" fn config_destroy(_instance: *mut c_void) -> i32 { 0 }

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

unsafe extern "C" fn query_device_ids(
    _instance: *mut c_void,
    ids: *mut KrunInputDeviceIds,
) -> i32 {
    unsafe {
        *ids = KrunInputDeviceIds {
            bustype: BUS_VIRTUAL,
            vendor: 0x0001,
            product: 0x0002,
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
            unsafe {
                write_bitmap(bitmap, bitmap_len, BTN_LEFT);
                write_bitmap(bitmap, bitmap_len, BTN_RIGHT);
                write_bitmap(bitmap, bitmap_len, BTN_MIDDLE);
            }
            BTN_MIDDLE
        }
        EV_ABS => {
            unsafe {
                write_bitmap(bitmap, bitmap_len, ABS_X);
                write_bitmap(bitmap, bitmap_len, ABS_Y);
            }
            ABS_Y
        }
        EV_REL => {
            unsafe {
                write_bitmap(bitmap, bitmap_len, REL_WHEEL);
                write_bitmap(bitmap, bitmap_len, REL_HWHEEL);
            }
            REL_WHEEL
        }
        _ => return 0,
    };
    (max_bit / 8 + 1) as i32
}

unsafe extern "C" fn query_abs_info(
    _instance: *mut c_void,
    code: u8,
    absinfo: *mut KrunInputAbsinfo,
) -> i32 {
    match code as u16 {
        ABS_X | ABS_Y => {
            unsafe {
                *absinfo = KrunInputAbsinfo {
                    min: 0,
                    max: ABS_MAX,
                    fuzz: 0,
                    flat: 0,
                    res: 0,
                };
            }
            0
        }
        _ => 0,
    }
}

unsafe extern "C" fn query_properties(
    _instance: *mut c_void,
    bitmap: *mut u8,
    bitmap_len: usize,
) -> i32 {
    unsafe { write_bitmap(bitmap, bitmap_len, INPUT_PROP_DIRECT) };
    (INPUT_PROP_DIRECT / 8 + 1) as i32
}

unsafe extern "C" fn event_create(
    instance: *mut *mut c_void,
    _userdata: *const c_void,
    _reserved: *const c_void,
) -> i32 {
    mouse_queue();
    unsafe { *instance = std::ptr::null_mut() };
    0
}

unsafe extern "C" fn event_destroy(_instance: *mut c_void) -> i32 { 0 }

unsafe extern "C" fn get_ready_efd(_instance: *mut c_void) -> i32 {
    mouse_queue().read_fd()
}

unsafe extern "C" fn next_event(
    _instance: *mut c_void,
    event: *mut KrunInputEvent,
) -> i32 {
    match mouse_queue().pop() {
        Some(e) => {
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
