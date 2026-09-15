# Teams trigger

<div class="trigger-guide">

Auto-builds a Microsoft Teams Adaptive Card from the notification and POSTs it
to a Teams Workflows endpoint. Shortcut over the [webhook](./webhook.md)
trigger; saves operators ~30 lines of Adaptive Card boilerplate per listener.

## YAML

```yaml
triggers:
  - type: teams
    url: "{{ env.TEAMS_WEBHOOK_URL }}"               # required
    title_template: "Custom title {{ notification.event_type }}"  # optional
    retries: 2                                        # optional, default 0
    required: true                                    # optional, default true
    timeout: 30s                                      # optional, default 30s
    fail_fast: true                                   # optional, default true
```

## Setting up the Teams webhook

Microsoft is deprecating the legacy "Incoming Webhook" connector; the current
recommendation is **Workflows** (Power Automate).

1. Open the Teams channel where notifications should arrive.
2. Click the `⋯` (More options) next to the channel name.
3. **Workflows** → search "Post to a channel when a webhook request is
   received".
4. Connect to your Teams and select the channel.
5. Click **Add workflow**. Teams shows the HTTPS URL once; save it carefully.

The URL looks like:

```text
https://prod-XX.YYY.logic.azure.com:443/workflows/.../triggers/manual/paths/invoke?api-version=1&sp=...&sv=1.0&sig=...
```

The `sig=` query parameter is a SAS token. Treat the entire URL as a secret:
anyone with it can post to your channel.

## Card body

The Adaptive Card aviso builds for each notification has three sections:

1. **Title TextBlock** - rendered from `title_template`. Default:
   `aviso {{ notification.event_type }} #{{ notification.sequence }}`.
2. **Identifier FactSet** - one row per identifier field plus `Event` and
   `Sequence` rows (auto-generated from the notification's runtime data).
3. **Payload section** - when the payload is an object, another FactSet with one
   row per payload key; when scalar/array, a monospace TextBlock with the JSON;
   when `null`, the section is omitted.

Illustrative card content for a `mars` event with payload
`{"seed": "x", "count": 5}`. Teams controls the card's final appearance:

```text
aviso mars #42

Event:    mars
Sequence: 42
class:    od
date:     20260601
domain:   g
...

Payload
seed:  x
count: 5
```

The card is built programmatically at dispatch time using the notification's
runtime data, so it correctly handles any identifier field set or payload shape.
Operators don't need to write the Adaptive Card JSON themselves.

## Title template

The `title_template` is rendered via the [template engine](./template-engine.md)
at dispatch:

```yaml
# Default
title_template: "aviso {{ notification.event_type }} #{{ notification.sequence }}"

# Custom with identifier fields
title_template: "ECMWF {{ notification.event_type }} step={{ notification.identifier.step }}"

# With env-var prefix for environment tagging
title_template: "[{{ env.DEPLOYMENT_TIER }}] aviso {{ notification.event_type }}"
```

JSON-special characters in the title (`"`, `\`) are correctly escaped before
embedding in the card; operators don't need to escape them manually.

## Securing the URL

Webhook URLs are secrets. Two safe patterns:

**Env var (recommended)**:

```yaml
triggers:
  - type: teams
    url: "{{ env.TEAMS_WEBHOOK_URL }}"
```

Then:

```bash
export TEAMS_WEBHOOK_URL='https://prod-XX.../workflow'
aviso listen my-listeners.yaml
```

**Gitignored separate listener file**: keep the listener YAML with the URL
outside the repo, or in a `.gitignore`'d directory; the CLI accepts the path as
a positional argument.

## How it relates to other triggers

Internally, the Teams trigger is sugar over [`webhook`](./webhook.md). The
dispatch flow:

1. Render the title via the template engine.
2. Build the Adaptive Card body programmatically (using the notification's
   identifier and payload).
3. Synthesise a webhook config with the body pre-rendered, method `POST`,
   `Content-Type: application/json`.
4. Delegate to the webhook dispatcher.

Operators wanting full control of the card shape (extra sections, custom colors,
action buttons) should use the [webhook](./webhook.md) trigger directly with a
hand-written `body_template`. The Teams trigger is the shortcut for the common
case; the webhook trigger is the way to express anything else.

## Failure modes

Inherits all of webhook's error semantics. Common Teams-specific issues:

| Symptom | Cause | Fix |
|---|---|---|
| HTTP 400 | Body shape mismatch | Check the schema |
| HTTP 401/403 | Expired token or deleted workflow | Regenerate the URL |
| HTTP 202, no card | Downstream flow error | Check run history |
| Plain-text card | Wrong output schema | Use Adaptive Card |

- **HTTP 400:** a customised Workflow may expect a different body schema. Use
  the default Adaptive Card shape or switch to `webhook` with a hand-written
  body.
- **HTTP 401/403:** regenerate the URL in Teams and update the environment
  variable. The URL's SAS token may have expired, or the workflow was deleted.
- **HTTP 202 with no card:** the Workflow accepted the request, but a later
  step failed. Open its run history in Power Automate to inspect the error.
- **Plain-text card:** check that the receiver Workflow's response schema
  uses Adaptive Card output.

## When to use

- Microsoft Teams channels via Workflows / Power Automate.
- Minimal YAML config (URL + optional title).
- Don't want to hand-write Adaptive Card JSON.

## When NOT to use

- You need custom Adaptive Card features (action buttons, custom colors,
  multiple FactSets, images, conditional sections): use
  [`webhook`](./webhook.md) with a hand-written `body_template`.
- The legacy "Incoming Webhook" connector instead of Workflows: the body shape
  is different (legacy uses `MessageCard`, Workflows uses Adaptive Card). For
  legacy, use [`webhook`](./webhook.md) with the `MessageCard` body. Microsoft
  is deprecating legacy connectors anyway.
- Non-Teams receivers: use [`webhook`](./webhook.md).

</div>
