# CLI overview

`aviso` is a single binary that talks to an aviso-server. Use it to publish
notifications, listen for new ones, replay history, and look around (schemas,
configuration).

Use it when you want to:

- Subscribe to a stream and pipe the output to `jq`, a log file, or any
  line-oriented tool.
- Run a long-lived listener that ships notifications to a webhook, a Teams
  channel, a log file, or a custom shell command.
- Drop a notification into a stream from a script.
- Inspect what schemas the server publishes, or wipe a stream during testing.

## Subcommands at a glance

| Command | What it does |
|---|---|
| [`aviso notify`](./publish-and-listen.md#publish) | Publish one notification. |
| [`aviso listen`](./publish-and-listen.md#listen) | Open a long-lived stream and run the configured triggers. |
| [`aviso replay`](./replay.md) | Re-read history from a cursor and run the configured triggers. |
| [`aviso schema`](./operations.md#schemas) | List the event types the server knows about, or fetch one schema. |
| [`aviso admin`](./operations.md#admin) | Destructive operations (wipe a stream, delete one notification). |
| [`aviso config dump`](./operations.md#config-dump) | Print the resolved configuration with sources. |
| [`aviso completions`](./operations.md#shell-completions) | Print shell completion scripts. |

## When to use the CLI vs the library

| Use the CLI when | Use the library when |
|---|---|
| You want a binary in your terminal, a cron job, or a systemd unit. | You are writing a Rust program and want notifications inside it. |
| You want to pipe notifications into another tool. | You want full control over how each notification is handled, in code. |
| You want a quick one-liner against a stream. | You want to share an authentication or connection pool across many subscriptions. |
| You want triggers configured in YAML. | You want triggers and listeners constructed programmatically. |

Both surfaces are built on the same code, so behaviour is identical at the wire
level.

## What comes next

- [Install](./install.md) the binary.
- [Quickstart](./quickstart.md): the smallest useful run, end to end.
- [Publish and listen](./publish-and-listen.md): the two commands you will use
  most.
