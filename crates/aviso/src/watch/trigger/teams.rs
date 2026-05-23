//! Teams trigger dispatch.
//!
//! Sugar over the webhook trigger that auto-builds a Microsoft Teams
//! Adaptive Card from the notification at dispatch time. The operator
//! configures a URL and an optional title template; aviso renders the
//! title, iterates the notification's identifier into a FactSet,
//! renders the payload into a second card section, and POSTs the
//! synthesised body via the same HTTP machinery as the webhook
//! trigger (so retry classification, response-body capture, and
//! error mapping are identical and the existing webhook hint
//! dispatcher applies).
//!
//! # Card shape
//!
//! ```text
//! [Title block]      <- rendered title_template
//! [FactSet block]    <- one row per identifier field, plus Event + Sequence
//! [Payload block]    <- another FactSet (flat object payload) OR
//!                       monospace TextBlock (scalar / array / null)
//! ```
//!
//! Operators wanting full control of the card body use the [`super::TriggerKind::Webhook`]
//! trigger directly with a hand-written `body_template`.

use std::time::Duration;

use crate::Notification;

use super::TriggerError;
use super::http_method::HttpMethod;
use super::template::{
    CompiledTemplate, TemplateError, TemplateErrorKind, compile, template_error_to_trigger_error,
};
use super::webhook::WebhookConfig;
use super::webhook::dispatch_webhook;

/// Configuration for the Teams trigger, held inside
/// [`super::TriggerKind::Teams`]. Mirrors the redaction discipline
/// of [`WebhookConfig`]: the URL (potentially carrying a SAS token in
/// query parameters) and the title template (potentially carrying
/// secrets via `{{ env.* }}`) never appear in Debug output.
#[derive(Clone)]
pub(super) struct TeamsConfig {
    pub(super) url_template: Result<CompiledTemplate, TemplateError>,
    pub(super) title_template: Result<CompiledTemplate, TemplateError>,
}

impl std::fmt::Debug for TeamsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let TeamsConfig {
            url_template,
            title_template,
        } = self;
        let url_state: &dyn std::fmt::Display = match url_template {
            Ok(_) => &"<compiled-url-template-redacted>",
            Err(_) => &"<bad-url-template-redacted>",
        };
        let title_state: &dyn std::fmt::Display = match title_template {
            Ok(_) => &"<compiled-title-template-redacted>",
            Err(_) => &"<bad-title-template-redacted>",
        };
        f.debug_struct("TeamsConfig")
            .field("url_template", &format_args!("{url_state}"))
            .field("title_template", &format_args!("{title_state}"))
            .finish()
    }
}

/// Dispatch one Teams trigger attempt.
///
/// Renders the title via the template engine, builds the Adaptive
/// Card body from the notification's runtime data, then delegates to
/// [`dispatch_webhook`] with a synthesised [`WebhookConfig`] so the
/// HTTP send, retry classification, response-body capture, and
/// error mapping match the webhook trigger exactly. Returns
/// `Ok(())` on 2xx; the various [`TriggerError`] variants on every
/// other outcome.
pub(super) async fn dispatch_teams(
    cfg: &TeamsConfig,
    http: &reqwest::Client,
    timeout: Option<Duration>,
    notification: &Notification,
) -> Result<(), TriggerError> {
    let title = render_title(cfg, notification)?;
    let card_body_value = build_adaptive_card(notification, &title);
    let body_string = serde_json::to_string(&card_body_value).map_err(TriggerError::Encode)?;
    let webhook_cfg = synthesise_webhook_config(cfg, &body_string);
    dispatch_webhook(&webhook_cfg, http, timeout, notification).await
}

fn render_title(cfg: &TeamsConfig, notification: &Notification) -> Result<String, TriggerError> {
    let title_template = cfg
        .title_template
        .as_ref()
        .map_err(|e| template_error_to_trigger_error(e.clone(), "teams title"))?;
    title_template
        .render_with_env(notification, &env_resolver)
        .map_err(|e| template_error_to_trigger_error(e, "teams title"))
}

fn env_resolver(name: &str) -> Result<String, TemplateErrorKind> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Err(TemplateErrorKind::EnvNotSet),
        Err(std::env::VarError::NotUnicode(_)) => Err(TemplateErrorKind::EnvNotUnicode),
    }
}

fn build_adaptive_card(notification: &Notification, title: &str) -> serde_json::Value {
    let mut identifier_facts: Vec<serde_json::Value> = vec![
        serde_json::json!({"title": "Event", "value": notification.event_type.as_str()}),
        serde_json::json!({"title": "Sequence", "value": notification.sequence.to_string()}),
    ];
    for (key, value) in &notification.identifier {
        identifier_facts.push(serde_json::json!({
            "title": key,
            "value": value,
        }));
    }

    let mut card_body: Vec<serde_json::Value> = vec![
        serde_json::json!({
            "type": "TextBlock",
            "text": title,
            "weight": "Bolder",
            "size": "Medium",
            "color": "Accent",
            "wrap": true,
        }),
        serde_json::json!({
            "type": "FactSet",
            "facts": identifier_facts,
        }),
    ];

    match &notification.payload {
        serde_json::Value::Null => {}
        serde_json::Value::Object(map) if !map.is_empty() => {
            let payload_facts: Vec<serde_json::Value> = map
                .iter()
                .map(|(k, v)| serde_json::json!({"title": k, "value": render_payload_value(v)}))
                .collect();
            card_body.push(serde_json::json!({
                "type": "TextBlock",
                "text": "Payload",
                "weight": "Bolder",
                "spacing": "Medium",
            }));
            card_body.push(serde_json::json!({
                "type": "FactSet",
                "facts": payload_facts,
            }));
        }
        other => {
            let json_repr =
                serde_json::to_string(other).unwrap_or_else(|_| String::from("<unrenderable>"));
            card_body.push(serde_json::json!({
                "type": "TextBlock",
                "text": format!("Payload: {json_repr}"),
                "fontType": "Monospace",
                "wrap": true,
                "spacing": "Medium",
            }));
        }
    }

    serde_json::json!({
        "type": "message",
        "attachments": [{
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": {
                "type": "AdaptiveCard",
                "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                "version": "1.4",
                "body": card_body,
            }
        }]
    })
}

fn render_payload_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn synthesise_webhook_config(cfg: &TeamsConfig, body_string: &str) -> WebhookConfig {
    // The body is already-rendered JSON and MUST NOT be re-interpreted
    // by the webhook template engine. A notification field containing
    // `{{ foo }}` (operator-supplied via the publish path, or any other
    // user-controlled string) would otherwise be parsed as a template
    // expression and either substituted, errored on, or corrupted.
    // Escape every literal `{{` to `\{{` so the template engine emits
    // the original two-brace sequence verbatim. `}}` is not a
    // template delimiter outside an opened `{{...}}` block, so it
    // does not need escaping.
    let escaped = body_string.replace("{{", "\\{{");
    WebhookConfig {
        url_template: cfg.url_template.clone(),
        method: HttpMethod::Post,
        headers: vec![("Content-Type".to_string(), compile("application/json"))],
        body_template: Some(compile(&escaped)),
    }
}

/// Default title template used when the operator does not supply one.
pub(super) const DEFAULT_TEAMS_TITLE_TEMPLATE: &str =
    "aviso {{ notification.event_type }} #{{ notification.sequence }}";

pub(super) fn build_teams_config(url: impl Into<String>, title: impl Into<String>) -> TeamsConfig {
    let url_str = url.into();
    let title_str = title.into();
    TeamsConfig {
        url_template: compile(&url_str),
        title_template: compile(&title_str),
    }
}

pub(super) fn teams_set_title_template(cfg: &mut TeamsConfig, title: impl Into<String>) {
    let title_str = title.into();
    cfg.title_template = compile(&title_str);
}

impl super::Trigger {
    /// Build a Teams trigger that posts an auto-generated Adaptive
    /// Card to a Microsoft Teams Workflows webhook URL. The default
    /// title is [`DEFAULT_TEAMS_TITLE_TEMPLATE`]; override via
    /// [`Self::title_template`]. URL is template-rendered at dispatch.
    /// Inherits the webhook default timeout
    /// ([`super::DEFAULT_WEBHOOK_TIMEOUT`]).
    #[must_use]
    pub fn teams(url: impl Into<String>) -> Self {
        Self {
            kind: super::kind::TriggerKind::Teams(Box::new(build_teams_config(
                url,
                DEFAULT_TEAMS_TITLE_TEMPLATE,
            ))),
            retries: 0,
            required: true,
            timeout: Some(super::DEFAULT_WEBHOOK_TIMEOUT),
            fail_fast: true,
        }
    }

    /// Override the title template on a Teams trigger. Silently
    /// ignored on every non-Teams trigger kind.
    #[must_use]
    pub fn title_template(mut self, title: impl Into<String>) -> Self {
        if let super::kind::TriggerKind::Teams(cfg) = &mut self.kind {
            teams_set_title_template(cfg, title);
        }
        self
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on synthesised inputs known to be valid is the standard test diagnostic"
)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::Notification;

    fn make_notification(
        event_type: &str,
        sequence: u64,
        identifier: BTreeMap<String, String>,
        payload: serde_json::Value,
    ) -> Notification {
        Notification {
            event_type: event_type.to_string(),
            sequence,
            identifier,
            payload,
            cloudevent: None,
        }
    }

    #[test]
    fn build_card_includes_title_event_sequence_and_each_identifier_field_as_facts() {
        let mut identifier = BTreeMap::new();
        identifier.insert("class".to_string(), "od".to_string());
        identifier.insert("step".to_string(), "42".to_string());
        let n = make_notification("mars", 99, identifier, serde_json::Value::Null);
        let card = build_adaptive_card(&n, "aviso mars #99");
        let facts = &card["attachments"][0]["content"]["body"][1]["facts"];
        let mut titles: Vec<String> = facts
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["title"].as_str().unwrap().to_string())
            .collect();
        titles.sort();
        assert_eq!(titles, vec!["Event", "Sequence", "class", "step"]);
        assert_eq!(
            card["attachments"][0]["content"]["body"][0]["text"]
                .as_str()
                .unwrap(),
            "aviso mars #99"
        );
    }

    #[test]
    fn build_card_renders_string_identifier_value_without_extra_quotes() {
        let mut identifier = BTreeMap::new();
        identifier.insert("class".to_string(), "od".to_string());
        let n = make_notification("mars", 1, identifier, serde_json::Value::Null);
        let card = build_adaptive_card(&n, "title");
        let facts = &card["attachments"][0]["content"]["body"][1]["facts"];
        let class_fact = facts
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["title"] == "class")
            .unwrap();
        assert_eq!(class_fact["value"], "od");
    }

    #[test]
    fn build_card_with_null_payload_omits_payload_section() {
        let n = make_notification("mars", 1, BTreeMap::new(), serde_json::Value::Null);
        let card = build_adaptive_card(&n, "title");
        let body = card["attachments"][0]["content"]["body"]
            .as_array()
            .unwrap();
        assert_eq!(
            body.len(),
            2,
            "null payload must omit the payload section (title + identifier FactSet only)"
        );
    }

    #[test]
    fn build_card_with_object_payload_renders_payload_factset() {
        let payload = serde_json::json!({"seed": "abc", "count": 5});
        let n = make_notification("mars", 1, BTreeMap::new(), payload);
        let card = build_adaptive_card(&n, "title");
        let body = card["attachments"][0]["content"]["body"]
            .as_array()
            .unwrap();
        assert_eq!(
            body.len(),
            4,
            "object payload must render as separator + payload FactSet (4 elements total)"
        );
        let payload_facts = &body[3]["facts"].as_array().unwrap();
        let titles: Vec<String> = payload_facts
            .iter()
            .map(|f| f["title"].as_str().unwrap().to_string())
            .collect();
        assert!(titles.contains(&"seed".to_string()));
        assert!(titles.contains(&"count".to_string()));
    }

    #[test]
    fn build_card_with_scalar_payload_renders_monospace_textblock_with_json_repr() {
        let n = make_notification(
            "mars",
            1,
            BTreeMap::new(),
            serde_json::json!("scalar-string"),
        );
        let card = build_adaptive_card(&n, "title");
        let body = card["attachments"][0]["content"]["body"]
            .as_array()
            .unwrap();
        assert_eq!(body.len(), 3);
        let payload_block = &body[2];
        assert_eq!(payload_block["fontType"], "Monospace");
        assert!(
            payload_block["text"]
                .as_str()
                .unwrap()
                .contains("\"scalar-string\""),
            "scalar payload must render via serde_json::to_string into the text block: {payload_block}"
        );
    }

    #[test]
    fn build_card_serialises_to_valid_json_with_special_chars_in_identifier_value() {
        let mut identifier = BTreeMap::new();
        identifier.insert(
            "weird".to_string(),
            "contains \"quotes\" and \\ backslash".to_string(),
        );
        let n = make_notification("mars", 1, identifier, serde_json::Value::Null);
        let card = build_adaptive_card(&n, "title");
        let serialised = serde_json::to_string(&card).unwrap();
        let reparsed: serde_json::Value = serde_json::from_str(&serialised).unwrap();
        let facts = &reparsed["attachments"][0]["content"]["body"][1]["facts"];
        let weird = facts
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["title"] == "weird")
            .unwrap();
        assert_eq!(
            weird["value"], "contains \"quotes\" and \\ backslash",
            "the identifier value round-trips through JSON serialisation: serde_json handles all escaping"
        );
    }
}
