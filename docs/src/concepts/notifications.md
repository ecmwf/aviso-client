# Notifications

A notification is one event that the server has published. aviso receives them
on a stream, deserialises them, and passes them through triggers (or your code).

## The four fields you care about

| Field | What it is |
|---|---|
| `event_type` | A string naming the kind of event. For example `mars`. |
| `sequence` | A 64-bit integer that strictly increases per event type. Used to resume. |
| `identifier` | A map of named fields that describe which event it is. |
| `payload` | A JSON value the publisher attached. Often the location of a file. May be `null`. |

## Reading a notification

```json
{
  "event_type": "mars",
  "sequence": 42,
  "identifier": {
    "class": "od",
    "stream": "oper",
    "date": "20260601",
    "time": "1200",
    "domain": "g",
    "expver": "0001",
    "step": "0"
  },
  "payload": {
    "location": "s3://bucket/key"
  }
}
```

This is exactly what `aviso listen` writes to stdout (one such object per line
in NDJSON when piped).

## Identifiers

The identifier map is the address of the event. Two notifications of the same
type but with different identifier values are different events.

Which fields belong in the map depends on the event type's schema. For `mars`,
you will see things like `class`, `stream`, `date`, `step`, `expver`. For
another event type the keys will be different.

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
aviso listen --event mars --identifiers '{"class":"od","stream":"oper"}'
```

You get notifications with `class=od` *and* `stream=oper`. Identifiers you do
not mention act as wildcards (subject to the schema's `required: true` fields,
which the server still demands).

The full filter rules, including spatial filters, are at
[Filters](./filters.md).

## Sequence numbers and ordering

Every notification has a sequence number. The server guarantees:

- Sequences are strictly increasing per event type.
- The server delivers them to one listener in order.

aviso uses the sequence to resume after a restart and to detect history gaps.
The number is a u64; it does not overflow in any realistic timeframe.

The sequence is also part of the notification id you can use with
`aviso admin delete`:

```bash
aviso admin delete 'mars@42' --yes
```

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
