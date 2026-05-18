//! Resume key: a stable hash identifying a watch subscription.

use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

/// Current hash-input format version baked into every [`ResumeKey`].
/// Bumping invalidates existing keys without breaking the file layout.
///
/// History:
///
/// - `1`: initial; NUL-byte separators between variable-length fields,
///   IPv6 hosts unbracketed.
/// - `2`: length-prefix framing for unambiguous concatenation; IPv6
///   hosts bracketed via `url::Host` Display. Both changes fix
///   theoretical collisions in version 1 (event-type or
///   schema-fingerprint NUL bytes at field boundaries; IPv6 host
///   plus port colliding with a literal `host:port` form).
pub(crate) const KEY_FORMAT_VERSION: u32 = 2;

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
    /// `base_url` is expected to be an HTTP(S) `aviso-server` URL.
    /// Other URL schemes are not rejected but are also not the
    /// intended workload; normalisation passes them through `url::Url`
    /// serialization plus the rules documented in the implementation
    /// (lowercased scheme and host, IPv6 host bracketing via
    /// `url::Host` Display, default-port stripping, userinfo
    /// removal, path preservation).
    ///
    /// The hash input uses length-prefix framing: each variable-length
    /// field is preceded by its byte length as a little-endian `u64`,
    /// and the optional schema fingerprint carries a one-byte tag
    /// (`0` absent, `1` present) before its length-and-bytes. Two
    /// distinct logical inputs cannot collide under this scheme,
    /// regardless of which bytes (including NUL) appear inside any
    /// field.
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
        write_field(&mut hasher, normalize_base_url(base_url).as_bytes());
        write_field(&mut hasher, event_type.as_bytes());
        write_field(&mut hasher, &canonical_filter);
        write_optional_field(&mut hasher, schema_fingerprint.map(str::as_bytes));

        Ok(Self {
            digest: hasher.finalize().into(),
            key_format_version: KEY_FORMAT_VERSION,
        })
    }

    /// Construct a `ResumeKey` from a raw digest and format version.
    /// Used by the file store when loading from disk.
    pub(crate) fn from_parts(digest: [u8; 32], key_format_version: u32) -> Self {
        Self {
            digest,
            key_format_version,
        }
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
/// - Render host via [`url::Host`] Display so IPv6 literals are
///   bracketed (e.g. `[::1]`), then lowercase. Bracketing prevents
///   `https://[::1]:8443/` from collapsing into a form ambiguous
///   with a (hypothetical) raw `host:port` literal.
/// - Strip default port (`:80` for http, `:443` for https).
/// - Strip userinfo (username/password embedded in URL).
/// - Strip query and fragment (base URLs should have neither).
/// - Preserve path; empty path becomes `/`.
fn normalize_base_url(url: &Url) -> String {
    let scheme = url.scheme().to_ascii_lowercase();
    let host = url
        .host()
        .map_or_else(String::new, |h| h.to_string().to_ascii_lowercase());
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

/// Hash one variable-length field with a `u64` little-endian length
/// prefix. Injective when called for a fixed schema of fields.
fn write_field(hasher: &mut Sha256, bytes: &[u8]) {
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    hasher.update(len.to_le_bytes());
    hasher.update(bytes);
}

/// Hash an optional field: one tag byte (`0` absent, `1` present),
/// then a length-prefix field when present.
fn write_optional_field(hasher: &mut Sha256, bytes: Option<&[u8]>) {
    match bytes {
        Some(b) => {
            hasher.update([1u8]);
            write_field(hasher, b);
        }
        None => {
            hasher.update([0u8]);
        }
    }
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
    fn event_type_with_nul_byte_does_not_collide_with_different_inputs() {
        // The null-byte separator strategy requires inputs to not
        // contain raw null bytes. event_type is the most exposed
        // path because it goes straight into the hash unescaped.
        // This fixture pins the property: an event_type with a NUL
        // does not produce the same key as a different
        // (event_type, filter) combination where the NUL lands at a
        // boundary.
        let with_nul = ResumeKey::new(&url("https://a/"), "a\x00b", &json!({}), None).unwrap();
        let separate = ResumeKey::new(&url("https://a/"), "a", &json!({"_": "b"}), None).unwrap();
        assert_ne!(with_nul, separate);
    }

    #[test]
    fn schema_fingerprint_with_nul_does_not_collide_with_longer_event_type() {
        let fp_nul =
            ResumeKey::new(&url("https://a/"), "mars", &json!({}), Some("v\x001")).unwrap();
        let no_fp = ResumeKey::new(&url("https://a/"), "marsfp:v\x001", &json!({}), None).unwrap();
        assert_ne!(fp_nul, no_fp);
    }

    #[test]
    fn from_parts_with_different_format_versions_are_not_equal() {
        let a = ResumeKey::from_parts([0u8; 32], 1);
        let b = ResumeKey::from_parts([0u8; 32], 2);
        assert_ne!(a, b, "same digest, different format version must differ");
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
    fn normalize_brackets_ipv6_host() {
        // Regression fixture for the IPv6 collision bug:
        // before bracketing, `https://[::1]:8443/` normalised to
        // `https://::1:8443/`, ambiguous with a literal host:port
        // form. Bracketing makes the host boundary explicit.
        assert_eq!(
            normalize_base_url(&url("https://[::1]:8443/")),
            "https://[::1]:8443/"
        );
        assert_eq!(
            normalize_base_url(&url("https://[2001:db8::1]/")),
            "https://[2001:db8::1]/"
        );
    }

    #[test]
    fn ipv6_compressed_and_expanded_forms_produce_same_key() {
        // The url crate canonicalises IPv6 to compressed form before
        // we see the host. Two URLs that differ only in IPv6
        // formatting must therefore produce the same key.
        let compressed = ResumeKey::new(&url("https://[::1]/"), "mars", &json!({}), None).unwrap();
        let expanded =
            ResumeKey::new(&url("https://[0:0:0:0:0:0:0:1]/"), "mars", &json!({}), None).unwrap();
        assert_eq!(compressed, expanded);
    }

    #[test]
    fn distinct_ipv6_hosts_produce_distinct_keys() {
        let a = ResumeKey::new(&url("https://[::1]:8443/"), "mars", &json!({}), None).unwrap();
        let b = ResumeKey::new(&url("https://[::2]:8443/"), "mars", &json!({}), None).unwrap();
        assert_ne!(a, b);
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
