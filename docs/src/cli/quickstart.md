# CLI quickstart

Consumers use Aviso to receive notifications from data providers. Start by
discovering event types, then listen for new notifications or replay past ones.
Providers publish notifications to announce data; the optional publishing
section below is for them. You do not need to publish anything to listen.

First [install the CLI](./install.md) and
[point at a server](./configuration.md#tell-aviso-where-the-server-is).

Set these to the URL and credentials supplied by your server operator:

```bash
export AVISO_BASE_URL=https://aviso.example
export AVISO_TOKEN=your-bearer-token
```

## See what the server knows

```bash
aviso schema list
```

You get one event type per line. This lists notification schemas, not available
datasets or access rights. You need permission to receive notifications.

These examples assume your operator has configured a small `mars` event type
with the schema below. Check your server's schema and adapt the event name and
filters if it differs:

```bash
aviso schema get mars
```

Example response (JSON, with spacing compacted):

```json
{
  "event_type": "mars",
  "schema": {
    "identifier": {
      "class": {
        "required": true,
        "type": "EnumHandler",
        "values": ["od", "rd"]
      },
      "step": {
        "range": null,
        "required": false,
        "type": "IntHandler"
      }
    },
    "payload": {"required": false}
  },
  "status": "success"
}
```

The schema tells you which labels, called **identifiers**, describe a
notification and can be used as filters. Here, `class` is required in filters
and must be `od` or `rd` (`EnumHandler` means a choice from a list). In this
example, `od` means operational data. `step` is a whole number (`IntHandler`)
used for forecast hours; you can omit it from filters. `range: null` sets no
extra range limit. The server checks these values. The optional **payload**
carries extra information, such as a file location, rather than labels to
filter on.

## Listen for live notifications

Select `mars` notifications whose `class` is `od`. Leaving out the optional
`step` filter selects all forecast steps:

```bash
aviso listen --event mars --identifiers '{"class":"od"}'
```

On a first run, you will see matching new notifications as providers publish
them. Silence can simply mean none have arrived. Press Ctrl+C to stop. Later
runs resume from saved progress and may first deliver missed notifications.

If you have `jq` installed, show just the payload:

```bash
aviso listen --event mars --identifiers '{"class":"od"}' | jq -r '.payload'
```

When piped or redirected, output is one JSON object per line (NDJSON).

## Replay history

To run through past notifications from a cursor (a sequence id or a date):

```bash
aviso replay --event mars --identifiers '{"class":"od"}' --from 2026-05-01
aviso replay --event mars --identifiers '{"class":"od"}' --from 1000
```

Replay reads retained history once and ends when it catches up to the present.
The date refers to notification publication time. Replace `1000` with a sequence
id from your stream. Read more at
[Replay history](./replay.md).

## Catch up, then keep listening

To read past notifications and then keep listening:

```bash
aviso listen --event mars --identifiers '{"class":"od"}' --from 2026-05-01
```

This replays retained matches, then waits for new ones. See all
[Configuration: `--from` formats](./configuration.md#from-value-formats).

Unlike replay, it saves progress. Run the same listener without `--from` to
resume from where it stopped.

## Run a production listener with a YAML file

To save your filters and log notifications, create `my-listeners.yaml` in your
current directory:

```yaml
listeners:
  - name: mars-od
    event: mars
    identifiers:
      class: od
    triggers:
      - type: log
        path: mars-od.log
```

```bash
aviso listen my-listeners.yaml
```

Matching notifications are appended to `mars-od.log`. Press Ctrl+C to stop. For
more triggers and multiple listeners, see
[Listen with a YAML file](./publish-and-listen.md#listen-with-a-yaml-file).

## Publish a notification

Optional, for providers with permission to publish. Using the schema above,
announce an operational forecast at step 12:

```bash
aviso notify 'event=mars,class=od,step:=12,data={"location":"file:///data/forecast.grib"}'
```

Keep the comma-separated parameters inside single shell quotes. `event=mars`
names the event type; `class` and `step` are its identifiers. `class=od` sends
text. `step:=12` uses `:=` to send a JSON number, rather than the text `"12"`
sent by `step=12`. Both are accepted by this schema's integer validator, but
`:=` makes the number explicit in the request. Keep `event=` for the event name.

`data=` already parses JSON and supplies the payload; it does not need `:=`.
The location here is an example file reference. Publishing sends a notification,
not the file, and does not grant consumers access to it. An active matching
listener receives the notification, including this payload.

For nested arrays and spatial identifiers, see
[Publish and listen](./publish-and-listen.md#alternative-coordinate-format).

## See what configuration is in effect

```bash
aviso config dump --redact
```

This shows resolved settings and their sources. `--redact` masks tokens and
passwords for sharing in an issue.

## What next

- [Publish and listen](./publish-and-listen.md): the daily-use guide.
- [Configuration](./configuration.md): config files, environment variables, TLS.
- [Troubleshooting](./troubleshooting.md): the common things that go wrong.
