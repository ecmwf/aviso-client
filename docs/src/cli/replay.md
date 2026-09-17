<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Replay history

`aviso replay` re-reads past notifications and runs them through your triggers,
just like `aviso listen` does for live ones. The difference: replay always
starts from a cursor you supply, ends at a fixed history boundary, and never
touches the state file.

Use it when:

- You want to backfill a new downstream system with what already happened.
- You missed a window of notifications and want to re-process them.
- You are debugging a listener and need to feed it past traffic for repeatable
  runs.

## A first replay

These examples assume your server operator has installed the same small `mars`
schema used in [Publish and listen](./publish-and-listen.md#schema-assumptions).
This is **server YAML**, not a client listener file:

```yaml
notification_schema:
  mars:
    topic:
      base: mars
      key_order: [class, step]
    identifier:
      class:
        type: EnumHandler
        values: [od, rd]
        required: true
      step:
        type: IntHandler
        required: false
    payload:
      required: false
```

In filters, `class` is required and `step` is optional. The integer `step` has
no configured range, and the payload is optional. Use the connection and
authentication settings from [Configuration](./configuration.md), then inspect
the installed schema:

```bash
aviso schema get mars
```

This command only reads the schema; the operator configures it. Adapt the filter
if your server's schema differs. Replay reads notifications already published
by providers. You do not need to publish your own; for test history, see the
[publish example](./publish-and-listen.md#publish).

```bash
aviso replay --event mars --identifiers '{"class":"od"}' --from 2026-05-01
```

This selects stored `mars` notifications with `class=od`, with no restriction
on `step`. The date selects publication time from midnight UTC on 1 May 2026,
not a date in the notification's metadata. Replay prints matching retained
notifications and stops at the history boundary captured when the run starts.
It does not wait for new notifications.

Retention limits what is available, and a server replay cap can stop a run
before catch-up completes. See the server's
[historical replay limits](https://sites.ecmwf.int/docs/aviso-server/main/streaming-semantics.html#historical-replay-limits).
An empty matching history prints no notifications.

`--from` takes either a sequence id or a date. The rules are the same as for
`aviso listen --from`; full list at
[Configuration: `--from` formats](./configuration.md#from-value-formats).

You can also save this client listener as `my-listeners.yaml`. Its `echo`
trigger prints each matching notification:

```yaml
listeners:
  - name: mars-od
    event: mars
    identifiers:
      class: od
    triggers:
      - type: echo
```

```bash
aviso replay --from 0 my-listeners.yaml
```

`--from 0` reads retained history after sequence zero. To resume from a known
notification, replace `0` with its actual sequence id; only later sequences are
selected. If a file defines several listeners, select one by name:

```bash
aviso replay --from 0 --listener mars-od my-listeners.yaml
```

## Repeated identifiers

For shell scripts, pass each identifier as a separate argument:

```bash
MARS_CLASS=od
STEP=12
aviso replay --event mars \
  --identifier "class=$MARS_CLASS" --identifier "step:=$STEP" --from 0
```

This reads retained `mars` records with `class=od` and `step=12`. `key=value`
preserves the exact string; `key:=JSON` parses a typed value. See
[scripting rules](./publish-and-listen.md#repeated-identifiers) for quoting and
validation. Choose repeated `--identifier` or one `--identifiers` JSON object;
the two sources conflict. Either requires `--event`, which requires a source.
Inline arguments take precedence over YAML and `--listener`, with a default
echo trigger.

## Replay does not write the state file

The state file (`~/.config/aviso/state.json`) is for `aviso listen` only. Replay
never reads or writes it.

The reason: a replay run shares the same resume key as the equivalent listen
run, and letting replay update the cursor could push the listen cursor past
notifications the listen has not actually processed. To keep the at-least-once
delivery guarantee, replay stays stateless by design.

The trade-off: if you interrupt a replay, the next replay needs an explicit
`--from` to resume. There is no "resume my replay" mode.

`--identifiers` accepts structured JSON values just like `aviso listen`. Quote
the whole object for the shell, but leave arrays unquoted inside it.

The following assumes the operator has installed the server's
[point-cloud schema](https://sites.ecmwf.int/docs/aviso-server/main/practical-examples/point-cloud-filtering.html#schema),
also used in
[Publish and listen](./publish-and-listen.md#spatial-schema-assumptions).
Its `date` is required (`DateHandler`, format `%Y%m%d`). Providers publish the
required `point_cloud` array and payload; replay filters supply `date` and a
closed `polygon` array instead of `point_cloud`. Coordinates are
`[latitude,longitude]`, with the first pair repeated last to close the polygon.

```bash
aviso replay --event observations \
  --identifiers '{"date":"20260601","polygon":[[46,8],[46,9],[47,9],[47,8],[46,8]]}' \
  --from 2026-05-01
```

A cloud matches when any point lies inside or on the polygon's boundary.

## Replay the weather example {#weather-constraints}

After publishing the five records in the
[weather tutorial](./publish-and-listen.md#weather-constraints), reuse exactly
the same identifier filter:

```bash
aviso replay --event weather \
  --identifiers '{"date":"20260913","severity":{"gte":5},"anomaly":{"between":[40,50]},"region":{"in":["north","south"]}}' \
  --from 0
```

The same filter can be supplied as repeated arguments instead:

```bash
aviso replay --event weather --identifier date=20260913 \
  --identifier 'severity:={"gte":5}' \
  --identifier 'anomaly:={"between":[40,50]}' \
  --identifier 'region:={"in":["north","south"]}' --from 0
```

On the fresh test stream either form prints `payload.id` values B and C, then
exits. Do not combine the two identifier sources in one command.
`--from 0` reads retained history after sequence zero; it cannot recover records
that the backend no longer retains. The wire filter is the same object used by
listen, with JSON numbers inside the constraint objects. The
[operator rules](../concepts/filters.md#constraint-filters) are shared by both
commands.

## Replay vs listen

| Behavior | Listen | Replay |
|---|---|---|
| Starts from | The state file, or `--from` if you set it | `--from` (required) |
| Writes the state file? | Yes, unless disabled | No |
| After catching up | Stays open for new notifications | Stops at the fixed history boundary |
| Reconnect on routine server close? | Yes | Yes |

## When you want both

To read retained history and continue with new notifications, use `listen` with
`--from` and the same schema and filter:

```bash
aviso listen --event mars --identifiers '{"class":"od"}' --from 2026-05-01
```

Press Ctrl+C to stop. Listen records progress in its state file. Running replay
and then starting a separate listener without `--from` or saved state can leave
a gap: that listener starts at the server's current tip, after replay's fixed
boundary. Seeding a state file separately is not an exact-handover guarantee.

## What next

- [Publish and listen](./publish-and-listen.md): the live equivalent.
- [State file](../reference/state-file.md): what listen writes and how to edit
  it.
