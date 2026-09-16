// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! OpenTelemetry-aligned JSON log formatter.
//!
//! `tracing-subscriber`'s built-in `.json()` formatter emits a
//! Rust-idiomatic shape (`level`, `fields.message`, `target`) that
//! diverges from the field names defined in the [OpenTelemetry logs
//! data model][otel]: `severityText`, `severityNumber`, `body`,
//! `resource.*`, `attributes.*`. This module ships an [`OtelLogFormat`]
//! that emits the OTel shape directly, with the CLI's `service.name`
//! and `service.version` injected on every event as resource
//! attributes per the OTel "minimum contract".
//!
//! Adopted because ECMWF's [Observability guidelines][ecmwf] mandate
//! that ECMWF software SHOULD emit structured logs aligned with the
//! OpenTelemetry log data model. The format itself is OTel's; the
//! ECMWF document is the consumer-side requirement that points back
//! at the OTel spec.
//!
//! The formatter does NOT include `traceId`/`spanId` because the CLI
//! does not currently instrument distributed tracing (the OTel spec
//! marks those fields `MUST when available`; they are not available).
//!
//! [otel]: https://opentelemetry.io/docs/specs/otel/logs/data-model/
//! [ecmwf]: https://raw.githubusercontent.com/ecmwf/codex/refs/heads/main/Guidelines/Observability.md

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::registry::LookupSpan;

/// Custom `FormatEvent` impl that emits OTel-shape JSON.
///
/// Each event renders as a single JSON object containing:
///
/// - `timestamp`: UTC RFC3339 with microsecond precision.
/// - `severityText`: `TRACE` / `DEBUG` / `INFO` / `WARN` / `ERROR`.
/// - `severityNumber`: numeric per OTel data model (1/5/9/13/17).
/// - `body`: the human-readable message string.
/// - `resource.service.name` + `resource.service.version`: from the
///   binary's compile-time metadata, captured at startup.
/// - `resource.deployment.environment`: set from the
///   `AVISO_DEPLOYMENT_ENV` env var when present; omitted otherwise.
/// - `attributes.*`: every other recorded field, INCLUDING the
///   stable `event.name` value when the emission carries one.
pub(crate) struct OtelLogFormat {
    service_name: &'static str,
    service_version: &'static str,
    deployment_environment: Option<String>,
}

impl OtelLogFormat {
    /// Constructs the formatter with `service.name = "aviso-cli"` and
    /// `service.version` pinned to the library's compile-time
    /// [`aviso::VERSION`] constant. The optional
    /// `deployment.environment` is read from `AVISO_DEPLOYMENT_ENV`
    /// at construction time (the CLI invokes `init_tracing` once,
    /// so this snapshot is stable for the process lifetime).
    pub(crate) fn new() -> Self {
        Self {
            service_name: "aviso-cli",
            service_version: aviso::VERSION,
            deployment_environment: std::env::var("AVISO_DEPLOYMENT_ENV")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }
}

impl<S, N> FormatEvent<S, N> for OtelLogFormat
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let level = *metadata.level();

        let mut visitor = OtelFieldVisitor::default();
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                let extensions = span.extensions();
                if let Some(fields) =
                    extensions.get::<tracing_subscriber::fmt::FormattedFields<N>>()
                {
                    let attributes: Map<String, Value> =
                        serde_json::from_str(fields.as_str()).map_err(|_| fmt::Error)?;
                    visitor.attributes.extend(attributes);
                }
            }
        }
        event.record(&mut visitor);

        let mut record = Map::new();
        record.insert("timestamp".into(), Value::String(now_rfc3339_micros()));
        record.insert(
            "severityText".into(),
            Value::String(severity_text(level).into()),
        );
        record.insert(
            "severityNumber".into(),
            Value::Number(severity_number(level).into()),
        );
        record.insert("body".into(), Value::String(visitor.body));

        let mut resource = Map::new();
        resource.insert(
            "service.name".into(),
            Value::String(self.service_name.into()),
        );
        resource.insert(
            "service.version".into(),
            Value::String(self.service_version.into()),
        );
        if let Some(env) = &self.deployment_environment {
            resource.insert("deployment.environment".into(), Value::String(env.clone()));
        }
        record.insert("resource".into(), Value::Object(resource));

        if !visitor.attributes.is_empty() {
            record.insert("attributes".into(), Value::Object(visitor.attributes));
        }

        let line = serde_json::to_string(&record).map_err(|_| fmt::Error)?;
        writeln!(writer, "{line}")
    }
}

/// Formats `SystemTime::now()` as a UTC RFC3339 string with
/// microsecond precision. Uses `humantime::format_rfc3339_micros`
/// to avoid adding the `clock` feature to `chrono` (which would
/// otherwise grow the dep graph for one format call).
fn now_rfc3339_micros() -> String {
    humantime::format_rfc3339_micros(SystemTime::now()).to_string()
}

/// `FormatTime` impl emitting a UTC `HH:MM:SS.mmm` time-only stamp
/// for interactive operator use. Drops the date because operators
/// running the CLI interactively know what day it is, and the
/// full RFC3339 form `2026-05-22T12:19:15.333678Z` dwarfs the
/// actual log body on a terminal. Headless output continues to
/// emit full RFC3339 via the OTel JSON format; this timer is only
/// installed on the TTY branch of [`crate::init_tracing`].
pub(crate) struct ShortClockTimer;

impl FormatTime for ShortClockTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> fmt::Result {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        let secs_today = now.as_secs() % 86_400;
        let h = (secs_today / 3_600) % 24;
        let m = (secs_today / 60) % 60;
        let s = secs_today % 60;
        let millis = now.subsec_millis();
        write!(w, "{h:02}:{m:02}:{s:02}.{millis:03}")
    }
}

/// Maps a `tracing::Level` to the matching `severityText` value
/// from OTel's logs data model.
fn severity_text(level: tracing::Level) -> &'static str {
    match level {
        tracing::Level::ERROR => "ERROR",
        tracing::Level::WARN => "WARN",
        tracing::Level::INFO => "INFO",
        tracing::Level::DEBUG => "DEBUG",
        tracing::Level::TRACE => "TRACE",
    }
}

/// Converts a `f64` field value into a JSON value, emitting
/// non-finite values (`NaN`, `+Infinity`, `-Infinity`) as strings.
/// JSON has no native representation for non-finite floats, so
/// `serde_json::Number::from_f64` returns `None` in those cases;
/// the previous behaviour of silently substituting `0` was
/// misleading because a numerically distinct sentinel becomes
/// indistinguishable from a real `0.0` measurement. Routing
/// non-finite values through a stable string representation
/// preserves the operator's ability to diagnose them in
/// downstream log queries.
fn f64_to_value(value: f64) -> Value {
    match serde_json::Number::from_f64(value) {
        Some(n) => Value::Number(n),
        None if value.is_nan() => Value::String("NaN".into()),
        None if value.is_sign_negative() => Value::String("-Infinity".into()),
        None => Value::String("Infinity".into()),
    }
}

/// Maps a `tracing::Level` to the matching `severityNumber` value
/// from OTel's logs data model. Uses the lowest number in each
/// range per the spec's "use the lowest value unless a finer
/// distinction is needed" guidance.
fn severity_number(level: tracing::Level) -> u8 {
    match level {
        tracing::Level::TRACE => 1,
        tracing::Level::DEBUG => 5,
        tracing::Level::INFO => 9,
        tracing::Level::WARN => 13,
        tracing::Level::ERROR => 17,
    }
}

/// `tracing::field::Visit` that extracts the message into `body`
/// and every other recorded field into `attributes`.
///
/// `tracing` records the format-string message via `record_debug`
/// with a `&format_args!()` argument. `format_args` implements
/// `Debug` to match `Display` (no surrounding quotes), so the
/// rendered value lands in `body` as-is.
#[derive(Default)]
struct OtelFieldVisitor {
    body: String,
    attributes: Map<String, Value>,
}

impl Visit for OtelFieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.body = value.to_string();
        } else {
            self.attributes
                .insert(field.name().into(), Value::String(value.into()));
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.attributes
            .insert(field.name().into(), Value::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.attributes
            .insert(field.name().into(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.attributes
            .insert(field.name().into(), Value::Number(value.into()));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.attributes
            .insert(field.name().into(), f64_to_value(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let formatted = format!("{value:?}");
        if field.name() == "message" {
            self.body = formatted;
        } else {
            self.attributes
                .insert(field.name().into(), Value::String(formatted));
        }
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.attributes
            .insert(field.name().into(), Value::String(value.to_string()));
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect/panic on JSON roundtrip is the expected diagnostic"
)]
mod tests {
    use super::*;

    #[test]
    fn severity_text_uppercase_matches_otel_spec() {
        assert_eq!(severity_text(tracing::Level::ERROR), "ERROR");
        assert_eq!(severity_text(tracing::Level::WARN), "WARN");
        assert_eq!(severity_text(tracing::Level::INFO), "INFO");
        assert_eq!(severity_text(tracing::Level::DEBUG), "DEBUG");
        assert_eq!(severity_text(tracing::Level::TRACE), "TRACE");
    }

    #[test]
    fn severity_number_lowest_in_each_otel_band() {
        assert_eq!(severity_number(tracing::Level::TRACE), 1);
        assert_eq!(severity_number(tracing::Level::DEBUG), 5);
        assert_eq!(severity_number(tracing::Level::INFO), 9);
        assert_eq!(severity_number(tracing::Level::WARN), 13);
        assert_eq!(severity_number(tracing::Level::ERROR), 17);
    }

    #[test]
    fn timestamp_is_rfc3339_with_microsecond_precision() {
        let ts = now_rfc3339_micros();
        assert!(ts.ends_with('Z'), "expected trailing Z: {ts}");
        let dot = ts.find('.').expect("expected . separator");
        let frac = &ts[dot + 1..ts.len() - 1];
        assert_eq!(
            frac.len(),
            6,
            "expected 6-digit fractional seconds, got {frac:?}"
        );
    }

    #[test]
    fn visitor_default_is_empty() {
        let v = OtelFieldVisitor::default();
        assert!(v.body.is_empty());
        assert!(v.attributes.is_empty());
    }

    #[test]
    fn f64_to_value_emits_nan_and_infinity_as_distinct_strings() {
        assert_eq!(f64_to_value(f64::NAN), Value::String("NaN".into()));
        assert_eq!(
            f64_to_value(f64::INFINITY),
            Value::String("Infinity".into())
        );
        assert_eq!(
            f64_to_value(f64::NEG_INFINITY),
            Value::String("-Infinity".into())
        );
    }

    #[test]
    fn f64_to_value_emits_finite_values_as_numbers() {
        match f64_to_value(1.5) {
            Value::Number(n) => assert!((n.as_f64().expect("finite") - 1.5).abs() < f64::EPSILON),
            other => panic!("expected Number for 1.5, got {other:?}"),
        }
        match f64_to_value(0.0) {
            Value::Number(n) => assert_eq!(n.as_f64(), Some(0.0)),
            other => panic!("expected Number for 0.0, got {other:?}"),
        }
    }
}
