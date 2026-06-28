use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::krun_ffi::*;
use crate::vm::{self, VmConfig};
use anyhow::Result;

const NUM_BUFFERS: usize = 3;

pub struct HeadlessDisplayState {
    inner: Mutex<BufferPool>,
    frame_seq: AtomicU64,
    last_seen: AtomicU64,
    width: AtomicU32,
    height: AtomicU32,
    stride: AtomicU32,
}

struct BufferPool {
    buffers: [Vec<u8>; NUM_BUFFERS],
    write_idx: usize,
    ready_idx: Option<usize>,
    read_idx: Option<usize>,
}

impl HeadlessDisplayState {
    fn new() -> Self {
        Self {
            inner: Mutex::new(BufferPool {
                buffers: [Vec::new(), Vec::new(), Vec::new()],
                write_idx: 0,
                ready_idx: None,
                read_idx: None,
            }),
            frame_seq: AtomicU64::new(0),
            last_seen: AtomicU64::new(0),
            width: AtomicU32::new(0),
            height: AtomicU32::new(0),
            stride: AtomicU32::new(0),
        }
    }

    pub fn vm_width(&self) -> u32 {
        self.width.load(Ordering::Relaxed)
    }

    pub fn vm_height(&self) -> u32 {
        self.height.load(Ordering::Relaxed)
    }

    /// Returns a copy of the latest completed frame as (pixels, width, height),
    /// or `None` if no new frame is available since the last call.
    pub fn capture_framebuffer(&self) -> Option<(Vec<u8>, u32, u32)> {
        let seq = self.frame_seq.load(Ordering::Acquire);
        let seen = self.last_seen.load(Ordering::Relaxed);
        if seq == seen {
            return None;
        }
        self.last_seen.store(seq, Ordering::Relaxed);

        let w = self.width.load(Ordering::Relaxed);
        let h = self.height.load(Ordering::Relaxed);
        if w == 0 || h == 0 {
            return None;
        }

        let mut guard = self.inner.lock().ok()?;
        let ready = guard.ready_idx.take()?;

        // Retire the previous read buffer so it can be reused for writing.
        guard.read_idx = Some(ready);

        let pixels = guard.buffers[ready].clone();
        Some((pixels, w, h))
    }

    pub fn capture_latest_framebuffer(&self) -> Option<(Vec<u8>, u32, u32)> {
        let w = self.width.load(Ordering::Relaxed);
        let h = self.height.load(Ordering::Relaxed);
        if w == 0 || h == 0 {
            return None;
        }

        let guard = self.inner.lock().ok()?;
        let idx = guard.ready_idx.or(guard.read_idx)?;
        let pixels = guard.buffers[idx].clone();
        Some((pixels, w, h))
    }
}

static HEADLESS_DISPLAY: OnceLock<Arc<HeadlessDisplayState>> = OnceLock::new();

pub fn global_headless_display() -> Arc<HeadlessDisplayState> {
    HEADLESS_DISPLAY
        .get_or_init(|| Arc::new(HeadlessDisplayState::new()))
        .clone()
}

// ---------------------------------------------------------------------------
// KrunDisplayBackend C callbacks
// ---------------------------------------------------------------------------

unsafe extern "C" fn cb_create_headless(
    instance: *mut *mut c_void,
    _userdata: *const c_void,
    _reserved: *const c_void,
) -> i32 {
    unsafe { *instance = ptr::null_mut() };
    tracing::info!("headless display backend created");
    0
}

unsafe extern "C" fn cb_destroy_headless(_instance: *mut c_void) -> i32 {
    0
}

unsafe extern "C" fn cb_configure_scanout_headless(
    _instance: *mut c_void,
    _scanout_id: u32,
    _display_width: u32,
    _display_height: u32,
    width: u32,
    height: u32,
    _format: u32,
) -> i32 {
    let stride = width * 4;
    let display = global_headless_display();

    let old_w = display.width.load(Ordering::Relaxed);
    let old_h = display.height.load(Ordering::Relaxed);
    if old_w == width && old_h == height {
        return 0;
    }

    tracing::info!(width, height, "headless display scanout configured");
    display.width.store(width, Ordering::Relaxed);
    display.height.store(height, Ordering::Relaxed);
    display.stride.store(stride, Ordering::Relaxed);

    let buf_size = (stride as usize) * (height as usize);
    let new_buffers: [Vec<u8>; NUM_BUFFERS] = [
        vec![0u8; buf_size],
        vec![0u8; buf_size],
        vec![0u8; buf_size],
    ];

    let mut guard = display.inner.lock().unwrap();
    guard.buffers = new_buffers;
    guard.write_idx = 0;
    guard.ready_idx = None;
    guard.read_idx = None;

    0
}

unsafe extern "C" fn cb_disable_scanout_headless(_instance: *mut c_void, _scanout_id: u32) -> i32 {
    0
}

unsafe extern "C" fn cb_alloc_frame_headless(
    _instance: *mut c_void,
    _scanout_id: u32,
    buffer: *mut *mut u8,
    buffer_size: *mut usize,
) -> i32 {
    let display = global_headless_display();
    let mut guard = display.inner.lock().unwrap();

    let idx = guard.write_idx;
    let write_buf = &mut guard.buffers[idx];
    if write_buf.is_empty() {
        return -1;
    }

    unsafe {
        *buffer = write_buf.as_mut_ptr();
        *buffer_size = write_buf.len();
    }
    1
}

unsafe extern "C" fn cb_present_frame_headless(
    _instance: *mut c_void,
    _scanout_id: u32,
    _frame_id: u32,
    _damage_area: *const KrunRect,
) -> i32 {
    let display = global_headless_display();
    let mut guard = display.inner.lock().unwrap();

    let finished_idx = guard.write_idx;

    // The finished write buffer becomes the ready (latest) buffer.
    guard.ready_idx = Some(finished_idx);

    // Pick a new write buffer that is not the ready buffer or the read buffer.
    let read = guard.read_idx.unwrap_or(usize::MAX);
    for candidate in 0..NUM_BUFFERS {
        if candidate != finished_idx && candidate != read {
            guard.write_idx = candidate;
            break;
        }
    }

    display.frame_seq.fetch_add(1, Ordering::Release);

    0
}

// ---------------------------------------------------------------------------
// Public constructor
// ---------------------------------------------------------------------------

pub fn create_headless_backend() -> KrunDisplayBackend {
    let mut backend: KrunDisplayBackend = unsafe { std::mem::zeroed() };
    backend.features = KRUN_DISPLAY_FEATURE_BASIC_FRAMEBUFFER;
    backend.create_userdata = ptr::null_mut();
    backend.create = Some(cb_create_headless);
    backend.vtable.basic_framebuffer =
        std::mem::ManuallyDrop::new(KrunDisplayBasicFramebufferVtable {
            destroy: Some(cb_destroy_headless),
            disable_scanout: Some(cb_disable_scanout_headless),
            configure_scanout: Some(cb_configure_scanout_headless),
            alloc_frame: Some(cb_alloc_frame_headless),
            present_frame: Some(cb_present_frame_headless),
        });
    backend
}

// ---------------------------------------------------------------------------
// Headless entry point
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
pub fn run(config: VmConfig) -> Result<()> {
    let mcp_socket_path = format!("/tmp/agentos-mcp-{}.sock", std::process::id());
    let fs_socket_path = format!("/tmp/agentos-fs-{}.sock", std::process::id());

    let (ctx, slirp_fd) = vm::krun::configure_vm(&config, &mcp_socket_path, &fs_socket_path)?;

    crate::slirp::start(slirp_fd)?;

    tracing::info!("starting VM in headless mode");
    vm::krun::start_vm(ctx);

    let fs_server = crate::fs_server::FsServer::new(&fs_socket_path, config.allow_mount.clone());
    fs_server.start();

    if config.mcp_test {
        crate::mcp::run_mcp_test(&mcp_socket_path);
    }

    if let Some(mcp_http) = config.mcp_http.clone() {
        crate::mcp_http::start_server(mcp_socket_path.clone(), mcp_http)?;
    }

    if config.mcp_stdio {
        crate::mcp_stdio::run_stdio_proxy(&mcp_socket_path)?;
    } else {
        tracing::info!("headless mode running (Ctrl+C to stop)");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    Ok(())
}
