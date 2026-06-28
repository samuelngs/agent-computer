mod app;
mod display;
pub mod fs_server;
pub mod headless;
pub mod input;
mod krun_ffi;
pub mod mcp;
mod mcp_capture;
pub mod mcp_http;
pub mod mcp_stdio;
pub mod slirp;
mod vm;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "apc-host",
    about = "Agent Personal Computer (APC) — your AI Agent's personal computer"
)]
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
    #[arg(
        long,
        default_value = "console=ttyAMA0 root=/dev/vda rootfstype=ext4 modules=virtio_mmio,virtio_blk,virtio_input rw"
    )]
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

    /// Run without GUI window (framebuffer in memory only)
    #[arg(long)]
    headless: bool,

    /// Expose MCP over stdin/stdout (implies --headless)
    #[arg(long)]
    mcp_stdio: bool,

    /// Expose MCP over Streamable HTTP on /mcp
    #[arg(long)]
    mcp_http_port: Option<u16>,

    /// Host/address for --mcp-http-port
    #[arg(long, default_value = "127.0.0.1")]
    mcp_http_host: String,

    /// Bearer token for --mcp-http-port (or APC_MCP_HTTP_TOKEN)
    #[arg(long)]
    mcp_http_token: Option<String>,
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

    let headless = cli.headless || cli.mcp_stdio;

    let mcp_http = if let Some(port) = cli.mcp_http_port {
        let token = cli
            .mcp_http_token
            .or_else(|| std::env::var("APC_MCP_HTTP_TOKEN").ok())
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("--mcp-http-port requires --mcp-http-token or APC_MCP_HTTP_TOKEN")
            })?;
        Some(mcp_http::McpHttpConfig {
            host: cli.mcp_http_host,
            port,
            token,
        })
    } else {
        None
    };

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
        headless,
        mcp_stdio: cli.mcp_stdio,
        mcp_http,
    };

    #[cfg(target_os = "macos")]
    if headless {
        headless::run(config)?;
    } else {
        app::run(config)?;
    }

    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("APC host requires macOS with Hypervisor.framework");

    Ok(())
}
