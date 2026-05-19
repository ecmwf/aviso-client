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

#[cfg(test)]
mod tests;

/// Categorises a template-engine failure. Public because it appears as
/// the `kind` field of [`crate::watch::TriggerError::Template`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateErrorKind {
    /// A `{{ notification.<path> }}` expression resolved to no value
    /// (one of the path segments did not exist on the notification JSON).
    Missing,
    /// A `{{ env.<NAME> }}` expression's environment variable was not
    /// set in the process environment.
    EnvNotSet,
    /// The template source itself was malformed. The accompanying
    /// `field` on [`crate::watch::TriggerError::Template`] names the
    /// parse failure category, NOT a snippet of the raw template, so
    /// it is safe to surface even when the template contains secrets.
    BadSyntax,
}

/// Crate-private carrier returned by the template engine. The raw
/// template is retained for DEBUG-level tracing and never reaches the
/// public [`crate::watch::TriggerError::Template`] variant.
///
/// `Clone` is required because [`CompiledTemplate`] is cloneable and
/// dispatchers may store the compile result by value.
#[derive(Debug, Clone)]
pub(crate) struct TemplateError {
    /// The original template source. Useful for DEBUG logging; not
    /// surfaced in public errors.
    pub raw_template: String,
    /// What failed: a JSON path (`"notification.payload.target"`), an
    /// env-var name (`"SLACK_TOKEN"`), or a safe static label for
    /// `BadSyntax` (`"unclosed_braces"`, `"empty_path_segment"`, etc.).
    pub field: String,
    /// Categorisation of the failure.
    pub kind: TemplateErrorKind,
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
            if let Some(&(_, next1)) = chars.peek() {
                if next1 == '{' {
                    chars.next();
                    if let Some(&(_, next2)) = chars.peek() {
                        if next2 == '{' {
                            chars.next();
                            literal_buf.push_str("{{");
                            continue;
                        }
                    }
                    // `\{` not followed by another `{`: treat as literal `\` and `{`.
                    literal_buf.push('\\');
                    literal_buf.push('{');
                    continue;
                }
            }
            literal_buf.push('\\');
            continue;
        }

        if ch == '{' {
            if let Some(&(_, '{')) = chars.peek() {
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
        if ch == '}' {
            if let Some(&(_, '}')) = chars.peek() {
                chars.next();
                closed = true;
                break;
            }
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
    pub(crate) fn render(&self, notification: &Notification) -> Result<String, TemplateError> {
        self.render_with_env(notification, |name| std::env::var(name).ok())
    }

    /// Renders the template with an injected env-var resolver.
    ///
    /// The resolver returns `Some(value)` when the variable is set, or
    /// `None` when it is not. Production wires `std::env::var`; tests
    /// pass a closure that returns hardcoded values.
    pub(crate) fn render_with_env<F>(
        &self,
        notification: &Notification,
        env_resolver: F,
    ) -> Result<String, TemplateError>
    where
        F: Fn(&str) -> Option<String>,
    {
        // Serialise the notification ONCE per render; cache the value
        // for the duration of this call so multiple notification-path
        // segments share the same JSON walk basis.
        let notification_json: serde_json::Value =
            serde_json::to_value(notification).map_err(|_| TemplateError {
                raw_template: self.raw.clone(),
                field: "notification".to_string(),
                kind: TemplateErrorKind::Missing,
            })?;

        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Literal(s) => out.push_str(s),
                Segment::NotificationPath(path) => {
                    let value =
                        walk_path(&notification_json, path).ok_or_else(|| TemplateError {
                            raw_template: self.raw.clone(),
                            field: if path.is_empty() {
                                "notification".to_string()
                            } else {
                                format!("notification.{}", path.join("."))
                            },
                            kind: TemplateErrorKind::Missing,
                        })?;
                    out.push_str(&render_value(value));
                }
                Segment::EnvVar(name) => {
                    let value = env_resolver(name).ok_or_else(|| TemplateError {
                        raw_template: self.raw.clone(),
                        field: name.clone(),
                        kind: TemplateErrorKind::EnvNotSet,
                    })?;
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

/// Converts the crate-private [`TemplateError`] to the public
/// [`crate::watch::TriggerError::Template`] variant.
///
/// The `context` is a SAFE static label set by the dispatch boundary
/// (`"command"`, `"webhook url"`, etc.), NOT the raw template text.
/// The raw template is emitted at DEBUG level for operators who
/// control the logging sink, but never reaches the public error.
pub(crate) fn template_error_to_trigger_error(
    e: TemplateError,
    context: impl Into<String>,
) -> crate::watch::TriggerError {
    let context_str = context.into();
    tracing::debug!(
        event.name = "client.trigger.template.render_failed",
        context = %context_str,
        raw_template = %e.raw_template,
        field = %e.field,
        kind = ?e.kind,
        "template render failed (raw template suppressed from public error)"
    );
    crate::watch::TriggerError::Template {
        context: context_str,
        field: e.field,
        kind: e.kind,
    }
}
