#![allow(non_camel_case_types, dead_code)]

use std::ffi::c_void;

pub const KRUN_KERNEL_FORMAT_RAW: u32 = 0;
pub const KRUN_KERNEL_FORMAT_ELF: u32 = 1;
pub const KRUN_KERNEL_FORMAT_PE_GZ: u32 = 2;
pub const KRUN_KERNEL_FORMAT_IMAGE_BZ2: u32 = 3;
pub const KRUN_KERNEL_FORMAT_IMAGE_GZ: u32 = 4;
pub const KRUN_KERNEL_FORMAT_IMAGE_ZSTD: u32 = 5;

pub const KRUN_DISK_FORMAT_RAW: u32 = 0;

pub const VIRGLRENDERER_USE_EGL: u32 = 1 << 0;
pub const VIRGLRENDERER_THREAD_SYNC: u32 = 1 << 1;
pub const VIRGLRENDERER_USE_GLES: u32 = 1 << 4;
pub const VIRGLRENDERER_USE_EXTERNAL_BLOB: u32 = 1 << 5;
pub const VIRGLRENDERER_VENUS: u32 = 1 << 6;
pub const VIRGLRENDERER_NO_VIRGL: u32 = 1 << 7;
pub const VIRGLRENDERER_USE_ASYNC_FENCE_CB: u32 = 1 << 8;
pub const VIRGLRENDERER_RENDER_SERVER: u32 = 1 << 9;

pub const KRUN_LOG_LEVEL_OFF: u32 = 0;
pub const KRUN_LOG_LEVEL_ERROR: u32 = 1;
pub const KRUN_LOG_LEVEL_WARN: u32 = 2;
pub const KRUN_LOG_LEVEL_INFO: u32 = 3;

pub const KRUN_DISPLAY_FEATURE_BASIC_FRAMEBUFFER: u64 = 1;

pub const KRUN_DISPLAY_FORMAT_B8G8R8A8_UNORM: u32 = 1;
pub const KRUN_DISPLAY_FORMAT_B8G8R8X8_UNORM: u32 = 2;

pub const KRUN_FEATURE_GPU: u64 = 2;
pub const KRUN_FEATURE_INPUT: u64 = 4;
pub const KRUN_FEATURE_NET: u64 = 0;

pub const NET_FLAG_VFKIT: u32 = 1 << 0;
pub const NET_FLAG_DHCP_CLIENT: u32 = 1 << 1;

pub const NET_FEATURE_CSUM: u32 = 1 << 0;
pub const NET_FEATURE_GUEST_CSUM: u32 = 1 << 1;
pub const NET_FEATURE_GUEST_TSO4: u32 = 1 << 7;
pub const NET_FEATURE_HOST_TSO4: u32 = 1 << 11;

pub type krun_display_create_fn = Option<
    unsafe extern "C" fn(
        instance: *mut *mut c_void,
        userdata: *const c_void,
        reserved: *const c_void,
    ) -> i32,
>;
pub type krun_display_destroy_fn = Option<unsafe extern "C" fn(instance: *mut c_void) -> i32>;
pub type krun_display_configure_scanout_fn = Option<
    unsafe extern "C" fn(
        instance: *mut c_void,
        scanout_id: u32,
        display_width: u32,
        display_height: u32,
        width: u32,
        height: u32,
        format: u32,
    ) -> i32,
>;
pub type krun_display_disable_scanout_fn =
    Option<unsafe extern "C" fn(instance: *mut c_void, scanout_id: u32) -> i32>;
pub type krun_display_alloc_frame_fn = Option<
    unsafe extern "C" fn(
        instance: *mut c_void,
        scanout_id: u32,
        buffer: *mut *mut u8,
        buffer_size: *mut usize,
    ) -> i32,
>;
pub type krun_display_present_frame_fn = Option<
    unsafe extern "C" fn(
        instance: *mut c_void,
        scanout_id: u32,
        frame_id: u32,
        damage_area: *const KrunRect,
    ) -> i32,
>;

#[repr(C)]
pub struct KrunRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
pub struct KrunDisplayBasicFramebufferVtable {
    pub destroy: krun_display_destroy_fn,
    pub disable_scanout: krun_display_disable_scanout_fn,
    pub configure_scanout: krun_display_configure_scanout_fn,
    pub alloc_frame: krun_display_alloc_frame_fn,
    pub present_frame: krun_display_present_frame_fn,
}

#[repr(C)]
pub union KrunDisplayVtable {
    pub basic_framebuffer: std::mem::ManuallyDrop<KrunDisplayBasicFramebufferVtable>,
}

#[repr(C)]
pub struct KrunDisplayBackend {
    pub features: u64,
    pub create_userdata: *mut c_void,
    pub create: krun_display_create_fn,
    pub vtable: KrunDisplayVtable,
}

unsafe impl Send for KrunDisplayBackend {}
unsafe impl Sync for KrunDisplayBackend {}

unsafe extern "C" {
    pub fn krun_set_log_level(level: u32) -> i32;
    pub fn krun_set_console_output(ctx_id: u32, filepath: *const libc::c_char) -> i32;
    pub fn krun_create_ctx() -> i32;
    pub fn krun_free_ctx(ctx_id: u32) -> i32;
    pub fn krun_set_vm_config(ctx_id: u32, num_vcpus: u8, ram_mib: u32) -> i32;

    pub fn krun_set_kernel(
        ctx_id: u32,
        kernel_path: *const libc::c_char,
        kernel_format: u32,
        initramfs: *const libc::c_char,
        cmdline: *const libc::c_char,
    ) -> i32;

    pub fn krun_set_gpu_options2(ctx_id: u32, virgl_flags: u32, shm_size: u64) -> i32;
    pub fn krun_add_display(ctx_id: u32, width: u32, height: u32) -> i32;
    pub fn krun_set_display_backend(
        ctx_id: u32,
        display_backend: *const c_void,
        backend_size: usize,
    ) -> i32;

    pub fn krun_add_vsock_port2(
        ctx_id: u32,
        port: u32,
        c_filepath: *const libc::c_char,
        listen: bool,
    ) -> i32;

    pub fn krun_add_disk2(
        ctx_id: u32,
        block_id: *const libc::c_char,
        disk_path: *const libc::c_char,
        disk_format: u32,
        read_only: bool,
    ) -> i32;

    pub fn krun_add_virtiofs(
        ctx_id: u32,
        c_tag: *const libc::c_char,
        c_path: *const libc::c_char,
    ) -> i32;

    pub fn krun_add_virtio_console_default(
        ctx_id: u32,
        input_fd: libc::c_int,
        output_fd: libc::c_int,
        err_fd: libc::c_int,
    ) -> i32;

    pub fn krun_disable_implicit_console(ctx_id: u32) -> i32;

    pub fn krun_add_input_device(
        ctx_id: u32,
        config_backend: *const c_void,
        config_backend_size: usize,
        events_backend: *const c_void,
        events_backend_size: usize,
    ) -> i32;

    pub fn krun_add_serial_console_default(
        ctx_id: u32,
        input_fd: libc::c_int,
        output_fd: libc::c_int,
    ) -> i32;

    pub fn krun_add_net_unixgram(
        ctx_id: u32,
        c_path: *const libc::c_char,
        fd: libc::c_int,
        c_mac: *mut u8,
        features: u32,
        flags: u32,
    ) -> i32;

    pub fn krun_has_feature(feature: u64) -> i32;
    pub fn krun_start_enter(ctx_id: u32) -> i32;
}
