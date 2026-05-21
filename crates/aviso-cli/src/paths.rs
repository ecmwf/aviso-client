//! Cross-platform path resolution for the CLI's config and state files.
//!
//! User direction (verbatim from the design walk): "Just let's make it
//! ~/.config/aviso for the location of the config and the state". The
//! same directory holds both files on every platform; one place to
//! back up, consistent across Linux / macOS / Windows even where each
//! platform has its own native convention.
//!
//! `directories::UserDirs::home_dir()` is the cross-platform shim
//! (more reliable than `std::env::home_dir`, which was deprecated for
//! correctness issues). The resolver appends `.config/aviso` manually
//! rather than going through `directories::BaseDirs::config_dir`,
//! which returns `~/Library/Application Support` on macOS and
//! `%APPDATA%` on Windows: both diverge from the requested
//! `~/.config/aviso` convention.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

use crate::exit::usage_error;

/// Returns the resolved path to the config file, applying the
/// flag-over-env-over-default precedence per Q3.
///
/// Precedence:
/// 1. `--config <PATH>` flag (passed in as `flag`).
/// 2. `AVISO_CLIENT_CONFIG_FILE` environment variable (passed in as
///    `env_value` rather than read internally, so the surrounding
///    glue can decide whether to read live or from a parsed
///    snapshot at test time).
/// 3. Default: `<home>/.config/aviso/config.yaml`.
///
/// The returned path is guaranteed absolute. Absolute inputs are
/// returned as-is (no symlink resolution or `..` normalisation).
/// Relative inputs are joined with the current working directory
/// and then best-effort canonicalised via
/// [`std::path::Path::canonicalize`]; if canonicalisation fails
/// (e.g. the file does not exist yet) the cwd-joined path is
/// returned unchanged so a not-yet-created default file still
/// surfaces as absolute in error messages per Error UX rule 3.
///
/// # Errors
///
/// Returns a usage error tagged via [`crate::exit::usage_error`] when
/// no home directory can be resolved AND the default path is needed
/// (i.e. neither flag nor env value was supplied). On most systems
/// `home_dir()` returns `Some` so this path is rarely taken; it
/// exists for portability against environments where `HOME` is not
/// set.
pub(crate) fn resolve_config_path(
    flag: Option<&PathBuf>,
    env_value: Option<&str>,
) -> Result<PathBuf> {
    let raw = pick_path(flag, env_value, default_config_path)?;
    absolutize(&raw)
}

/// Returns the resolved path to the state file. Same precedence as
/// [`resolve_config_path`] but with the env var
/// `AVISO_STATE_FILE` and the default
/// `<home>/.config/aviso/state.json`.
pub(crate) fn resolve_state_path(
    flag: Option<&PathBuf>,
    env_value: Option<&str>,
) -> Result<PathBuf> {
    let raw = pick_path(flag, env_value, default_state_path)?;
    absolutize(&raw)
}

/// Builds the default config-file path
/// `<home>/.config/aviso/config.yaml`. Returns a usage error when
/// the home directory cannot be resolved.
pub(crate) fn default_config_path() -> Result<PathBuf> {
    Ok(aviso_config_dir()?.join("config.yaml"))
}

/// Builds the default state-file path
/// `<home>/.config/aviso/state.json`. Returns a usage error when
/// the home directory cannot be resolved.
pub(crate) fn default_state_path() -> Result<PathBuf> {
    Ok(aviso_config_dir()?.join("state.json"))
}

/// Returns the canonical `<home>/.config/aviso` directory.
///
/// The directory is NOT created here. Callers that need it on disk
/// (e.g. the `JsonFileStore` lazy-open path) call
/// `std::fs::create_dir_all(...)` themselves with `0700` perms on
/// Unix.
pub(crate) fn aviso_config_dir() -> Result<PathBuf> {
    let home = directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .ok_or_else(|| {
            usage_error("could not resolve home directory; set HOME (Unix) or %USERPROFILE% (Windows), or pass --config <PATH> explicitly")
        })?;
    Ok(home.join(".config").join("aviso"))
}

fn pick_path<F>(flag: Option<&PathBuf>, env_value: Option<&str>, default_fn: F) -> Result<PathBuf>
where
    F: FnOnce() -> Result<PathBuf>,
{
    if let Some(p) = flag {
        return Ok(p.clone());
    }
    if let Some(s) = env_value {
        return Ok(PathBuf::from(s));
    }
    default_fn()
}

fn absolutize(path: &PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.clone());
    }
    let cwd =
        std::env::current_dir().context("resolve current working directory for relative path")?;
    let joined = cwd.join(path);
    let canonical = joined.canonicalize().unwrap_or_else(|_| joined.clone());
    if !canonical.is_absolute() {
        return Err(anyhow!(
            "could not render path absolute: {}",
            path.display()
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on path resolution is the expected diagnostic"
)]
mod tests {
    use super::*;

    #[test]
    fn flag_value_wins_over_env_and_default() {
        let flag = PathBuf::from("/tmp/aviso-test-flag.yaml");
        let got = resolve_config_path(Some(&flag), Some("/tmp/aviso-test-env.yaml")).unwrap();
        assert_eq!(got, flag);
    }

    #[test]
    fn env_value_wins_when_no_flag() {
        let env = "/tmp/aviso-test-env.yaml";
        let got = resolve_config_path(None, Some(env)).unwrap();
        assert_eq!(got, PathBuf::from(env));
    }

    #[test]
    fn default_used_when_neither_flag_nor_env() {
        let got = resolve_config_path(None, None).expect("home_dir resolves on this platform");
        assert!(
            got.ends_with(".config/aviso/config.yaml"),
            "default path should end with .config/aviso/config.yaml; got {}",
            got.display()
        );
    }

    #[test]
    fn state_default_resolves_to_state_json_in_aviso_config_dir() {
        let got = resolve_state_path(None, None).expect("home_dir resolves on this platform");
        assert!(
            got.ends_with(".config/aviso/state.json"),
            "default state path should end with .config/aviso/state.json; got {}",
            got.display()
        );
    }

    #[test]
    fn relative_path_made_absolute() {
        let rel = PathBuf::from("relative/config.yaml");
        let got = resolve_config_path(Some(&rel), None).unwrap();
        assert!(got.is_absolute(), "got {}", got.display());
    }

    #[test]
    fn aviso_config_dir_path_shape() {
        let dir = aviso_config_dir().expect("home_dir resolves on this platform");
        assert!(
            dir.ends_with(".config/aviso"),
            "config dir should end with .config/aviso; got {}",
            dir.display()
        );
    }
}
