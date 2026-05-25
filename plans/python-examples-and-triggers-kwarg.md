# Python examples + `triggers=` kwarg

A focused plan covering two related improvements to the Python API that a fresh reader of the docs surfaced:

1. The `client.listen(...)` kwargs path covers `event_type`, `filter`, `from_`, `mode`, `request` but not triggers; the moment a user wants to attach a trigger they are forced into the `WatchRequest.watch(...).with_filter(...).with_triggers([...])` builder. Two paths through the API, the simpler one missing one common surface, the more verbose one being the only way to reach that surface. Real friction; reader explicitly flagged it as confusing.
2. We ship no example scripts. `pyaviso` had [aviso-examples](https://github.com/ecmwf/aviso-examples/tree/main/examples/python-api): four trigger kinds (echo, function, log, post) each with an env-based and a config-based variant. Useful as a structural reference. The pyaviso version is repetitive (each file repeats request setup, env check, listener-wrap boilerplate) and the trigger is buried under that scaffolding. We can do better by leaning on the new kwargs-primary API.

This plan covers both because they touch the same surface and would conflict if shipped separately.

## Motivation in one sentence

A first-time Python user should be able to read a 10-line example, copy it, swap in their own server URL plus event type, and have it work; everything beyond that should be incremental.

## Design decisions

### D-K1. `triggers=` is added to `listen(...)` as a keyword-only argument

Both `AvisoClient.listen` and `AsyncAvisoClient.listen` accept `triggers: Sequence[Trigger] | None = None` (a list in practice, but `Sequence` in the stub for callers passing an immutable tuple or similar). The implementation rejects a bare `Trigger` (no isinstance branch in the binding; the type checker catches the missing brackets at call sites). When set, the supervisor attaches the triggers to the watch request the kwargs build. The common case becomes:

```python
for n in client.listen(
    "test_polygon",
    filter={"polygon": "0,0,1,0,1,1,0,0"},
    triggers=[aviso.Trigger.echo(), aviso.Trigger.log("/tmp/out.log")],
):
    process(n)
```

`WatchRequest` stays exported for the build-once-reuse-many case. The docs lead with the kwargs path; the builder gets one dedicated subsection on `listen.md` and one dedicated example file under `python/examples/advanced/`.

### D-K2. Conflict detection mirrors the existing `request=` + `mode=` rule

If both `request=` and `triggers=` are passed, the call errors with the same shape as the existing `request= + mode=` conflict. The message is `"triggers= cannot be combined with request=; add triggers to the WatchRequest instead"` (not "the request already carries its own triggers" — a triggerless `WatchRequest` is a legitimate construction, so the message has to describe the kwarg/builder mutual exclusion, not invent a state the request might not be in). The kwargs path and the builder path stay mutually exclusive; nobody can construct an ambiguous request.

### D-K3.5. `NotificationIterator` and `AsyncNotificationIterator` become context managers

The merged API requires `try/finally` plus `iter.close()` for clean teardown when `flush_cursor_on_exit=True` is set. That ceremony is what the planned `advanced/02_explicit_close.py` example existed to teach. The cleaner answer is to make the iterators support `with`/`async with`:

```python
with client.listen("test_polygon", filter={...}, triggers=[...]) as iterator:
    for n in iterator:
        process(n)
# close() is called on exit, even if the loop body raises or breaks
```

`PyNotificationIterator` gains `__enter__` (returns self) and `__exit__` (calls `close()`). `PyAsyncNotificationIterator` gains `__aenter__` and `__aexit__` (calls `aclose()`). Stubs updated. Two short tests cover the context-manager shapes (sync + async). This removes the only previously-documented lifecycle ceremony from the API and lets us cut `advanced/02_explicit_close.py` entirely; the resume example demonstrates the `with` form alongside `flush_cursor_on_exit=True`.

### D-K3. `WatchRequest` stays public, demoted in docs

We do not move `WatchRequest` to `aviso._internal`. It is still exported and type-checked because:
- Power users do construct one from configuration and reuse it across `client.listen()` calls.
- Removing it would be a breaking API change two days after the package merged.
- The type alias is small (one class) and pyright/mypy/ty users benefit from typing it.

But the user-facing pages stop leading with it. `listen.md` keeps a single "Reusing a watch request" subsection at the bottom; `triggers.md` drops the builder from the lead example; `api-reference.md` keeps the full description because it is a reference page.

### D-E1. Examples live in `python/examples/`, grouped thematically

```
python/examples/
├── README.md                  ← index, when-to-pick-which
├── basics/
│   ├── README.md
│   ├── 01_publish.py
│   ├── 02_listen.py
│   └── 03_schema_discovery.py
├── triggers/
│   ├── README.md
│   ├── 01_echo.py
│   ├── 02_log.py
│   ├── 03_multiple.py
│   ├── 04_command.py          ← Unix-only; gated with a skip-on-Windows note
│   └── 05_webhook.py
├── resilience/
│   ├── README.md
│   ├── 01_resume_with_state_store.py
│   └── 02_error_handling.py
├── async/
│   ├── README.md
│   ├── 01_basic.py
│   └── 02_multiplex.py
└── advanced/
    ├── README.md
    ├── 01_builder_pattern.py  ← same scenario as triggers/01_echo, side-by-side comparison
    ├── 02_explicit_close.py
    └── 03_replay_only.py
```

The numeric prefix gives reading order within a directory. The thematic grouping lets a reader say "I want to see how listening works" or "I want to see how triggers work" without scrolling through 15 flat files. Every script is standalone, runnable, and includes a module docstring that names the scenario plus the expected output.

### D-E2. Every example reads `AVISO_BASE_URL` + `aviso.Env()` and gives a clear error if missing

Same pattern as the docs. Reader sets two env vars once; every example runs. If the env vars are missing, the script prints a one-line "set AVISO_BASE_URL plus AVISO_USERNAME/PASSWORD or AVISO_TOKEN" message and exits non-zero rather than crashing with a confusing `KeyError`. A `_common.py` module under `python/examples/` carries the env check + a tiny `bounded_listen` helper used by the listener examples (see D-E6) so each example body stays focused on the scenario, not the boilerplate.

### D-E6. Listener examples terminate deterministically

Every listener example breaks out of its `for n in client.listen(...)` loop after a fixed N notifications (3 by default), printing a final line such as `received 3 notifications; exiting`. This keeps the harness hermetic: the example runs end-to-end without depending on a soft kill, and a user copying the example sees explicit "I stop after N" code they can remove for a real always-on listener. The pattern is one helper line in `_common.py` (`break_after(iterator, n)`); each example body stays focused on the scenario.

### D-E7. Examples with side effects use deterministic temp paths

Log, command, and resume examples write to `tempfile.NamedTemporaryFile` paths or `tempfile.mkdtemp` directories rather than hardcoded `/tmp/...` paths. Two parallel example runs do not collide; a re-run of the harness does not see stale data from a previous run.

### D-E8. Webhook example pair: simple (non-runnable construction) + advanced (runnable against a local server)

`triggers/05_webhook.py` shows the API shape with a placeholder URL (`https://your-collector.example/notify`); it is marked non-runnable in the harness because it would otherwise spam a placeholder host. `advanced/03_webhook_with_local_server.py` ships the runnable variant: starts a `http.server.HTTPServer` in a background thread that accepts POSTs and exits after recording one delivery, then runs the listener with `Trigger.webhook("http://127.0.0.1:<port>/...")`. Two files because the API-shape lesson is small and the end-to-end-runnable lesson needs the local-server scaffolding; mixing both into one file would muddy both.

### D-E3. The builder pattern gets one dedicated example, not one variant per scenario

`pyaviso-examples` shipped `<trigger>-env.py` AND `<trigger>-config.py` for every trigger — eight files where four would do, because the config-vs-env split is orthogonal to the trigger kind. We do not repeat that mistake. The builder pattern gets exactly one file (`advanced/01_builder_pattern.py`) that takes the simplest trigger example (`triggers/01_echo.py`) and shows it again with the builder, with the diff explained in the docstring. Readers learn the pattern once and apply it to whatever scenario they need.

### D-E4. Each example is fact-checked end-to-end against `aviso-server.ecmwf.int`

Re-uses the existing `/tmp/aviso-py-fact-check/` harness. The harness extends to walk `python/examples/**/*.py` in addition to `docs/src/python/*.md` code blocks. Same sidecar publisher publishing test_polygon notifications so listener examples receive notifications during their soft timeout window.

### D-E5. Each directory carries a `README.md` indexing its examples; root `python/examples/README.md` indexes the directories

Reader landing on the repo can browse `python/examples/` on GitHub and immediately see what is there without opening files. Per-directory READMEs say one paragraph per example.

## Example scenarios (the actual list)

| Path | What it shows |
|---|---|
| `basics/01_publish.py` | Construct a client, publish one notification, print the response. |
| `basics/02_listen.py` | Construct a client, listen for notifications with a filter, print each. |
| `basics/03_schema_discovery.py` | Call `schema()` and `schema_for(...)`, print the available event types and one schema. |
| `triggers/01_echo.py` | Listen with `Trigger.echo()` attached via `triggers=` kwarg. Server-side dispatch demonstrated. |
| `triggers/02_log.py` | Listen with `Trigger.log(path)` writing NDJSON to a file. |
| `triggers/03_multiple.py` | Combine `Trigger.echo()` + `Trigger.log(path)` in one watch. |
| `triggers/04_command.py` | `Trigger.command("./process.sh")` (Unix only; one-line skip on Windows). |
| `triggers/05_webhook.py` | `Trigger.webhook("https://...")` with method + headers + body template. |
| `resilience/01_resume_with_state_store.py` | Listen with `JsonFileStore` so restarts pick up the last cursor. |
| `resilience/02_error_handling.py` | Catch `HttpError`, `HistoryGapError`, `TriggerError`, demonstrate `.status`, `.reason`, `.trigger_kind` access. |
| `async/01_basic.py` | The simplest `AsyncAvisoClient` listener using `async for`. |
| `async/02_multiplex.py` | Drain two streams concurrently with `asyncio.gather`. |
| `advanced/01_builder_pattern.py` | Same scenario as `triggers/01_echo.py` but built with `WatchRequest.watch(...).with_filter(...).with_triggers([...])`. Shown for comparison; the docstring walks the diff. |
| `advanced/02_replay_only.py` | `mode="replay_only"` with `from_=N` for a backfill script that exits at end-of-stream. |
| `advanced/03_webhook_with_local_server.py` | The runnable counterpart to `triggers/05_webhook.py`: spins up a local `http.server.HTTPServer` to receive the webhook POST and asserts a delivery before exit. |

15 example files. Each file is approximately 30-50 lines including the docstring. The cut `advanced/02_explicit_close.py` is replaced by the context-manager form demonstrated in `resilience/01_resume_with_state_store.py` (per D-K3.5).

## Roll-out

Four commits on `feat/python-examples-and-kwarg`. The first two are split (per the design review): both are public API changes and each deserves its own focused commit.

1. **Commit 1: iterator context-manager support.** Add `__enter__`/`__exit__` to `PyNotificationIterator` and `__aenter__`/`__aexit__` to `PyAsyncNotificationIterator` (`crates/aviso-py/src/streams.rs`). Update `python/aviso/__init__.pyi` and `python/aviso/_native.pyi`. Two tests in `python/tests/test_iter_close.py`: sync `with`, async `async with`.

2. **Commit 2: `triggers=` kwarg.** Add the kwarg to `AvisoClient.listen` and `AsyncAvisoClient.listen` (Rust, `crates/aviso-py/src/clients.rs`). Update `build_watch_request` to apply `triggers=` when `request=` is absent and to error when both are passed (including when `triggers=` is an empty sequence — any non-None value alongside `request=` is the conflict). Reject bare `Trigger` with a clear runtime error ("triggers= must be a sequence of Trigger; pass [trigger] for a single one"); accept tuples and lists alike via Python sequence extraction. Stubs updated to `Sequence[Trigger] | None`. A brief `api-reference.md` signature touch keeps the reference in sync. Four tests in `python/tests/test_clients_sync.py`: kwargs happy path, kwargs+request conflict (with both non-empty and empty `triggers`), tuple acceptance, bare-`Trigger` rejection.

3. **Commit 3: `python/examples/` tree + harness extension.** All 15 example files plus 5 READMEs (root + 4 directory) plus a `_common.py` helper (env check, `break_after(iterator, n)`, `temp_dir()` context-manager wrapping `tempfile.TemporaryDirectory` with helper paths for child files). Each example carries a tiny top-of-file path-bootstrap (`sys.path.insert(0, str(Path(__file__).resolve().parents[1]))`) so users can run `python python/examples/triggers/01_echo.py` without manual `PYTHONPATH`. Non-runnable examples are marked with a top-of-file `# AVISO_EXAMPLE_NOT_RUNNABLE: <reason>` comment that the harness scans for (the harness already handles `<!-- not-runnable -->` for markdown blocks; the new comment marker is the example-script equivalent). Side-effect examples use `tempfile.TemporaryDirectory` + named child files, never `NamedTemporaryFile`, to avoid races between the trigger writer and the example's verification step. The doc-examples harness extends to walk `python/examples/**/*.py` in addition to `docs/src/python/*.md` code blocks. The local-server webhook example uses `http.server.HTTPServer` with a randomly-bound port (`server_address=("127.0.0.1", 0)`, then read `server.server_port`).

4. **Commit 4: docs rewrite.** `docs/src/python/triggers.md` lead example becomes the kwargs path; the "Low-level WatchRequest" subsection on `listen.md` becomes "Reusing a watch request" and is the final subsection of the page (not embedded mid-flow); the explicit-close paragraph at the bottom of `listen.md` is rewritten to use the new `with` form; the same rewrite lands in `state-and-resume.md` and `troubleshooting.md` where the old try/finally pattern was the documented answer. `docs/src/python/api-reference.md` signature line picks up the context-manager methods on the iterators. Fact-check the doc snippets again with the extended harness; the count rises with the new runnable example files, all pass.

## What stays the same

- The 6 trigger kinds (`echo`, `log`, `command`, `webhook`, `teams`, `post`) and their chainable setters.
- Auth providers (`Bearer`, `Basic`, `Env`, `ConfigFile`, `Chain`) and their constructors.
- State stores (`MemoryStore`, `JsonFileStore`).
- Exception hierarchy.
- Value types (`Notification`, `NotifyResponse`, `SchemaCatalog`, `SchemaResponse`).
- `WatchRequest` is still exported with the same surface (still `.watch()`, `.watch_from()`, `.replay_only()`, `.with_filter()`, `.with_triggers()`).

## What changes for users

- Common case becomes one method call instead of method-call-plus-builder. The kwargs path covers every trigger scenario without ever touching `WatchRequest`.
- The docs lead with the kwargs path. The builder is documented in one place (one subsection in `listen.md`, one example in `advanced/`) framed as "if you want to reuse the same configured request across multiple `listen()` calls, here is the builder".
- `listen(request=..., triggers=[...])` is a new error (it was a silent ambiguity before, kind of — triggers could only be attached to the request, so passing triggers= AND request= was not expressible; the explicit error catches the case if/when users try it).

## Validation

- Local pre-push gates (cargo + ruff + ty + 110+ pytest tests).
- New tests for the kwarg and the conflict (Commit 1).
- mdbook build + test clean (after Commit 3).
- Doc-examples harness extended to walk `python/examples/**/*.py`; every example runs against `aviso-server.ecmwf.int` with the sidecar publisher; all pass.

## Out of scope

- Renaming the Python distribution (that is the Phase 7 / PyPI rename question; this work is name-agnostic and the examples will be renamed automatically when the rename PR lands).
- Wheel matrix / PyPI publish workflow (Phase 7).
- Mock-server pytest harness (a follow-up; the live-server fact-check is enough for this PR's validation purposes).
- Removing `WatchRequest` from the public surface (kept exported per D-K3).
- Pure-Python CLI or bundled-Rust-binary CLI work (the separate Phase 7 question that the user wants to discuss later).

## Design review settlements

The pre-implementation design review (oracle round 1) closed these:

1. **`triggers=` accepts a `Sequence[Trigger]`, never a bare `Trigger`.** Type checker catches the missing brackets at call sites; no isinstance branch in the binding. (D-K1.)
2. **Per-directory READMEs are lists, not tables.** Short directories (3-5 entries each), prose lists read better.
3. **`python/examples/` is repository-only.** Not installed as package data, not importable. Reached by cloning the repo or browsing GitHub.
4. **`WatchRequest` stays public.** The complaint was not that the builder exists; it was that triggers forced users into it. Once kwargs cover triggers, the builder becomes a legitimate advanced surface for the reuse case.
5. **Conflict error message wording.** "triggers= cannot be combined with request=; add triggers to the WatchRequest instead" (the proposed "the request already carries its own triggers" was false for a triggerless `WatchRequest`).
6. **Iterator becomes a context manager.** `__enter__/__exit__` on sync, `__aenter__/__aexit__` on async; the `try/finally + close()` pattern from the merged API becomes `with` / `async with`. (D-K3.5.)
7. **Cut `advanced/02_explicit_close.py`.** Subsumed by the context-manager addition; resume example demonstrates the `with` form alongside `flush_cursor_on_exit=True`. (Replaced in the advanced/ slot by `03_webhook_with_local_server.py`.)
8. **Listener examples terminate deterministically + use deterministic temp paths + webhook example is paired (construction-only + runnable-with-local-server).** Per D-E6, D-E7, D-E8.

## Status snapshot

- Branch: `feat/python-examples-and-kwarg`
- Plan: this file
- Not yet implemented
