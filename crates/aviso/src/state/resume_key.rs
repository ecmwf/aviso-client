//! Resume key: a stable hash identifying a watch subscription.

use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

/// Current hash-input format version baked into every [`ResumeKey`].
/// Bumping invalidates existing keys without breaking the file layout.
pub(crate) const KEY_FORMAT_VERSION: u32 = 1;

/// A logical identifier for a watch subscription.
///
/// Same server + same event type + same filter = same key, regardless of
/// which `AvisoClient` instance computed it. The hash deliberately
/// excludes any server-side resume position (`from_id`, `from_date`):
/// those are not subscription identity (see D3 in the ADR log).
///
/// The key is a SHA-256 digest plus the `key_format_version` of the
/// hash input. Two keys with the same digest but different format
/// versions are NOT equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResumeKey {
    digest: [u8; 32],
    key_format_version: u32,
}

impl ResumeKey {
    /// Compute a resume key from its components.
    ///
    /// Fallible because the filter is canonicalised via RFC 8785 JSON
    /// Canonicalization Scheme, which rejects inputs `serde_json::Value`
    /// would otherwise accept (such as floats outside finite range).
    ///
    /// # Errors
    ///
    /// Returns [`ResumeKeyError::CanonicaliseFilter`] if `filter` cannot
    /// be canonicalised.
    pub fn new(
        base_url: &Url,
        event_type: &str,
        filter: &serde_json::Value,
        schema_fingerprint: Option<&str>,
    ) -> Result<Self, ResumeKeyError> {
        let canonical_filter =
            serde_jcs::to_vec(filter).map_err(ResumeKeyError::CanonicaliseFilter)?;

        let mut hasher = Sha256::new();
        hasher.update(KEY_FORMAT_VERSION.to_le_bytes());
        hasher.update([0u8]);
        hasher.update(normalize_base_url(base_url).as_bytes());
        hasher.update([0u8]);
        hasher.update(event_type.as_bytes());
        hasher.update([0u8]);
        hasher.update(&canonical_filter);
        hasher.update([0u8]);
        if let Some(fp) = schema_fingerprint {
            hasher.update(b"fp:");
            hasher.update(fp.as_bytes());
        }

        Ok(Self {
            digest: hasher.finalize().into(),
            key_format_version: KEY_FORMAT_VERSION,
        })
    }

    /// The 32-byte SHA-256 digest.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.digest
    }

    /// The digest as a 64-character lowercase hex string.
    ///
    /// Note: `key_format_version` is part of [`PartialEq`] for `ResumeKey`
    /// but is NOT encoded in this hex form. The file format stores the
    /// key format version at the file level, not per-key.
    #[must_use]
    pub fn as_hex(&self) -> String {
        hex::encode(self.digest)
    }

    /// Hash-input format version baked into this key.
    #[must_use]
    pub fn key_format_version(&self) -> u32 {
        self.key_format_version
    }
}

/// Errors specific to resume-key construction.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum ResumeKeyError {
    /// Filter could not be canonicalised under RFC 8785.
    #[error("filter canonicalisation failed: {0}")]
    CanonicaliseFilter(#[source] serde_json::Error),
}

/// Normalise a base URL for stable hashing.
///
/// Rules:
/// - Lowercase scheme.
/// - Lowercase host.
/// - Strip default port (`:80` for http, `:443` for https).
/// - Strip userinfo (username/password embedded in URL).
/// - Strip query and fragment (base URLs should have neither).
/// - Preserve path; empty path becomes `/`.
fn normalize_base_url(url: &Url) -> String {
    let scheme = url.scheme().to_ascii_lowercase();
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let port_part = match (url.port(), scheme.as_str()) {
        (Some(80), "http") | (Some(443), "https") | (None, _) => String::new(),
        (Some(p), _) => format!(":{p}"),
    };
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    format!("{scheme}://{host}{port_part}{path}")
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use serde_json::json;
    use url::Url;

    use super::{KEY_FORMAT_VERSION, ResumeKey, normalize_base_url};

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn deterministic_for_same_inputs() {
        let a = ResumeKey::new(&url("https://a/"), "mars", &json!({}), None).unwrap();
        let b = ResumeKey::new(&url("https://a/"), "mars", &json!({}), None).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_base_url_gives_different_key() {
        let a = ResumeKey::new(&url("https://a/"), "mars", &json!({}), None).unwrap();
        let b = ResumeKey::new(&url("https://b/"), "mars", &json!({}), None).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_event_type_gives_different_key() {
        let a = ResumeKey::new(&url("https://a/"), "mars", &json!({}), None).unwrap();
        let b = ResumeKey::new(&url("https://a/"), "atms", &json!({}), None).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_filter_value_gives_different_key() {
        let a = ResumeKey::new(&url("https://a/"), "mars", &json!({"k": "1"}), None).unwrap();
        let b = ResumeKey::new(&url("https://a/"), "mars", &json!({"k": "2"}), None).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn filter_key_reordering_does_not_affect_key() {
        let a = ResumeKey::new(
            &url("https://a/"),
            "mars",
            &json!({"a": "1", "b": "2"}),
            None,
        )
        .unwrap();
        let b = ResumeKey::new(
            &url("https://a/"),
            "mars",
            &json!({"b": "2", "a": "1"}),
            None,
        )
        .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn nested_filter_object_reordering_does_not_affect_key() {
        let a = ResumeKey::new(
            &url("https://a/"),
            "mars",
            &json!({"outer": {"x": "1", "y": "2"}}),
            None,
        )
        .unwrap();
        let b = ResumeKey::new(
            &url("https://a/"),
            "mars",
            &json!({"outer": {"y": "2", "x": "1"}}),
            None,
        )
        .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn schema_fingerprint_none_vs_some_empty_differ() {
        let a = ResumeKey::new(&url("https://a/"), "mars", &json!({}), None).unwrap();
        let b = ResumeKey::new(&url("https://a/"), "mars", &json!({}), Some("")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn schema_fingerprint_distinct_values_differ() {
        let a = ResumeKey::new(&url("https://a/"), "mars", &json!({}), Some("v1")).unwrap();
        let b = ResumeKey::new(&url("https://a/"), "mars", &json!({}), Some("v2")).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn empty_filter_object_hashes_deterministically() {
        let a = ResumeKey::new(&url("https://a/"), "mars", &json!({}), None).unwrap();
        let b = ResumeKey::new(&url("https://a/"), "mars", &json!({}), None).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.as_hex().len(), 64);
    }

    #[test]
    fn as_hex_is_64_lowercase_chars() {
        let k = ResumeKey::new(&url("https://a/"), "mars", &json!({}), None).unwrap();
        let hex = k.as_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }

    #[test]
    fn key_format_version_is_current() {
        let k = ResumeKey::new(&url("https://a/"), "mars", &json!({}), None).unwrap();
        assert_eq!(k.key_format_version(), KEY_FORMAT_VERSION);
    }

    #[test]
    fn normalize_lowercases_scheme_and_host() {
        assert_eq!(
            normalize_base_url(&url("HTTPS://Aviso.Example/")),
            "https://aviso.example/"
        );
    }

    #[test]
    fn normalize_strips_default_ports() {
        assert_eq!(
            normalize_base_url(&url("https://aviso.example:443/")),
            "https://aviso.example/"
        );
        assert_eq!(
            normalize_base_url(&url("http://aviso.example:80/")),
            "http://aviso.example/"
        );
    }

    #[test]
    fn normalize_preserves_non_default_port() {
        assert_eq!(
            normalize_base_url(&url("https://aviso.example:8443/")),
            "https://aviso.example:8443/"
        );
    }

    #[test]
    fn normalize_strips_userinfo() {
        assert_eq!(
            normalize_base_url(&url("https://user:pass@aviso.example/")),
            "https://aviso.example/"
        );
    }

    #[test]
    fn normalize_preserves_path() {
        assert_eq!(
            normalize_base_url(&url("https://aviso.example/path/")),
            "https://aviso.example/path/"
        );
    }

    #[test]
    fn normalize_equivalent_urls_produce_same_key() {
        let a = ResumeKey::new(&url("HTTPS://Aviso.Example/"), "mars", &json!({}), None).unwrap();
        let b = ResumeKey::new(
            &url("https://user:pass@aviso.example:443/"),
            "mars",
            &json!({}),
            None,
        )
        .unwrap();
        assert_eq!(a, b);
    }
}
