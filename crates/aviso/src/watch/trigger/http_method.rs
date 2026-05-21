//! HTTP method enum for the webhook trigger.
//!
//! Wraps [`reqwest::Method`] to keep aviso's public API decoupled from
//! the reqwest crate, matching the pattern used by abstraction layers
//! that may want to swap HTTP backends without a breaking surface
//! change. The five variants cover the verbs commonly required by
//! webhook receivers (Slack, Teams, Discord, generic JSON HTTP
//! servers); `HEAD` / `OPTIONS` / `TRACE` / `CONNECT` are deferred
//! until a real use case appears.

/// HTTP method for [`crate::watch::Trigger::webhook`].
///
/// Deserialises from uppercase strings (`POST`, `GET`, `PUT`,
/// `PATCH`, `DELETE`) so it round-trips through the YAML trigger
/// configuration shipped with the library.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    /// `POST`. The default method when `Trigger::webhook(url)` is
    /// constructed without a subsequent `.method(...)` call.
    Post,
    /// `GET`.
    Get,
    /// `PUT`.
    Put,
    /// `PATCH`.
    Patch,
    /// `DELETE`.
    Delete,
}

impl HttpMethod {
    /// Converts to the corresponding [`reqwest::Method`]. Crate-private
    /// so the reqwest type stays an implementation detail.
    pub(crate) fn to_reqwest(self) -> reqwest::Method {
        match self {
            Self::Post => reqwest::Method::POST,
            Self::Get => reqwest::Method::GET,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
            Self::Delete => reqwest::Method::DELETE,
        }
    }
}
