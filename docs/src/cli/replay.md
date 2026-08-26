# Replay history

`aviso replay` re-reads past notifications and runs them through your triggers,
just like `aviso listen` does for live ones. The difference: replay always
starts from a cursor you supply, ends when it catches up, and never touches the
state file.

Use it when:

- You want to backfill a new downstream system with what already happened.
- You missed a window of notifications and want to re-process them.
- You are debugging a listener and need to feed it past traffic for repeatable
  runs.

## A first replay

```bash
aviso replay --event mars --identifiers '{"class":"od"}' --from 2026-05-01
```

This re-streams every matching notification from 1 May 2026 onwards. When replay
catches up to the live edge, it stops.

`--from` takes either a sequence id or a date. The rules are the same as for
`aviso listen --from`; full list at
[Configuration: `--from` formats](./configuration.md#from-value-formats).

You can also use a YAML file:

```bash
aviso replay --from 1000 my-listeners.yaml
```

When more than one listener resolves, pick one with `--listener <NAME>`.

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
the whole object for the shell, but leave arrays unquoted inside it:

```bash
aviso replay --event observations \
  --identifiers '{"point_cloud":[[46,8],[47,9]]}' --from 2026-05-01
```

## Replay vs listen

| | `aviso listen` | `aviso replay` |
|---|---|---|
| Starts from | The state file, or `--from` if you set it | `--from` (required) |
| Writes the state file? | Yes | No |
| Runs forever? | Yes (until you stop it) | No (ends at the live edge) |
| Reconnect on routine server close? | Yes | Yes |

## When you want both

Run replay first to backfill, then start listen for live:

```bash
# Backfill from a known starting point.
aviso replay --event mars --identifiers '{"class":"od"}' --from 2026-05-01

# Then start the live listener.
aviso listen --event mars --identifiers '{"class":"od"}'
```

The live listener's first run with an empty state file picks up from "now" (the
server's current tip). There can be a small gap between where replay ended and
where listen starts. For an exact handover, run listen first with
`--from <a known cursor>` to seed the state file, then run replay to fill in any
older history.

## What next

- [Publish and listen](./publish-and-listen.md): the live equivalent.
- [State file](../reference/state-file.md): what listen writes and how to edit
  it.
