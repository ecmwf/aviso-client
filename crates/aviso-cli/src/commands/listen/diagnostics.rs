// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Listen-only presentation of untrusted endpoint errors. Raw API errors remain
//! available to library callers; recognizing an error shape does not authenticate it.

use aviso::ClientError;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Deserialize)]
struct AvisoError {
    code: String,
    message: Option<String>,
    details: Option<String>,
    request_id: Option<String>,
    configured_event_types: Option<Vec<String>>,
}

pub(super) fn summary(error: &ClientError) -> String {
    let ClientError::Http {
        status,
        body,
        request_id,
    } = error
    else {
        return redact_urls(&error.to_string());
    };
    let prefix = format!("http {status} from watch endpoint");
    let Some(fields) = recognized_fields(body, request_id.as_deref()) else {
        return format!("{prefix} (unrecognized response body omitted)");
    };
    format!("{prefix}: {}", Value::Object(fields))
}

fn recognized_fields(body: &str, header_request_id: Option<&str>) -> Option<Map<String, Value>> {
    // For example, INVALID_WATCH_REQUEST plus a string details field is
    // recognized; {"proxy_debug":"..."} and unknown codes are not.
    let error: AvisoError = serde_json::from_str(body).ok()?;
    if !matches!(
        error.code.as_str(),
        "INVALID_JSON"
            | "UNKNOWN_FIELD"
            | "INVALID_REQUEST_SHAPE"
            | "INVALID_WATCH_REQUEST"
            | "UNKNOWN_EVENT_TYPE"
            | "SSE_STREAM_INITIALIZATION_FAILED"
            | "INTERNAL_ERROR"
    ) || (error.message.is_none() && error.details.is_none())
    {
        return None;
    }
    let mut fields = Map::new();
    for (name, value) in [
        ("code", Some(error.code.as_str())),
        ("message", error.message.as_deref()),
        ("details", error.details.as_deref()),
        (
            "request_id",
            error.request_id.as_deref().or(header_request_id),
        ),
    ] {
        if let Some(value) = value {
            fields.insert(name.into(), Value::String(redact_urls(value)));
        }
    }
    if error.code == "UNKNOWN_EVENT_TYPE"
        && let Some(types) = error.configured_event_types
    {
        fields.insert(
            "configured_event_types".into(),
            Value::Array(
                types
                    .iter()
                    .map(|name| Value::String(redact_urls(name)))
                    .collect(),
            ),
        );
    }
    Some(fields)
}

/// Omit entire whitespace-delimited URL-like tokens, including their userinfo,
/// path, query and fragment. This deliberately avoids guessing which query keys
/// contain credentials. It is not a detector for arbitrary secrets in prose.
pub(super) fn redact_urls(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for part in text.split_inclusive(char::is_whitespace) {
        let token = part.trim_end_matches(char::is_whitespace);
        // Covers `https://user:password@host/path?token=value#fragment` and
        // quoted protocol-relative URLs, without changing e.g. `[0, 100]`.
        if token.contains("://")
            || token
                .trim_start_matches(['\'', '"', '`', '<', '(', '['])
                .starts_with("//")
        {
            result.push_str("[URL omitted]");
            result.push_str(&part[token.len()..]);
        } else {
            result.push_str(part);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_urls_in_error_and_hint_text_without_losing_constraints() {
        let text = "Field 'step' outside allowed range [0, 100]. Check (HtTp://user:pass@host/private?token=query#fragment) or '//user:pass@host/path?key=value'.";
        let expected =
            "Field 'step' outside allowed range [0, 100]. Check [URL omitted] or [URL omitted]";
        assert_eq!(redact_urls(text), expected);
        assert_eq!(redact_urls("no URL\n[0, 100]"), "no URL\n[0, 100]");
    }

    #[test]
    fn presentation_does_not_modify_raw_api_error_body() {
        let body = r#"{"proxy_debug":"http://user:secret@host/path?token=secret"}"#;
        let error = ClientError::Http {
            status: 403,
            body: body.into(),
            request_id: None,
        };
        assert!(summary(&error).contains("unrecognized response body omitted"));
        assert!(matches!(error, ClientError::Http { body: actual, .. } if actual == body));
    }

    #[test]
    fn other_listener_errors_also_redact_url_tokens() {
        let error = ClientError::StreamProtocol {
            message: "rejected https://user:SECRET@host/private?key=SECRET#SECRET".into(),
            request_id: Some("https://host/private?key=SECRET".into()),
        };
        let display = summary(&error);
        assert!(display.contains("rejected"));
        assert!(display.contains("[URL omitted]"));
        assert!(!display.contains("SECRET"));
    }
}
