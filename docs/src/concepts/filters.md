# Filters

A filter is the set of identifiers you want notifications for. The server returns only events whose identifier map matches your filter on every field you set.

## A scalar filter

```yaml
listeners:
  - name: mars-od
    event: mars
    identifiers:
      class: od
      stream: oper
```

This listener gets notifications where `class=od` *and* `stream=oper`. Fields you do not mention act as wildcards (subject to the server's `required: true` rule, below).

Inline equivalent:

```bash
aviso listen --event mars --identifiers '{"class":"od","stream":"oper"}'
```

## Required vs optional identifiers

The schema for each event type declares which identifier fields are `required: true` and which are `required: false`. The flag means different things on the listen side and on the notify side.

- **When you listen**: `required: true` fields must appear in the filter. `required: false` fields can be omitted; an omitted field acts as a wildcard so the server returns every value.
- **When you publish**: every identifier in the schema is required, regardless of the flag. Omitting any of them returns `400` with a `Required field '<name>' missing for notify operation` message. The `required: false` flag is a listen-time wildcard semantic only; it does not relax `notify` validation.

To see which fields are required for an event type:

```bash
aviso schema get mars
```

## The empty filter

```bash
aviso listen --event mars --identifiers '{}'
```

This asks for every notification of type `mars` regardless of identifier. The server accepts it only when the schema's `required: true` fields are all marked as missing-is-ok, which is rare. For most schemas, you will need to set the required fields.

## Spatial filters

Some event types support geospatial filtering. The two shapes are:

```yaml
identifiers:
  polygon:
    type: polygon
    points: [[46, 8], [46, 9], [47, 9], [47, 8], [46, 8]]

identifiers:
  point:
    latitude: 46.5
    longitude: 8.5
```

`polygon` returns notifications whose geometry overlaps the polygon. `point` returns notifications matching exactly that point (or covering it, depending on the publisher's geometry).

Polygons must close: the last point must equal the first.

On the command line, the polygon shape comes through with quoting:

```bash
aviso listen --event mars --identifiers '{"polygon":"46,8,46,9,47,9,47,8,46,8"}'
```

For `aviso notify`, the same quoting rule applies to comma-bearing values; see [Publish and listen: quoting values](../cli/publish-and-listen.md#quoting-values-that-contain-commas).

## When the filter does not match

If you set a field to a value the server's schema does not allow, the server rejects the request with a clear error. aviso surfaces the message verbatim.

If you set a field to a value that is allowed but matches nothing right now, the listener simply waits. New matching notifications will arrive when they happen.

## How the filter affects resume

The hash key in [the state file](./resume-and-state.md) is computed from the server URL, the event type, and the canonicalised filter. Two listeners with the same URL and event type but different filters get different cursors.

This is the right behaviour: a listener with `class=od` should resume independently from a listener with `class=ai`.

It also means that changing the filter on a running listener (without changing its name) will, on next start, look like a fresh listener for resume purposes. You will start from `--from` if you pass one, or from "now" if you do not.

## What next

- [Notifications](./notifications.md): the shape of what arrives.
- [Resume and state](./resume-and-state.md): how the filter affects the cursor.
- [CLI publish and listen](../cli/publish-and-listen.md): the commands that use filters.
