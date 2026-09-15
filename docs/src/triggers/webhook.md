# Webhook trigger

<div class="webhook-guide">

Generic HTTP request per notification. Operators get full control of method,
URL, headers, and body via the [template engine](./template-engine.md). Use this
trigger to forward notifications to any REST endpoint: Slack, Discord,
PagerDuty, custom internal services, GitHub Actions, log aggregators, and so on.

## YAML

```yaml
triggers:
  - type: webhook
    # Required, template-rendered.
    url: "https://hooks.example.com/notify"
    # Optional; POST is the default.
    method: POST
    # Optional headers and body.
    headers:
      Authorization: "Bearer {{ env.WEBHOOK_TOKEN }}"
      X-Source: "aviso-listener"
    body_template: |
      {
        "seq": {{ notification.sequence }},
        "payload": {{ notification.payload }}
      }
    # Optional; defaults to 30s.
    timeout: 30s
    # Optional; defaults to 0.
    retries: 2
    # Optional; both default to true.
    required: true
    fail_fast: true
```

## Method, URL, headers

- `method`: one of `GET`, `POST`, `PUT`, `PATCH`, `DELETE` (uppercase required).
  Default `POST`.
- `url`: template-rendered at dispatch time. Operators commonly use
  `{{ env.WEBHOOK_URL }}` to keep the URL out of the YAML.
- `headers`: header NAMES are taken literally; header VALUES are
  template-rendered. The YAML `headers` block is a map so each header name
  appears once; multi-value headers (e.g. multiple `Set-Cookie`) are not
  representable via the YAML config. Operators needing repeated header names
  should use the lib's `Trigger::webhook(...).header(name, value)` programmatic
  builder, which is repeatable.

The dispatcher auto-injects `Content-Type: application/json` when the operator
does not supply one. If you set your own `Content-Type`, the dispatcher does NOT
override it.

## Body template

When `body_template` is **absent**, the body defaults to the notification
serialised as compact JSON (matching the [echo](./echo.md) trigger's pipe-mode
shape). This abbreviated example is wrapped for readability; the actual body
is compact JSON:

```text
{
  "event_type": "mars",
  "sequence": 42,
  "identifier": {...},
  "payload": {...}
}
```

When `body_template` is **set**, that string is template-rendered at dispatch.
Two patterns are common:

The following fragments belong inside a webhook trigger. The selected-fields
example expects `class` and `date` in the notification's identifier; the metadata
example embeds its payload as JSON.

**Forward selected fields**:

```yaml
body_template: |
  {
    "who": "{{ notification.identifier.class }}",
    "what": "{{ notification.event_type }}",
    "when": "{{ notification.identifier.date }}"
  }
```

**Wrap with metadata**:

```yaml
body_template: |
  {
    "alert": {
      "title": "aviso {{ notification.event_type }} #{{ notification.sequence }}",
      "details": {{ notification.payload }},
      "source": "aviso-listener"
    }
  }
```

When embedding JSON-typed notification fields (`identifier`, `payload`) inside a
JSON string literal, see
[Template engine: value rendering](./template-engine.md#value-rendering-rules)
for the escaping rules - short version, embed them OUTSIDE a string field (as in
the example above), not inside `"text": "..."`.

## Retry classifier

The `fail_fast` setting controls which failures can be retried. It is enabled
by default (`true`); set it to `false` to disable it.

| Outcome | Fail-fast on | Fail-fast off |
|---|---|---|
| 2xx response | success | success |
| 4xx response | **terminal** (no retry) | retryable |
| 5xx response | retryable | retryable |
| Transport error | retryable | retryable |
| Timeout | retryable | retryable |
| Request build error | **terminal** | retryable |
| Template render error | **terminal** | retryable |

Transport errors include DNS, TCP, TLS, and mid-stream interrupts. A request
build error means the HTTP client refused the rendered request, for example an
invalid URL or bad header. With fail-fast off, every failure is retryable, even
deterministic request-build or template errors that will fail identically for
the same notification. The dispatcher still honours the retry budget.

Terminal failures bypass the `retries` budget. Retryable failures are retried up
to `retries + 1` total attempts with the supervisor's exponential backoff (250
ms base, doubling, 30 s cap, full jitter).

## Response body capture

The response body is captured into a 4 KiB ring buffer via streaming chunks.
Operators see the tail of the response on a failure error. This is essential for
debugging 4xx responses from services that include validation details in the
body.

Example failure surface for a 4xx, wrapped for readability (not exact output
line breaks):

```text
Error in listener my-listener: trigger webhook failed: webhook:
status=400 body_tail={"error":"missing_required_field","field":"alert.title"}
  Hint: webhook returned 4xx (400 Bad Request) which is TERMINAL
  (no retries) per the dispatcher contract: 4xx means the receiver
  rejected the request, retrying with the same notification will
  fail identically. Check the webhook URL, headers, and body_template;
  the receiver's response body is included above and may name the
  specific field that failed validation.
  Other listeners continue.
```

The 4 KiB cap is per-response, not per-listener-session. A server streaming a
multi-gigabyte body in the timeout window does not cause memory bloat: the ring
buffer caps at 4 KiB regardless of total body size.

## Security: secret-bearing surfaces

The URL, header values, and body template can carry secrets (bearer tokens,
signed payloads, embedded API keys). The dispatcher's `Debug` impl for the
trigger config **redacts** all three. Compiled templates use the markers below;
failed compilation uses `<bad-url-template-redacted>` or
`<bad-body-template-redacted>`. Headers appear only as a count. An absent body
template appears as `<default-notification-json>`.

Formatting example, expanded and wrapped for readability:

```text
WebhookConfig {
  url_template: <compiled-url-template-redacted>,
  method: Post,
  header_count: 2,
  body_template: <compiled-body-template-redacted>
}
```

This redaction applies to the trigger config's `Debug` representation, including
templates that failed to compile. It is not a guarantee for all error or log
content: template diagnostics can include expressions at DEBUG level, and a
receiver's response body is included in failure errors.

## TLS

The webhook reuses the supervisor's shared `reqwest::Client`. Any TLS
configuration (`--ca-bundle`, `--danger-accept-invalid-certs`) inherits
automatically.

## Examples

### Slack incoming webhook

```yaml
triggers:
  - type: webhook
    url: "{{ env.SLACK_WEBHOOK_URL }}"
    body_template: >-
      {"text": "aviso {{ notification.event_type }}
      sequence {{ notification.sequence }}"}
```

### Discord webhook

```yaml
triggers:
  - type: webhook
    url: "{{ env.DISCORD_WEBHOOK_URL }}"
    body_template: >-
      {"content": "aviso {{ notification.event_type }}
      sequence {{ notification.sequence }}"}
```

### Internal alerting service

```yaml
triggers:
  - type: webhook
    url: "https://alerts.internal/api/v1/incidents"
    method: POST
    headers:
      Authorization: "Bearer {{ env.ALERT_TOKEN }}"
      X-Source: "aviso-mars"
    body_template: |
      {
        "title": "Mars notification #{{ notification.sequence }}",
        "severity": "info",
        "tags": ["aviso", "mars", "{{ notification.identifier.class }}"]
      }
    retries: 3
    timeout: 10s
```

### GitHub Actions repository dispatch

```yaml
triggers:
  - type: webhook
    url: "https://api.github.com/repos/example/repo/dispatches"
    method: POST
    headers:
      Authorization: "Bearer {{ env.GITHUB_TOKEN }}"
      Accept: "application/vnd.github+json"
    body_template: |
      {
        "event_type": "aviso",
        "client_payload": {
          "sequence": {{ notification.sequence }},
          "event": "{{ notification.event_type }}"
        }
      }
```

## When to use

- Any HTTP endpoint without a kind-specific shortcut.
- Custom body shapes that don't match `teams` or `post`.
- Multi-step workflows where each step is a separate webhook.

## When NOT to use

- Microsoft Teams: use [`teams`](./teams.md) for the Adaptive Card auto-build.
- Generic CloudEvent forwarding: use [`post`](./post.md) for the server's
  verbatim CloudEvent.
- Heavy receivers that need timeouts > 30 s: increase `timeout:` explicitly (the
  default is 30 s).

</div>
