// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Auth-provider selection for the `aviso` binary.
//!
//! Four sources, tried in this order; the first that has a credential wins:
//!
//! 1. **Flag tier**: `--token <T>` -> `Bearer::new(T)`; else
//!    `--username <U>` + `--password <P>` -> `Basic::new(U, P)`;
//!    else no flag tier. Clap's `conflicts_with` rejects mixing
//!    `--token` with `--username`/`--password` at parse time.
//! 2. **Environment**: `AVISO_TOKEN`, or `AVISO_USERNAME` with
//!    `AVISO_PASSWORD`.
//! 3. **Config file**: the `auth:` block of the file the binary
//!    resolved, so `--config` is honoured.
//! 4. **Credentials file**: `credentials.yaml` in the aviso config
//!    directory, or the path in `AVISO_CREDENTIALS_FILE`. It is
//!    written by tools rather than by hand, so it ranks last and
//!    never overrides a credential the operator typed. It is the
//!    only source that re-reads on a 401, so a rotated token
//!    reaches a running listener.
//!
//! The last three are `aviso::auth::discover_with`, which is what
//! the library and the Python package use, so every surface agrees
//! on the order. The search stops at the first source that has a
//! credential: a later one is not read at all, and cannot fail a
//! command that was never going to use it.
//!
//! When no source has a credential the client runs anonymous; the
//! schema and health endpoints work this way against any
//! aviso-server.

use std::sync::Arc;

use anyhow::{Context, Result};
use aviso::ClientError;
use std::path::Path;

use aviso::auth::{AuthProvider, Basic, Bearer};

/// Builds an auth provider from the flag tier.
///
/// Returns `Ok(None)` if neither `--token` nor
/// `--username`/`--password` is supplied. Mutual exclusion between
/// `--token` and the basic-auth flags is enforced by clap at parse
/// time via `conflicts_with`, so this function trusts the input is
/// already mutually consistent.
///
/// # Errors
///
/// Propagates `Bearer::new` / `Basic::new` failures (e.g. empty
/// token) as `anyhow::Error`.
pub(crate) fn provider_from_flags(
    token: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<Option<Arc<dyn AuthProvider>>> {
    if let Some(t) = token {
        let bearer =
            Bearer::new(t.to_string()).context("build Bearer auth provider from --token flag")?;
        return Ok(Some(Arc::new(bearer)));
    }
    if let (Some(u), Some(p)) = (username, password) {
        let basic = Basic::new(u.to_string(), p.to_string())
            .context("build Basic auth provider from --username/--password flags")?;
        return Ok(Some(Arc::new(basic)));
    }
    Ok(None)
}

/// A selected provider and the name of the source it came from.
pub(crate) type SelectedProvider = (Option<Arc<dyn AuthProvider>>, Option<&'static str>);

/// Selects the provider and names the source it came from.
///
/// The flag tier is checked first because it is the most immediate
/// expression of intent. Everything below it is the shared search
/// in `aviso::auth::discover_with`, pointed at the config file this
/// invocation resolved.
///
/// This only finds the credential. Whether it may be sent to the
/// configured address is decided in `client_builder`, when a command
/// is about to make a request: `config dump` must be able to report
/// a refused source rather than fail on it.
///
/// `config_content` is the text the config file was already parsed
/// from, when it exists, so the `auth:` block is read from the same
/// snapshot as `base_url` and is never a second read of the path.
///
/// # Errors
///
/// Propagates a source that exists but cannot be used: a half-set
/// environment, or a file that cannot be read or parsed. A source
/// that is simply absent is skipped.
pub(crate) fn resolve_provider(
    flag_provider: Option<Arc<dyn AuthProvider>>,
    config_path: &Path,
    config_content: Option<String>,
) -> Result<SelectedProvider> {
    if let Some(provider) = flag_provider {
        return Ok((Some(provider), Some("flag")));
    }
    let mut paths = aviso::auth::DiscoveryPaths::from_env();
    // The credential must come from the same snapshot as every other
    // setting. When the file was read, hand over its text; when it was
    // absent at that moment, skip the config tier rather than reopen the
    // path, which could see a file created since and pair its credential
    // with settings parsed from nothing.
    match config_content {
        Some(content) => {
            paths.config_file = Some(config_path.to_path_buf());
            paths.config_content = Some(content);
        }
        None => paths.config_file = None,
    }
    let found = aviso::auth::discover_with(&paths).map_err(|e| match e {
        ClientError::Auth(reason) => crate::exit::usage_error(reason),
        other => anyhow::Error::from(other),
    })?;
    Ok(match found {
        Some(found) => {
            let label = found.source().label();
            (Some(found.into_provider()), Some(label))
        }
        None => (None, None),
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on provider construction is the expected diagnostic"
)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_snapshot_does_not_reopen_a_config_file_that_appeared_later() {
        // The file was absent when the configuration was loaded, so the
        // snapshot is None. A file written since must not supply the
        // credential, because every other setting came from the absent one.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "auth:\n  bearer_token: appeared-later\n").expect("write");
        let _isolate = IsolatedSources::new(dir.path());

        let (provider, source) = resolve_provider(None, &path, None).expect("resolve");

        assert!(provider.is_none(), "the later file must not be read");
        assert!(source.is_none());
    }

    #[test]
    fn a_present_snapshot_is_used_even_if_the_file_changed_since() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "auth:\n  bearer_token: on-disk-now\n").expect("write");
        let _isolate = IsolatedSources::new(dir.path());
        let snapshot = "auth:\n  bearer_token: from-snapshot\n".to_string();

        let (provider, source) = resolve_provider(None, &path, Some(snapshot)).expect("resolve");

        assert!(provider.is_some());
        assert_eq!(source, Some("config file"));
    }

    /// Points the environment tiers at nothing for the duration of a test.
    /// Serialised, because the process environment is shared.
    struct IsolatedSources {
        _guard: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    impl IsolatedSources {
        fn new(dir: &Path) -> Self {
            let guard = ENV_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let names = [
                "AVISO_TOKEN",
                "AVISO_USERNAME",
                "AVISO_PASSWORD",
                "AVISO_CREDENTIALS_FILE",
            ];
            let saved = names.iter().map(|k| (*k, std::env::var_os(k))).collect();
            // SAFETY: ENV_LOCK is held, so no other test in this binary reads
            // or writes these variables while they are changed.
            unsafe {
                for name in &names[..3] {
                    std::env::remove_var(name);
                }
                std::env::set_var(
                    "AVISO_CREDENTIALS_FILE",
                    dir.join("absent-credentials.yaml"),
                );
            }
            Self {
                _guard: guard,
                saved,
            }
        }
    }

    impl Drop for IsolatedSources {
        fn drop(&mut self) {
            // SAFETY: the lock is still held for the lifetime of this value.
            unsafe {
                for (name, value) in &self.saved {
                    match value {
                        Some(v) => std::env::set_var(name, v),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }

    #[test]
    fn flag_token_yields_bearer_provider() {
        let p = provider_from_flags(Some("the-token"), None, None).unwrap();
        assert!(p.is_some());
    }

    #[test]
    fn flag_username_password_yield_basic_provider() {
        let p = provider_from_flags(None, Some("alice"), Some("hunter2")).unwrap();
        assert!(p.is_some());
    }

    #[test]
    fn flag_username_only_yields_none() {
        let p = provider_from_flags(None, Some("alice"), None).unwrap();
        assert!(p.is_none());
    }

    #[test]
    fn flag_empty_token_errors() {
        let err = provider_from_flags(Some(""), None, None).unwrap_err();
        let s = err.to_string();
        assert!(
            s.contains("Bearer") || s.contains("--token"),
            "error should name the source: {s}"
        );
    }

    #[test]
    fn empty_flag_yields_none() {
        let p = provider_from_flags(None, None, None).unwrap();
        assert!(p.is_none());
    }
}
