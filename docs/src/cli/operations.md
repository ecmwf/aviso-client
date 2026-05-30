# Operations

The supporting commands: schema inspection, destructive admin, configuration
introspection, shell completions.

## Schemas {#schemas}

The server publishes a schema per event type, describing which identifier fields
exist, which are required, and what types they hold. Two commands let you look
at them.

### List event types

```bash
aviso schema list
```

On a terminal you get a header line and a bullet list of event types. When piped
(or with `--json`), each entry comes out as a JSON object, one per line:

```bash
aviso schema list | jq -r .event_type
```

### Get one schema

```bash
aviso schema get mars
```

You get the schema as pretty JSON. Use it to discover which identifiers the
server insists on when publishing or filtering.

To fetch every schema in one command:

```bash
aviso schema list | jq -r .event_type | xargs -I{} aviso schema get {}
```

aviso does not validate notifications against schemas locally. The server is the
single source of truth; these commands are for human discovery.

## Admin {#admin}

Three destructive commands sit under `aviso admin`. They all require a `--yes`
flag on the command line; the flag cannot be set in the configuration file, by
design.

### Wipe one event-type stream

```bash
aviso admin wipe-stream mars --yes
```

Deletes every notification of type `mars` on the server. Useful in test
environments. Operator-level credentials required.

### Wipe everything

```bash
aviso admin wipe-all --yes
```

Deletes every notification of every type. Useful when you want a completely
clean slate.

### Delete a single notification

```bash
aviso admin delete 'mars@42' --yes
```

The argument is the notification id (the `<event_type>@<sequence>` form the
server emits).

## Configuration introspection {#config-dump}

```bash
aviso config dump --redact
```

Prints the resolved configuration to stdout with a comment on each line saying
where the value came from (flag, env, file, or default). `--redact` masks tokens
and passwords.

The dump is what aviso would use right now, given your current flags,
environment, and config file. It is the fastest way to debug "why is aviso not
picking up my setting".

For JSON output (so you can `jq` it):

```bash
aviso config dump --redact --json
```

The `auth` block is summarised as `provider: <set>` or `<unset>` rather than
per-field; the listeners block is summarised by name, event, identifier count,
and trigger count.

## Shell completions {#shell-completions}

```bash
# Bash
aviso completions bash > ~/.local/share/bash-completion/completions/aviso

# Zsh
aviso completions zsh > ~/.local/share/zsh/site-functions/_aviso

# Fish
aviso completions fish > ~/.config/fish/completions/aviso.fish

# PowerShell
aviso completions powershell >> $PROFILE

# Elvish
aviso completions elvish > ~/.config/elvish/lib/aviso.elv
```

Restart the shell (or source the file) for the completions to take effect.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. A clean Ctrl+C with no prior failure also returns 0. |
| `1` | Runtime error. Server returned 4xx/5xx, network failure, or a listener task errored out. |
| `2` | Usage error. Missing required flag, invalid argument, destructive admin command without `--yes`, no listeners resolved, unparseable `--from` value. |
| `130` | Second Ctrl+C within five seconds. Hard exit; no drain. |

## What next

- [Configuration](./configuration.md): where settings come from and how to
  override them.
- [Troubleshooting](./troubleshooting.md): the common things that go wrong.
