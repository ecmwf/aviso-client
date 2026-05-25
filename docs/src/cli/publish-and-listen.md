# Publish and listen

The two commands you will use most: `aviso notify` to send notifications, `aviso listen` to receive them.

## Publish {#publish}

`aviso notify` sends one notification to the server.

```bash
aviso notify 'event=mars,class=od,stream=oper,date=20260601,domain=g,expver=0001,step=0,time=1200,data={"location":"s3://bucket/path"}'
```

The single argument is a comma-separated list:

- `event=<TYPE>` is required. It names the event type.
- `data=<JSON>` is optional. Whatever you set here becomes the notification's payload.
- Every other `key=value` pair lands in the identifier map.

### Quoting values that contain commas

For a value that itself contains commas (a polygon, a comma-separated list), wrap it in double quotes:

```bash
aviso notify 'event=test_polygon,polygon="46,8,46,9,47,9,47,8,46,8",date=20260601,time=1200'
```

The quotes are CLI-side; they are stripped before the value is sent to the server.

### Identifier fields the server requires

Every event type's schema declares which identifier fields the server insists on. If you omit one, the server rejects the notification with a helpful error.

To see what fields a schema asks for:

```bash
aviso schema get mars
```

### What you see on success

When `aviso notify` succeeds, it prints the server's response. In your terminal you get a human-readable line; piped to a file or another command, you get one line of compact JSON.

## Listen {#listen}

`aviso listen` opens a long-lived connection to the server and prints (or trigger-handles) each matching notification as it arrives.

There are two ways to set up a listener: a YAML file (for anything you care about), and inline flags (for quick exploration).

### Listen with inline flags

```bash
aviso listen --event mars --identifiers '{"class":"od"}'
```

This runs one listener with a single echo trigger. Press Ctrl+C to stop.

`--event` and `--identifiers` come as a pair: pass both or pass neither. `--identifiers` takes a JSON object literal (`'{"key":"value"}'`).

For an empty identifier map (every notification of this event type), pass `'{}'`. The server may still require certain fields to be present, depending on the schema.

The inline mode runs with one default trigger: `echo`. For any other trigger, use a YAML file.

### Listen with a YAML file {#listen-with-a-yaml-file}

Write the listeners you want once, and run them on demand or under systemd:

```yaml
# my-listeners.yaml
listeners:
  - name: mars-od
    event: mars
    identifiers:
      class: od
    triggers:
      - type: log
        path: /var/log/aviso/mars-od.log
      - type: webhook
        url: "{{ env.WEBHOOK_URL }}"
        headers:
          Authorization: "Bearer {{ env.WEBHOOK_TOKEN }}"

  - name: cosmo-fc
    event: cosmo
    identifiers:
      class: od
      type: fc
    triggers:
      - type: command
        command: "./on-cosmo.sh {{ notification.identifier.step }}"
```

Run them:

```bash
aviso listen my-listeners.yaml
```

The CLI spawns one task per listener and runs them concurrently. Each listener resumes independently from the [state file](../reference/state-file.md).

A full reference for the file format is at [Listener YAML](../reference/listener-yaml.md).

### Triggers, briefly

A trigger is the action aviso takes for each matching notification. Six kinds are built in:

- [`echo`](../triggers/echo.md) prints the notification.
- [`log`](../triggers/log.md) appends it to a file.
- [`command`](../triggers/command.md) runs a shell command (Unix only).
- [`webhook`](../triggers/webhook.md) makes an HTTP request to a URL of your choice.
- [`teams`](../triggers/teams.md) posts to a Microsoft Teams channel.
- [`post`](../triggers/post.md) forwards the original event envelope.

You can attach as many triggers as you want to a listener. They run in declaration order.

### What `aviso listen` prints

On a TTY, the echo trigger prints a multi-line pretty JSON block per notification with a one-line header. When the output is piped to a file or another command, it switches to one compact JSON object per line, so `aviso listen | jq` and `aviso listen >> notifications.ndjson` both work.

Other triggers write to their own destinations (a file, a webhook, a shell command); the CLI itself stays quiet on stdout for those.

### Stopping

Ctrl+C drains in-flight work and exits. A second Ctrl+C within five seconds exits immediately (return code 130).

### Resuming

When you stop and restart `aviso listen` against the same server and identifiers, it resumes from the last notification it fully processed. The cursor lives in `~/.config/aviso/state.json` by default. For a one-off run that does not write to the file, pass `--no-state-store`.

To start from an explicit point (a sequence id or a date) just for this run:

```bash
aviso listen my-listeners.yaml --from 2026-05-01
aviso listen my-listeners.yaml --from 1000
```

The full rules for `--from` (pure-digit input is always a sequence id, dashes mean a date, and so on) live in [Configuration](./configuration.md#from-value-formats).

### Listening for several event types at once

Put multiple listeners in the same YAML file (or in separate files; the CLI accepts a list):

```bash
aviso listen mars.yaml cosmo.yaml
```

Each listener has its own connection, its own resume cursor, and its own triggers. A failure in one does not stop the others.

## What next

- [Replay history](./replay.md): re-read past notifications.
- [Configuration](./configuration.md): config files, environment variables, TLS.
- [Triggers overview](../triggers/overview.md): pick the right trigger.
- [Troubleshooting](./troubleshooting.md): the usual snags.
