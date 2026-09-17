<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Filters

<div class="reference-guide">

A filter selects the notifications you want to receive. For example, you might
want only one forecast class or only observations inside an area. Every
condition you give must match; this is what combining conditions with AND means.

First check the event's schema, the server's rules for its identifier fields,
with `aviso schema get <TYPE>`. Include every field required for listening.
Your server operator supplies the schema. As a listener, you do not need
permission to publish data or change it.

Filters use supported identifier fields, not values inside the payload. Spatial
filters can use different field names: `polygon` can select point clouds, and
`point` can select polygons. See [Spatial filters](#spatial-filters).

## A scalar filter

A scalar is a single value, such as `od`, rather than a list or range. These
examples use the
[small quickstart schema](../python/quickstart.md#what-is-on-your-server).
It requires `class` in filters and lets you omit `step` to receive all steps.

```yaml
listeners:
  - name: mars-od
    event: mars
    identifiers:
      class: od
      step: 12
    triggers:
      - type: echo
```

This listener gets notifications where `class=od` and `step=12`. Fields
you do not mention act as wildcards (subject to the server's `required: true`
rule, below). The echo trigger prints each match. Save this as a listener
configuration and pass its path with `aviso listen --config <PATH>`.

Inline equivalent:

```bash
aviso listen --event mars --identifiers '{"class":"od","step":12}'
```

## Required vs optional identifiers

The schema for each event type declares which identifier fields are
`required: true` and which are `required: false`. The flag means different
things on the listen side and on the notify side.

- When you listen, `required: true` fields must appear in the filter.
  `required: false` fields can be omitted; an omitted field acts as a wildcard
  so the server returns every value.
- When you publish, every identifier in the schema is required, regardless
  of the flag. Omitting any of them returns `400` with a
  `Required field '<name>' missing for notify operation` message. The
  `required: false` flag only lets listeners omit a field. Providers still
  supply it when publishing.

To see which fields are required for an event type:

```bash
aviso schema get mars
```

## Constraint filters

Constraints let you select a range or a choice of identifier values. They work
in both listen and replay. Data providers publish actual values, not conditions.
Use the event's schema to choose valid fields and types; the server operator
controls that schema, not the client.

The [weather tutorial](../cli/publish-and-listen.md#weather-constraints)
provides the schema and five synthetic records. This filter selects records B
and C:

```json
{
  "date": "20260913",
  "severity": {"gte": 5},
  "anomaly": {"between": [40, 50]},
  "region": {"in": ["north", "south"]}
}
```

Read it as: on this date, severity is at least 5 and anomaly is from 40
through 50 and region is either north or south. Fields combine with AND; the
choices inside one `in` combine with OR. Keep the required `date` even when
other fields use constraints.

### Operators

| Operator | Meaning | Example field value |
|---|---|---|
| `eq` | Equal to | `{"eq": 6}` |
| `in` | Equal to any listed value | `{"in": [3, 7]}` |
| `gt` | Greater than | `{"gt": 6}` |
| `gte` | Greater than or equal to | `{"gte": 6}` |
| `lt` | Less than | `{"lt": 6}` |
| `lte` | Less than or equal to | `{"lte": 6}` |
| `between` | Within an inclusive range | `{"between": [5, 7]}` |

The schema calls each field's validation rules a handler. `IntHandler` is for
whole numbers, `FloatHandler` for numbers that can include decimals, and
`EnumHandler` for a fixed list of choices.

- `IntHandler` and `FloatHandler` support all seven operators.
- `EnumHandler` supports only `eq` and `in`, with strings from its `values`
  list. Matching ignores case, but does not trim whitespace: `"NORTH"` matches
  `"north"`; `" north "` is not the same value.
- Use exactly one lowercase operator per object. To bound a range, use
  `between`, not an object containing both `gte` and `lte`.
- `in` needs a nonempty list. `between` needs exactly two endpoints in ascending
  order; equal endpoints are allowed. Both endpoints are included.
- Values inside a constraint must fit the limits in the field's schema.
  `IntHandler` needs JSON whole numbers, not `5.0`, quoted strings, or booleans
  (true/false values). `FloatHandler` accepts finite JSON numbers, including
  whole numbers and decimals, but not infinity or NaN ("not a number"). For
  either numeric handler, write `{"gte": 5}`, not `{"gte": "5"}`.
  `EnumHandler` needs strings from its list of choices.
- Keep objects as objects. `"{\"gte\":5}"` is a string, not a constraint.
  Other handlers, such as dates and plain strings, do not accept these objects.

For numeric and enum fields, a single value selects one exact match. The server
can convert a quoted number used on its own to a numeric value. Inside a
numeric constraint, however, quoted numbers are rejected. Float `eq` and `in`
require numbers to match exactly; they do not allow a small difference. Use
`between` to accept numbers within a range.

## The empty filter

An empty filter asks for all values. This is only accepted when the schema has
no required filter fields. For a `mars` schema with required `class`, the
following command demonstrates a rejected request:

```bash
aviso listen --event mars --identifiers '{}'
```

With the example schema, the server rejects this request because `class` is
missing. Include `class` to receive all steps for that class. Other schemas may
have different required fields; check yours before using an empty filter.

## Spatial filters

Use these examples with the spatial schemas in
[Publish and listen](../cli/publish-and-listen.md). A point is one location, a
polygon is a closed boundary around an area, and a point cloud is a collection
of locations.

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
schema. Subscribers to polygon events can use `polygon` for overlap or `point`
for containment. Run `aviso schema get <TYPE>` to see the published identifier
fields and their types.

## When the filter does not match

If you set a field to a value the server's schema does not allow, the server
rejects the request with a clear error. aviso surfaces the message verbatim.
The server also rejects unknown filter fields. Use the schema's supported
fields and the spatial filter names described above.

If you set a field to a value that is allowed but matches nothing right now, the
listener waits. New matching notifications will arrive when they happen.

## How the filter affects resume

The key in [the state file](./resume-and-state.md) is calculated from the server
URL, event type and filter. The client puts filter values in a
consistent form before calculating it. Different filters can have different
saved positions, called cursors.

A listener with `class=od` resumes independently from one with `class=rd`.

If you change the filter, the next run looks for a saved position for that
filter, even if the listener name stays the same. An explicit `--from` takes
precedence. With no explicit start and no matching saved position, the listener
starts from "now".

## What next

- [Notifications](./notifications.md): the shape of what arrives.
- [Resume and state](./resume-and-state.md): how the filter affects the cursor.
- [CLI publish and listen](../cli/publish-and-listen.md): the commands that use
  filters.

</div>
