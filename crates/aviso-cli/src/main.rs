//! Command-line client for `aviso-server`.

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "aviso",
    version = aviso::VERSION,
    about = "Command-line client for aviso-server"
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
        .map_err(anyhow::Error::from_boxed)
        .context("initialising tracing subscriber")?;

    Ok(())
}

fn main() -> Result<()> {
    init_tracing()?;
    let _cli = Cli::parse();

    tracing::info!(
        service.name = "aviso-cli",
        service.version = aviso::VERSION,
        event.name = "cli.startup",
        "aviso CLI started"
    );

    Ok(())
}
