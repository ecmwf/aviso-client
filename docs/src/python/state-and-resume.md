# State and resume

A state store saves a **cursor**, the sequence position used to resume a
listener. This helps a restarted script read retained notifications published
while it was stopped. It does not record whether your Python analysis finished.

**A saved cursor is not an acknowledgement of completed work.** The client can
save progress while notifications are still waiting in the iterator's buffer.
A crash can therefore leave application work unfinished even for a saved
sequence. Make repeated processing safe and track completed work separately
when it matters. The cursor alone gives neither lossless processing nor
exactly-once execution.

## A complete resuming listener

Use the [quickstart environment](./quickstart.md#set-the-environment), including
`AVISO_BASE_URL` and credentials for `pyaviso.Env()`. For an anonymous server,
omit `auth=pyaviso.Env()` from the client initialization.

This uses the [small `mars` schema](./quickstart.md#what-is-on-your-server):
`class` is a required choice of `od` or `rd`; `step` is a whole number optional
in filters. Omitting it receives all steps. Providers supply both identifiers;
the payload is optional. You only need receiving permission for this script.

Save this as `listen_saved.py` and run `python listen_saved.py`:

```python
import os
from pathlib import Path

import pyaviso

state_path = Path.home() / ".config" / "aviso" / "state.json"
state_path.parent.mkdir(parents=True, exist_ok=True)
client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=pyaviso.Env(),
    state_store=pyaviso.JsonFileStore(state_path),
)

try:
    with client.listen("mars", filter={"class": "od"}) as notifications:
        for notification in notifications:
            print(notification)
except KeyboardInterrupt:
    print("Stopped listening")
```

It prints each original CloudEvent as indented JSON. Press Ctrl+C to stop. With
no saved cursor, the first run waits for new notifications. Later runs with the
same settings resume after the saved sequence, if one exists. The last printed
notification may be delivered again. If nothing was saved, restarting begins at
the live edge, so do not assume a one-notification first run has saved progress.

For a local trial, start this listener, then run the
[provider publish script](./publish.md#a-complete-publish-script) in another
terminal. Publish several different steps, stop the listener, publish another
step, then restart it. What remains available depends on server retention.

## Where state lives

- With `state_store=None` (the default), there is no saved state across runs.
- `MemoryStore()` holds checkpoints in memory for clients using that store
  instance. They disappear when the process exits.
- `JsonFileStore(path)` writes checkpoints to a local JSON file, using an atomic
  rename and a sidecar lockfile for cooperating writers.

The example creates the parent directory first. Choose a local path your user
can write to, including permission to create the lockfile and replace the state
file. Relative paths are relative to the working directory. `~` is expanded.
Keep the state file if you want to resume; deleting it discards that position.

## Local filesystems only

Use local storage rather than NFS or CIFS for `JsonFileStore`. File locking does
not make several listeners a work-sharing queue: they can receive the same
notifications. The Python API accepts the two built-in stores, not arbitrary
custom Python store objects.

## How it works

The Python client derives a resume key from the server URL, event type and
filter. Changing one of these can select a different key with no saved cursor.
A server schema change alone does not change the key.

For each notification, the background listener:

1. Saves the previous pending sequence, if any.
2. Runs this notification's triggers.
3. Sends it to the iterator's buffer.
4. Records its sequence as pending if it advances the current position.

These steps do not wait for your Python loop to finish processing the previous
notification. A required trigger failure prevents the failing notification from
becoming pending, but the previous position may already have been saved.
Checkpoints do not move backwards within a watch session; out-of-order
notifications can still reach triggers and your loop.

Redelivery depends on retained history and a usable cursor. A sequence is a
resume boundary, not a separate acknowledgement for every event. See
[Listening](./listen.md#replay-only) for retention and replay limits.

## Choose a starting position

An explicit `start_from` takes precedence over saved state. An integer is
exclusive: `start_from=1024` reads after sequence 1024; `start_from=0` requests
all retained history after zero. A UTC timestamp such as
`start_from="2026-06-01T00:00:00Z"` selects publication time, not identifier
labels. Subsequent checkpoints use sequences.

With `start_from=None`, a matching saved cursor is used if available; otherwise
listening starts live. To deliberately start fresh without affecting existing
state, construct a client without that state store. Do not delete your state
file just to investigate a problem.

## Flush on exit

Keep the default `flush_cursor_on_exit=False` for a processing script unless you
have a reason to advance the final pending position during shutdown. With the
default, the last pending sequence is not saved just because the iterator
closes. It may be replayed on restart, provided a usable starting position and
retained record exist. This does not protect all unfinished buffered work:
earlier checkpoints can already be ahead of your processing.

Setting `flush_cursor_on_exit=True` on the client attempts to save the last
pending cursor on shutdown. This can reduce final-notification repeats for a
display-only listener. It can also skip unfinished application work on restart,
including after your loop raises an exception. A failed storage write can
prevent the flush. It is not a successful-work acknowledgement.

The iterator's `with` block calls `close()`, which cancels and waits for the
background listener, including any exit-flush attempt. A `break` alone does not
close an iterator you still hold outside a context manager.

## With `AsyncAvisoClient`

Use the same store and constructor options. Wrap the returned iterator in
`async with`; it awaits `aclose()` on exit. See the complete
[async listener](./async.md#a-complete-async-listener). The checkpoint and
unfinished-work limits above apply equally to async code.
