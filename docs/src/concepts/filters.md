# Filters

A filter is the set of identifiers you want notifications for. The server
returns only events whose identifier map matches your filter on every field you
set.

## A scalar filter

```yaml
listeners:
  - name: mars-od
    event: mars
    identifiers:
      class: od
      stream: oper
```

This listener gets notifications where `class=od` *and* `stream=oper`. Fields
you do not mention act as wildcards (subject to the server's `required: true`
rule, below).

Inline equivalent:

```bash
aviso listen --event mars --identifiers '{"class":"od","stream":"oper"}'
```

## Required vs optional identifiers

The schema for each event type declares which identifier fields are
`required: true` and which are `required: false`. The flag means different
things on the listen side and on the notify side.

- **When you listen**: `required: true` fields must appear in the filter.
  `required: false` fields can be omitted; an omitted field acts as a wildcard
  so the server returns every value.
- **When you publish**: every identifier in the schema is required, regardless
  of the flag. Omitting any of them returns `400` with a
  `Required field '<name>' missing for notify operation` message. The
  `required: false` flag is a listen-time wildcard semantic only; it does not
  relax `notify` validation.

To see which fields are required for an event type:

```bash
aviso schema get mars
```

## The empty filter

```bash
aviso listen --event mars --identifiers '{}'
```

This asks for every notification of type `mars` regardless of identifier. The
server accepts it only when the schema declares no `required: true` identifier
fields. For schemas that have any (which is most of them, including `mars`), the
server rejects the request with
`400 Required field '<name>' missing for watch operation` and you must include
each required field in the filter.

## Spatial filters

Spatial identifiers use latitude-longitude arrays. A point is `[lat, lon]`. A
polygon needs at least four pairs, with the first pair repeated last. A point
cloud is also a list of points, but does not describe a ring and does not need a
closing repeat. Duplicate cloud points are valid and their order is preserved.

```yaml
identifiers:
  polygon:
    - [46, 8]
    - [46, 9]
    - [47, 9]
    - [47, 8]
    - [46, 8]
```

You get notifications whose geometry overlaps the polygon.

On the command line, the same value comes through with quoting because of the
embedded commas:

```bash
aviso listen --event test_polygon \
  --identifiers '{"polygon":[[46,8],[46,9],[47,9],[47,8],[46,8]]}'
```

Providers publish point clouds using the same nested-array shape:

```json
{"point_cloud":[[46,8],[47,9],[46.5,8.5]]}
```

Subscribers use `polygon`, not `point_cloud`, to select clouds with any point
inside or on the polygon boundary. See
[Publish and listen](../cli/publish-and-listen.md) for the complete request and
schema. Run `aviso schema get <TYPE>` to see which identifier fields are valid.

## When the filter does not match

If you set a field to a value the server's schema does not allow, the server
rejects the request with a clear error. aviso surfaces the message verbatim.

If you set a field to a value that is allowed but matches nothing right now, the
listener waits. New matching notifications will arrive when they happen.

## How the filter affects resume

The hash key in [the state file](./resume-and-state.md) is computed from the
server URL, the event type, and the canonicalised filter. Two listeners with the
same URL and event type but different filters get different cursors.

This is the right behaviour: a listener with `class=od` should resume
independently from a listener with `class=ai`.

It also means that changing the filter on a running listener (without changing
its name) will, on next start, look like a fresh listener for resume purposes.
You will start from `--from` if you pass one, or from "now" if you do not.

## What next

- [Notifications](./notifications.md): the shape of what arrives.
- [Resume and state](./resume-and-state.md): how the filter affects the cursor.
- [CLI publish and listen](../cli/publish-and-listen.md): the commands that use
  filters.
