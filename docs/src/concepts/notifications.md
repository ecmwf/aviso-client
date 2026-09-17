<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Notifications

<div class="reference-guide">

A notification tells you that something happened, such as a dataset becoming
available. A data provider publishes it to the server. You receive matching
notifications with a listener, then use their details in your analysis or in
[triggers](../triggers/overview.md), actions that aviso runs for you.

The notification usually describes the data rather than containing the dataset
itself. A file location in a notification does not download the file for you.

## The four fields you care about

| Field | What it is |
|---|---|
| `event_type` | Kind of event, such as `mars`. |
| `sequence` | Number used to resume listening. |
| `identifier` | Named values describing the event. |
| `payload` | Extra information from the provider. |

## Reading a notification

This example uses the
[small quickstart schema](../python/quickstart.md#what-is-on-your-server).
It has a required `class` filter and an optional `step` filter. Data providers
supply both identifiers when publishing. Here is the original server message
printed by Python's `print(notification)`; the id and publication time vary:

```json
{
  "data": {
    "identifier": {
      "class": "od",
      "step": "12"
    },
    "payload": {
      "location": "file:///data/forecast.grib"
    }
  },
  "datacontenttype": "application/json",
  "dataschema": "https://aviso.example/schema/mars",
  "id": "mars@1",
  "source": "https://aviso.example",
  "specversion": "1.0",
  "time": "2026-09-15T15:15:15.799578652Z",
  "type": "int.ecmwf.aviso.mars"
}
```

This format is called a CloudEvent. The client exposes `event_type` as `mars`
and `sequence` as `1`, taken from the message's `id`. The `data` object holds
the identifiers and payload. The server represents scalar identifiers such as
`step` as strings. `source` identifies the service, and `time` is publication
time, not a forecast date.

The CLI's default echo output uses the four client fields in the table above,
not the original CloudEvent. When piped to another command, it writes one JSON
object per line. See [Python listening](../python/listen.md) for a complete
script and how to access individual fields.

## Identifiers

Identifiers describe what a notification concerns. Different values distinguish
different data, but the same values can appear in more than one notification.

Which fields belong in the map depends on the event type's schema. The small
`mars` example has `class` and `step`. Your service may define more forecast
fields, or use a different event type. Check the schema before choosing a
filter.

The server determines which identifiers are valid; aviso just passes them along.
Values may have any JSON shape. For spatial identifiers a point is `[lat, lon]`,
while polygons and point clouds are `[[lat, lon], ...]`. That shape is preserved
when a notification is published, delivered, printed, or passed to a trigger.
To see the schema for a type:

```bash
aviso schema get mars
```

## Filtering

When you listen, you pass aviso an identifier map that acts as a filter. The
server returns only notifications whose identifier matches *every* field you
set:

```bash
aviso listen --event mars --identifiers '{"class":"od","step":12}'
```

You get notifications with `class=od` and `step=12`. Identifiers you do
not mention act as wildcards (subject to the schema's `required: true` fields,
which the server still demands).

The full filter rules, including spatial filters, are at
[Filters](./filters.md).

## Sequence numbers and ordering

The server assigns increasing sequence numbers within each event type. They
are 64-bit whole numbers. A filtered listener can skip numbers because other
notifications do not match its filter.

Do not assume every delivery arrives in increasing order. Repeated or older
notifications can still reach your code and triggers. Saved resume positions
only move forwards. See [Resume and state](./resume-and-state.md) for the limits
of recovery after a crash.

The sequence is also part of the notification id, such as `mars@42`. Server
operators use that id for [administration](../cli/operations.md).

## Payloads

The payload is whatever the publisher put there. aviso does not interpret it,
validate it, or modify it. Common patterns:

- A `location` URL pointing at where a freshly written dataset lives.
- A small JSON object with the publisher's own metadata.
- `null`, when the event itself is all there is to know.

In a trigger, you reach the payload as `notification.payload`:

```yaml
triggers:
  - type: webhook
    url: "https://hooks.example/notify"
    body_template: '{"event": "{{ notification.event_type }}", "payload": {{ notification.payload }}}'
```

## What next

- [Streams](./streams.md): how aviso talks to the server.
- [Filters](./filters.md): the matching rules.
- [Triggers overview](../triggers/overview.md): what aviso does with each
   notification.

</div>
