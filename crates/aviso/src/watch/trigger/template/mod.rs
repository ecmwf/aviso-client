// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Tiny in-crate template engine used by command and (later) webhook
//! triggers.
//!
//! Two namespaces are recognised inside `{{ ... }}` expressions:
//!
//! - `{{ notification.<dotted.path> }}`: resolves into the notification's
//!   serialised JSON value via a top-down walk of object keys. Empty
//!   path means the whole notification.
//! - `{{ env.<NAME> }}`: resolves into `std::env::var(NAME)`.
//!
//! Literal `{{` is escaped as `\{{`.
//!
//! # Where the text goes
//!
//! Notification values come from whoever published the notification,
//! not from the operator who wrote the template. A render therefore
//! names its [`Sink`], and every substituted notification value is
//! neutralised for it: quoted for the shell context it lands in when
//! the text is a command, percent-encoded when the text is a URL, and
//! verbatim when the caller builds its own encoding around the value
//! (JSON bodies, header values). `{{ env.* }}` values are the
//! operator's own and are always inserted verbatim. See [`shell`] and
//! [`url`].
//!
//! # Resolution rules
//!
//! - Scalar string: rendered UNQUOTED (the inner string only). Both
//!   `serde_json::to_string` and `Value::Display` would add quotes;
//!   we destructure the `Value::String(s)` arm to avoid them.
//! - Scalar number / bool: rendered via `Value::to_string()` which gives
//!   `"42"` / `"true"` / `"false"` / `"3.14"` without quotes.
//! - `null`: rendered as the literal four-character string `null`.
//! - Object / Array: rendered as compact JSON via `serde_json::to_string`.
//!   This is the useful answer for `{{ notification.identifier }}` and
//!   `{{ notification.payload }}` whole-blob dumps.
//! - Missing path (any `Value::get` returns `None`): [`TemplateErrorKind::Missing`].
//! - Missing env var (`std::env::var` returns `Err(NotPresent)`):
//!   [`TemplateErrorKind::EnvNotSet`].
//! - `BadSyntax` from [`compile`]: unclosed `{{`, empty path segment in
//!   `notification.a..b`, unknown namespace (not `notification` or `env`),
//!   or escape character used inside an expression.
//!
//! # Public vs crate-private surface
//!
//! [`TemplateErrorKind`] is public because it appears as a field of the
//! public [`crate::watch::TriggerError::Template`] variant. The
//! crate-private [`TemplateError`] is a carrier struct used internally;
//! [`template_error_to_trigger_error`] converts it to the public
//! variant at the dispatch boundary, emitting a DEBUG tracing event
//! with the raw template (which never reaches the public error).

use crate::Notification;

mod error;
mod shell;
#[cfg(test)]
mod tests;
mod url;

pub use error::TemplateErrorKind;
pub(crate) use error::{TemplateError, template_error_to_trigger_error};
use shell::ShellTracker;
use url::{UrlTracker, percent_encode};

/// What the rendered text is used for, which decides how substituted
/// notification values are neutralised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sink {
    /// Values are inserted verbatim. For text whose caller supplies the
    /// surrounding encoding, such as a JSON body or a header value.
    Raw,
    /// The text is handed to `/bin/sh -c`. Each value is quoted for the
    /// shell context it lands in, so it is read as one literal word.
    Shell,
    /// The text is a URL. Each value is percent-encoded.
    Url,
}

/// A template parsed into segments and ready to render. Constructed via
/// [`compile`] and rendered via [`CompiledTemplate::render`].
#[derive(Debug, Clone)]
pub(crate) struct CompiledTemplate {
    raw: String,
    segments: Vec<Segment>,
}

#[derive(Debug, Clone)]
enum Segment {
    /// A literal piece of the template (the text outside `{{ ... }}`
    /// pairs, with `\{{` escapes already converted to literal `{{`).
    Literal(String),
    /// `{{ notification }}` or `{{ notification.a.b.c }}`. Empty `path`
    /// means the whole notification.
    NotificationPath(Vec<String>),
    /// `{{ env.NAME }}`.
    EnvVar(String),
}

/// Parses a template source into a [`CompiledTemplate`].
///
/// Returns [`TemplateError`] with `kind = TemplateErrorKind::BadSyntax`
/// when the template is malformed. The `field` is a safe static label
/// naming the parse failure (`"unclosed_braces"`, `"empty_path_segment"`,
/// `"unknown_namespace"`, `"escape_inside_expr"`).
pub(crate) fn compile(template: &str) -> Result<CompiledTemplate, TemplateError> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut literal_buf = String::new();
    let mut chars = template.char_indices().peekable();

    while let Some((_, ch)) = chars.next() {
        if ch == '\\' {
            // `\{{` escape: emit literal `{{` and skip both braces.
            if let Some(&(_, next1)) = chars.peek()
                && next1 == '{'
            {
                chars.next();
                if let Some(&(_, next2)) = chars.peek()
                    && next2 == '{'
                {
                    chars.next();
                    literal_buf.push_str("{{");
                    continue;
                }
                // `\{` not followed by another `{`: treat as literal `\` and `{`.
                literal_buf.push('\\');
                literal_buf.push('{');
                continue;
            }
            literal_buf.push('\\');
            continue;
        }

        if ch == '{'
            && let Some(&(_, '{')) = chars.peek()
        {
            chars.next();
            // Found `{{`. Flush the literal buffer if non-empty,
            // then parse the expression up to the matching `}}`.
            if !literal_buf.is_empty() {
                segments.push(Segment::Literal(std::mem::take(&mut literal_buf)));
            }
            let segment = parse_expression(&mut chars, template)?;
            segments.push(segment);
            continue;
        }

        literal_buf.push(ch);
    }

    if !literal_buf.is_empty() {
        segments.push(Segment::Literal(literal_buf));
    }

    Ok(CompiledTemplate {
        raw: template.to_string(),
        segments,
    })
}

/// Parses the inside of a `{{ ... }}` pair plus the closing `}}`.
///
/// The opening `{{` has already been consumed. Returns the parsed
/// segment on success, or a `BadSyntax` error with a safe static
/// `field` label on failure.
fn parse_expression(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    template: &str,
) -> Result<Segment, TemplateError> {
    let mut expr = String::new();
    let mut closed = false;

    while let Some((_, ch)) = chars.next() {
        if ch == '\\' {
            // No escapes are honoured inside expressions; surfacing as
            // BadSyntax with a safe label avoids any chance of leaking
            // a raw fragment back through the public error.
            return Err(TemplateError {
                raw_template: template.to_string(),
                field: "escape_inside_expr".to_string(),
                kind: TemplateErrorKind::BadSyntax,
            });
        }
        if ch == '}'
            && let Some(&(_, '}')) = chars.peek()
        {
            chars.next();
            closed = true;
            break;
        }
        expr.push(ch);
    }

    if !closed {
        return Err(TemplateError {
            raw_template: template.to_string(),
            field: "unclosed_braces".to_string(),
            kind: TemplateErrorKind::BadSyntax,
        });
    }

    let trimmed = expr.trim();
    if let Some(rest) = trimmed.strip_prefix("notification") {
        if rest.is_empty() {
            return Ok(Segment::NotificationPath(Vec::new()));
        }
        if let Some(path_str) = rest.strip_prefix('.') {
            let path: Vec<String> = path_str.split('.').map(str::to_string).collect();
            if path.iter().any(String::is_empty) {
                return Err(TemplateError {
                    raw_template: template.to_string(),
                    field: "empty_path_segment".to_string(),
                    kind: TemplateErrorKind::BadSyntax,
                });
            }
            return Ok(Segment::NotificationPath(path));
        }
        return Err(TemplateError {
            raw_template: template.to_string(),
            field: "unknown_namespace".to_string(),
            kind: TemplateErrorKind::BadSyntax,
        });
    }
    if let Some(rest) = trimmed.strip_prefix("env.") {
        if rest.is_empty() {
            return Err(TemplateError {
                raw_template: template.to_string(),
                field: "empty_path_segment".to_string(),
                kind: TemplateErrorKind::BadSyntax,
            });
        }
        return Ok(Segment::EnvVar(rest.to_string()));
    }
    Err(TemplateError {
        raw_template: template.to_string(),
        field: "unknown_namespace".to_string(),
        kind: TemplateErrorKind::BadSyntax,
    })
}

impl CompiledTemplate {
    /// Renders the template against `notification`, returning the
    /// substituted output string.
    ///
    /// Production callers use [`Self::render`]; tests inject a fake env
    /// resolver via [`Self::render_with_env`] to avoid mutating the
    /// process environment (`std::env::set_var` is `unsafe` and the
    /// crate forbids unsafe).
    pub(crate) fn render(
        &self,
        notification: &Notification,
        sink: Sink,
    ) -> Result<String, TemplateError> {
        // Match VarError variants explicitly so a present-but-not-UTF-8
        // env var surfaces as the distinct `EnvNotUnicode` error rather
        // than collapsing into `EnvNotSet` (which would mislead the
        // operator looking for a misconfigured deployment when the
        // actual bug is in the value).
        self.render_with_env(
            notification,
            |name| match std::env::var(name) {
                Ok(value) => Ok(value),
                Err(std::env::VarError::NotPresent) => Err(TemplateErrorKind::EnvNotSet),
                Err(std::env::VarError::NotUnicode(_)) => Err(TemplateErrorKind::EnvNotUnicode),
            },
            sink,
        )
    }

    /// Names of every `{{ env.NAME }}` the template reads, in order of
    /// first appearance. The command trigger uses this to keep those
    /// variables out of the child process it spawns.
    pub(crate) fn env_names(&self) -> impl Iterator<Item = &str> {
        self.segments.iter().filter_map(|segment| match segment {
            Segment::EnvVar(name) => Some(name.as_str()),
            Segment::Literal(_) | Segment::NotificationPath(_) => None,
        })
    }

    /// Renders the template with an injected env-var resolver.
    ///
    /// The resolver returns `Ok(value)` when the variable is set and
    /// usable, or `Err(kind)` when it is not; `kind` is the
    /// [`TemplateErrorKind`] that gets carried into the resulting
    /// `TemplateError`. Production wires `std::env::var` and maps
    /// `VarError::NotPresent` -> `EnvNotSet` and
    /// `VarError::NotUnicode` -> `EnvNotUnicode`. Tests pass a
    /// closure that returns hardcoded values.
    pub(crate) fn render_with_env<F>(
        &self,
        notification: &Notification,
        env_resolver: F,
        sink: Sink,
    ) -> Result<String, TemplateError>
    where
        F: Fn(&str) -> Result<String, TemplateErrorKind>,
    {
        // Serialise the notification ONCE per render; cache the value
        // for the duration of this call so multiple notification-path
        // segments share the same JSON walk basis.
        //
        // A `serde_json::to_value` failure is reported with the
        // distinct `NotificationEncode` kind so the operator's
        // diagnosis points at the notification itself, not at a
        // missing template path. The well-typed `Notification`
        // shape makes this path practically unreachable, but the
        // mapping is correct if it ever fires.
        let notification_json: serde_json::Value =
            serde_json::to_value(notification).map_err(|_| TemplateError {
                raw_template: self.raw.clone(),
                field: "notification".to_string(),
                kind: TemplateErrorKind::NotificationEncode,
            })?;

        // For the shell sink, follow everything the shell will read, the
        // operator's text and the quoted values alike, so each value is
        // quoted for the context it lands in. Env values are the
        // operator's own and are read as written.
        let mut shell = ShellTracker::new();
        let mut url = UrlTracker::new();
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Literal(s) => {
                    match sink {
                        Sink::Shell => shell.advance(s),
                        Sink::Url => url.advance(s),
                        Sink::Raw => {}
                    }
                    out.push_str(s);
                }
                Segment::NotificationPath(path) => {
                    let value =
                        walk_path(&notification_json, path).ok_or_else(|| TemplateError {
                            raw_template: self.raw.clone(),
                            field: field_name(path),
                            kind: TemplateErrorKind::Missing,
                        })?;
                    let rendered = render_value(value);
                    match sink {
                        Sink::Raw => out.push_str(&rendered),
                        Sink::Shell => {
                            if let Some(construct) = shell.unsupported() {
                                return Err(TemplateError {
                                    raw_template: self.raw.clone(),
                                    field: construct.to_string(),
                                    kind: TemplateErrorKind::ValueAfterUnsupportedShellSyntax,
                                });
                            }
                            let quoted = shell.quote(&rendered);
                            shell.advance(&quoted);
                            out.push_str(&quoted);
                        }
                        Sink::Url => {
                            if !url.in_path() {
                                return Err(TemplateError {
                                    raw_template: self.raw.clone(),
                                    field: field_name(path),
                                    kind: TemplateErrorKind::ValueInUrlAuthority,
                                });
                            }
                            let encoded = percent_encode(&rendered);
                            url.advance(&encoded);
                            out.push_str(&encoded);
                        }
                    }
                }
                Segment::EnvVar(name) => {
                    let value = env_resolver(name).map_err(|kind| TemplateError {
                        raw_template: self.raw.clone(),
                        field: name.clone(),
                        kind,
                    })?;
                    match sink {
                        Sink::Shell => shell.advance(&value),
                        Sink::Url => url.advance(&value),
                        Sink::Raw => {}
                    }
                    out.push_str(&value);
                }
            }
        }
        Ok(out)
    }

    /// Returns the original template source for tests that need to
    /// verify the engine retained the raw string for DEBUG-level
    /// tracing. Never include this in public error variants.
    #[cfg(test)]
    pub(crate) fn raw(&self) -> &str {
        &self.raw
    }
}

/// The `field` label for a notification path: `notification` for the
/// whole notification, `notification.a.b` otherwise.
fn field_name(path: &[String]) -> String {
    if path.is_empty() {
        "notification".to_string()
    } else {
        format!("notification.{}", path.join("."))
    }
}

fn walk_path<'a>(root: &'a serde_json::Value, path: &[String]) -> Option<&'a serde_json::Value> {
    let mut cursor = root;
    for segment in path {
        cursor = cursor.get(segment)?;
    }
    Some(cursor)
}

/// Renders a single [`serde_json::Value`] into a substituted string per
/// the resolution rules in the module docs.
fn render_value(value: &serde_json::Value) -> String {
    match value {
        // Pattern-match the String variant to extract the inner str
        // without unwrap()/expect(); this preserves the "render scalar
        // strings UNQUOTED" rule while staying lint-clean.
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        // Numbers, bools, objects, arrays: Display does the right
        // thing. Numbers and bools have no quotes; objects and arrays
        // serialise as compact JSON.
        other => other.to_string(),
    }
}
