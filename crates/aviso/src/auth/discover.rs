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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::ClientError;
use crate::auth::{AuthProvider, Basic, Bearer, ConfigFile, Env};

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
pub enum CredentialSource {
    /// Read from the process environment.
    Environment,
    /// Read from the `auth:` block of the config file at this path.
    ConfigFile(PathBuf),
    /// Read from the credentials file at this path.
    CredentialsFile(PathBuf),
}

impl std::fmt::Display for CredentialSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Environment => write!(f, "environment"),
            Self::ConfigFile(p) => write!(f, "config file {}", p.display()),
            Self::CredentialsFile(p) => write!(f, "credentials file {}", p.display()),
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

/// Finds credentials using explicit paths.
///
/// # Errors
///
/// Same as [`discover`].
pub fn discover_with(paths: &DiscoveryPaths) -> crate::Result<Option<Discovered>> {
    if let Some(provider) = env_provider()? {
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

/// Reads the environment step.
///
/// Returns `Ok(None)` only when none of the credential variables is set, which
/// is a genuine "nothing here, try the next place". A partial setting such as
/// `AVISO_USERNAME` without `AVISO_PASSWORD` is an error: silently dropping to
/// a file would hand the caller a credential they did not ask for.
///
/// # Errors
///
/// Returns the error from [`Env::from_process_env`] when a variable is set but
/// the combination is unusable.
pub fn env_provider() -> crate::Result<Option<Arc<dyn AuthProvider>>> {
    match Env::from_process_env() {
        Ok(env) => Ok(Some(Arc::new(env))),
        Err(ClientError::Auth(_)) if !any_env_var_set() => Ok(None),
        Err(other) => Err(other),
    }
}

/// Reads the `auth:` block of a config file.
///
/// Returns `Ok(None)` when the file does not exist, or exists without an
/// `auth:` block, or has an `auth:` block with neither credential set. Unknown
/// keys outside `auth:` are ignored, because the config file belongs to the
/// `aviso` binary and carries settings this crate does not model.
///
/// Valid:
///
/// ```yaml
/// auth:
///   bearer_token: "abc123"
/// ```
///
/// Invalid, because the two are mutually exclusive:
///
/// ```yaml
/// auth:
///   bearer_token: "abc123"
///   basic: { username: "alice", password: "s3cret" }
/// ```
///
/// # Errors
///
/// Returns [`ClientError::Config`] when the file cannot be read or parsed, and
/// [`ClientError::Auth`] when both credentials are set or a value is empty.
pub fn config_file_provider(path: &Path) -> crate::Result<Option<Arc<dyn AuthProvider>>> {
    let Some(content) = read_optional(path)? else {
        return Ok(None);
    };
    let doc: ConfigDoc = serde_norway::from_str(&content)
        .map_err(|e| ClientError::Config(format!("parse config file {}: {e}", path.display())))?;
    let Some(auth) = doc.auth else {
        return Ok(None);
    };
    match (auth.bearer_token, auth.basic) {
        (Some(_), Some(_)) => Err(ClientError::Auth(format!(
            "config file {} sets both auth.bearer_token and auth.basic; keep one",
            path.display()
        ))),
        (Some(token), None) => Ok(Some(Arc::new(Bearer::new(token)?))),
        (None, Some(basic)) => Ok(Some(Arc::new(Basic::new(basic.username, basic.password)?))),
        (None, None) => Ok(None),
    }
}

/// Reads a credentials file through [`ConfigFile`], so it re-reads on `401`.
///
/// Returns `Ok(None)` when the file does not exist. A file that exists must be
/// usable: an empty or malformed one is an error rather than a silent skip.
///
/// # Errors
///
/// Returns the error from [`ConfigFile::from_path`].
pub fn credentials_file_provider(path: &Path) -> crate::Result<Option<Arc<dyn AuthProvider>>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(Arc::new(ConfigFile::from_path(path)?)))
}

/// Reads a file, mapping "not found" to `None` and other IO errors to an error.
fn read_optional(path: &Path) -> crate::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ClientError::Config(format!(
            "read config file {}: {e}",
            path.display()
        ))),
    }
}

fn any_env_var_set() -> bool {
    [
        crate::auth::env::ENV_TOKEN,
        crate::auth::env::ENV_USERNAME,
        crate::auth::env::ENV_PASSWORD,
    ]
    .iter()
    .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn config_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|dirs| dirs.home_dir().join(".config").join("aviso"))
}

/// Only `auth:` is modelled. The config file carries other settings that this
/// crate does not read, so unknown keys at the top level are ignored.
#[derive(Deserialize)]
struct ConfigDoc {
    #[serde(default)]
    auth: Option<AuthBlock>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthBlock {
    #[serde(default)]
    bearer_token: Option<String>,
    #[serde(default)]
    basic: Option<BasicBlock>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BasicBlock {
    username: String,
    password: String,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap and expect on fixture setup are the expected diagnostics"
)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, body).expect("write fixture");
        path
    }

    #[tokio::test]
    async fn config_file_bearer_token_is_used() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "config.yaml", "auth:\n  bearer_token: from-config\n");

        let provider = config_file_provider(&path).unwrap().expect("provider");

        assert_eq!(
            provider.authorization_header().await.unwrap(),
            "Bearer from-config"
        );
    }

    #[tokio::test]
    async fn config_file_basic_credentials_are_used() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "config.yaml",
            "auth:\n  basic:\n    username: alice\n    password: s3cret\n",
        );

        let provider = config_file_provider(&path).unwrap().expect("provider");

        assert_eq!(
            provider.authorization_header().await.unwrap(),
            "Basic YWxpY2U6czNjcmV0"
        );
    }

    #[test]
    fn config_file_ignores_keys_this_crate_does_not_model() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "config.yaml",
            "base_url: https://aviso.example.org\nlisteners: []\nauth:\n  bearer_token: t\n",
        );

        assert!(config_file_provider(&path).unwrap().is_some());
    }

    #[test]
    fn config_file_without_auth_block_is_skipped() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "config.yaml", "base_url: https://aviso.example.org\n");

        assert!(config_file_provider(&path).unwrap().is_none());
    }

    #[test]
    fn missing_config_file_is_skipped() {
        let dir = TempDir::new().unwrap();

        assert!(
            config_file_provider(&dir.path().join("absent.yaml"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn config_file_with_both_credentials_is_rejected() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "config.yaml",
            "auth:\n  bearer_token: t\n  basic:\n    username: alice\n    password: s3cret\n",
        );

        let error = config_file_provider(&path).unwrap_err();

        assert!(matches!(error, ClientError::Auth(_)), "got {error:?}");
    }

    #[test]
    fn config_file_with_unknown_auth_key_is_rejected() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "config.yaml", "auth:\n  bearer_tokne: typo\n");

        let error = config_file_provider(&path).unwrap_err();

        assert!(matches!(error, ClientError::Config(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn credentials_file_is_used_when_present() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "credentials.yaml",
            "bearer:\n  token: from-credentials\n",
        );

        let provider = credentials_file_provider(&path).unwrap().expect("provider");

        assert_eq!(
            provider.authorization_header().await.unwrap(),
            "Bearer from-credentials"
        );
    }

    #[test]
    fn missing_credentials_file_is_skipped() {
        let dir = TempDir::new().unwrap();

        assert!(
            credentials_file_provider(&dir.path().join("absent.yaml"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn malformed_credentials_file_is_rejected_rather_than_skipped() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "credentials.yaml", "bearer:\n  toke: typo\n");

        assert!(credentials_file_provider(&path).is_err());
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

        let found = discover_with(&paths).unwrap().expect("credential");

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

        let found = discover_with(&paths).unwrap().expect("credential");

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

        assert!(discover_with(&paths).unwrap().is_none());
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
