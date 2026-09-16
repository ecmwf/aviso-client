# Troubleshooting

Choose the symptom that matches what you see. Open a panel for checks and links
to the relevant reference.

<div class="troubleshooting">

<details>
<summary id="server-url-or-connection-errors">Server URL or connection errors</summary>

If aviso says `base_url is required`, supply the server URL through
`--base-url`, `AVISO_BASE_URL`, or `base_url` in your config file. Flags override
environment variables, which override the file. See
[Configuration](./configuration.md#tell-aviso-where-the-server-is).

For connection failures, check the URL and whether the server is reachable from
your machine. Confirm the hostname, port, and HTTP or HTTPS scheme with your
server operator. For certificate errors, see
[TLS errors](#tls-errors-when-connecting).

If `Connecting` never becomes `Listening`, the server has not confirmed an Aviso
stream. HTTP 200 alone is not enough. A missing or incorrect
`Content-Type: text/event-stream`, an unexpected opening event, or no opening
confirmation within ten seconds stops the listener with a protocol error.
Check for a website URL, a login redirect, or a proxy routing the request to the
wrong service. Rejected HTTP 200 bodies are not printed.

For HTTP failures, `listen` omits unrecognized response bodies, including JSON
objects from proxies. Recognized Aviso error codes with string messages or
details retain only those fields, a request ID, and the configured event types
when relevant. Other fields are omitted. URL-like whitespace-delimited tokens
in retained text and listener hints are replaced with `[URL omitted]`, including
their userinfo, path, query, and fragment. This does not detect arbitrary
secrets in ordinary message text. Library callers can still inspect the raw
error body.

`Retrying listener connection` means a recoverable failure, such as a connection
refusal, HTTP 429, or HTTP 5xx. The default initial budget is 30 seconds across
retries. See
[Listener startup timeout](./configuration.md#listener-startup-timeout)
to change or disable that budget. Once `Listening` appears, startup has
finished; an idle stream with heartbeats does not need to deliver data to stay
healthy.

</details>

<details>
<summary id="authentication-errors">Authentication fails with 401 or 403</summary>

A 401 means credentials are missing, invalid, or expired. Check the credentials
selected by your flags, environment variables, and config file. A 403 can mean
your credentials were accepted but lack permission for the operation or event
type. Ask your server operator to check access. See
[Authentication](./configuration.md#authentication).

## A listener that worked before suddenly returns 401 after I rotated the token

aviso retries once after a 401 by asking the auth provider to refresh. For
static credentials (`--token`, `AVISO_TOKEN`), refresh is a no-op: a second 401
surfaces as an error.

The fix: update the token in your env var or config file, then restart aviso.

## Token or password values appearing in logs

They should not be. aviso marks the `Authorization` header as sensitive and the
`Debug` impl on every auth provider redacts the secret. If you see a credential
in a log line, please file a bug.

</details>

<details>
<summary id="no-events">The listener is running, but no events arrive</summary>

Check that the event type and identifier filters match the notifications you
expect. A listener without a saved cursor or a starting point waits for new
notifications; it does not read all existing history. A saved cursor resumes
after the last committed notification. See
[Resume and state](../concepts/resume-and-state.md).

## `aviso listen` runs forever, but I am pointing at a finite test stream

Expected. `aviso listen` is the live mode: it reconnects on close and waits for
more events. To re-read history once and stop, use
`aviso replay --from <cursor>`.

## `--from 20260601` is treated as a sequence id, not a date

Pure-digit input is always a sequence id. To pass a date, use the dashed form:

```bash
aviso listen --from 2026-06-01
```

The full set of accepted `--from` formats is in
[Configuration: `--from` value formats](./configuration.md#from-value-formats).

</details>

<details>
<summary id="invalid-request-or-filters">The server rejects the request or identifier filters</summary>

Read the server's error message for the field it rejected. For listen and
replay, identifiers marked `required: true` in the server's schema must be
supplied. Omitting a `required: false` identifier makes it a wildcard. Supplied
values must still satisfy the schema's type and constraints.

Publishing is different: every identifier in the schema must be supplied, even
those marked `required: false`. See
[Publish and listen](./publish-and-listen.md) and the
[Listener YAML reference](../reference/listener-yaml.md).

</details>

<details>
<summary id="schema-or-event-type-errors">The event type is unknown or the schema is unexpected</summary>

Event types and identifier rules come from the server you connect to. Check the
server URL and the spelling of your event type against that server's schema.
An example event type is not guaranteed to exist on your server. See
[Schema discovery](./operations.md) for how to inspect the available types and
their identifiers.

</details>

<details>
<summary id="configuration-or-state-errors">Listener configuration, saved state, or duplicates after restart</summary>

## "no listeners to run"

```text
error: no listeners to run
```

You ran `aviso listen` without any listener configuration. Fix one of these:

- Pass a YAML file on the command line: `aviso listen my-listeners.yaml`.
- Add a `listeners:` block to your config file (`~/.config/aviso/config.yaml`).
- Use the inline form:
  `aviso listen --event mars --identifiers '{"class":"od"}'`.

## "state file format version 2 does not match supported version 1"

The file uses format version 2, but this binary supports version 1. Two recovery
paths:

- Install an aviso whose format matches the file.
- Stop all clients using the state file, then delete it and start fresh:
  `rm ~/.config/aviso/state.json ~/.config/aviso/state.json.lock`. You will
  start from "now" (or from `--from` if you pass one).

The full recovery story is at [State file](../reference/state-file.md).

## Multiple notifications delivered after a restart

Expected, by design. aviso guarantees **at-least-once** delivery: when a
notification finishes processing (every required trigger ran successfully), the
cursor advances. If aviso crashes between running a trigger and saving the
cursor, the next run will redeliver that notification.

Triggers should be designed to be idempotent. For example,
`kubectl annotate ... --overwrite` is idempotent; `mail -s subject ...` is not.

</details>

<details>
<summary id="tls-errors">TLS certificate errors or insecure-mode warnings</summary>

## TLS errors when connecting

```text
error: client error (Connect): invalid peer certificate: ...
```

Your aviso-server is behind a certificate aviso does not trust. Two paths:

- **The right path**: get the CA's PEM file from your server operator, then pass
  `--ca-bundle <PATH>` or set `tls.ca_bundle` in the config file. See
  [Configuration: trust an internal CA](./configuration.md#trust-an-internal-ca).
- **Last resort, dev only**: `--danger-accept-invalid-certs`. aviso logs a
  `WARN` at startup whenever this is set.

## A WARN about insecure TLS keeps firing

```text
... WARN cli.tls.insecure_mode: TLS certificate validation disabled by --danger-accept-invalid-certs ...
```

You have `--danger-accept-invalid-certs` set (or
`tls.danger_accept_invalid_certs: true` in the config file). The warning is
intentional: log scrapers can flag the situation in production. Remove the
insecure setting and use `--ca-bundle` to trust your CA instead.

</details>

<details>
<summary id="command-trigger-errors">A command trigger times out or leaves processes behind</summary>

## A `command` trigger times out and leaves background processes behind

The dispatcher's `SIGKILL` reaches the `/bin/sh -c ...` child but not pipelines,
backgrounded jobs, or grandchildren.

- For a single binary, use `exec`: `command: "exec ./my-binary"`. The shell
  replaces itself with the binary, so the kill reaches it directly.
- A shell `trap` cannot catch `SIGKILL`. Pipelines and background jobs need
  process-tree cleanup outside the dispatcher's timeout handling.

</details>

<details>
<summary id="webhook-or-log-trigger-errors">A webhook keeps retrying or a log file cannot be opened</summary>

## A `webhook` trigger keeps retrying despite a 4xx

It should not. 4xx responses are terminal by default (the receiver is rejecting
the request; retrying with the same body will not change the answer). Check
`fail_fast` in your YAML: if you set `fail_fast: false`, every failure becomes
retryable.

## A `log` trigger fails with "No such file or directory"

The log trigger does not create directories. Make sure the parent exists:

```bash
mkdir -p /var/log/aviso
```

</details>

<details>
<summary id="admin-command-errors">An admin command requires confirmation</summary>

## "admin wipe-all requires --yes"

```text
error: aviso admin wipe-all requires --yes
```

The destructive admin commands need an explicit `--yes` flag on the command
line:

```bash
aviso admin wipe-all --yes
```

The flag cannot be set in the config file (a config-file `--yes` would defeat
the safety).

</details>

</div>

## How to file a useful issue

Every server response carries an `X-Request-ID` header. The same UUID appears in
aviso's tracing events as `request_id`. Quote it when reporting an issue against
aviso-server or aviso-client.

If you can, attach the output of `aviso config dump --redact` (which masks
tokens and passwords). It is the fastest way for someone else to see what aviso
thinks it is configured with.
