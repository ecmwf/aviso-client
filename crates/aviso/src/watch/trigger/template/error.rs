// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Template-engine failures: the public kind, the crate-private carrier,
//! and the conversion into the public trigger error.

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
    /// A `{{ env.<NAME> }}` expression's environment variable WAS set
    /// but contained bytes that are not valid UTF-8. Distinct from
    /// `EnvNotSet` because the operator's diagnosis differs: a
    /// not-set variable means a misconfigured deployment, while a
    /// not-unicode variable means the value itself needs fixing.
    EnvNotUnicode,
    /// The template source itself was malformed. The accompanying
    /// `field` on [`crate::watch::TriggerError::Template`] names the
    /// parse failure category, NOT a snippet of the raw template, so
    /// it is safe to surface even when the template contains secrets.
    BadSyntax,
    /// A `{{ notification.<path> }}` expression sits in the scheme or
    /// authority of a URL, where the value would choose the host the
    /// request goes to. Notification values may only appear in the
    /// path, query or fragment.
    ValueInUrlAuthority,
    /// A `{{ notification.<path> }}` expression in a command comes after
    /// shell syntax the engine does not follow (a here-document,
    /// arithmetic expansion, backticks or a `case` statement), or
    /// directly after a `$`, so
    /// it cannot tell how the shell would read the value there. The
    /// `field` names the reason. Reach the notification through the `AVISO_*`
    /// environment variables in such a command instead.
    ValueAfterUnsupportedShellSyntax,
    /// The notification could not be serialised to JSON. Practically
    /// unreachable given the well-typed [`crate::Notification`]
    /// shape (every field is a concrete scalar or `serde_json::Value`
    /// that already round-trips through `serde_json::to_value`), but
    /// kept as a distinct kind so the operator's diagnosis points at
    /// the notification itself rather than chasing a missing-path
    /// template bug.
    NotificationEncode,
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
