// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use crate::ClientError;

/// Parses a JSON response, returning `T` on success and [`ClientError::Http`] (with verbatim body
/// and `X-Request-ID`) on any non-success status. Shared by notify/schema.
pub(crate) async fn parse_json_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> crate::Result<T> {
    let status = response.status();
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .map(String::from);
    let body = response.bytes().await?;
    if status.is_success() {
        let parsed: T = serde_json::from_slice(&body)?;
        Ok(parsed)
    } else {
        let body_str = String::from_utf8_lossy(&body).into_owned();
        Err(ClientError::Http {
            status: status.as_u16(),
            body: body_str,
            request_id,
        })
    }
}

/// Validates a user-supplied path segment before it is spliced into a relative URL via
/// [`format!`]. Rejects:
///
/// - empty input,
/// - the path-traversal tokens `.` and `..` (which would normalize into a sibling endpoint after
///   [`url::Url::join`] resolves the relative reference),
/// - URL structural characters (`/`, `\`, `?`, `#`) that would change the request target,
/// - any ASCII control character.
///
/// `@` and `%` are deliberately permitted because the server's notification ids use `@` literally
/// and we want callers to pass already-encoded segments through verbatim.
pub(crate) fn validate_path_segment(segment: &str) -> crate::Result<()> {
    if segment.is_empty() {
        return Err(ClientError::Config(
            "dynamic path segment must not be empty".into(),
        ));
    }
    if segment == "." || segment == ".." {
        return Err(ClientError::Config(format!(
            "dynamic path segment {segment:?} would traverse into a sibling endpoint"
        )));
    }
    for c in segment.chars() {
        if matches!(c, '/' | '\\' | '?' | '#') || c.is_control() {
            return Err(ClientError::Config(format!(
                "dynamic path segment {segment:?} contains forbidden character {c:?}"
            )));
        }
    }
    Ok(())
}

/// Like [`parse_json_response`] but for endpoints that return either `204 No Content` or a body
/// the client does not need. Success drains the body so the connection can be pooled and any
/// late transport failure during that drain (truncated read, broken connection mid-body) is
/// surfaced rather than silently discarded.
pub(crate) async fn parse_json_response_optional(response: reqwest::Response) -> crate::Result<()> {
    let status = response.status();
    if status.is_success() {
        let _body = response.bytes().await?;
        Ok(())
    } else {
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|h| h.to_str().ok())
            .map(String::from);
        let body = response.bytes().await?;
        let body_str = String::from_utf8_lossy(&body).into_owned();
        Err(ClientError::Http {
            status: status.as_u16(),
            body: body_str,
            request_id,
        })
    }
}
