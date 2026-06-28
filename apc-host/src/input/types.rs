#![allow(dead_code)]

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const EV_ABS: u16 = 0x03;

pub const SYN_REPORT: u16 = 0x00;

pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;
pub const REL_HWHEEL: u16 = 0x06;
pub const REL_WHEEL: u16 = 0x08;

pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;
pub const ABS_MAX: u32 = 32767;

pub const INPUT_PROP_POINTER: u16 = 0x00;
pub const INPUT_PROP_DIRECT: u16 = 0x01;

pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;

pub const BUS_VIRTUAL: u16 = 0x00;

pub const KRUN_INPUT_CONFIG_FEATURE_QUERY: u64 = 1;
pub const KRUN_INPUT_EVENT_PROVIDER_FEATURE_QUEUE: u64 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KrunInputEvent {
    pub r#type: u16,
    pub code: u16,
    pub value: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KrunInputDeviceIds {
    pub bustype: u16,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KrunInputAbsinfo {
    pub min: u32,
    pub max: u32,
    pub fuzz: u32,
    pub flat: u32,
    pub res: u32,
}

pub type KrunInputCreateFn = Option<
    unsafe extern "C" fn(
        instance: *mut *mut c_void,
        userdata: *const c_void,
        reserved: *const c_void,
    ) -> i32,
>;
pub type KrunInputDestroyFn = Option<unsafe extern "C" fn(instance: *mut c_void) -> i32>;
pub type KrunInputQueryDeviceNameFn = Option<
    unsafe extern "C" fn(instance: *mut c_void, name_buf: *mut u8, name_buf_len: usize) -> i32,
>;
pub type KrunInputQuerySerialNameFn = Option<
    unsafe extern "C" fn(instance: *mut c_void, name_buf: *mut u8, name_buf_len: usize) -> i32,
>;
pub type KrunInputQueryDeviceIdsFn =
    Option<unsafe extern "C" fn(instance: *mut c_void, ids: *mut KrunInputDeviceIds) -> i32>;
pub type KrunInputQueryEventCapabilitiesFn = Option<
    unsafe extern "C" fn(
        instance: *mut c_void,
        ev_type: u8,
        bitmap: *mut u8,
        bitmap_len: usize,
    ) -> i32,
>;
pub type KrunInputQueryAbsInfoFn = Option<
    unsafe extern "C" fn(instance: *mut c_void, code: u8, absinfo: *mut KrunInputAbsinfo) -> i32,
>;
pub type KrunInputQueryPropertiesFn =
    Option<unsafe extern "C" fn(instance: *mut c_void, bitmap: *mut u8, bitmap_len: usize) -> i32>;

pub type KrunInputGetReadyEfdFn = Option<unsafe extern "C" fn(instance: *mut c_void) -> i32>;
pub type KrunInputNextEventFn =
    Option<unsafe extern "C" fn(instance: *mut c_void, event: *mut KrunInputEvent) -> i32>;

#[repr(C)]
pub struct KrunInputConfigVtable {
    pub destroy: KrunInputDestroyFn,
    pub query_device_name: KrunInputQueryDeviceNameFn,
    pub query_serial_name: KrunInputQuerySerialNameFn,
    pub query_device_ids: KrunInputQueryDeviceIdsFn,
    pub query_event_capabilities: KrunInputQueryEventCapabilitiesFn,
    pub query_abs_info: KrunInputQueryAbsInfoFn,
    pub query_properties: KrunInputQueryPropertiesFn,
}

#[repr(C)]
pub struct KrunInputConfig {
    pub features: u64,
    pub create_userdata: *mut c_void,
    pub create: KrunInputCreateFn,
    pub vtable: KrunInputConfigVtable,
}

unsafe impl Send for KrunInputConfig {}
unsafe impl Sync for KrunInputConfig {}

#[repr(C)]
pub struct KrunInputEventProviderVtable {
    pub destroy: KrunInputDestroyFn,
    pub get_ready_efd: KrunInputGetReadyEfdFn,
    pub next_event: KrunInputNextEventFn,
}

#[repr(C)]
pub struct KrunInputEventProvider {
    pub features: u64,
    pub create_userdata: *mut c_void,
    pub create: KrunInputCreateFn,
    pub vtable: KrunInputEventProviderVtable,
}

unsafe impl Send for KrunInputEventProvider {}
unsafe impl Sync for KrunInputEventProvider {}

pub struct DeviceQueue {
    events: Mutex<VecDeque<KrunInputEvent>>,
    pipe: [libc::c_int; 2],
}

impl DeviceQueue {
    pub fn new() -> Self {
        let mut fds = [0i32; 2];
        unsafe {
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                panic!("pipe() failed: {}", std::io::Error::last_os_error());
            }
            libc::fcntl(fds[0], libc::F_SETFL, libc::O_NONBLOCK);
            libc::fcntl(fds[1], libc::F_SETFL, libc::O_NONBLOCK);
        }
        DeviceQueue {
            events: Mutex::new(VecDeque::new()),
            pipe: fds,
        }
    }

    pub fn push_batch(&self, events: &[KrunInputEvent]) {
        if let Ok(mut q) = self.events.lock() {
            q.extend(events.iter().copied());
            unsafe {
                libc::write(self.pipe[1], &1u8 as *const u8 as *const _, 1);
            }
        }
    }

    pub fn pop(&self) -> Option<KrunInputEvent> {
        let mut q = self.events.lock().ok()?;
        let event = q.pop_front();
        if event.is_none() {
            let mut buf = [0u8; 64];
            loop {
                let n = unsafe { libc::read(self.pipe[0], buf.as_mut_ptr() as *mut _, buf.len()) };
                if n <= 0 {
                    break;
                }
            }
        }
        event
    }

    pub fn read_fd(&self) -> i32 {
        self.pipe[0]
    }
}

pub static KEYBOARD_QUEUE: OnceLock<DeviceQueue> = OnceLock::new();
pub static MOUSE_QUEUE: OnceLock<DeviceQueue> = OnceLock::new();

pub fn keyboard_queue() -> &'static DeviceQueue {
    KEYBOARD_QUEUE.get_or_init(DeviceQueue::new)
}

pub fn mouse_queue() -> &'static DeviceQueue {
    MOUSE_QUEUE.get_or_init(DeviceQueue::new)
}

pub unsafe fn write_bitmap(bitmap: *mut u8, bitmap_len: usize, n: u16) {
    let byte_idx = (n / 8) as usize;
    let bit_idx = n % 8;
    if byte_idx < bitmap_len {
        unsafe {
            let ptr = bitmap.add(byte_idx);
            *ptr |= 1 << bit_idx;
        }
    }
}
