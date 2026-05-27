//! YAML config-file auth provider.
//!
//! Accepts files of the shape:
//!
//! ```yaml
//! bearer:
//!   token: "<opaque-or-jwt>"
//! ```
//!
//! or:
//!
//! ```yaml
//! basic:
//!   username: "alice"
//!   password: "..."
//! ```
//!
//! Specifying both sections, or neither, is a [`ClientError::Config`].
//!
//! [`ConfigFile::refresh`] re-reads the file when the provider was built via
//! [`ConfigFile::from_path`], so a user-side credential rewrite is reflected on the next
//! `Authorization` header lookup. The supervisor's refresh-then-retry-once contract (D8) honours
//! the new credential automatically.

use std::path::{Path, PathBuf};

use reqwest::header::HeaderValue;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::ClientError;
use crate::auth::{AuthProvider, Basic, Bearer};

/// Auth provider that reads credentials from a YAML file.
///
/// The provider remembers the source path when built via [`Self::from_path`]; on a 401 the watch
/// supervisor calls [`AuthProvider::refresh`] which re-reads the same path and swaps the cached
/// credential atomically.
#[derive(Debug)]
pub struct ConfigFile {
    inner: RwLock<ConfigSource>,
    /// Source path for [`Self::refresh`]. `None` when the provider was built from a YAML string
    /// (`from_yaml_str`); in that case refresh is a no-op.
    path: Option<PathBuf>,
}

#[derive(Debug)]
enum ConfigSource {
    Bearer(Bearer),
    Basic(Basic),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Doc {
    #[serde(default)]
    bearer: Option<BearerSection>,
    #[serde(default)]
    basic: Option<BasicSection>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BearerSection {
    token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BasicSection {
    username: String,
    password: String,
}

impl ConfigFile {
    /// Reads and parses the YAML config file at `path`.
    ///
    /// The path is stored on the returned provider so [`AuthProvider::refresh`] can re-read it.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] when the file cannot be read, the YAML cannot be parsed,
    /// or the file has zero or two of the `bearer`/`basic` sections (exactly one is required).
    pub fn from_path(path: impl AsRef<Path>) -> crate::Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)
            .map_err(|e| ClientError::Config(format!("read auth file {}: {e}", path.display())))?;
        let inner = Self::parse_config_source(&content).map_err(|e| match e {
            ClientError::Config(msg) => {
                ClientError::Config(format!("auth file {}: {msg}", path.display()))
            }
            other => other,
        })?;
        Ok(Self {
            inner: RwLock::new(inner),
            path: Some(path.to_path_buf()),
        })
    }

    /// Parses the YAML content directly. The testable kernel of [`Self::from_path`].
    ///
    /// Providers built this way have no source path, so [`AuthProvider::refresh`] is a no-op for
    /// them. Use [`Self::from_path`] when refresh semantics matter.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Config`] when the YAML cannot be parsed, or when the file has zero
    /// or two of the `bearer`/`basic` sections.
    pub fn from_yaml_str(yaml: &str) -> crate::Result<Self> {
        let inner = Self::parse_config_source(yaml)?;
        Ok(Self {
            inner: RwLock::new(inner),
            path: None,
        })
    }

    fn parse_config_source(yaml: &str) -> crate::Result<ConfigSource> {
        let doc: Doc = serde_norway::from_str(yaml)
            .map_err(|e| ClientError::Config(format!("parse YAML: {e}")))?;
        match (doc.bearer, doc.basic) {
            (Some(b), None) => Ok(ConfigSource::Bearer(Bearer::new(b.token)?)),
            (None, Some(b)) => Ok(ConfigSource::Basic(Basic::new(b.username, b.password)?)),
            (Some(_), Some(_)) => Err(ClientError::Config(
                "has both 'bearer' and 'basic' sections; only one is allowed".into(),
            )),
            (None, None) => Err(ClientError::Config(
                "has neither 'bearer' nor 'basic' section".into(),
            )),
        }
    }
}

#[async_trait::async_trait]
impl AuthProvider for ConfigFile {
    async fn authorization_header(&self) -> crate::Result<HeaderValue> {
        match &*self.inner.read().await {
            ConfigSource::Bearer(b) => b.authorization_header().await,
            ConfigSource::Basic(b) => b.authorization_header().await,
        }
    }

    async fn refresh(&self) -> crate::Result<()> {
        let Some(path) = self.path.as_ref() else {
            // Built from in-memory YAML; nothing to re-read.
            return Ok(());
        };
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| ClientError::Auth(format!("refresh read {}: {e}", path.display())))?;
        // Parse to a fresh ConfigSource BEFORE acquiring the write lock so a malformed rewrite
        // leaves the previously-cached credential intact. The swap that follows is a single
        // synchronous assignment; per the trait's cancel-safety contract a dropped future cannot
        // leave a torn intermediate.
        let new_inner = Self::parse_config_source(&content).map_err(|e| match e {
            ClientError::Config(msg) => {
                ClientError::Auth(format!("refresh parse {}: {msg}", path.display()))
            }
            other => other,
        })?;
        *self.inner.write().await = new_inner;
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test code: unwrap on constructor success is the expected diagnostic"
)]
mod tests {
    use super::{AuthProvider, ClientError, ConfigFile};

    #[tokio::test]
    async fn parses_bearer_section() {
        let yaml = "bearer:\n  token: opaque-jwt\n";
        let cfg = ConfigFile::from_yaml_str(yaml).unwrap();
        let header = cfg.authorization_header().await.unwrap();
        assert_eq!(header, "Bearer opaque-jwt");
    }

    #[tokio::test]
    async fn parses_basic_section() {
        let yaml = "basic:\n  username: alice\n  password: wonderland\n";
        let cfg = ConfigFile::from_yaml_str(yaml).unwrap();
        let header = cfg.authorization_header().await.unwrap();
        assert_eq!(header, "Basic YWxpY2U6d29uZGVybGFuZA==");
    }

    #[test]
    fn rejects_both_sections() {
        let yaml = "bearer:\n  token: x\nbasic:\n  username: a\n  password: b\n";
        let err = ConfigFile::from_yaml_str(yaml).unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn rejects_no_sections() {
        let yaml = "{}\n";
        let err = ConfigFile::from_yaml_str(yaml).unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn rejects_malformed_yaml() {
        let yaml = "bearer: {\n";
        let err = ConfigFile::from_yaml_str(yaml).unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn rejects_unknown_top_level_key_to_surface_typos() {
        // 'beare' instead of 'bearer' must not silently fall back to "neither section present".
        let yaml = "beare:\n  token: x\n";
        let err = ConfigFile::from_yaml_str(yaml).unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn rejects_unknown_field_inside_bearer_section() {
        // 'tokn' inside bearer (typo of 'token') must surface, not be silently dropped.
        let yaml = "bearer:\n  tokn: x\n";
        let err = ConfigFile::from_yaml_str(yaml).unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn rejects_unknown_field_inside_basic_section() {
        let yaml = "basic:\n  usrname: alice\n  password: pw\n";
        let err = ConfigFile::from_yaml_str(yaml).unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn rejects_empty_bearer_token() {
        let yaml = "bearer:\n  token: \"\"\n";
        let err = ConfigFile::from_yaml_str(yaml).unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    #[test]
    fn rejects_empty_basic_username() {
        let yaml = "basic:\n  username: \"\"\n  password: pw\n";
        let err = ConfigFile::from_yaml_str(yaml).unwrap_err();
        assert!(matches!(err, ClientError::Config(_)), "got {err:?}");
    }

    mod from_path {
        #![allow(
            clippy::unwrap_used,
            clippy::panic,
            reason = "test code: panic on unexpected variant is the standard test diagnostic"
        )]

        use std::io::Write;

        use super::{AuthProvider, ClientError, ConfigFile};

        #[tokio::test]
        async fn reads_and_parses_yaml_from_disk() {
            let file = tempfile::Builder::new()
                .prefix("aviso-auth-")
                .suffix(".yaml")
                .tempfile()
                .unwrap();
            writeln!(file.as_file(), "bearer:\n  token: opaque-jwt").unwrap();

            let cfg = ConfigFile::from_path(file.path()).unwrap();
            let header = cfg.authorization_header().await.unwrap();
            assert_eq!(header, "Bearer opaque-jwt");
        }

        #[test]
        fn missing_file_produces_config_error_with_path_context() {
            let path = std::env::temp_dir().join("aviso-test-nonexistent-auth-file.yaml");
            let _ = std::fs::remove_file(&path);

            let err = ConfigFile::from_path(&path).unwrap_err();
            match err {
                ClientError::Config(msg) => assert!(
                    msg.contains(&path.display().to_string()),
                    "error must mention the offending path: {msg}"
                ),
                other => panic!("expected Config error, got {other:?}"),
            }
        }

        #[test]
        fn malformed_yaml_on_disk_produces_config_error_with_path_context() {
            let file = tempfile::Builder::new()
                .prefix("aviso-auth-bad-")
                .suffix(".yaml")
                .tempfile()
                .unwrap();
            writeln!(file.as_file(), "bearer: {{").unwrap();
            let path_str = file.path().display().to_string();

            let err = ConfigFile::from_path(file.path()).unwrap_err();
            match err {
                ClientError::Config(msg) => assert!(
                    msg.contains(&path_str),
                    "error must mention the offending path: {msg}"
                ),
                other => panic!("expected Config error, got {other:?}"),
            }
        }
    }

    mod refresh {
        #![allow(
            clippy::unwrap_used,
            clippy::panic,
            reason = "test code: panic on unexpected variant is the standard test diagnostic"
        )]

        use std::io::Write;

        use super::{AuthProvider, ClientError, ConfigFile};

        fn rewrite_file(path: &std::path::Path, new_contents: &str) {
            std::fs::write(path, new_contents).unwrap();
        }

        #[tokio::test]
        async fn refresh_rereads_file_after_rewrite() {
            let file = tempfile::Builder::new()
                .prefix("aviso-auth-refresh-")
                .suffix(".yaml")
                .tempfile()
                .unwrap();
            writeln!(file.as_file(), "bearer:\n  token: old-token").unwrap();

            let cfg = ConfigFile::from_path(file.path()).unwrap();
            assert_eq!(
                cfg.authorization_header().await.unwrap(),
                "Bearer old-token"
            );

            rewrite_file(file.path(), "bearer:\n  token: new-token\n");
            cfg.refresh().await.unwrap();

            assert_eq!(
                cfg.authorization_header().await.unwrap(),
                "Bearer new-token"
            );
        }

        #[tokio::test]
        async fn refresh_swaps_section_kind() {
            let file = tempfile::Builder::new()
                .prefix("aviso-auth-swap-")
                .suffix(".yaml")
                .tempfile()
                .unwrap();
            writeln!(file.as_file(), "bearer:\n  token: starter").unwrap();

            let cfg = ConfigFile::from_path(file.path()).unwrap();
            assert_eq!(cfg.authorization_header().await.unwrap(), "Bearer starter");

            rewrite_file(
                file.path(),
                "basic:\n  username: alice\n  password: wonderland\n",
            );
            cfg.refresh().await.unwrap();

            assert_eq!(
                cfg.authorization_header().await.unwrap(),
                "Basic YWxpY2U6d29uZGVybGFuZA==",
            );
        }

        #[tokio::test]
        async fn refresh_parse_failure_leaves_previous_credential() {
            let file = tempfile::Builder::new()
                .prefix("aviso-auth-bad-refresh-")
                .suffix(".yaml")
                .tempfile()
                .unwrap();
            writeln!(file.as_file(), "bearer:\n  token: original").unwrap();

            let cfg = ConfigFile::from_path(file.path()).unwrap();
            assert_eq!(cfg.authorization_header().await.unwrap(), "Bearer original");

            rewrite_file(
                file.path(),
                "bearer:\n  token: x\nbasic:\n  username: a\n  password: b\n",
            );
            let err = cfg.refresh().await.unwrap_err();
            match err {
                ClientError::Auth(msg) => assert!(
                    msg.contains("both 'bearer' and 'basic' sections"),
                    "refresh error must explain parse failure: {msg}"
                ),
                other => panic!("expected Auth error from refresh, got {other:?}"),
            }

            assert_eq!(cfg.authorization_header().await.unwrap(), "Bearer original");
        }

        #[tokio::test]
        async fn refresh_is_noop_for_from_yaml_str_constructor() {
            let cfg = ConfigFile::from_yaml_str("bearer:\n  token: in-memory-token\n").unwrap();
            assert_eq!(
                cfg.authorization_header().await.unwrap(),
                "Bearer in-memory-token"
            );

            cfg.refresh().await.unwrap();

            assert_eq!(
                cfg.authorization_header().await.unwrap(),
                "Bearer in-memory-token"
            );
        }
    }
}
