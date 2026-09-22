// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Credential discovery: find credentials without being told where they are.
//!
//! [`discover`] looks in three places and stops at the first one that has
//! credentials:
//!
//! 1. the environment, via [`Env`];
//! 2. the `auth:` block of the config file;
//! 3. the credentials file, via [`ConfigFile`].
//!
//! The order puts the most immediate source first. Environment variables are
//! set for one command or one session. The config file is written by hand and
//! holds a deliberate choice. The credentials file is written by tools, so it
//! ranks last and never overrides a credential its owner typed themselves.
//!
//! Only the credentials file refreshes on `401`, because [`ConfigFile`]
//! re-reads its path. The other two produce a fixed credential. A token that
//! is rotated on disk is therefore picked up by a running listener when it
//! lives in the credentials file, and not when it lives in the config file.
//!
//! Discovery returns `Ok(None)` when it finds nothing, which leaves the client
//! anonymous. It returns an error when a source exists but cannot be used, so
//! a typo in a credentials file is reported rather than silently ignored.
//!
//! Because a discovered credential was never named by the caller, it is not
//! sent to a plaintext address. [`discover_for_url`] refuses one unless the
//! address is `https`, or points at the loopback interface where a local
//! server has no network exposure. Naming a provider explicitly bypasses this:
//! a caller who writes the credential into the call has already chosen where
//! it goes.

use std::path::PathBuf;
use std::sync::Arc;

use crate::auth::AuthProvider;
mod policy;
mod sources;

pub use policy::url_keeps_credentials_private;
pub use sources::{config_file_provider, credentials_file_provider, env_provider};

use policy::refuse_public_plaintext;

/// Environment variable that overrides the config-file path.
pub const ENV_CONFIG_FILE: &str = "AVISO_CLIENT_CONFIG_FILE";

/// Environment variable that overrides the credentials-file path.
pub const ENV_CREDENTIALS_FILE: &str = "AVISO_CREDENTIALS_FILE";

/// Where a discovered credential came from.
///
/// Carried alongside the provider so callers can report the winning source.
/// With three places to look, "I set a token but requests are still anonymous"
/// is otherwise hard to diagnose.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialSource {
    /// Read from the process environment.
    Environment,
    /// Read from the `auth:` block of the config file at this path.
    ConfigFile(PathBuf),
    /// Read from the credentials file at this path.
    CredentialsFile(PathBuf),
}

impl CredentialSource {
    /// Short, stable name of the source, for reports and dumps.
    ///
    /// Callers use this instead of matching, so adding a source later does
    /// not break them.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::ConfigFile(_) => "config file",
            Self::CredentialsFile(_) => "credentials file",
        }
    }
}

impl std::fmt::Display for CredentialSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Environment => write!(f, "environment"),
            Self::ConfigFile(p) | Self::CredentialsFile(p) => {
                write!(f, "{} {}", self.label(), p.display())
            }
        }
    }
}

/// A credential found by [`discover`], with the place it came from.
#[derive(Debug, Clone)]
pub struct Discovered {
    provider: Arc<dyn AuthProvider>,
    source: CredentialSource,
}

impl Discovered {
    /// The provider to hand to the client builder.
    #[must_use]
    pub fn provider(&self) -> Arc<dyn AuthProvider> {
        Arc::clone(&self.provider)
    }

    /// Where the credential came from.
    #[must_use]
    pub fn source(&self) -> &CredentialSource {
        &self.source
    }

    /// Consumes the pair and returns the provider.
    #[must_use]
    pub fn into_provider(self) -> Arc<dyn AuthProvider> {
        self.provider
    }
}

/// The files discovery reads.
///
/// [`Self::from_env`] applies the documented defaults. Callers that resolve
/// their own paths (the `aviso` binary honours a `--config` flag) set the
/// fields directly so discovery reads the same file the caller does.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct DiscoveryPaths {
    /// Config file whose `auth:` block is consulted. `None` skips that step.
    pub config_file: Option<PathBuf>,
    /// Credentials file. `None` skips that step.
    pub credentials_file: Option<PathBuf>,
}

impl DiscoveryPaths {
    /// Default paths, with both entries overridable by environment variable.
    ///
    /// The defaults are `config.yaml` and `credentials.yaml` under
    /// `$HOME/.config/aviso`. When the home directory cannot be resolved both
    /// fields stay `None` and discovery falls back to the environment alone,
    /// which keeps the client usable in containers that set no `HOME`.
    #[must_use]
    pub fn from_env() -> Self {
        let dir = config_dir();
        Self {
            config_file: env_path(ENV_CONFIG_FILE)
                .or_else(|| dir.as_ref().map(|d| d.join("config.yaml"))),
            credentials_file: env_path(ENV_CREDENTIALS_FILE)
                .or_else(|| dir.as_ref().map(|d| d.join("credentials.yaml"))),
        }
    }
}

/// Finds credentials using the default paths.
///
/// # Errors
///
/// Returns [`ClientError::Auth`] when a source is present but unusable, and
/// [`ClientError::Config`] when a file exists but cannot be read or parsed.
pub fn discover() -> crate::Result<Option<Discovered>> {
    discover_with(&DiscoveryPaths::from_env())
}

/// Finds credentials for a specific server address.
///
/// Behaves like [`discover_with`], and additionally refuses to hand a
/// credential to an address that would send it in the clear. Use this wherever
/// the credential is found rather than supplied.
///
/// # Errors
///
/// Returns [`ClientError::Auth`] when a credential was found but `base_url` is
/// neither `https` nor a loopback address. Otherwise as [`discover_with`].
pub fn discover_for_url(
    base_url: &str,
    paths: &DiscoveryPaths,
) -> crate::Result<Option<Discovered>> {
    refuse_public_plaintext(discover_with(paths)?, base_url)
}

/// Finds credentials using explicit paths.
///
/// This does not apply the address check in [`discover_for_url`]. Prefer that
/// function when the address is known, so a credential nobody named cannot
/// travel in the clear.
///
/// # Errors
///
/// Same as [`discover`].
pub fn discover_with(paths: &DiscoveryPaths) -> crate::Result<Option<Discovered>> {
    resolve(env_provider()?, paths)
}

/// The search itself, with the environment step already performed.
///
/// Taking the environment result as an argument keeps the file steps testable
/// without a process-wide environment, which no test can hold exclusively.
fn resolve(
    from_env: Option<Arc<dyn AuthProvider>>,
    paths: &DiscoveryPaths,
) -> crate::Result<Option<Discovered>> {
    if let Some(provider) = from_env {
        return Ok(Some(Discovered {
            provider,
            source: CredentialSource::Environment,
        }));
    }
    if let Some(path) = paths.config_file.as_deref()
        && let Some(provider) = config_file_provider(path)?
    {
        return Ok(Some(Discovered {
            provider,
            source: CredentialSource::ConfigFile(path.to_path_buf()),
        }));
    }
    if let Some(path) = paths.credentials_file.as_deref()
        && let Some(provider) = credentials_file_provider(path)?
    {
        return Ok(Some(Discovered {
            provider,
            source: CredentialSource::CredentialsFile(path.to_path_buf()),
        }));
    }
    Ok(None)
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn config_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|dirs| dirs.home_dir().join(".config").join("aviso"))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap and expect on fixture setup are the expected diagnostics"
)]
mod tests {
    use super::*;
    use crate::ClientError;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, body).expect("write fixture");
        path
    }

    #[tokio::test]
    async fn config_file_wins_over_credentials_file() {
        let dir = TempDir::new().unwrap();
        let paths = DiscoveryPaths {
            config_file: Some(write(
                &dir,
                "config.yaml",
                "auth:\n  bearer_token: from-config\n",
            )),
            credentials_file: Some(write(
                &dir,
                "credentials.yaml",
                "bearer:\n  token: from-credentials\n",
            )),
        };

        let found = resolve(None, &paths).unwrap().expect("credential");

        assert_eq!(
            found.provider().authorization_header().await.unwrap(),
            "Bearer from-config"
        );
        assert!(matches!(found.source(), CredentialSource::ConfigFile(_)));
    }

    #[tokio::test]
    async fn credentials_file_is_used_when_config_file_has_no_auth() {
        let dir = TempDir::new().unwrap();
        let paths = DiscoveryPaths {
            config_file: Some(write(&dir, "config.yaml", "base_url: https://a.example\n")),
            credentials_file: Some(write(
                &dir,
                "credentials.yaml",
                "bearer:\n  token: from-credentials\n",
            )),
        };

        let found = resolve(None, &paths).unwrap().expect("credential");

        assert_eq!(
            found.provider().authorization_header().await.unwrap(),
            "Bearer from-credentials"
        );
        assert!(matches!(
            found.source(),
            CredentialSource::CredentialsFile(_)
        ));
    }

    #[test]
    fn nothing_anywhere_leaves_the_client_anonymous() {
        let dir = TempDir::new().unwrap();
        let paths = DiscoveryPaths {
            config_file: Some(dir.path().join("absent-config.yaml")),
            credentials_file: Some(dir.path().join("absent-credentials.yaml")),
        };

        assert!(resolve(None, &paths).unwrap().is_none());
    }

    #[test]
    fn a_discovered_credential_is_refused_for_a_plaintext_address() {
        let dir = TempDir::new().unwrap();
        let paths = DiscoveryPaths {
            config_file: None,
            credentials_file: Some(write(
                &dir,
                "credentials.yaml",
                "bearer:\n  token: secret\n",
            )),
        };

        let found = resolve(None, &paths).unwrap();
        let error = refuse_public_plaintext(found, "http://aviso.example.org").unwrap_err();

        assert!(matches!(error, ClientError::Auth(_)), "got {error:?}");
        assert!(
            !format!("{error}").contains("secret"),
            "the error must not repeat the credential"
        );
    }

    #[test]
    fn nothing_found_is_not_refused_even_for_a_plaintext_address() {
        let dir = TempDir::new().unwrap();
        let paths = DiscoveryPaths {
            config_file: None,
            credentials_file: Some(dir.path().join("absent.yaml")),
        };

        let found = resolve(None, &paths).unwrap();

        assert!(
            refuse_public_plaintext(found, "http://aviso.example.org")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn source_display_names_the_file() {
        assert_eq!(CredentialSource::Environment.to_string(), "environment");
        assert_eq!(
            CredentialSource::CredentialsFile(PathBuf::from("/tmp/c.yaml")).to_string(),
            "credentials file /tmp/c.yaml"
        );
    }
}
