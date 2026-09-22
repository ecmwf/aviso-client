// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The individual places a credential can come from.
//!
//! Each reader answers the same question: is there a usable credential here?
//! `Ok(None)` means "nothing here, try the next place". An error means "there
//! is something here and it is broken", which is reported rather than skipped.

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

use crate::ClientError;
use crate::auth::{AuthProvider, Basic, Bearer, ConfigFile, Env};

/// Reads the environment step.
///
/// Returns `Ok(None)` only when none of the credential variables has a
/// non-empty value, which is a genuine "nothing here, try the next place". An
/// empty `AVISO_TOKEN` or `AVISO_USERNAME` counts as absent, so `AVISO_TOKEN=`
/// behaves like `unset AVISO_TOKEN`. An empty `AVISO_PASSWORD` next to a
/// non-empty username is a real credential with an empty password, which some
/// services accept. A partial setting such as `AVISO_USERNAME` without
/// `AVISO_PASSWORD` is an error: silently dropping to a file would hand the
/// caller a credential they did not ask for.
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
/// Returns [`ClientError::Auth`] when both credentials are set, and
/// [`ClientError::Config`] when the file cannot be read or parsed, or when a
/// credential is present but empty, which is what `Bearer::new` and
/// `Basic::new` report.
pub fn config_file_provider(path: &Path) -> crate::Result<Option<Arc<dyn AuthProvider>>> {
    let Some(content) = read_optional(path)? else {
        return Ok(None);
    };
    config_content_provider(&content, path)
}

/// Reads the `auth:` block from config-file content that is already in hand.
///
/// A caller that has parsed the file for its other settings passes the same
/// bytes here, so the credential and the rest of the configuration come from
/// one read. Reopening the path could see a newer file and pair a fresh
/// credential with a stale server address. `path` is only used in messages.
///
/// # Errors
///
/// As [`config_file_provider`], minus the read failures.
pub fn config_content_provider(
    content: &str,
    path: &Path,
) -> crate::Result<Option<Arc<dyn AuthProvider>>> {
    let doc: ConfigDoc = serde_norway::from_str(content)
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
    // `Path::exists` reports false for every metadata error, so an unreadable
    // file would look absent and fall through to anonymous. Ask for the
    // metadata instead and only treat "not found" as absent.
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(Some(Arc::new(ConfigFile::from_path(path)?))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ClientError::Config(format!(
            "read credentials file {}: {e}",
            path.display()
        ))),
    }
}

/// Reads a file, mapping a missing directory entry to `None` and every other
/// failure to an error.
///
/// The existence check and the read are separate steps on purpose. A dangling
/// symlink makes `read_to_string` report not-found even though the entry is
/// there, and that must surface as an error: an operator who created the
/// link meant for it to be read.
fn read_optional(path: &Path) -> crate::Result<Option<String>> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(ClientError::Config(format!(
                "read config file {}: {e}",
                path.display()
            )));
        }
    }
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(|e| ClientError::Config(format!("read config file {}: {e}", path.display())))
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
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

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
}
