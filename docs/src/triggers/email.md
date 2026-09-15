# Sending email

<div class="trigger-guide">

aviso has no built-in email trigger. Email is one more per-notification side
effect, sent through the generic triggers: you point a trigger at your own SMTP
relay or mail endpoint and supply the credentials, and aviso runs the generic
client (it ships no managed-service email client).

Two paths, depending on what you have:

- The [command](./command.md) trigger driving a local SMTP client (`msmtp`,
  `sendmail`, `mailx`). Unix only, but the most direct SMTP route.
- The [webhook](./webhook.md) trigger posting to your mail gateway's HTTP API.
  Cross-platform.

Either way, **email is not idempotent**: at-least-once delivery means a
redelivered notification (for example after a crash before the cursor is
checkpointed) sends a second message. `required: false` keeps an email *failure*
from terminating the listener or forcing a retry-on-restart, but it does not
prevent duplicate sends. If duplicate emails are unacceptable, deduplicate on
`event_type@sequence`. See [Idempotency](./command.md#idempotency).

## With the command trigger (SMTP)

`msmtp` is a small self-contained SMTP client. Put the relay host, port, TLS,
and credentials in its own config (`~/.msmtprc`), so no secret lives in the
listener YAML, then template the message from the notification:

```yaml
listeners:
  - name: ops-email
    event: mars
    triggers:
      - type: command
        command: >
          printf 'From: %s\nTo: %s\nSubject: aviso %s #%s\n\n%s\n'
          "$MAIL_FROM" "$MAIL_TO" "$AVISO_EVENT_TYPE" "$AVISO_SEQUENCE" "$AVISO_NOTIFICATION_JSON"
          | msmtp --from="$MAIL_FROM" "$MAIL_TO"
        env:
          MAIL_FROM: aviso@example.com
          MAIL_TO: ops@example.com
        required: false        # email is non-idempotent; do not force redelivery
        timeout: 30s
        retries: 2             # transient SMTP failures are retried
```

The notification fields arrive as the `AVISO_*` environment variables the
command trigger injects: `AVISO_EVENT_TYPE`, `AVISO_SEQUENCE`,
`AVISO_NOTIFICATION_JSON`, and so on. See the
[command trigger reference](./command.md#environment-variables-injected-by-the-dispatcher)
for the full list. Keep credentials out of the YAML: `msmtp` reads them from its
own config, or
export them in the environment running aviso and reference them as `$VAR` in the
command (the child inherits the environment). If your host already has a
configured MTA, `printf ... | sendmail -t` or `mailx` works the same way.

## With the webhook trigger (HTTP)

If you reach mail through an HTTP endpoint (a self-hosted gateway, an internal
relay's REST API), use the webhook trigger with a `body_template` matching the
endpoint and the auth header from the environment:

```yaml
triggers:
  - type: webhook
    url: "https://mail-gateway.internal/send"
    headers:
      Authorization: "Bearer {{ env.MAIL_TOKEN }}"
      Content-Type: "application/json"
    body_template: >
      {"to": "ops@example.com",
       "subject": "aviso {{ notification.event_type }} #{{ notification.sequence }}",
       "text": "aviso {{ notification.event_type }} #{{ notification.sequence }} fired"}
    required: false
```

This is cross-platform and goes through the same
[template engine](./template-engine.md) as the other HTTP triggers. That engine
substitutes values verbatim and does not quote or JSON-escape them, so keep
substitutions in quoted, scalar positions (`event_type`, `sequence`) as above.
Avoid dropping a raw `{{ notification.payload }}` into the body: a string or
number payload renders unquoted and produces invalid JSON, while an object or
array payload stays valid JSON but changes the field's type and likely violates
the receiver's schema. If the body needs the full payload, send it through the
command trigger using `AVISO_PAYLOAD_JSON` / `AVISO_NOTIFICATION_JSON` instead.

## From code

These are ordinary triggers, so the same two recipes work from the library
builders, not just the CLI YAML: `Trigger::command(...)` and
`Trigger::webhook(...)` in Rust and the [C++ binding](../cpp/triggers.md), and
the equivalent trigger arguments in the Python package. The CLI YAML above is
just the declarative form of those builders.

</div>
