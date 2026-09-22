<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Troubleshooting

Choose the symptom that matches what you see. Open a panel for checks and links
to the relevant guide. Keep the exception message and any request ID when
asking your server operator for help; do not include credentials.

<div class="troubleshooting">

<details>
<summary id="import-pyaviso-fails-with-modulenotfounderror-no-module-named-pyaviso_native">Import fails: no module named pyaviso._native</summary>

The compiled extension is missing from the Python environment running your
script. Check that your terminal, editor or notebook uses the environment where
you installed pyaviso. See [Install](./install.md).

For a source checkout, run the build commands under
[From source](./install.md#from-source). The dev group supplies `maturin`, which
builds the extension. If the build fails, check the Rust and C compiler
requirements there. Reinstalling into a different environment will not fix the
one your notebook uses.

</details>

<details>
<summary id="pyavisoconfigerror-invalid-base_url">ConfigError: invalid base_url</summary>

Pass the complete URL supplied by your operator, including `https://` or
`http://` and any required port. A hostname alone, such as `localhost`, is not
a complete URL. The Python constructor requires `base_url`; the examples read
it from `os.environ["AVISO_BASE_URL"]`. A `KeyError` for that name means the
environment variable is missing before the client is even constructed.

See [Set the environment](./quickstart.md#set-the-environment).

</details>

<details>
<summary id="pyavisohttperror-401">Credentials fail: AuthError, HTTP 401 or 403</summary>

`Env()` requires credentials. Set `AVISO_TOKEN`, or unset it and set both
`AVISO_USERNAME` and `AVISO_PASSWORD`. A non-empty token takes precedence. For
an anonymous server, pass `auth=pyaviso.Anonymous()`. Omitting `auth`
does not mean anonymous: the client then searches for a credential.

A 401 means the server did not accept the request's credentials. A 403 can mean
the credentials lack permission for the operation or event type. Ask your
operator to check receiving or publishing access as appropriate. Listing schemas
does not prove those permissions.

`ConfigFile` expects an auth-only file with one top-level `bearer` or `basic`
section, not a general CLI config. `Env` caches credentials at construction;
`ConfigFile` rereads its file on a requested refresh after 401. See
[Authentication](./auth.md) for the exact shapes and retry behavior.

</details>

<details>
<summary id="pyavisotransporterror">TransportError: connection or TLS failure</summary>

Check the URL, network access and server availability. The exception message
can identify a DNS, TCP or certificate problem. For TLS, check the hostname and
trusted certificate chain with your operator. Disabling certificate validation
is not a fix for a production certificate problem.

A transport error can also occur while receiving a response. It does not prove
the server stored nothing. Check before retrying a publish to avoid duplicates.
See [Error handling](./error-handling.md).

</details>

<details>
<summary id="no-notifications">The listener is running, but nothing arrives</summary>

A fresh listener waits for new notifications. Silence can be normal. Confirm
the event type and filter against the
[server's schema](./quickstart.md#what-is-on-your-server). In the example `mars`
schema, `class=od` excludes `rd`; omitting `step` selects all steps.

Use [replay-only mode](./listen.md#replay-only) to read retained history once
and exit. `start_from=0` requests retained history after sequence zero.
It cannot restore expired records. You do not need to publish anything yourself.

</details>

<details>
<summary id="pyavisohistorygaperror">HistoryGapError: replay stopped early</summary>

Inspect `reason`. `replay_limit_reached` means the server capped the requested
replay; `max_allowed` reports the cap. It does not by itself mean all missing
records expired. `sequence_jump` reports an unexpected protocol sequence
boundary through `expected` and `observed`.

The iterator stops rather than silently treating that replay as complete. Check
retention and replay limits with your operator before choosing a new start.
`start_from=None` still uses a saved cursor if available. Starting live can skip
historical work. See
[State and resume](./state-and-resume.md#choose-a-starting-position).

</details>

<details>
<summary id="pyavisotriggererror-command-failed">TriggerError: a required action failed</summary>

Check `trigger_kind`, `error_kind` and the exception message. For a failed
command, `exit_code` and `stderr_tail` help identify the cause. A timeout has
`error_kind="timeout"` and `timeout_seconds`; inspect fields appropriate to that
kind rather than assuming every command error has an exit code.

Fix the command, path or receiving service. Retries can help a transient
failure, but fail-fast may stop immediately. Set `required=False` only if
continuing without that action is acceptable; it can leave work undone. See
[trigger settings](./triggers.md#tunables).

</details>

<details>
<summary id="ctrlc-does-not-stop-a-listening-loop">Ctrl+C or closing a listener takes time</summary>

Use `with client.listen(...)` so context exit calls `close()`. This cancels and
waits for the background listener. The sync iterator checks Python signals
while waiting for notifications, but that is not a 100 ms shutdown guarantee.
Your loop's work and cleanup can take longer.

For async code, use `async with` on the iterator; exit awaits `aclose()`. Catch
`KeyboardInterrupt` outside `asyncio.run()` and let cancellation propagate
inside tasks. See the [async listener](./async.md#a-complete-async-listener).

</details>

<details>
<summary id="pyavisostatestoreerror-on-first-run">StateStoreError or unexpected resume behavior</summary>

Create the state file's parent directory before constructing `JsonFileStore`.
Choose local storage where your account can create the lockfile and replace the
state file. Check file contents and permissions if an existing store fails;
preserve the file while investigating rather than deleting your resume data.
See the
[complete resuming listener](./state-and-resume.md#a-complete-resuming-listener).

A changed URL, event type or filter can select a different resume key. A server
schema change alone does not change the key. With no saved cursor, the listener
starts live. The default exit policy can leave the final
notification uncommitted, so repeats are possible. Enabling exit flushing can
skip unfinished buffered work on restart; it is not a work acknowledgement.
See [Flush on exit](./state-and-resume.md#flush-on-exit).

</details>

<details>
<summary id="mixing-sync-and-async-clients">Other async tasks stop while Aviso is waiting</summary>

Synchronous methods block the event-loop thread. Use `AsyncAvisoClient` in async
code and await its HTTP methods. `listen()` returns an async iterator directly;
use `async for`, not `await client.listen(...)`. See [Async](./async.md).

</details>

</div>
