# Publish and listen

The two commands you will use most: `aviso notify` to send notifications,
`aviso listen` to receive them.

## Publish {#publish}

`aviso notify` sends one notification to the server.

```bash
aviso notify 'event=mars,class=od,stream=oper,date=20260601,domain=g,expver=0001,step=0,time=1200,data={"location":"s3://bucket/path"}'
```

The single argument is a comma-separated list:

- `event=<TYPE>` is required. It names the event type.
- `data=<JSON>` is optional. Whatever you set here becomes the notification's
  payload.
- Every other `key=value` pair lands in the identifier map.
- Use `key:=JSON` when an identifier must be a JSON scalar rather than a string.

Identifier values beginning with `[` or `{` are parsed as JSON. This sends a
point cloud as an array rather than a quoted JSON string:

```bash
aviso notify 'event=observations,point_cloud=[[46,8],[47,9]],date=20260601,data={"source":"stations"}'
```

A point has shape `[latitude,longitude]`. A polygon and a point cloud both use
`[[latitude,longitude],...]`. Polygons need at least four pairs, with the first
pair repeated last. Clouds do not need a closing repeat.
The outer single quotes protect the argument from the shell. Do not add double
quotes around the array.

Bare scalar values keep the CLI's existing string behavior. For example,
`step=12` and `enabled=true` send the strings `"12"` and `"true"`.
Use the explicit JSON delimiter to retain scalar types:

```bash
aviso notify 'event=observations,count:=12,enabled:=true,missing:=null'
```

This sends the number `12`, the boolean `true`, and JSON `null`. The explicit
form also accepts strings, arrays, and objects, such as `label:="archive"`,
`point:=[46,8]`, or `area:={"north":47,"south":46}`.
If a string itself starts with `[` or `{`, wrap that value in double quotes:

```bash
aviso notify 'event=mars,label="[archive]",region="{region}"'
```

The outer single quotes are interpreted by the shell. The inner double quotes
are interpreted and removed by aviso. The values sent are the strings
`"[archive]"` and `"{region}"`, not malformed JSON structures.

### Alternative coordinate format

For a value that itself contains commas (a polygon, a comma-separated list),
wrap it in double quotes:

```bash
aviso notify 'event=test_polygon,polygon="46,8,46,9,47,9,47,8,46,8",date=20260601,time=1200'
```

The quotes are CLI-side; they are stripped before the value is sent to the
server. The HTTP API also accepts point strings such as `"46,8"` in watch and
replay filters. Point clouds have no string format. Prefer arrays for spatial
values; CloudEvent spatial identifiers are always arrays.

### Identifier fields the server requires

Every event type's schema declares which identifier fields the server insists
on. If you omit one, the server rejects the notification with a helpful error.

To see what fields a schema asks for:

```bash
aviso schema get mars
```

### What you see on success

When `aviso notify` succeeds, it prints the server's response. In your terminal
you get a human-readable line; piped to a file or another command, you get one
line of compact JSON.

## Listen {#listen}

`aviso listen` opens a long-lived connection to the server and prints (or
trigger-handles) each matching notification as it arrives.

There are two ways to set up a listener: a YAML file (for anything you care
about), and inline flags (for quick exploration).

### Listen with inline flags

```bash
aviso listen --event mars --identifiers '{"class":"od"}'
```

This runs one listener with a single echo trigger. Press Ctrl+C to stop.

`--event` and `--identifiers` come as a pair: pass both or pass neither.
`--identifiers` takes a JSON object literal. Values may have any JSON shape:

```bash
aviso listen --event observations \
  --identifiers '{"date":"20260601","polygon":[[46,8],[46,9],[47,9],[47,8],[46,8]]}'
```

For `observations`, use the server's
[point-cloud schema](https://sites.ecmwf.int/docs/aviso-server/main/practical-examples/point-cloud-filtering.html).
Providers send `point_cloud`; subscribers send a closed `polygon` and the
required `date`. A cloud matches when any point is inside or on the boundary.

For an empty identifier map (every notification of this event type), pass
`'{}'`. The server may still require certain fields to be present, depending on
the schema.

The inline mode runs with one default trigger: `echo`. For any other trigger,
use a YAML file.

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

The CLI spawns one task per listener and runs them concurrently. Each listener
resumes independently from the [state file](../reference/state-file.md).

A full reference for the file format is at
[Listener YAML](../reference/listener-yaml.md).

### Triggers, briefly

A trigger is the action aviso takes for each matching notification. Six kinds
are built in:

- [`echo`](../triggers/echo.md) prints the notification.
- [`log`](../triggers/log.md) appends it to a file.
- [`command`](../triggers/command.md) runs a shell command (Unix only).
- [`webhook`](../triggers/webhook.md) makes an HTTP request to a URL of your
  choice.
- [`teams`](../triggers/teams.md) posts to a Microsoft Teams channel.
- [`post`](../triggers/post.md) forwards the original event envelope.

You can attach as many triggers as you want to a listener. They run in
declaration order.

### What `aviso listen` prints

On a TTY, the echo trigger prints a multi-line pretty JSON block per
notification with a one-line header. When the output is piped to a file or
another command, it switches to one compact JSON object per line, so
`aviso listen | jq` and `aviso listen >> notifications.ndjson` both work.

Other triggers write to their own destinations (a file, a webhook, a shell
command); the CLI itself stays quiet on stdout for those.

### Stopping

Ctrl+C drains in-flight work and exits. A second Ctrl+C within five seconds
exits immediately (return code 130).

### Resuming

When you stop and restart `aviso listen` against the same server and
identifiers, it resumes from the last notification it fully processed. The
cursor lives in `~/.config/aviso/state.json` by default. For a one-off run that
does not write to the file, pass `--no-state-store`.

To start from an explicit point (a sequence id or a date) just for this run:

```bash
aviso listen my-listeners.yaml --from 2026-05-01
aviso listen my-listeners.yaml --from 1000
```

The full rules for `--from` (pure-digit input is always a sequence id, dashes
mean a date, and so on) live in
[Configuration](./configuration.md#from-value-formats).

### Listening for several event types at once

Put multiple listeners in the same YAML file (or in separate files; the CLI
accepts a list):

```bash
aviso listen mars.yaml cosmo.yaml
```

Each listener has its own connection, its own resume cursor, and its own
triggers. A failure in one does not stop the others.

## Worked example: weather constraints {#weather-constraints}

This synthetic example uses no external data services. You need a test server
whose operator has installed the following schema in its server configuration.
This is **server YAML**, not a client listener file. It declares the complete
`weather` event type; it is not a schema shipped on every server.

The operator adds this block to the server configuration; see the
[schema guide](https://sites.ecmwf.int/docs/aviso-server/main/schema-guide.html).
The client can inspect a schema, but `aviso schema get` does not install one.

```yaml
notification_schema:
  weather:
    topic:
      base: weather
      key_order: [date, severity, anomaly, region]
    identifier:
      date:
        type: DateHandler
        required: true
        canonical_format: '%Y%m%d'
      severity:
        type: IntHandler
        required: false
        range: [0, 10]
      anomaly:
        type: FloatHandler
        required: false
        range: [-100, 100]
      region:
        type: EnumHandler
        required: false
        values: [north, south, west]
```

Set `AVISO_BASE_URL` in both terminals to your test server's address, using the
connection and authentication settings from [Configuration](./configuration.md).
For example, `http://127.0.0.1:8000` is only a placeholder for a locally running
server; it is not a hosted service. Inspect the installed schema before running
the example:

```bash
aviso schema get weather
```

All four fields are required when publishing. Only `date` is required in a
filter; omitting an optional filter field accepts all its values. Each seed
below has a distinct combination of routing identifiers, so the records do not
replace one another on a backend that retains only the latest record per
subject. Use a fresh test stream to get exactly the results shown.

### Start the listener first

In the first terminal, run:

```bash
aviso listen --event weather \
  --identifiers '{"date":"20260913","severity":{"gte":5},"anomaly":{"between":[40,50]},"region":{"in":["north","south"]}}' \
  --from 0 --no-state-store
```

Leave it running, then publish in the second terminal. `--from 0` reads retained
records after sequence zero and continues with live notifications, without
reading or writing a saved cursor. On this fresh test stream, seeds published
before or during connection setup are still included; there is no readiness
message to wait for. This cannot recover records removed by retention.

The filter means severity at least 5, anomaly from 40 through 50 inclusive, and
region either north or south. See
[Filters](../concepts/filters.md#constraint-filters) for the operator rules.

### Publish five records

In the second terminal, run these commands in order:

```bash
aviso notify 'event=weather,date=20260913,severity:=3,anomaly:=39.5,region=north,data={"id":"A"}'
aviso notify 'event=weather,date=20260913,severity:=5,anomaly:=40,region=NORTH,data={"id":"B"}'
aviso notify 'event=weather,date=20260913,severity:=6,anomaly:=42.5,region=south,data={"id":"C"}'
aviso notify 'event=weather,date=20260913,severity:=7,anomaly:=50,region=west,data={"id":"D"}'
aviso notify 'event=weather,date=20260913,severity:=8,anomaly:=50.5,region=south,data={"id":"E"}'
```

`severity:=5` sends a JSON number; `severity=5` sends a string. These commands
use concrete numbers, not constraint objects. `data={"id":"B"}` is the payload
label used to recognise a record, not a filter or the server's notification ID.
The single quotes protect each complete argument from the shell.

| Record | Severity | Anomaly | Region | Selected? |
|---|---|---|---|---|
| A | 3 | 39.5 | north | No: severity and anomaly too low |
| B | 5 | 40 | NORTH | Yes: lower boundaries included, case normalised |
| C | 6 | 42.5 | south | Yes |
| D | 7 | 50 | west | No: region excluded |
| E | 8 | 50.5 | south | No: anomaly too high |

The listener prints two notifications, with `payload.id` values **B** then
**C**. Press Ctrl+C to stop it. To read the same retained records again, use
[replay with this filter](./replay.md#weather-constraints). To keep the filter
in a file, use the [YAML equivalent](../reference/listener-yaml.md#constraints).

## What next

- [Replay history](./replay.md): re-read past notifications.
- [Configuration](./configuration.md): config files, environment variables, TLS.
- [Triggers overview](../triggers/overview.md): pick the right trigger.
- [Troubleshooting](./troubleshooting.md): the usual snags.
