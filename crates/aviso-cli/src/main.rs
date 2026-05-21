//! Command-line client for `aviso-server`.

#![allow(
    clippy::doc_markdown,
    reason = "clap derive doc-comments are operator-facing --help text; backticks render literally in clap output and degrade UX"
)]

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod auth;
mod cancel;
mod client_builder;
mod commands;
mod config;
mod error;
mod exit;
mod from_value;
mod listener;
mod listener_file;
mod output;
mod paths;

/// Top-level CLI. Holds the global flags shared across every
/// subcommand plus the dispatch into [`Commands`].
#[derive(Debug, Parser)]
#[command(
    name = "aviso",
    version = aviso::VERSION,
    about = "Command-line client for aviso-server",
    long_about = "The `aviso` command-line client for ECMWF's aviso-server notification service. \
                  Configuration lives in ~/.config/aviso/config.yaml by default; flag and env \
                  overrides take precedence per the documented config-layering rule. See \
                  `aviso <SUBCOMMAND> --help` for per-command details and docs/usage/cli.md for \
                  full operator documentation.",
)]
pub(crate) struct Cli {
    /// Path to the YAML config file. Default:
    /// ~/.config/aviso/config.yaml. Env override:
    /// AVISO_CLIENT_CONFIG_FILE.
    #[arg(short = 'c', long, value_name = "PATH", global = true)]
    config: Option<PathBuf>,

    /// Path to the JsonFileStore state file. Default:
    /// ~/.config/aviso/state.json. Env override: AVISO_STATE_FILE.
    #[arg(long, value_name = "PATH", global = true)]
    state_file: Option<PathBuf>,

    /// Override the aviso-server base URL. Env override:
    /// AVISO_BASE_URL.
    #[arg(long, value_name = "URL", global = true)]
    base_url: Option<String>,

    /// Bearer auth token. Mutually exclusive with --username and
    /// --password. Env override: AVISO_TOKEN.
    #[arg(
        long,
        value_name = "TOKEN",
        global = true,
        conflicts_with_all = ["username", "password"]
    )]
    token: Option<String>,

    /// Basic auth username. Requires --password. Mutually exclusive
    /// with --token. Env override: AVISO_USERNAME.
    #[arg(long, value_name = "USERNAME", global = true, requires = "password")]
    username: Option<String>,

    /// Basic auth password. Requires --username. Mutually exclusive
    /// with --token. Env override: AVISO_PASSWORD.
    #[arg(long, value_name = "PASSWORD", global = true, requires = "username")]
    password: Option<String>,

    /// Path to a PEM-encoded CA bundle to trust in addition to the
    /// system root store. Repeatable: pass --ca-bundle multiple
    /// times to add multiple certificates.
    #[arg(
        long,
        value_name = "PATH",
        global = true,
        long_help = "Path to PEM-encoded CA bundle to trust IN ADDITION TO the system roots. \
                     Use when the aviso-server is fronted by an internal CA not in the system \
                     trust store (private deployments behind corporate roots, self-hosted \
                     clusters with their own ACME setup, similar). The system root store stays \
                     in effect; --ca-bundle only adds, never replaces. Repeatable: pass \
                     --ca-bundle multiple times for multiple certificates. See \
                     docs/usage/cli.md 'TLS configuration' for end-to-end setup steps including \
                     how to fetch a PEM cert from a running server."
    )]
    ca_bundle: Vec<PathBuf>,

    /// Disable TLS certificate validation entirely. Insecure by
    /// design.
    #[arg(
        long,
        global = true,
        long_help = "Disable TLS certificate validation entirely. INSECURE; intended only for \
                     short-lived dev work against a self-signed aviso-server when shipping the \
                     cert via --ca-bundle is not practical. Logs WARN \
                     `event.name=cli.tls.insecure_mode` once per invocation so log scrapers can \
                     flag misuse. The right production move is always --ca-bundle, never this. \
                     See docs/usage/cli.md 'TLS configuration'."
    )]
    danger_accept_invalid_certs: bool,

    /// Force JSON output (overrides TTY-aware default).
    #[arg(long, global = true)]
    json: bool,

    /// Disable ANSI colour codes in human-readable output.
    #[arg(long, global = true)]
    no_color: bool,

    /// Increase verbosity. Repeatable: -v = DEBUG, -vv = TRACE.
    /// Overrides any default set via the AVISO_LOG env var.
    #[arg(short = 'v', long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

/// Top-level subcommand enum. Each variant maps to one subcommand
/// of the `aviso` binary; the handler dispatch lives in [`run`].
#[derive(Debug, Subcommand)]
enum Commands {
    /// Publish one notification to /api/v1/notification.
    ///
    /// Pyaviso-parity: parameters are a comma-separated key=value
    /// list; `event=<TYPE>` is required, `data=<JSON>` is optional
    /// (becomes the payload), all other key=value pairs enter the
    /// identifier map.
    Notify {
        /// Comma-separated key=value list. e.g.
        /// `event=mars,class=od,stream=oper,data={"x":1}`.
        parameters: String,
    },

    /// Run one or more listeners against /api/v1/watch.
    ///
    /// Listeners come from the positional YAML files (each carrying
    /// its own top-level `listeners:` list) when supplied, OR from
    /// the `listeners:` section of the global config when not.
    /// Spawns every resolved listener concurrently; a single
    /// listener's error WARNs but does not cancel siblings.
    Listen {
        /// Listener YAML files. Each file's `listeners:` list is
        /// concatenated in argv order; positional files REPLACE
        /// (not merge with) the global config's `listeners:`
        /// section for this invocation.
        listener_files: Vec<PathBuf>,

        /// Force MemoryStore for the invocation. Ignores any
        /// configured `state_file`.
        #[arg(long)]
        no_state_store: bool,
    },

    /// Replay historical notifications from a server-side cursor.
    Replay {
        /// Listener name from the resolved listener set. Required
        /// when more than one listener resolves.
        #[arg(long, value_name = "NAME")]
        listener: Option<String>,

        /// Override the listener's `event:` for an ad-hoc replay.
        /// Requires --identifiers.
        #[arg(long, value_name = "TYPE", requires = "identifiers")]
        event: Option<String>,

        /// Override the listener's `identifiers:` for an ad-hoc
        /// replay (JSON object). Requires --event.
        #[arg(long, value_name = "JSON", requires = "event")]
        identifiers: Option<String>,

        /// Required cursor. Accepts a u64 sequence id OR one of
        /// six date forms; see docs/usage/cli.md '--from value
        /// formats' for the full list and the pure-digit-always-id
        /// ambiguity rule.
        #[arg(long, value_name = "VALUE", required = true)]
        from: String,

        /// Listener YAML files. Same resolution semantics as
        /// `aviso listen`.
        listener_files: Vec<PathBuf>,
    },

    /// Schema operations.
    #[command(subcommand)]
    Schema(SchemaSubcommand),

    /// Destructive admin operations. Each leaf requires --yes.
    #[command(subcommand)]
    Admin(AdminSubcommand),

    /// Configuration introspection.
    #[command(subcommand)]
    Config(ConfigSubcommand),

    /// Print shell completions for the chosen shell to stdout.
    Completions {
        /// Target shell. One of: bash, zsh, fish, powershell,
        /// elvish.
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Subcommand)]
enum SchemaSubcommand {
    /// List all schemas registered on the server.
    List,
    /// Get the schema for one event type.
    Get {
        /// Event type whose schema to fetch.
        event_type: String,
    },
}

#[derive(Debug, Subcommand)]
enum AdminSubcommand {
    /// Wipe every notification for one event-type stream.
    WipeStream {
        /// Event type whose stream to wipe.
        event_type: String,
        /// Required confirmation. Without it the command exits 2
        /// with usage.
        #[arg(long)]
        yes: bool,
    },
    /// Wipe every notification across every stream.
    WipeAll {
        /// Required confirmation. Without it the command exits 2
        /// with usage.
        #[arg(long)]
        yes: bool,
    },
    /// Delete a single notification by its CloudEvents id
    /// (`<event_type>@<sequence>`).
    Delete {
        /// CloudEvents id of the notification to delete.
        notification_id: String,
        /// Required confirmation. Without it the command exits 2
        /// with usage.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigSubcommand {
    /// Dump the resolved config (flag-over-env-over-file applied)
    /// to stdout.
    Dump {
        /// Mask tokens and passwords in the output.
        #[arg(long)]
        redact: bool,
    },
}

fn init_tracing(verbose: u8) -> Result<()> {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt;

    let default = match verbose {
        0 => LevelFilter::INFO,
        1 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    };

    let filter = EnvFilter::builder()
        .with_default_directive(default.into())
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

async fn run(cli: Cli) -> Result<()> {
    tracing::info!(
        service.name = "aviso-cli",
        service.version = aviso::VERSION,
        event.name = "cli.startup",
        verbose = cli.verbose,
        "aviso CLI started"
    );

    let resolved = config::resolve(
        cli.config.as_ref(),
        cli.state_file.as_ref(),
        cli.base_url.as_deref(),
        cli.token.as_deref(),
        cli.username.as_deref(),
        cli.password.as_deref(),
        &cli.ca_bundle,
        cli.danger_accept_invalid_certs,
        cli.json,
        cli.no_color,
        cli.verbose,
    )?;

    if resolved.tls_danger_accept_invalid_certs.value {
        tracing::warn!(
            event.name = "cli.tls.insecure_mode",
            "TLS certificate validation disabled by --danger-accept-invalid-certs; do not use in production"
        );
    }

    tracing::debug!(
        event.name = "cli.config.resolved",
        config_path = %resolved.config_path.display(),
        state_path = %resolved.state_path.value.display(),
        base_url_set = resolved.base_url.is_some(),
        auth_provider_set = resolved.auth_provider.is_some(),
        listeners_count = resolved.listeners.len(),
        "resolved configuration"
    );

    match cli.command {
        Commands::Notify { parameters } => commands::notify::run(&resolved, &parameters).await,
        Commands::Listen {
            listener_files,
            no_state_store,
        } => commands::listen::run(&resolved, &listener_files, no_state_store).await,
        Commands::Replay {
            listener,
            event,
            identifiers,
            from,
            listener_files,
        } => {
            commands::replay::run(
                &resolved,
                &listener_files,
                listener.as_deref(),
                event.as_deref(),
                identifiers.as_deref(),
                &from,
            )
            .await
        }
        Commands::Schema(sub) => match sub {
            SchemaSubcommand::List => commands::schema::run_list(&resolved).await,
            SchemaSubcommand::Get { event_type } => {
                commands::schema::run_get(&resolved, &event_type).await
            }
        },
        Commands::Admin(sub) => match sub {
            AdminSubcommand::WipeStream { event_type, yes } => {
                if !yes {
                    return Err(exit::usage_error("aviso admin wipe-stream requires --yes"));
                }
                commands::admin::run_wipe_stream(&resolved, &event_type).await
            }
            AdminSubcommand::WipeAll { yes } => {
                if !yes {
                    return Err(exit::usage_error("aviso admin wipe-all requires --yes"));
                }
                commands::admin::run_wipe_all(&resolved).await
            }
            AdminSubcommand::Delete {
                notification_id,
                yes,
            } => {
                if !yes {
                    return Err(exit::usage_error("aviso admin delete requires --yes"));
                }
                commands::admin::run_delete(&resolved, &notification_id).await
            }
        },
        Commands::Config(ConfigSubcommand::Dump { redact }) => {
            commands::config_dump::run(&resolved, redact)
        }
        Commands::Completions { shell } => commands::completions::run(shell),
    }
}

#[tokio::main]
async fn main() {
    use std::io::Write as _;
    let cli = Cli::parse();
    if let Err(e) = init_tracing(cli.verbose) {
        let stderr = std::io::stderr();
        let mut guard = stderr.lock();
        let _ = writeln!(guard, "error: failed to initialise tracing: {e:#}");
        std::process::exit(exit::RUNTIME_ERROR);
    }
    match run(cli).await {
        Ok(()) => std::process::exit(exit::SUCCESS),
        Err(e) => {
            let code = exit::exit_code_for_anyhow(&e);
            error::format_chain(&e);
            std::process::exit(code);
        }
    }
}
