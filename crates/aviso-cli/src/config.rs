// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Configuration model for the `aviso` binary.
//!
//! Two responsibilities:
//!
//! 1. Deserialise the YAML config file (typically
//!    `~/.config/aviso/config.yaml`) into a typed [`ConfigFile`].
//! 2. Materialise a [`Resolved`] by walking flag, env, and file
//!    layers per Q3's per-field precedence (`flag > env > file >
//!    default`).
//!
//! The `auth:` block is accepted here so the key is known, but it is
//! not interpreted here. `aviso::auth::discover_with` parses it and
//! builds the provider, so the binary and the library read it the
//! same way. That search also covers the environment and the
//! credentials file; see `crate::auth`.
//!
//! The `auth:` section is OPTIONAL. A config file with no `auth:`
//! block is fine; the search falls back to the flag, the
//! environment, the credentials file, or no auth at all (anonymous
//! access to schema / health endpoints).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use aviso::auth::AuthProvider;
use serde::Deserialize;
use serde_norway as yaml;

use crate::auth as cli_auth;
use crate::exit::usage_error;
use crate::paths;

/// YAML schema for the CLI's config file.
///
/// `#[serde(deny_unknown_fields)]` catches misspelled top-level keys
/// loudly so the operator sees a clear "unknown field" error rather
/// than the value being silently ignored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigFile {
    #[serde(default)]
    pub(crate) base_url: Option<String>,
    /// Accepted so `auth:` is a known key under `deny_unknown_fields`. The
    /// block is parsed by `aviso::auth::discover_with`, which reads it only
    /// when no higher-priority source supplied a credential, so the value is
    /// deliberately not kept here.
    #[serde(default, rename = "auth")]
    pub(crate) _auth: Option<serde_norway::Value>,
    #[serde(default, with = "humantime_serde::option")]
    pub(crate) timeout: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    pub(crate) heartbeat_interval: Option<Duration>,
    #[serde(default)]
    pub(crate) state_file: Option<PathBuf>,
    #[serde(default)]
    pub(crate) tls: Option<TlsConfig>,
    #[serde(default)]
    pub(crate) listeners: Vec<ListenerSpec>,
}

/// `tls:` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TlsConfig {
    #[serde(default)]
    pub(crate) ca_bundle: Vec<PathBuf>,
    #[serde(default)]
    pub(crate) danger_accept_invalid_certs: bool,
}

/// A single listener entry under top-level `listeners:`.
///
/// The shape is pyaviso-compatible with one rename: pyaviso's
/// `request:` key is `identifiers:` here per Amendment B (matching
/// the lib's `Notification::identifier` field name).
///
/// Triggers reuse the lib's `TriggerConfig` deserialiser so all six
/// shipped trigger kinds (echo, log, command, webhook, teams, post)
/// are available from YAML unchanged.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListenerSpec {
    #[serde(default)]
    pub(crate) name: Option<String>,
    pub(crate) event: String,
    #[serde(default)]
    pub(crate) identifiers: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub(crate) from_id: Option<u64>,
    #[serde(default)]
    pub(crate) from_date: Option<String>,
    #[serde(default)]
    pub(crate) triggers: Vec<aviso::watch::TriggerConfig>,
}

/// Source tag attached to every field in [`Resolved`] so the
/// `config dump` subcommand can attribute each value to its origin.
/// The tag tracks ONLY the precedence layer that supplied the value;
/// it carries no payload of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// Value came from a command-line flag.
    Flag,
    /// Value came from an environment variable.
    Env,
    /// Value came from the config file.
    File,
    /// Value came from a built-in default.
    Default,
}

/// One layered value plus its source tag.
#[derive(Debug, Clone)]
pub(crate) struct Sourced<T> {
    pub(crate) value: T,
    pub(crate) source: Source,
}

/// The materialised configuration walked through every layer.
///
/// `resolve` builds this once at CLI startup; subcommand handlers
/// consume the resolved values. The `auth_provider` is the single
/// provider from the first source that had a credential, per Q8 +
/// amendment A2, and `auth_source` names that source; the TLS knobs
/// feed into the `AvisoClientBuilder` setters `.ca_bundle` and
/// `.danger_accept_invalid_certs`. Each path is rendered absolute
/// per Error UX rule 3.
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
    pub(crate) config_path: Sourced<PathBuf>,
    pub(crate) state_path: Sourced<PathBuf>,
    pub(crate) base_url: Option<Sourced<String>>,
    pub(crate) timeout: Option<Sourced<Duration>>,
    pub(crate) heartbeat_interval: Option<Sourced<Duration>>,
    pub(crate) tls_ca_bundle_paths: Sourced<Vec<PathBuf>>,
    pub(crate) tls_danger_accept_invalid_certs: Sourced<bool>,
    pub(crate) auth_provider: Option<Arc<dyn AuthProvider>>,
    /// Highest-priority tier that produced [`Self::auth_provider`].
    pub(crate) auth_source: Option<&'static str>,
    pub(crate) listeners: Vec<ListenerSpec>,
    pub(crate) force_json: bool,
    pub(crate) verbose: u8,
}

/// Walks the precedence layers and produces a [`Resolved`].
///
/// The `cli_*` arguments come from the parsed clap `Cli`. The
/// environment is read via [`read_env`], which surfaces a usage
/// error (exit 2) when an `AVISO_*` env var is set but its value
/// is not valid UTF-8 rather than silently falling back to a
/// lower-precedence layer.
///
/// # Errors
///
/// - Bad YAML in the config file (parse error with file
///   `<line>:<col>` location surfaced via the `serde_norway`
///   `Display` impl).
/// - Invalid auth combination (empty `--token`, etc.).
/// - Non-UTF-8 value in any `AVISO_*` env var (surfaced as a usage
///   error naming the offending variable).
/// - Home directory not resolvable AND no `--config` / env override
///   supplied (rare; needs to be a deliberate environment for the
///   home-dir lookup to fail).
#[allow(
    clippy::too_many_arguments,
    reason = "the resolver takes one argument per layered field; bundling them into a struct would only add a one-off type with no further consumers"
)]
pub(crate) fn resolve(
    cli_config: Option<&PathBuf>,
    cli_state_file: Option<&PathBuf>,
    cli_base_url: Option<&str>,
    cli_token: Option<&str>,
    cli_username: Option<&str>,
    cli_password: Option<&str>,
    cli_ca_bundle: &[PathBuf],
    cli_danger_accept_invalid_certs: bool,
    cli_force_json: bool,
    cli_verbose: u8,
) -> Result<Resolved> {
    let env_config_path = read_env("AVISO_CLIENT_CONFIG_FILE")?;
    let env_state_path = read_env("AVISO_STATE_FILE")?;
    let env_base_url = read_env("AVISO_BASE_URL")?;

    let config_path = {
        let value = paths::resolve_config_path(cli_config, env_config_path.as_deref())?;
        let source = if cli_config.is_some() {
            Source::Flag
        } else if env_config_path.is_some() {
            Source::Env
        } else {
            Source::Default
        };
        Sourced { value, source }
    };
    let file = load_optional(&config_path.value)
        .with_context(|| format!("at: {}", config_path.value.display()))?;

    let state_path = if let Some(p) = cli_state_file {
        Sourced {
            value: paths::resolve_state_path(Some(p), None)?,
            source: Source::Flag,
        }
    } else if let Some(s) = env_state_path.as_deref() {
        Sourced {
            value: paths::resolve_state_path(None, Some(s))?,
            source: Source::Env,
        }
    } else if let Some(p) = file.state_file.as_ref() {
        Sourced {
            value: paths::resolve_state_path(Some(p), None)?,
            source: Source::File,
        }
    } else {
        Sourced {
            value: paths::resolve_state_path(None, None)?,
            source: Source::Default,
        }
    };

    let base_url = cli_base_url
        .map(|s| Sourced {
            value: s.to_string(),
            source: Source::Flag,
        })
        .or_else(|| {
            env_base_url.clone().map(|s| Sourced {
                value: s,
                source: Source::Env,
            })
        })
        .or_else(|| {
            file.base_url.clone().map(|s| Sourced {
                value: s,
                source: Source::File,
            })
        });

    let timeout = file.timeout.map(|v| Sourced {
        value: v,
        source: Source::File,
    });
    let heartbeat_interval = file.heartbeat_interval.map(|v| Sourced {
        value: v,
        source: Source::File,
    });

    let (tls_ca_bundle_paths, tls_danger_accept_invalid_certs) = resolve_tls(
        cli_ca_bundle,
        cli_danger_accept_invalid_certs,
        file.tls.as_ref(),
    )?;

    let flag_provider = cli_auth::provider_from_flags(cli_token, cli_username, cli_password)?;
    let (auth_provider, auth_source) = cli_auth::resolve_provider(
        flag_provider,
        &config_path.value,
        base_url.as_ref().map(|b| b.value.as_str()),
    )?;

    Ok(Resolved {
        config_path,
        state_path,
        base_url,
        timeout,
        heartbeat_interval,
        tls_ca_bundle_paths,
        tls_danger_accept_invalid_certs,
        auth_provider,
        auth_source,
        listeners: file.listeners,
        force_json: cli_force_json,
        verbose: cli_verbose,
    })
}

fn resolve_tls(
    cli_ca_bundle: &[PathBuf],
    cli_danger: bool,
    file_tls: Option<&TlsConfig>,
) -> Result<(Sourced<Vec<PathBuf>>, Sourced<bool>)> {
    let ca_bundle = if !cli_ca_bundle.is_empty() {
        Sourced {
            value: absolutize_all(cli_ca_bundle)?,
            source: Source::Flag,
        }
    } else if let Some(tls) = file_tls {
        // The presence of a `tls:` block in the config IS a
        // statement of intent from the operator; tag the bundle as
        // File regardless of whether the list is empty so
        // `config dump` source attribution reflects the layer that
        // actually supplied the value rather than the bundle's
        // emptiness.
        Sourced {
            value: absolutize_all(&tls.ca_bundle)?,
            source: Source::File,
        }
    } else {
        Sourced {
            value: Vec::new(),
            source: Source::Default,
        }
    };

    let danger = if cli_danger {
        Sourced {
            value: true,
            source: Source::Flag,
        }
    } else if let Some(tls) = file_tls {
        Sourced {
            value: tls.danger_accept_invalid_certs,
            source: Source::File,
        }
    } else {
        Sourced {
            value: false,
            source: Source::Default,
        }
    };

    Ok((ca_bundle, danger))
}

/// Renders every path in `paths_in` absolute via [`paths::absolutize`]
/// so subsequent error messages quote absolute paths per the Error
/// UX rule 3 convention regardless of whether the operator supplied
/// relative or absolute inputs.
fn absolutize_all(paths_in: &[PathBuf]) -> Result<Vec<PathBuf>> {
    paths_in.iter().map(|p| paths::absolutize(p)).collect()
}

fn read_env(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Ok(Some(v)),
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(raw)) => Err(usage_error(format!(
            "env var {name} is set but its value is not valid UTF-8 ({}); set a UTF-8 value or unset the variable",
            raw.display()
        ))),
    }
}

/// Loads and parses `path` as a `ConfigFile`.
///
/// Returns `Ok(ConfigFile::default())` when `path` does not exist
/// (operating without a config file is supported; flag and env
/// overrides cover the common case). All other I/O errors and any
/// YAML parse error surface verbatim.
pub(crate) fn load_optional(path: &Path) -> Result<ConfigFile> {
    if !path.exists() {
        return Ok(ConfigFile::default());
    }
    let bytes =
        std::fs::read(path).with_context(|| format!("read config file: {}", path.display()))?;
    let cfg: ConfigFile = yaml::from_slice(&bytes)
        .with_context(|| format!("parse config file: {}", path.display()))?;
    Ok(cfg)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on yaml round-trip is the expected diagnostic"
)]
mod tests {
    use super::*;

    fn parse(yaml_text: &str) -> ConfigFile {
        yaml::from_str(yaml_text).expect("test YAML should parse")
    }

    #[test]
    fn parse_empty_yaml_yields_defaults() {
        let cfg = parse("");
        assert!(cfg.base_url.is_none());
        assert!(cfg.base_url.is_none());
        assert!(cfg.listeners.is_empty());
    }

    #[test]
    fn parse_full_config_round_trip() {
        let yaml_text = r#"
base_url: "https://aviso.example.org"
auth:
  bearer_token: "secret"
timeout: 30s
heartbeat_interval: 30s
state_file: /var/lib/aviso/state.json
tls:
  danger_accept_invalid_certs: false
listeners:
  - name: mars-od
    event: mars
    identifiers:
      class: od
      stream: oper
    triggers:
      - type: echo
"#;
        let cfg = parse(yaml_text);
        assert_eq!(cfg.base_url.as_deref(), Some("https://aviso.example.org"));
        assert_eq!(cfg.timeout, Some(Duration::from_secs(30)));
        assert_eq!(cfg.heartbeat_interval, Some(Duration::from_secs(30)));
        assert_eq!(
            cfg.state_file,
            Some(PathBuf::from("/var/lib/aviso/state.json"))
        );
        let tls = cfg.tls.expect("tls block present");
        assert!(!tls.danger_accept_invalid_certs);
        assert!(tls.ca_bundle.is_empty());
        assert_eq!(cfg.listeners.len(), 1);
        let listener = &cfg.listeners[0];
        assert_eq!(listener.name.as_deref(), Some("mars-od"));
        assert_eq!(listener.event, "mars");
        assert_eq!(listener.identifiers.len(), 2);
        assert_eq!(listener.triggers.len(), 1);
    }

    #[test]
    fn parse_accepts_a_nested_auth_block() {
        // The block's shape is the shared search's concern; this file only
        // has to accept the key. See aviso::auth::config_file_provider.
        let cfg = parse("auth:\n  basic:\n    username: alice\n    password: hunter2\n");

        assert!(cfg.listeners.is_empty());
    }

    #[test]
    fn parse_rejects_unknown_top_level_field() {
        let err = yaml::from_str::<ConfigFile>("bogus_key: 1").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("bogus_key") || msg.contains("unknown field"),
            "error should name the bad field: {msg}"
        );
    }

    #[test]
    fn parse_accepts_an_auth_block_it_does_not_interpret() {
        // A mistyped key inside auth: is reported by the shared search when
        // the block is actually read, not here, so a command that gets its
        // credential elsewhere is not failed by an unused block.
        let cfg = parse("auth:\n  bogus_key: 1\n");

        assert!(cfg.listeners.is_empty());
    }

    #[test]
    fn load_optional_returns_default_when_file_absent() {
        let cfg = load_optional(Path::new("/tmp/this-path-does-not-exist-aviso-test")).unwrap();
        assert!(cfg.base_url.is_none());
    }

    #[test]
    fn listeners_list_with_identifiers_field_name() {
        let yaml_text = r"
listeners:
  - event: mars
    identifiers:
      class: od
";
        let cfg = parse(yaml_text);
        assert_eq!(cfg.listeners.len(), 1);
        let l = &cfg.listeners[0];
        assert_eq!(l.event, "mars");
        assert_eq!(l.identifiers.len(), 1);
    }

    #[test]
    fn resolve_tls_absolutizes_relative_cli_ca_bundle_paths() {
        let rel = PathBuf::from("aviso-test-relative-flag-ca.pem");
        let (bundle, _) = resolve_tls(std::slice::from_ref(&rel), false, None).unwrap();
        assert_eq!(bundle.source, Source::Flag);
        assert_eq!(bundle.value.len(), 1);
        assert!(
            bundle.value[0].is_absolute(),
            "CA bundle path supplied via flag should be absolutized so error messages quote absolute paths; got {}",
            bundle.value[0].display()
        );
        assert!(
            bundle.value[0].ends_with("aviso-test-relative-flag-ca.pem"),
            "file name should be preserved; got {}",
            bundle.value[0].display()
        );
    }

    #[test]
    fn resolve_tls_absolutizes_relative_file_ca_bundle_paths() {
        let rel = PathBuf::from("aviso-test-relative-file-ca.pem");
        let tls = TlsConfig {
            ca_bundle: vec![rel.clone()],
            danger_accept_invalid_certs: false,
        };
        let (bundle, _) = resolve_tls(&[], false, Some(&tls)).unwrap();
        assert_eq!(bundle.source, Source::File);
        assert_eq!(bundle.value.len(), 1);
        assert!(
            bundle.value[0].is_absolute(),
            "CA bundle path supplied via file should be absolutized; got {}",
            bundle.value[0].display()
        );
    }

    #[test]
    fn resolve_tls_passes_absolute_ca_bundle_paths_through_unchanged() {
        let abs = PathBuf::from("/tmp/aviso-test-already-absolute.pem");
        let (bundle, _) = resolve_tls(std::slice::from_ref(&abs), false, None).unwrap();
        assert_eq!(bundle.value, vec![abs]);
    }
}
