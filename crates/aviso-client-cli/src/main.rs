use anyhow::{Context, Result};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "aviso-client",
    version = aviso_client::VERSION,
    about = "Command-line client for aviso-server",
    long_about = "Phase 0 scaffold. Subcommands land in Phase 4."
)]
struct Cli {}

fn init_tracing() -> Result<()> {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt;

    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .with_env_var("AVISO_LOG")
        .from_env_lossy();

    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .json()
        .try_init()
        .map_err(|e| anyhow::anyhow!("initialising tracing subscriber: {e}"))?;

    Ok(())
}

fn main() -> Result<()> {
    init_tracing().context("tracing init")?;
    let _cli = Cli::parse();

    tracing::info!(
        service.name = "aviso-client-cli",
        service.version = aviso_client::VERSION,
        event.name = "cli.startup.scaffold",
        "Phase 0 scaffold; no subcommands wired yet"
    );

    Ok(())
}
