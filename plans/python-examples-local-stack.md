# python/examples/ against the local e2e stack

Follow-up to PR #22. PR #22 shipped the docker-compose stack (`aviso-server` + `auth-o-tron` + JetStream NATS), `tests/e2e/shared/stack.sh` lifecycle helper, and the Python + Rust e2e test suites. The natural downstream win the e2e plan flagged in D-S9 is to make the same stack the recommended "try the examples" path for users new to the Python client. This plan covers that follow-up.

The friction that this plan resolves: today the 15 example scripts under `python/examples/` assume an aviso-server that already has the `test_polygon` schema configured. A user evaluating the Python API has to find a server, get credentials, hope the schema is there, and only then can they run the examples. After this PR lands, a new user can `bash tests/e2e/shared/stack.sh up`, set three env vars, and every example just works.

## Status before this plan

- PR #22 ships the stack and the test suites. `tests/e2e/aviso-server.config.yaml` mirrors the bologna production overlay: `test_polygon` with `auth.required: true`, `write_roles: ["producer"]`, `allow_duplicates: true`. Three test accounts: `admin-user`, `reader-user`, `producer-user`.
- `python/examples/README.md` currently documents the workflow against a generic aviso-server (export `AVISO_BASE_URL`, `AVISO_USERNAME`, `AVISO_PASSWORD`, then `python python/examples/basics/01_publish.py`). It also includes a "if your server has no test_polygon" fallback that points at a copy-paste schema snippet in `docs/src/python/quickstart.md`.
- `_common.py::require_env()` validates the three env vars and exits with a clear error if any are missing. The 15 scripts all call `aviso.Env()` which reads those env vars.
- Three examples were spot-checked against the local stack during PR #22 (basics/01, basics/03, triggers/01). The remaining 12 are NOT validated.

## Scope: what this PR changes

### 1. Validate every example against the local stack

Twelve un-validated example files. For each, the validation step is the same: bring the stack up, export the three env vars, run the script, check it terminates cleanly with expected output. Some examples will need code adjustments to be reliable; the matrix below pins the changes.

| Example | Hardcoded `(date, time)` | Expected behaviour | Likely change needed |
|---|---|---|---|
| `basics/01_publish.py` | `(20260601, 1200)` (3 publishes, 1 s apart) | Each publish prints `published #N: status=success request_id=... processed_at=...`. | None (validated). |
| `basics/02_listen.py` | reads from `basics/01_publish.py`'s polygon | Iterates 3 notifications via `with client.listen(...) as iterator:` then exits. | Validate; the publish-listen race timing has been tuned in PR #21. |
| `basics/03_schema_discovery.py` | n/a | Prints event types, prints schema for `test_polygon`. | None (validated). |
| `triggers/01_echo.py` | reads from publisher | Echo NDJSON to stdout for each notification, 3 times. | None (validated). |
| `triggers/02_log.py` | reads from publisher | Log NDJSON to a tempdir file, 3 times; print final tail of the log. | Validate. |
| `triggers/03_multiple.py` | reads from publisher | Two triggers fire on each notification. | Validate. |
| `triggers/04_command.py` | reads from publisher | Spawn a child command per notification with `AVISO_*` env vars set. | Validate. POSIX-only; document on README. |
| `triggers/05_webhook.py` | placeholder URL `https://example.com/hook` | Currently NOT-RUNNABLE (placeholder URL). Promote to runnable using the local-HTTP-server pattern from `advanced/03_webhook_with_local_server.py`, OR keep as construction-only and remove the not-runnable label once the local-stack path is documented. Decide during implementation. | Possibly replace with a runnable variant. |
| `resilience/01_resume_with_state_store.py` | reads from publisher | Listen with `JsonFileStore`, exit, re-listen, no replay. | Validate; verify the example uses `temp_dir()` (so it does not pollute `~/.config/aviso/state.json`). Currently uses default path - **fix needed**. |
| `resilience/02_error_handling.py` | n/a | Construct an `AvisoClient` against a bad URL or trigger a known error path; assert specific exception class. | Validate; the bad-URL pattern might need adjustment if it currently hits `aviso-server.ecmwf.int` (which is no longer the assumed default). |
| `async/01_basic.py` | reads from publisher | Iterate one notification via `async with client.listen(...) as iterator:`. | Validate. |
| `async/02_multiplex.py` | reads from publisher | Two concurrent async listeners via `asyncio.gather`. | Validate. |
| `advanced/01_builder_pattern.py` | reads from publisher | Same as basics/02 but via `WatchRequest.watch(...).with_filter(...)` builder. | Validate. |
| `advanced/02_replay_only.py` | reads from publisher | Replay mode (terminates when stream reaches end_of_stream). | Validate; the example may need a `from_id` or `from_date` that has data on the local stack. |
| `advanced/03_webhook_with_local_server.py` | n/a (self-contained) | Spawn an in-process HTTP server, configure webhook trigger to hit it, publish one, assert the server got a POST. | Validate; already self-contained. |

### 2. Update `python/examples/README.md`

Add a new top section, BEFORE the existing "Prerequisites":

```markdown
## Run the examples against the local stack (recommended)

The fastest way to try the examples is the docker-compose stack from `tests/e2e/`:

```bash
# bring the stack up (auth-o-tron + aviso-server with test_polygon configured + NATS JetStream)
bash tests/e2e/shared/stack.sh up

# point the examples at the local stack
export AVISO_BASE_URL=http://localhost:8000 \
       AVISO_USERNAME=producer-user \
       AVISO_PASSWORD=producer-pass

# run any example
python python/examples/basics/01_publish.py

# when done
bash tests/e2e/shared/stack.sh down
```

The stack ships `test_polygon` already configured with auth required, write role `producer`. The `producer-user` account has both read and write permissions. See [`tests/e2e/README.md`](../../tests/e2e/README.md) for the other two test accounts (`admin-user`, `reader-user`) and the full lifecycle commands.

## Run the examples against your own server

If you have an aviso-server you can use, set the same three env vars to point at it. The examples assume `test_polygon` is configured on the server; see [the quickstart's "What is on your server" section](../../docs/src/python/quickstart.md#what-is-on-your-server) for the schema snippet to add to your aviso-server config if it is not.
```

Existing "Prerequisites" and "Layout" sections stay, possibly shortened (some of the schema-substitution prose is no longer needed if the local stack is the lead path).

### 3. Update `docs/src/python/quickstart.md`

The "What is on your server" section currently presents the schema snippet as the canonical fallback. Reframe as two options:

> If your server does not have `test_polygon`, two paths:
>
> 1. **Spin up the local stack** included in this repo (`bash tests/e2e/shared/stack.sh up`); it ships the schema pre-configured. See [`python/examples/README.md`](../../python/examples/README.md#run-the-examples-against-the-local-stack-recommended) for the per-step flow.
> 2. **Add the schema to your existing aviso-server**'s config. Copy-paste:
>     ```yaml
>     ... (existing snippet)
>     ```

### 4. Address the env-var friction

The user (correctly) flagged that the env-var setup is friction. Three options to consider in implementation, pick one:

**Option A. Document only.** Keep `aviso.Env()` as-is in the examples; document the three exports in README + quickstart. User pastes three lines. Simplest, no new dependency. Minor friction.

**Option B. Ship `python/examples/source-me.sh`.** A shell script that exports the three local-stack vars:
```bash
#!/usr/bin/env bash
export AVISO_BASE_URL=http://localhost:8000
export AVISO_USERNAME=producer-user
export AVISO_PASSWORD=producer-pass
```
User runs `source python/examples/source-me.sh && python python/examples/basics/01_publish.py`. Slightly easier; still one extra command. Documented in `python/examples/README.md`.

**Option C. Smart default in `_common.py::require_env()`.** If `AVISO_BASE_URL` is unset but `localhost:8000` answers `/health` (or just check if it answers within 100ms), default to `http://localhost:8000` + `producer-user`/`producer-pass`. Most magical, but the magic surprises users who expect a "missing env var" error and instead get a working run. Reject.

**Recommendation: Option A.** The friction is real but the documentation cost is low and zero magic is added. Option B is an easy follow-up if option A turns out to be too friction-heavy for new users.

### 5. The `triggers/05_webhook.py` not-runnable status

Currently flagged `# AVISO_EXAMPLE_NOT_RUNNABLE` because it points at a placeholder URL. Now that `advanced/03_webhook_with_local_server.py` exists as the runnable webhook example, two options:

**Option A. Keep both, drop the placeholder label.** Make `triggers/05_webhook.py` a no-publish example: it constructs a `Trigger.webhook(url)` and prints the trigger description, exits without listening. Marks the API surface for users who only want to see the builder. Stays unsplit from the runnable cousin.

**Option B. Delete `triggers/05_webhook.py`** and renumber the rest. Cleaner directory, one webhook example to maintain.

**Option C. Promote `05_webhook.py` itself to runnable** using the same in-process HTTP server pattern; delete `advanced/03_webhook_with_local_server.py`. Reduces example count by 1.

**Recommendation: Option C.** The split-into-two-files pedagogy was a workaround for "real webhook URLs are not safe to test against in examples"; with the in-process pattern proven in `advanced/03_*`, that workaround is no longer needed.

## Validation harness

The fact-check harness at `/tmp/aviso-py-fact-check/run_doc_examples.py` walks every Python code block in `docs/src/python/*.md` and every example file under `python/examples/**/*.py`. After this PR lands the harness's server target switches from `aviso-server.ecmwf.int` to `http://localhost:8000` and credentials come from the local stack producer account.

This is the right time to also clean up the harness's hardcoded knobs: the sidecar publisher's polygons (`0,0,1,0,1,1,0,0` and `2,2,3,2,3,3,2,2`) should match what the examples use; the `<!-- not-runnable -->` and `# AVISO_EXAMPLE_NOT_RUNNABLE` markers may need updating depending on the `triggers/05_webhook.py` decision.

## Open questions

1. **`resilience/01_resume_with_state_store.py` state file path**: the example may currently write to `~/.config/aviso/state.json`. That pollutes the user's machine state across runs and bleeds into the local-stack workflow. Audit during implementation; switch to `temp_dir()` if so.
2. **`resilience/02_error_handling.py` error trigger**: needs validation that the chosen error trigger still produces the asserted exception class against the local stack (the local server returns different error bodies than `aviso-server.ecmwf.int` in some cases; specifically auth-required schemas return 401/403 where a no-auth dev server might have returned a different shape).
3. **`advanced/02_replay_only.py` `from_id` value**: the example needs a sequence/date that has data on the local stack. Either the example publishes first then replays, or it documents the prerequisite.
4. **Should the examples README's old "Prerequisites" section stay?** Specifically the schema-fallback prose. Probably reduce to one line pointing at the local stack as the primary; defer the operator schema-snippet path to the quickstart cross-reference.

## Roll-out

Three focused commits:

1. **Commit 1: `fix(py-examples): per-example adjustments for the auth-required local stack`**. Walk through all 15 examples; fix each one to:
   - Use `temp_dir()` for any state-store / log-file path (audit current state of `resilience/01`).
   - Use `producer-user` credentials implicitly via `aviso.Env()` (no example-side change; just confirm the env-driven path works).
   - Verify the publish/listen race and the `with client.listen(...) as iterator:` shape work against the local stack.
   - Apply the `triggers/05_webhook.py` decision (Option C: promote to runnable, delete `advanced/03_*`).
   Manually validate every example end-to-end against the running local stack.

2. **Commit 2: `docs(py-examples): lead with the local-stack workflow`**. Rewrite `python/examples/README.md` per section 2 above; rewrite per-directory READMEs only where they reference the old prerequisites. Update `docs/src/python/quickstart.md`'s "What is on your server" section per section 3 above.

3. **Commit 3: `docs(fact-check): point the doc-examples harness at the local stack`**. Update the maintainer's fact-check harness's defaults to point at the local stack. Useful for the maintainer; not user-facing.

## What stays the same

- The 15-script structure (`basics/`, `triggers/`, `resilience/`, `async/`, `advanced/`).
- The `_common.py` helpers (`require_env`, `break_after`, `temp_dir`).
- The `aviso.Env()` env-var conventions.

## Status snapshot

- Branch: `feat/python-examples-local-stack` off `main` at `8461466` (the PR #22 merge commit).
- Plan: this file.
- **Implemented** in 4 focused commits on the branch:
  1. `fix(py-examples): replay_only example self-publishes and uses from_=0 to mean stream start` (`b5778b2`).
  2. `refactor(py-examples): consolidate triggers/05_webhook with advanced/03 into one runnable example` (`e080a33`).
  3. `fix(py-examples): align expected output and force-flush prints under captured stdout` (`e05d5ef`).
  4. `docs(py-examples): lead with the local-stack workflow in README and quickstart` (`2d6710d`).
- Per-example walk: all 14 examples (15 minus the consolidated webhook) pass against the local stack. The maintainer's fact-check harness at `/tmp/aviso-py-fact-check/run_doc_examples.py` reports 47 / 47 runnable blocks pass, 0 fails.
- Decisions taken (all three matched the plan's recommendations):
  - Env-var friction: **Option A** (doc-only exports). No new files; the README documents the three env vars inline in the local-stack workflow.
  - Webhook example consolidation: **Option C**. `triggers/05_webhook.py` promoted to a runnable, self-contained example (in-process HTTP server + background publisher thread + delivery assertion); `advanced/03_webhook_with_local_server.py` deleted.
  - README prose retention: reduced. The "if your server has no `test_polygon`" content moved out of `python/examples/README.md` and into the quickstart, with a one-line cross-reference back from the examples README.
- The plan's Commit 3 (fact-check harness retarget) turned out to be a no-op: the harness already reads `AVISO_BASE_URL` / `AVISO_USERNAME` / `AVISO_PASSWORD` from the environment at runtime, and the polygon set in its sidecar publisher already matches what the examples use. No code change needed; the verification was running the harness against the local stack and seeing 47 / 47 pass.
- Depends on: PR #22 (the e2e stack + test_polygon schema with `allow_duplicates`). PR #22 merged at `8461466` before this work started.
