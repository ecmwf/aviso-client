# Troubleshooting

The most common things that surprise people. If you hit one of these, the fix is usually a one-liner.

## "no listeners to run"

```text
error: no listeners to run
```

You ran `aviso listen` without any listener configuration. Fix one of these:

- Pass a YAML file on the command line: `aviso listen my-listeners.yaml`.
- Add a `listeners:` block to your config file (`~/.config/aviso/config.yaml`).
- Use the inline form: `aviso listen --event mars --identifiers '{"class":"od"}'`.

## "admin wipe-all requires --yes"

```text
error: aviso admin wipe-all requires --yes
```

The destructive admin commands need an explicit `--yes` flag on the command line:

```bash
aviso admin wipe-all --yes
```

The flag cannot be set in the config file (a config-file `--yes` would defeat the safety).

## `--from 20260601` is treated as a sequence id, not a date

Pure-digit input is always a sequence id. To pass a date, use the dashed form:

```bash
aviso listen --from 2026-06-01
```

The full set of accepted `--from` formats is in [Configuration: `--from` value formats](./configuration.md#from-value-formats).

## TLS errors when connecting

```text
error: client error (Connect): invalid peer certificate: ...
```

Your aviso-server is behind a certificate aviso does not trust. Two paths:

- **The right path**: get the CA's PEM file from your server operator, then pass `--ca-bundle <PATH>` or set `tls.ca_bundle` in the config file. See [Configuration: trust an internal CA](./configuration.md#trust-an-internal-ca).
- **Last resort, dev only**: `--danger-accept-invalid-certs`. aviso logs a `WARN` at startup whenever this is set.

## A WARN about insecure TLS keeps firing

```text
... WARN cli.tls.insecure_mode: TLS certificate validation disabled by --danger-accept-invalid-certs ...
```

You have `--danger-accept-invalid-certs` set (or `tls.danger_accept_invalid_certs: true` in the config file). The warning is intentional: log scrapers can flag the situation in production. Switch to `--ca-bundle` to silence it.

## `aviso listen` runs forever, but I am pointing at a finite test stream

Expected. `aviso listen` is the live mode: it reconnects on close and waits for more events. To re-read history once and stop, use `aviso replay --from <cursor>`.

## A listener that worked before suddenly returns 401 after I rotated the token

aviso retries once after a 401 by asking the auth provider to refresh. For static credentials (`--token`, `AVISO_TOKEN`), refresh is a no-op: a second 401 surfaces as an error.

The fix: update the token in your env var or config file, then restart aviso.

## Token or password values appearing in logs

They should not be. aviso marks the `Authorization` header as sensitive and the `Debug` impl on every auth provider redacts the secret. If you see a credential in a log line, please file a bug.

## "state file format version 2 does not match supported version 1"

You upgraded the aviso binary and the state-file format has moved on. Two recovery paths:

- Install an aviso whose format matches the file (revert to the older version).
- Delete the state file and start fresh: `rm ~/.config/aviso/state.json ~/.config/aviso/state.json.lock`. You will start from "now" (or from `--from` if you pass one).

The full recovery story is at [State file](../reference/state-file.md).

## Multiple notifications delivered after a restart

Expected, by design. aviso guarantees **at-least-once** delivery: when a notification finishes processing (every required trigger ran successfully), the cursor advances. If aviso crashes between running a trigger and saving the cursor, the next run will redeliver that notification.

Triggers should be designed to be idempotent. For example, `kubectl annotate ... --overwrite` is idempotent; `mail -s subject ...` is not.

## A `command` trigger times out and leaves background processes behind

The dispatcher's `SIGKILL` reaches the `/bin/sh -c ...` child but not pipelines, backgrounded jobs, or grandchildren. Two patterns help:

- For a single binary, use `exec`: `command: "exec ./my-binary"`. The shell replaces itself with the binary, so the kill reaches it directly.
- For a pipeline, wrap in a script with a `trap` handler that cleans up child processes on signal.

## A `webhook` trigger keeps retrying despite a 4xx

It should not. 4xx responses are terminal by default (the receiver is rejecting the request; retrying with the same body will not change the answer). Check `fail_fast` in your YAML: if you set `fail_fast: false`, every failure becomes retryable.

## A `log` trigger fails with "No such file or directory"

The log trigger does not create directories. Make sure the parent exists:

```bash
mkdir -p /var/log/aviso
```

## How to file a useful issue

Every server response carries an `X-Request-ID` header. The same UUID appears in aviso's tracing events as `request_id`. Quote it when reporting an issue against aviso-server or aviso-client.

If you can, attach the output of `aviso config dump --redact` (which masks tokens and passwords). It is the fastest way for someone else to see what aviso thinks it is configured with.
