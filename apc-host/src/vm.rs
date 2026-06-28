use std::path::PathBuf;

#[derive(Debug)]
pub struct VmConfig {
    pub kernel: PathBuf,
    pub initrd: Option<PathBuf>,
    pub disk: Option<PathBuf>,
    pub cmdline: String,
    pub cpus: usize,
    pub memory_mb: u64,
    pub display_width: u32,
    pub display_height: u32,
    pub display_scale: u32,
    pub shared_dir: Option<PathBuf>,
    pub mcp_test: bool,
    pub allow_mount: Vec<String>,
    pub headless: bool,
    pub mcp_stdio: bool,
    pub mcp_http: Option<crate::mcp_http::McpHttpConfig>,
}

#[cfg(target_os = "macos")]
pub mod krun {
    use super::VmConfig;
    use crate::display;
    use crate::input;
    use crate::krun_ffi::*;
    use anyhow::{Context, Result};
    use std::ffi::CString;

    fn check(ret: i32, op: &str) -> Result<i32> {
        if ret < 0 {
            Err(anyhow::anyhow!("{op} failed: {ret}"))
        } else {
            Ok(ret)
        }
    }

    pub fn configure_vm(
        config: &VmConfig,
        mcp_socket_path: &str,
        fs_socket_path: &str,
    ) -> Result<(u32, i32)> {
        unsafe {
            // Pre-load ANGLE's libEGL on the main thread.
            // ANGLE's static initializers access Cocoa/Metal and deadlock
            // if first loaded on a background thread (libkrun GPU worker).
            {
                use std::ffi::CStr;
                let name = CStr::from_bytes_with_nul(b"libEGL.dylib\0").unwrap();
                let handle = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
                if handle.is_null() {
                    tracing::warn!("failed to pre-load libEGL.dylib (GPU accel may fail)");
                } else {
                    tracing::info!("pre-loaded libEGL.dylib for ANGLE");
                }
            }

            krun_set_log_level(3); // INFO
            let ctx = check(krun_create_ctx(), "krun_create_ctx")? as u32;

            check(
                krun_set_vm_config(ctx, config.cpus as u8, config.memory_mb as u32),
                "krun_set_vm_config",
            )?;

            let kernel_path = CString::new(config.kernel.to_str().context("invalid kernel path")?)?;

            let initramfs = config
                .initrd
                .as_ref()
                .map(|p| CString::new(p.to_str().unwrap_or("")))
                .transpose()?;

            let cmdline_with_scale =
                format!("{} apc.scale={}", config.cmdline, config.display_scale);
            let cmdline = CString::new(cmdline_with_scale.as_str())?;

            check(
                krun_set_kernel(
                    ctx,
                    kernel_path.as_ptr(),
                    KRUN_KERNEL_FORMAT_RAW,
                    initramfs
                        .as_ref()
                        .map(|s| s.as_ptr())
                        .unwrap_or(std::ptr::null()),
                    cmdline.as_ptr(),
                ),
                "krun_set_kernel",
            )?;

            let gpu_supported = krun_has_feature(KRUN_FEATURE_GPU as u64);
            tracing::info!(gpu_supported, "GPU feature check");

            if gpu_supported == 1 {
                let gpu_flags: u32 =
                    VIRGLRENDERER_USE_EGL | VIRGLRENDERER_USE_GLES | VIRGLRENDERER_USE_ASYNC_FENCE_CB;
                let shm_size: u64 = 512 * 1024 * 1024;
                check(
                    krun_set_gpu_options2(ctx, gpu_flags, shm_size),
                    "krun_set_gpu_options2",
                )?;

                let phys_w = config.display_width * config.display_scale;
                let phys_h = config.display_height * config.display_scale;
                let display_id = check(krun_add_display(ctx, phys_w, phys_h), "krun_add_display")?;
                tracing::info!(
                    display_id,
                    phys_w,
                    phys_h,
                    scale = config.display_scale,
                    "display added"
                );

                let backend = if config.headless {
                    Box::new(crate::headless::create_headless_backend())
                } else {
                    Box::new(display::create_backend())
                };
                let backend_ptr = &*backend as *const KrunDisplayBackend as *const std::ffi::c_void;
                check(
                    krun_set_display_backend(
                        ctx,
                        backend_ptr,
                        std::mem::size_of::<KrunDisplayBackend>(),
                    ),
                    "krun_set_display_backend",
                )?;
                std::mem::forget(backend);
            } else {
                tracing::warn!("GPU feature not available in libkrun build, skipping GPU config");
            }

            if krun_has_feature(KRUN_FEATURE_INPUT as u64) == 1 {
                let (kbd_config, kbd_events) = input::create_keyboard_backend();
                let kbd_config = Box::new(kbd_config);
                let kbd_events = Box::new(kbd_events);
                check(
                    krun_add_input_device(
                        ctx,
                        &*kbd_config as *const input::KrunInputConfig as *const std::ffi::c_void,
                        std::mem::size_of::<input::KrunInputConfig>(),
                        &*kbd_events as *const input::KrunInputEventProvider
                            as *const std::ffi::c_void,
                        std::mem::size_of::<input::KrunInputEventProvider>(),
                    ),
                    "krun_add_input_device (keyboard)",
                )?;
                std::mem::forget(kbd_config);
                std::mem::forget(kbd_events);

                let (mouse_config, mouse_events) = input::create_mouse_backend();
                let mouse_config = Box::new(mouse_config);
                let mouse_events = Box::new(mouse_events);
                check(
                    krun_add_input_device(
                        ctx,
                        &*mouse_config as *const input::KrunInputConfig as *const std::ffi::c_void,
                        std::mem::size_of::<input::KrunInputConfig>(),
                        &*mouse_events as *const input::KrunInputEventProvider
                            as *const std::ffi::c_void,
                        std::mem::size_of::<input::KrunInputEventProvider>(),
                    ),
                    "krun_add_input_device (mouse)",
                )?;
                std::mem::forget(mouse_config);
                std::mem::forget(mouse_events);
                tracing::info!("input devices registered (keyboard + mouse)");
            }

            {
                use std::fs::File;
                use std::os::unix::io::IntoRawFd;
                let log = File::create("/tmp/apc-console.log").context("create console log")?;
                let fd = log.into_raw_fd();
                check(
                    krun_add_serial_console_default(ctx, -1, fd),
                    "krun_add_serial_console_default",
                )?;
            }

            let socket_path = CString::new(mcp_socket_path)?;
            check(
                krun_add_vsock_port2(ctx, apc_protocol::VSOCK_PORT, socket_path.as_ptr(), true),
                "krun_add_vsock_port2 (mcp)",
            )?;

            let fs_path = CString::new(fs_socket_path)?;
            check(
                krun_add_vsock_port2(ctx, apc_protocol::fs::VSOCK_FS_PORT, fs_path.as_ptr(), true),
                "krun_add_vsock_port2 (fs)",
            )?;

            let (krun_net_fd, slirp_fd) =
                crate::slirp::create_socketpair().context("create net socketpair")?;

            let mut mac: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
            // No checksum/TSO offload — slirp expects valid checksums in all frames
            let features: u32 = 0;
            check(
                krun_add_net_unixgram(
                    ctx,
                    std::ptr::null(),
                    krun_net_fd,
                    mac.as_mut_ptr(),
                    features,
                    0,
                ),
                "krun_add_net_unixgram",
            )?;
            tracing::info!("virtio-net configured via slirp (disables TSI)");

            if let Some(disk) = &config.disk {
                let block_id = CString::new("root")?;
                let disk_path = CString::new(disk.to_str().context("invalid disk path")?)?;
                check(
                    krun_add_disk2(
                        ctx,
                        block_id.as_ptr(),
                        disk_path.as_ptr(),
                        KRUN_DISK_FORMAT_RAW,
                        false,
                    ),
                    "krun_add_disk2",
                )?;
            }

            if let Some(share) = &config.shared_dir {
                let tag = CString::new("shared")?;
                let path = CString::new(share.to_str().context("invalid shared dir path")?)?;
                check(
                    krun_add_virtiofs(ctx, tag.as_ptr(), path.as_ptr()),
                    "krun_add_virtiofs",
                )?;
            }

            tracing::info!(
                cpus = config.cpus,
                memory_mb = config.memory_mb,
                display = format!(
                    "{}x{}@{}x",
                    config.display_width, config.display_height, config.display_scale
                ),
                mcp_socket = mcp_socket_path,
                "VM configured via libkrun"
            );

            Ok((ctx, slirp_fd))
        }
    }

    pub fn start_vm(ctx: u32) {
        std::thread::Builder::new()
            .name("krun-vm".into())
            .spawn(move || unsafe {
                tracing::info!("starting microVM via krun_start_enter");
                let ret = krun_start_enter(ctx);
                tracing::error!(ret, "krun_start_enter returned (should not happen)");
            })
            .expect("failed to spawn VM thread");
    }
}
