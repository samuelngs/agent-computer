mod bitmap;
mod cursor;
mod font;
mod grabs;
mod input;
mod mcp;
mod mcp_dispatch;
mod render;
mod state;
mod taskbar;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tracing::info!("AgentOS compositor starting");

    #[cfg(target_os = "linux")]
    {
        state::run()?;
    }

    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!("Compositor only runs inside Linux guest VM");
    }

    Ok(())
}
