//! `aviso config dump` subcommand.
//!
//! Prints the resolved config to stdout in YAML by default (or
//! JSON when `--json` is set globally). Source-attribution comments
//! `# from: flag|env|file|default` appear in the YAML form only;
//! the JSON form drops them because JSON has no native comment
//! syntax (the `--json` consumer is presumably a tool that does
//! not need them).
//!
//! `--redact` masks `auth.bearer_token`, `auth.basic.password`,
//! and the captured `--token` / `--password` flag values so the
//! dumped output is safe to paste into an issue tracker.

use std::fmt::Write as _;

use anyhow::Result;

use crate::config::{Resolved, Source, Sourced};
use crate::output;

/// Runs the `aviso config dump` subcommand.
///
/// When `redact` is true the auth-bearing values are replaced with
/// `<redacted>`. YAML is the default output format in BOTH TTY and
/// pipe contexts (config dump is a configuration-inspection
/// command, not a streaming-data command); `--json` forces JSON
/// output per Q5.
pub(crate) fn run(resolved: &Resolved, redact: bool) -> Result<()> {
    if resolved.force_json {
        let payload = build_json_payload(resolved, redact);
        let line = serde_json::to_string(&payload)?;
        output::write_stdout_line(&line)
    } else {
        let yaml = build_yaml(resolved, redact);
        output::write_stdout_bytes(yaml.as_bytes())
    }
}

fn build_yaml(resolved: &Resolved, redact: bool) -> String {
    let mut out = String::new();
    out.push_str("# resolved configuration (per-field source tags shown)\n");
    out.push_str(&yaml_line(
        "config_path",
        &resolved.config_path.display().to_string(),
        Source::File,
    ));
    out.push_str(&yaml_sourced("state_file", &resolved.state_path, |p| {
        p.display().to_string()
    }));
    if let Some(b) = &resolved.base_url {
        out.push_str(&yaml_sourced("base_url", b, String::clone));
    } else {
        out.push_str(&yaml_line("base_url", "<unset>", Source::Default));
    }
    if let Some(t) = &resolved.timeout {
        out.push_str(&yaml_sourced("timeout", t, |d| format!("{}s", d.as_secs())));
    }
    if let Some(h) = &resolved.heartbeat_interval {
        out.push_str(&yaml_sourced("heartbeat_interval", h, |d| {
            format!("{}s", d.as_secs())
        }));
    }
    out.push_str("tls:\n");
    let _ = writeln!(
        out,
        "  ca_bundle_count: {} # from: {}",
        resolved.tls_ca_bundle_paths.value.len(),
        source_label(resolved.tls_ca_bundle_paths.source)
    );
    let _ = writeln!(
        out,
        "  danger_accept_invalid_certs: {} # from: {}",
        resolved.tls_danger_accept_invalid_certs.value,
        source_label(resolved.tls_danger_accept_invalid_certs.source)
    );
    out.push_str("auth:\n");
    let provider_label = if resolved.auth_provider.is_some() {
        if redact { "<set; redacted>" } else { "<set>" }
    } else {
        "<unset>"
    };
    let _ = writeln!(out, "  provider: {provider_label}");
    let _ = writeln!(out, "listeners_count: {}", resolved.listeners.len());
    if !resolved.listeners.is_empty() {
        out.push_str("listeners:\n");
        for listener in &resolved.listeners {
            let _ = writeln!(
                out,
                "  - name: {}\n    event: {}\n    identifiers: {}",
                listener.name.as_deref().unwrap_or("<unnamed>"),
                listener.event,
                listener.identifiers.len()
            );
            if let Some(id) = listener.from_id {
                let _ = writeln!(out, "    from_id: {id}");
            }
            if let Some(d) = listener.from_date.as_deref() {
                let _ = writeln!(out, "    from_date: {d}");
            }
            let _ = writeln!(out, "    triggers_count: {}", listener.triggers.len());
        }
    }
    let _ = writeln!(
        out,
        "verbose: {} # from: flag\nforce_json: {} # from: flag\nno_color: {} # from: flag",
        resolved.verbose, resolved.force_json, resolved.no_color
    );
    out
}

fn yaml_line(key: &str, value: &str, source: Source) -> String {
    format!("{key}: {value} # from: {}\n", source_label(source))
}

fn yaml_sourced<T, F>(key: &str, sourced: &Sourced<T>, render: F) -> String
where
    F: FnOnce(&T) -> String,
{
    let rendered = render(&sourced.value);
    yaml_line(key, &rendered, sourced.source)
}

fn build_json_payload(resolved: &Resolved, redact: bool) -> serde_json::Value {
    use serde_json::json;
    json!({
        "config_path": resolved.config_path.display().to_string(),
        "state_file": {
            "value": resolved.state_path.value.display().to_string(),
            "source": source_label(resolved.state_path.source),
        },
        "base_url": resolved.base_url.as_ref().map(|s| json!({
            "value": s.value,
            "source": source_label(s.source),
        })),
        "timeout_seconds": resolved.timeout.as_ref().map(|s| json!({
            "value": s.value.as_secs(),
            "source": source_label(s.source),
        })),
        "heartbeat_interval_seconds": resolved.heartbeat_interval.as_ref().map(|s| json!({
            "value": s.value.as_secs(),
            "source": source_label(s.source),
        })),
        "tls": {
            "ca_bundle_count": resolved.tls_ca_bundle_paths.value.len(),
            "ca_bundle_source": source_label(resolved.tls_ca_bundle_paths.source),
            "danger_accept_invalid_certs": resolved.tls_danger_accept_invalid_certs.value,
            "danger_source": source_label(resolved.tls_danger_accept_invalid_certs.source),
        },
        "auth": {
            "provider_set": resolved.auth_provider.is_some(),
            "redacted": redact,
        },
        "listeners_count": resolved.listeners.len(),
        "verbose": resolved.verbose,
        "force_json": resolved.force_json,
        "no_color": resolved.no_color,
    })
}

fn source_label(source: Source) -> &'static str {
    match source {
        Source::Flag => "flag",
        Source::Env => "env",
        Source::File => "file",
        Source::Default => "default",
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on synthetic fixtures is the expected diagnostic"
)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(redacted_provider: bool) -> Resolved {
        Resolved {
            config_path: PathBuf::from("/tmp/aviso-test/config.yaml"),
            state_path: Sourced {
                value: PathBuf::from("/tmp/aviso-test/state.json"),
                source: Source::Default,
            },
            base_url: Some(Sourced {
                value: "https://aviso.example".to_string(),
                source: Source::Flag,
            }),
            timeout: None,
            heartbeat_interval: None,
            tls_ca_bundle_paths: Sourced {
                value: Vec::new(),
                source: Source::Default,
            },
            tls_danger_accept_invalid_certs: Sourced {
                value: false,
                source: Source::Default,
            },
            auth_provider: redacted_provider.then(|| {
                std::sync::Arc::new(aviso::auth::Bearer::new("test-token").unwrap())
                    as std::sync::Arc<dyn aviso::auth::AuthProvider>
            }),
            listeners: Vec::new(),
            force_json: false,
            no_color: false,
            verbose: 0,
        }
    }

    #[test]
    fn yaml_form_includes_source_attribution_comments() {
        let yaml = build_yaml(&fixture(false), false);
        assert!(yaml.contains("# from: flag"));
        assert!(yaml.contains("# from: default"));
        assert!(yaml.contains("config_path"));
        assert!(yaml.contains("https://aviso.example"));
    }

    #[test]
    fn json_form_is_valid_json_with_source_fields() {
        let payload = build_json_payload(&fixture(false), false);
        let serialised = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&serialised).unwrap();
        let base = parsed
            .get("base_url")
            .and_then(|b| b.get("source"))
            .and_then(serde_json::Value::as_str)
            .unwrap();
        assert_eq!(base, "flag");
    }

    #[test]
    fn redact_set_when_auth_present_and_redact_requested() {
        let yaml = build_yaml(&fixture(true), true);
        assert!(yaml.contains("redacted"));
    }

    #[test]
    fn redact_not_set_when_auth_absent() {
        let yaml = build_yaml(&fixture(false), true);
        assert!(yaml.contains("<unset>"));
    }
}
