// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Library entry point for the `aviso` command-line client.
//!
//! The `aviso` binary (`src/main.rs`) is a thin shim over [`run`], and the
//! `pyaviso` Python wheel's bundled `aviso` console command calls the same
//! [`run`] entry point through the `aviso-py` extension. Keeping the whole
//! CLI in the library (clap parsing, tracing setup, async dispatch, and
//! exit-code mapping) means both surfaces share one code path. Aside from the
//! second-Ctrl+C hard-exit escape hatch in the private `cancel` module,
//! [`std::process::exit`] lives in the binary, not in this library.

#![allow(
    clippy::doc_markdown,
    reason = "clap derive doc-comments are operator-facing --help text; backticks render literally in clap output and degrade UX"
)]

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

/// Color output mode for the global `--color auto|always|never` flag.
///
/// Translated to a per-stream `bool` via [`color_enabled`]: tracing
/// uses `stderr`'s TTY state; the echo trigger uses `stdout`'s. The
/// `auto` variant honours the `NO_COLOR` env var; `always` overrides
/// it (operator-supplied explicit override wins); `never` always
/// suppresses ANSI escapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ColorMode {
    /// Emit colors when the target output stream is a TTY and `NO_COLOR`
    /// is unset.
    Auto,
    /// Emit colors regardless of TTY state. Overrides `NO_COLOR`.
    Always,
    /// Never emit colors. Default.
    Never,
}

/// Pure helper that resolves a `ColorMode` to a `bool` for a specific
/// output stream.
///
/// Inputs are explicit (no env access, no TTY probing) so the function
/// is unit-testable without `std::env::set_var` (which is `unsafe` in
/// Rust 2024 and unsound to call after worker threads have spawned).
/// The CLI computes the inputs once at startup and passes the result
/// to (a) the tracing subscriber's `.with_ansi(...)` and (b) the lib's
/// [`aviso::set_echo_color_enabled`] before any listener spawns.
fn color_enabled(mode: ColorMode, is_terminal: bool, no_color_present: bool) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => !no_color_present && is_terminal,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code: unwrap on pure logic assertions is the expected diagnostic"
)]
mod tests {
    use super::{ColorMode, color_enabled};

    #[test]
    fn always_emits_color_regardless_of_tty_and_no_color() {
        assert!(color_enabled(ColorMode::Always, false, false));
        assert!(color_enabled(ColorMode::Always, false, true));
        assert!(color_enabled(ColorMode::Always, true, false));
        assert!(color_enabled(ColorMode::Always, true, true));
    }

    #[test]
    fn never_suppresses_color_regardless_of_tty_and_no_color() {
        assert!(!color_enabled(ColorMode::Never, false, false));
        assert!(!color_enabled(ColorMode::Never, false, true));
        assert!(!color_enabled(ColorMode::Never, true, false));
        assert!(!color_enabled(ColorMode::Never, true, true));
    }

    #[test]
    fn auto_emits_color_only_when_tty_and_no_color_unset() {
        assert!(color_enabled(ColorMode::Auto, true, false));
        assert!(
            !color_enabled(ColorMode::Auto, true, true),
            "NO_COLOR set => suppressed in auto mode"
        );
        assert!(
            !color_enabled(ColorMode::Auto, false, false),
            "non-TTY => suppressed in auto mode"
        );
        assert!(!color_enabled(ColorMode::Auto, false, true));
    }

    #[test]
    fn always_overrides_no_color_per_explicit_operator_choice() {
        assert!(
            color_enabled(ColorMode::Always, true, true),
            "--color always must override NO_COLOR (explicit operator override wins)"
        );
    }
}

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
mod tracing_format;

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
                  `aviso <SUBCOMMAND> --help` for per-command details, or \
                  https://github.com/ecmwf/aviso-client/tree/main/docs/src/cli for the full \
                  operator documentation.",
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
                     --ca-bundle multiple times for multiple certificates. The 'TLS' section at \
                     https://github.com/ecmwf/aviso-client/blob/main/docs/src/cli/configuration.md \
                     has end-to-end setup steps including how to fetch a PEM cert from a \
                     running server."
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
                     See the 'TLS' section at \
                     https://github.com/ecmwf/aviso-client/blob/main/docs/src/cli/configuration.md."
    )]
    danger_accept_invalid_certs: bool,

    /// Force JSON output (overrides TTY-aware default).
    #[arg(long, global = true)]
    json: bool,

    /// Color output mode. `never` (default) disables all ANSI escapes;
    /// `always` emits colors in the human-readable output paths
    /// regardless of TTY (overrides NO_COLOR); `auto` emits colors
    /// in the human-readable paths when the target output stream is
    /// a TTY and NO_COLOR is unset. A value is REQUIRED:
    /// `--color auto|always|never`. ANSI is never emitted into JSON
    /// (machine consumers via pipe/file) regardless of this flag.
    /// Per-stream: tracing checks stderr, echo trigger checks stdout,
    /// so `aviso listen --color auto | jq` correctly keeps stderr
    /// colored (TTY) and stdout JSON (pipe).
    #[arg(long, value_enum, default_value_t = ColorMode::Never, global = true)]
    color: ColorMode,

    /// Increase verbosity. Repeatable: -v = DEBUG, -vv = TRACE.
    /// Affects the aviso crates only; third-party crates (hyper,
    /// h2, reqwest, rustls) stay at WARN regardless. When the
    /// AVISO_LOG env var is set, its EnvFilter directive overrides
    /// this flag (operator-supplied policy is authoritative); use
    /// AVISO_LOG=h2=debug,hyper=debug,aviso=debug to also see
    /// transport-level diagnostics.
    #[arg(short = 'v', long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

/// Top-level subcommand enum. Each variant maps to one subcommand
/// of the `aviso` binary; the handler dispatch lives in [`dispatch`].
#[derive(Debug, Subcommand)]
enum Commands {
    /// Publish one notification to /api/v1/notification.
    ///
    /// Parameters are comma-separated. `event=<TYPE>` is required,
    /// `data=<JSON>` is optional, and all other entries enter the
    /// identifier map. Use `key:=JSON` for explicitly typed JSON values.
    Notify {
        /// Comma-separated parameters, for example
        /// `event=mars,count:=12,class=od,data={"x":1}`.
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
        /// section for this invocation. Ignored when `--event` and
        /// `--identifiers` are both supplied (inline mode takes
        /// precedence, matching `aviso replay`).
        listener_files: Vec<PathBuf>,

        /// Force MemoryStore for the invocation. Ignores any
        /// configured `state_file`.
        #[arg(long)]
        no_state_store: bool,

        /// Listener-level cursor override applied uniformly to every
        /// resolved listener. Accepts the same seven forms as
        /// `aviso replay --from`. When set, the listener's per-YAML
        /// `from_id` / `from_date` is overridden.
        #[arg(long, value_name = "VALUE")]
        from: Option<String>,

        /// Inline ad-hoc listener: event type to listen for, without
        /// a YAML file. Requires `--identifiers`. The inline pair
        /// takes precedence over any positional YAML files.
        #[arg(long, value_name = "TYPE", requires = "identifiers")]
        event: Option<String>,

        /// Inline ad-hoc listener: identifiers filter as a JSON
        /// object (e.g. `'{"class":"od"}'`). Requires `--event`.
        /// The inline listener runs with a single default echo
        /// trigger; for other triggers, use a YAML file instead.
        #[arg(long, value_name = "JSON", requires = "event")]
        identifiers: Option<String>,
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
        /// six date forms; see the '`--from` value formats' section
        /// at <https://github.com/ecmwf/aviso-client/blob/main/docs/src/cli/configuration.md>
        /// for the full list and the pure-digit-always-id ambiguity rule.
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

fn init_tracing(verbose: u8, ansi: bool) -> Result<()> {
    use std::io::IsTerminal as _;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt;

    // Filter policy is per-crate. The CLI binary and the core
    // library both compile under the crate name `aviso` (the
    // binary's `[[bin]] name = "aviso"` makes its module_path
    // resolve to `aviso`, same as the lib), so a single `aviso`
    // directive covers both. Every other crate (hyper, h2,
    // reqwest, rustls, etc.) stays at WARN regardless of -v so
    // the operator does not get flooded with HTTP/2 frame logs
    // when they asked for "a bit more detail from aviso". Power
    // users who want transport diagnostics set `AVISO_LOG`
    // explicitly (e.g. `AVISO_LOG=h2=debug,hyper=debug,aviso=debug`),
    // and that operator-supplied directive overrides -v entirely.
    let our_level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = if let Ok(directives) = std::env::var("AVISO_LOG") {
        EnvFilter::builder()
            .with_default_directive(LevelFilter::WARN.into())
            .parse_lossy(directives)
    } else {
        let directive_str = format!("warn,aviso={our_level}");
        EnvFilter::try_new(directive_str).context("constructing default tracing filter")?
    };

    // Output format is TTY-aware. Interactive operators see a
    // compact human-readable line per event (colored only when the
    // operator opts in via `--color auto|always`, off by default);
    // headless deployments (piped stderr, systemd, CI) get OTel-JSON
    // for log aggregators (never colored regardless of the flag).
    // Detection is on stderr (not stdout) so the common
    // `aviso listen | tee log.txt` pattern correctly keeps the
    // operator's terminal human-friendly while the file gets the
    // operator's chosen trigger output.
    // `try_init` returns Err only when a global subscriber is already
    // installed. That happens when `run` is called more than once in a single
    // process: the test suite calls `_run_cli` repeatedly, and a host program
    // embedding the extension could too. A failed install is treated as
    // success there, leaving the first subscriber in place.
    if std::io::stderr().is_terminal() {
        let _ = fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_target(false)
            .with_timer(tracing_format::ShortClockTimer)
            .with_ansi(ansi)
            .compact()
            .try_init();
    } else {
        let _ = fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .event_format(tracing_format::OtelLogFormat::new())
            .try_init();
    }

    Ok(())
}

async fn dispatch(cli: Cli) -> Result<()> {
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
        config_path = %resolved.config_path.value.display(),
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
            from,
            event,
            identifiers,
        } => {
            commands::listen::run(
                &resolved,
                &listener_files,
                no_state_store,
                from.as_deref(),
                event.as_deref(),
                identifiers.as_deref(),
            )
            .await
        }
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

/// Runs the `aviso` command-line client to completion and returns the
/// process exit code.
///
/// This is the single entry point shared by the `aviso` binary
/// (`src/main.rs`) and the bundled `aviso` console command shipped in the
/// `pyaviso` Python wheel through the `aviso-py` extension. It owns argument
/// parsing, tracing setup, the async runtime, and the exit-code mapping, and
/// it does not call [`std::process::exit`] on its normal paths, so an embedding
/// process (the Python interpreter) keeps control of its own lifecycle. The one
/// exception is the second-Ctrl+C hard exit during `listen` / `replay`, which
/// terminates the process immediately by design.
///
/// `args` is the full argument vector including the program name at index 0,
/// matching [`std::env::args_os`] and `sys.argv`.
///
/// Exit codes: `0` success, `1` runtime error, `2` usage error. A clap parse
/// failure prints its message and returns clap's own exit code (`2`), while
/// `--help` and `--version` print and return `0`.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    use std::io::IsTerminal as _;

    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            let _ = err.print();
            return err.exit_code();
        }
    };
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let stderr_color = color_enabled(cli.color, std::io::stderr().is_terminal(), no_color);
    let stdout_color = color_enabled(cli.color, std::io::stdout().is_terminal(), no_color);
    aviso::set_echo_color_enabled(stdout_color);
    if let Err(e) = init_tracing(cli.verbose, stderr_color) {
        let _ = output::write_stderr_line(&format!("error: failed to initialise tracing: {e:#}"));
        return exit::RUNTIME_ERROR;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ =
                output::write_stderr_line(&format!("error: failed to start async runtime: {e:#}"));
            return exit::RUNTIME_ERROR;
        }
    };
    match runtime.block_on(dispatch(cli)) {
        Ok(()) => exit::SUCCESS,
        Err(e) => {
            let code = exit::exit_code_for_anyhow(&e);
            error::format_chain(&e);
            code
        }
    }
}
