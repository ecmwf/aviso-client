<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Post trigger

<div class="trigger-guide">

HTTP POST per notification with the CloudEvent envelope from aviso-server as the
request body. Custom headers are supported. Use this when migrating from
pyaviso's `post` trigger or sending to any receiver that expects CloudEvents.

## YAML

```yaml
triggers:
  - type: post
    url: "https://receiver.example/aviso"            # required
    headers:                                          # optional
      Authorization: "Bearer {{ env.RECEIVER_TOKEN }}"
      X-Source: "aviso-listener"
    retries: 2                                        # optional, default 0
    required: true                                    # optional, default true
    timeout: 30s                                      # optional, default 30s
    fail_fast: true                                   # optional, default true
```

No `body_template` field: the body is **always** the CloudEvent envelope. For
arbitrary body shapes, use the [`webhook`](./webhook.md) trigger directly. No
`method` field: always POST.

## Body shape

When the notification comes from the live watch stream, the body is the
CloudEvent envelope aviso-server sent, including its server-side fields:

```json
{
  "specversion": "1.0",
  "type": "int.ecmwf.aviso.mars",
  "source": "https://aviso-server.ecmwf.int",
  "id": "mars@42",
  "time": "2026-05-23T22:28:52.384985528Z",
  "datacontenttype": "application/json",
  "dataschema": "https://aviso-server.ecmwf.int/schema/mars",
  "data": {
    "identifier": {
      "class": "od",
      "date": "20260601",
      ...
    },
    "payload": { ... }
  }
}
```

Fields worth knowing:

- **`type`** is per-event-type (`int.ecmwf.aviso.mars`,
  `int.ecmwf.aviso.dissemination`, etc.). Downstream routing logic that branches
  on `type` works correctly.
- **`time`** is the server's emission timestamp with nanosecond precision.
  Receivers can use it for ordering and latency measurement.
- **`source`** is the aviso-server URL. Receivers can identify which server
  (production vs staging) emitted the event.
- **`dataschema`** is a URL pointing at the schema definition; receivers can
  fetch it for validation.

aviso preserves these server-provided values instead of rebuilding the
CloudEvent locally. Whitespace and object-key order may differ because aviso
parses and writes the JSON again; field values stay the same. Use the
[`webhook`](./webhook.md) trigger with a hand-written `body_template` if
byte-for-byte output matters.

## Fallback body

A live `aviso listen` run forwards the server's CloudEvent envelope.
Notifications built directly in library tests or custom code may not carry that
envelope; in that case the post trigger falls back to a minimal CloudEvent body.

## Headers

| Header | Default | When |
|---|---|---|
| `Content-Type` | `application/cloudevents+json` | Auto-injected when the operator does not set their own |
| Operator-supplied headers | template-rendered at dispatch | As declared in YAML |

The auto-injected `Content-Type` matches the CloudEvents structured-mode content
type. Official CloudEvents SDKs can parse it directly.

If the operator sets `Content-Type` explicitly, that wins:

```yaml
triggers:
  - type: post
    url: "https://receiver.example/aviso"
    headers:
      Content-Type: "application/json"        # explicit, overrides the default
```

## Migrating from pyaviso

pyaviso's `post` trigger forwards the notification as a POST body. The aviso
(Rust) equivalent matches the same wire contract:

| pyaviso config | aviso YAML |
|---|---|
| `type: post` | `type: post` |
| `url: ...` | `url: "..."` |
| `headers: {...}` | `headers: {...}` |
| AWS-specific options (S3, SNS) | **not supported** |

For AWS-specific options, use a custom receiver or the [`webhook`](./webhook.md)
trigger with a hand-written body.

The dispatched body shape matches pyaviso's (CloudEvent envelope with
`specversion`, `type`, `source`, `id`, `time`, `data`). Downstream receivers
built for pyaviso work without changes.

## Examples

### Generic CloudEvent receiver

```yaml
triggers:
  - type: post
    url: "https://events.example.com/ingest"
    headers:
      X-Service-Auth: "{{ env.INGEST_SERVICE_TOKEN }}"
```

### Knative Eventing

```yaml
triggers:
  - type: post
    url: "http://broker-ingress.knative-eventing.svc.cluster.local/default/aviso"
    # Content-Type defaults to application/cloudevents+json which Knative consumes natively
```

### Internal CloudEvent-aware queue

```yaml
triggers:
  - type: post
    url: "https://events.internal/queue/aviso"
    headers:
      Authorization: "Bearer {{ env.QUEUE_TOKEN }}"
      X-Routing-Key: "aviso.{{ notification.event_type }}"   # custom routing on the receiver side
    retries: 5
    timeout: 10s
```

## Failure modes

Inherits all of webhook's error semantics:

- 4xx terminal under `fail_fast: true` (default)
- 5xx retryable
- Transport errors retryable
- Timeout retryable
- Template render errors terminal (only `url` and header values are templated;
  the body is not)

The webhook hint dispatcher fires for post trigger failures too, with the same
operator-facing wording.

## When to use

- pyaviso migration: matching wire shape.
- Generic CloudEvent receivers (Knative Eventing, ArgoEvents, CloudEvent-aware
  message brokers).
- Receivers that want the server's actual `type` / `source` / `time` /
  `dataschema` (not a reconstruction).

## When NOT to use

- Custom body shape needed: use [`webhook`](./webhook.md) with a hand-written
  `body_template`.
- Microsoft Teams: use [`teams`](./teams.md) for the Adaptive Card auto-build.
- Receivers expecting the notification in a different envelope (Slack message,
  Discord embed, PagerDuty event): use [`webhook`](./webhook.md).

</div>
