# CLI quickstart

A whirlwind tour of the commands you will actually use. Each section is one or
two commands and the output you can expect.

This page assumes you have the binary installed and a server URL with
credentials. If not, start with [Install](./install.md) and then
[point at a server](./configuration.md#tell-aviso-where-the-server-is).

For these examples, set up your environment once:

```bash
export AVISO_BASE_URL=https://aviso.example
export AVISO_TOKEN=your-bearer-token
```

## See what the server knows

```bash
aviso schema list
```

You get one event type per line. Pick one to dig into:

```bash
aviso schema get mars
```

The output is the schema's JSON: which identifier fields exist, which are
required, and what types they hold.

## Publish a notification

```bash
aviso notify 'event=mars,class=od,stream=oper,date=20260601,domain=g,expver=0001,step=0,time=1200,data={"location":"s3://bucket/path"}'
```

The parameters are a comma-separated list. `event=<TYPE>` is required.
`data=<JSON>` is the optional payload. Every other `key=value` pair lands in the
identifier map.

Values that contain commas (a polygon, for example) must be wrapped in quotes:

```bash
aviso notify 'event=test_polygon,polygon="46,8,46,9,47,9,47,8,46,8",date=20260601,time=1200,data={"test":true}'
```

The quotes are part of the CLI syntax (so the comma inside the value is not
mistaken for a separator); they are stripped before the value is sent to the
server.

## Listen for live notifications

The short form, no YAML file required:

```bash
aviso listen --event mars --identifiers '{"class":"od"}'
```

You will see new notifications stream to your terminal as they arrive. Press
Ctrl+C to stop.

Pipe to `jq` to extract one field:

```bash
aviso listen --event mars --identifiers '{"class":"od"}' | jq -r '.payload'
```

Send to a file:

```bash
aviso listen --event mars --identifiers '{"class":"od"}' > mars.ndjson
```

aviso swaps to a one-line-per-notification (NDJSON) format automatically when
its output is not a terminal, so it composes cleanly with shell tools.

## Replay history

To run through past notifications from a cursor (a sequence id or a date):

```bash
aviso replay --event mars --identifiers '{"class":"od"}' --from 2026-05-01
aviso replay --event mars --identifiers '{"class":"od"}' --from 1000
```

Replay runs once and ends when it reaches the live edge. Read more at
[Replay history](./replay.md).

## Run a production listener with a YAML file

For anything beyond ad-hoc inspection, write a small listener file:

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
```

```bash
aviso listen my-listeners.yaml
```

Each listener gets its own task. The CLI runs all of them concurrently. Read
more at
[Listen with a YAML file](./publish-and-listen.md#listen-with-a-yaml-file).

## See what configuration is in effect

```bash
aviso config dump --redact
```

You get the resolved settings with a comment on each line saying where it came
from (flag, env, file, or default). The `--redact` option masks tokens and
passwords so you can paste the output into an issue.

## What next

- [Publish and listen](./publish-and-listen.md): the daily-use guide.
- [Configuration](./configuration.md): config files, environment variables, TLS.
- [Troubleshooting](./troubleshooting.md): the common things that go wrong.
