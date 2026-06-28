mod vfs;

use clap::Parser;
use fuser::MountOption;

#[derive(Parser)]
#[command(name = "apc-fuse")]
struct Cli {
    #[arg(long)]
    host_path: String,

    #[arg(long)]
    mount_point: String,

    #[arg(long, default_value_t = apc_protocol::fs::VSOCK_FS_PORT)]
    port: u32,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    std::fs::create_dir_all(&cli.mount_point)?;

    let fs = vfs::HostFs::listen_and_init(cli.port, &cli.host_path)?;

    let options = vec![
        MountOption::FSName("apc".into()),
        MountOption::AllowOther,
        MountOption::DefaultPermissions,
    ];

    eprintln!(
        "apc-fuse: mounting {} -> {}",
        cli.host_path, cli.mount_point
    );

    fuser::mount2(fs, &cli.mount_point, &options)?;

    Ok(())
}
