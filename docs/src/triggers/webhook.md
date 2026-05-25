# Webhook trigger

Generic HTTP request per notification. Operators get full control of method, URL, headers, and body via the [template engine](./template-engine.md). Use this trigger to forward notifications to any REST endpoint: Slack, Discord, PagerDuty, custom internal services, GitHub Actions, log aggregators, and so on.

## YAML

```yaml
triggers:
  - type: webhook
    url: "https://hooks.example.com/notify"          # required, template-rendered
    method: POST                                      # optional, default POST
    headers:                                          # optional
      Authorization: "Bearer {{ env.WEBHOOK_TOKEN }}"
      X-Source: "aviso-listener"
    body_template: '{"seq": {{ notification.sequence }}, "payload": {{ notification.payload }}}'  # optional
    timeout: 30s                                      # optional, default 30s
    retries: 2                                        # optional, default 0
    required: true                                    # optional, default true
    fail_fast: true                                   # optional, default true
```

## Method, URL, headers

- `method`: one of `GET`, `POST`, `PUT`, `PATCH`, `DELETE` (uppercase required). Default `POST`.
- `url`: template-rendered at dispatch time. Operators commonly use `{{ env.WEBHOOK_URL }}` to keep the URL out of the YAML.
- `headers`: header NAMES are taken literally; header VALUES are template-rendered. The YAML `headers` block is a map so each header name appears once; multi-value headers (e.g. multiple `Set-Cookie`) are not representable via the YAML config. Operators needing repeated header names should use the lib's `Trigger::webhook(...).header(name, value)` programmatic builder, which is repeatable.

The dispatcher auto-injects `Content-Type: application/json` when the operator does not supply one. If you set your own `Content-Type`, the dispatcher does NOT override it.

## Body template

When `body_template` is **absent**, the body defaults to the notification serialised as compact JSON (matching the [echo](./echo.md) trigger's pipe-mode shape):

```text
{"event_type":"mars","sequence":42,"identifier":{...},"payload":{...}}
```

When `body_template` is **set**, that string is template-rendered at dispatch. Two patterns are common:

**Forward selected fields**:

```yaml
body_template: '{"who": "{{ notification.identifier.class }}", "what": "{{ notification.event_type }}", "when": "{{ notification.identifier.date }}"}'
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

When embedding JSON-typed notification fields (`identifier`, `payload`) inside a JSON string literal, see [Template engine: value rendering](./template-engine.md#value-rendering-rules) for the escaping rules - short version, embed them OUTSIDE a string field (as in the example above), not inside `"text": "..."`.

## Retry classifier

| Outcome | `fail_fast: true` (default) | `fail_fast: false` |
|---|---|---|
| 2xx response | success | success |
| 4xx response | **terminal** (no retry) | retryable |
| 5xx response | retryable | retryable |
| Transport error (DNS, TCP, TLS, mid-stream interrupt) | retryable | retryable |
| Timeout | retryable | retryable |
| HTTP client refused the rendered request (invalid URL, bad header) | **terminal** | retryable (every failure is retryable when `fail_fast` is off, even ones that are deterministic against the same notification) |
| Template render error | **terminal** | retryable (same caveat: deterministic failures will fail identically on retry, but the dispatcher honours the retry budget) |

Terminal failures bypass the `retries` budget. Retryable failures are retried up to `retries + 1` total attempts with the supervisor's exponential backoff (250 ms base, doubling, 30 s cap, full jitter).

## Response body capture

The response body is captured into a 4 KiB ring buffer via streaming chunks. Operators see the tail of the response on a failure error. This is essential for debugging 4xx responses from services that include validation details in the body.

Example failure surface for a 4xx:

```text
Error in listener my-listener: trigger webhook failed: webhook: status=400 body_tail={"error":"missing_required_field","field":"alert.title"}
  Hint: webhook returned 4xx (400 Bad Request) which is TERMINAL (no retries) per the dispatcher contract: 4xx means the receiver rejected the request, retrying with the same notification will fail identically. Check the webhook URL, headers, and body_template; the receiver's response body is included above and may name the specific field that failed validation.
  Other listeners continue.
```

The 4 KiB cap is per-response, not per-listener-session. A server streaming a multi-gigabyte body in the timeout window does not cause memory bloat: the ring buffer caps at 4 KiB regardless of total body size.

## Security: secret-bearing surfaces

The URL, header values, and body template can carry secrets (bearer tokens, signed payloads, embedded API keys). The dispatcher's `Debug` impl for the trigger config **redacts** all three:

```text
Trigger { kind: Webhook(WebhookConfig { url_template: <compiled-url-template-redacted>, method: Post, header_count: 2, body_template: <compiled-body-template-redacted> }), ... }
```

So a `Debug`-formatted error chain through trigger configs won't leak secrets, even when a template fails to compile. The raw URL / header values / body templates surface only at DEBUG-level tracing (which is off by default).

## TLS

The webhook reuses the supervisor's shared `reqwest::Client`. Any TLS configuration (`--ca-bundle`, `--danger-accept-invalid-certs`) inherits automatically.

## Examples

### Slack incoming webhook

```yaml
triggers:
  - type: webhook
    url: "{{ env.SLACK_WEBHOOK_URL }}"
    body_template: '{"text": "aviso {{ notification.event_type }} sequence {{ notification.sequence }}"}'
```

### Discord webhook

```yaml
triggers:
  - type: webhook
    url: "{{ env.DISCORD_WEBHOOK_URL }}"
    body_template: '{"content": "aviso {{ notification.event_type }} sequence {{ notification.sequence }}"}'
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
    body_template: '{"event_type": "aviso", "client_payload": {"sequence": {{ notification.sequence }}, "event": "{{ notification.event_type }}"}}'
```

## When to use

- Any HTTP endpoint without a kind-specific shortcut.
- Custom body shapes that don't match `teams` or `post`.
- Multi-step workflows where each step is a separate webhook.

## When NOT to use

- Microsoft Teams: use [`teams`](./teams.md) for the Adaptive Card auto-build.
- Generic CloudEvent forwarding: use [`post`](./post.md) for the server's verbatim CloudEvent.
- Heavy receivers that need timeouts > 30 s: increase `timeout:` explicitly (the default is 30 s).
