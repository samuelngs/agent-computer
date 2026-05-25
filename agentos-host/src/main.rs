mod app;
mod display;
pub mod fs_server;
pub mod input;
mod krun_ffi;
pub mod mcp;
pub mod slirp;
mod vm;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "agentos-host", about = "AgentOS — virtualized agent computer")]
pub struct Cli {
    /// Path to Linux kernel (vmlinuz)
    #[arg(long)]
    kernel: PathBuf,

    /// Path to initial ramdisk
    #[arg(long)]
    initrd: Option<PathBuf>,

    /// Path to root disk image
    #[arg(long)]
    disk: Option<PathBuf>,

    /// Kernel command line
    #[arg(long, default_value = "console=ttyAMA0 root=/dev/vda rootfstype=ext4 modules=virtio_mmio,virtio_blk,virtio_input rw")]
    cmdline: String,

    /// Number of CPUs
    #[arg(long, default_value = "4")]
    cpus: usize,

    /// Memory in megabytes
    #[arg(long, default_value = "4096")]
    memory: u64,

    /// Display width
    #[arg(long, default_value = "1920")]
    width: u32,

    /// Display height
    #[arg(long, default_value = "1080")]
    height: u32,

    /// Display scale factor (2 = HiDPI/Retina)
    #[arg(long, default_value = "1")]
    scale: u32,

    /// Shared directory (VirtioFS)
    #[arg(long)]
    share: Option<PathBuf>,

    /// Run MCP connectivity test after VM starts
    #[arg(long)]
    mcp_test: bool,

    /// Allowlist of host paths that can be mounted into guest (comma-separated, default: *)
    #[arg(long, default_value = "*")]
    allow_mount: String,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let allow_mount: Vec<String> = cli
        .allow_mount
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let config = vm::VmConfig {
        kernel: cli.kernel,
        initrd: cli.initrd,
        disk: cli.disk,
        cmdline: cli.cmdline,
        cpus: cli.cpus,
        memory_mb: cli.memory,
        display_width: cli.width,
        display_height: cli.height,
        display_scale: cli.scale.max(1),
        shared_dir: cli.share,
        mcp_test: cli.mcp_test,
        allow_mount,
    };

    #[cfg(target_os = "macos")]
    app::run(config)?;

    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("AgentOS host requires macOS with Hypervisor.framework");

    Ok(())
}
