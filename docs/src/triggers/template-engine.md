<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Template engine

<div class="trigger-guide">

The shared template engine that `command`, `webhook`, `teams`, and `post`
triggers use to substitute notification fields and environment variables into
their templated inputs.

## Syntax

Two expression forms inside `{{ ... }}`:

```text
{{ notification.<dotted.path> }}    -- substitutes a notification field
{{ env.<NAME> }}                    -- substitutes a process environment variable
```

Literal `{{` is escaped as `\{{`. Everything outside `{{ ... }}` is passed
through verbatim.

## Notification paths

The `notification` namespace walks the notification's serialised JSON tree.
Empty path means the whole notification:

| Expression | Resolves to |
|---|---|
| `{{ notification }}` | The whole notification as compact JSON: `{"event_type":"mars","sequence":42,...}` |
| `{{ notification.event_type }}` | The event_type string, **unquoted**: `mars` |
| `{{ notification.sequence }}` | The sequence as a numeric-string: `42` |
| `{{ notification.identifier }}` | The identifier object as compact JSON: `{"class":"od","date":"20260601",...}` |
| `{{ notification.identifier.class }}` | A specific identifier value, unquoted: `od` |
| `{{ notification.payload }}` | The payload as compact JSON: `{"seed":"x"}` (or `null`) |
| `{{ notification.payload.seed }}` | A specific payload field, unquoted: `x` |

Paths can go arbitrarily deep into nested objects. Array indexing is not
supported (the engine walks object keys only).

## Env paths

```yaml
url: "{{ env.WEBHOOK_URL }}"
body_template: '{"token": "{{ env.SECRET }}"}'
```

`{{ env.<NAME> }}` reads `std::env::var(NAME)`. Two failure modes:

- **Variable not set**: `TemplateErrorKind::EnvNotSet`. Operator must
  `export NAME=value` before running.
- **Variable set but not UTF-8**: `TemplateErrorKind::EnvNotUnicode`. Rare;
  usually means a misconfigured deployment.

Both surface as `TriggerError::Template` at first dispatch (the constructor is
infallible; errors are deferred to render time).

## Value rendering rules

How each JSON value type renders into the template's output:

| JSON type | Rendered as | Example |
|---|---|---|
| String | The string contents, **unquoted** | `od` (NOT `"od"`) |
| Number | Decimal representation, **unquoted** | `42`, `3.14`, `-7` |
| Boolean | `true` or `false` | `true` |
| Null | Literal four-character string `null` | `null` |
| Object | Compact JSON, **including the surrounding braces** | `{"class":"od","date":"20260601"}` |
| Array | Compact JSON | `["a","b","c"]` |

These are the values as rendered for a JSON body or a header. When the text is
a shell command or a URL, each notification value is also neutralised for that
use; see [Where the text goes](#where-the-text-goes).

The string-unquoted rule is the load-bearing one for safe embedding in JSON
bodies. Compare:

```text
"value": "{{ notification.identifier.class }}"
                ↓ renders ↓
"value": "od"                              ← valid JSON; the surrounding quotes ARE the JSON string delimiters
```

vs. trying to embed an object value inside a JSON string:

```text
"value": "{{ notification.identifier }}"
                ↓ renders ↓
"value": "{"class":"od","date":"20260601"}"   ← INVALID JSON; inner quotes break the string
```

For object/array values, embed them **outside** a JSON string field:

```text
"identifier": {{ notification.identifier }}
                ↓ renders ↓
"identifier": {"class":"od","date":"20260601"}   ← valid JSON; object is a JSON value, not a string
```

## Where the text goes

Notification values are written by the publisher, not by you. Each place a
rendered template is used has its own idea of which characters are special, so
the engine neutralises every `{{ notification.* }}` value for that place:

| Rendered text is | Notification values are |
|---|---|
| a `command:` string | quoted for the shell context they land in: wrapped in single quotes when bare, `'` escaped inside single quotes, and backslash, `$`, backtick and `"` escaped inside double quotes. The shell reads the value as one literal argument. This covers a placeholder that is part of a command word; the [Command trigger](./command.md#notification-values-are-data-never-code) page lists the places it does not cover. |
| a webhook `url:` | percent-encoded (RFC 3986 unreserved characters kept), so a value cannot add a path segment or a query parameter. A notification placeholder in the scheme or authority, where the value would be the host, is refused with `ValueInUrlAuthority`. Put the host in the template or in an `{{ env.* }}` value and notification values in the path, query or fragment. |
| a header value or a body | inserted as written. You supply the encoding around the value, for example the quotes of a JSON string. |

`{{ env.* }}` values are your own and are inserted as written everywhere.

## Common patterns

### Pass-through scalar identifier values

```yaml
body_template: '{"who": "{{ notification.identifier.class }}", "what": "{{ notification.event_type }}"}'
```

Renders to: `{"who": "od", "what": "mars"}`

### Pass-through whole identifier as nested object

```yaml
body_template: '{"id": {{ notification.identifier }}, "seq": {{ notification.sequence }}}'
```

Renders to: `{"id": {"class":"od","date":"20260601",...}, "seq": 42}`

### Pass-through whole notification

```yaml
body_template: '{{ notification }}'
```

Renders to:
`{"event_type":"mars","sequence":42,"identifier":{...},"payload":{...}}`

This is the default body when `body_template:` is omitted on the
[webhook](./webhook.md) trigger. The [echo](./echo.md) trigger emits the same
shape in pipe mode.

### Secret-bearing URL

```yaml
url: "{{ env.WEBHOOK_URL }}"
```

The URL never appears in YAML, in logs, or in error messages. Set the env var in
your deployment:

```bash
export WEBHOOK_URL='https://hooks.example.com/notify?token=xxx'
```

### Conditional content via env tagging

```yaml
title_template: "[{{ env.DEPLOYMENT_TIER }}] aviso {{ notification.event_type }}"
```

`{{ env.DEPLOYMENT_TIER }}` can be `prod`, `staging`, `dev`, etc., setting
different prefixes per deployment.

## Error categories

Template errors fall into one of six `TemplateErrorKind` values, surfaced via
`TriggerError::Template { context, field, kind }`:

| Kind | When | Example template |
|---|---|---|
| `Missing` | A `{{ notification.<path> }}` resolved to nothing | `{{ notification.identifier.nonexistent }}` when the notification has no `nonexistent` field |
| `EnvNotSet` | A `{{ env.<NAME> }}` variable is not in the process environment | `{{ env.UNDEFINED_VAR }}` when `UNDEFINED_VAR` is not exported |
| `EnvNotUnicode` | A `{{ env.<NAME> }}` variable's value is not valid UTF-8 | (rare; usually a misconfigured deployment) |
| `BadSyntax` | Template parse failure: unclosed `{{`, empty path segment, unknown namespace | `{{ unclosed`, `{{ notification..empty }}`, `{{ unknown.foo }}` |
| `ValueInUrlAuthority` | A `{{ notification.<path> }}` in the scheme or authority of a webhook URL | `https://{{ notification.payload.host }}/hook` |
| `NotificationEncode` | Notification could not be serialised to JSON | Practically unreachable |

`NotificationEncode` occurs when resolving a path. It is practically
unreachable for well-typed notifications. The variant points the diagnosis at
the notification rather than a missing-path template bug.

All six are **terminal** under `fail_fast: true` (the default): retrying with
the same notification and environment will produce the same template error.

The `context` carried on `TriggerError::Template` is the safe static label
(`"webhook url"`, `"command"`, `"teams title"`, etc.) of where the failure
occurred. `field` carries the JSON path / env-var name / parse-failure category
(whichever applies to the kind). Neither echoes the raw template, which may
carry secrets.

## What the template engine does NOT support

- **Conditionals** - no `{% if %}` / `{% else %}`. Use Rust code outside aviso
  if you need branching.
- **Loops** - no `{% for %}` over object keys or array elements. Templates
  render specific paths; the [teams](./teams.md) and [post](./post.md) triggers
  iterate identifier fields at dispatch time via Rust code (not via the template
  engine).
- **Filters / pipes** - no `{{ value | uppercase }}` or `{{ value | json }}`.
  Escaping for the shell and for URLs is applied by the engine according to
  where the text goes, so no filter is needed for that. Operators wanting other
  transformations should do them in the receiver.
- **Macros / includes** - templates are flat strings; no recursion.

This is intentional: a more featureful template engine adds attack surface and
complexity for marginal value. Operators wanting full programmability should use
the [`command`](./command.md) trigger (which runs arbitrary shell code) or
process the notification downstream.

</div>
