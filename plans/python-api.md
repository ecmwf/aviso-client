# Python API plan

The Rust core, the Rust CLI, and the documentation tree are all in place. This plan turns the
existing PyO3 scaffold under `crates/aviso-py/` and `python/aviso/` into a real, ergonomic
Python API for `aviso-server`. The end state for this PR: a Python user clones the repo, runs
`uv sync && uv run maturin develop`, writes `import aviso`, and has a first-class library that
feels like httpx or async-pg, not a thin wrapper around a subprocess. Wheels on PyPI come in a
follow-up PR.

This file is the plan for one PR (`feat/python-api`). The plan is intentionally bounded: bindings,
docs, tests, CI integration. It explicitly excludes a Python-side CLI wrapper, a wheel matrix,
PyPI publishing, custom-Python-side `StateStore`/`AuthProvider` impls, and a Python callback
trigger. Each is captured in §11 with a reason.

---

## 1. Goal and non-goals

### Goal

A complete, well-tested, well-documented Python API for `aviso` that exposes the Rust core's
publish-notify-listen-replay-trigger surface to Python users in an idiomatic way. The user-facing
guarantees:

- `import aviso` works.
- `client = aviso.AvisoClient(base_url="...")` constructs a sync client.
- `client = aviso.AsyncAvisoClient(base_url="...")` constructs an async client.
- `client.notify(...)` publishes a notification, sync or async depending on which class.
- `for notification in client.listen(...)` iterates live notifications, sync.
- `async for notification in client.listen(...)` iterates live notifications, async.
- Auth, triggers, state store, and schema discovery are all reachable from Python with the same
  semantics as the Rust core.
- Errors are typed exceptions, not opaque strings.
- Type hints are complete; `ty check` passes strict.
- `pytest` runs without a network connection (mock server); integration tests against a real
  `aviso-server` are opt-in via env var.
- `mdBook` documentation under `docs/src/python/` covers install, quickstart, listening,
  publishing, triggers, auth, state, errors, and troubleshooting; the existing
  `docs/src/python/status.md` placeholder is replaced.

### Non-goals (this PR)

- A Python-side CLI wrapper. The Rust CLI is the canonical CLI; replicating it in Python adds a
  second surface to maintain with no benefit. Python users who need a CLI run `aviso ...` from a
  shell or `subprocess.run`; the Python *library* is for in-process embedding.
- A wheel matrix or PyPI publishing. The PR establishes the bindings and a single-platform CI
  job (Linux + macOS, same matrix as the Rust workflow). A separate PR builds the full wheel
  matrix (manylinux x86_64/aarch64, macOS universal2, Windows x86_64) with `cibuildwheel` or
  `maturin-action`.
- Custom Python-side `StateStore` or `AuthProvider` implementations. The Rust traits are async
  and `Send + Sync + Debug`; exposing them as inheritable Python classes requires significant
  PyO3 plumbing (GIL handling, Python-side `__await__` integration). Both shipped Rust
  implementations are exposed as Python classes the user can pass through, which covers every
  current use case. A custom-implementation surface lands when someone asks for it.
- A Python callback / function trigger. The async iteration loop body already does what a
  function trigger would do: receive a notification, decide what to do with it. Adding a second
  way to express the same thing (a Python callable handed to the Rust supervisor) splits the
  surface and pulls in tricky cross-language async semantics. Operators who want declarative
  side effects use the four shipped trigger kinds (echo, log, command, webhook) plus the two
  HTTP-flavoured sugar kinds (teams, post); operators who want programmatic behaviour use the
  iteration loop.

---

## 2. Constraints in force

Everything in `AGENTS.md` applies, with these especially load-bearing:

- No `unwrap`/`expect`/`panic!` outside test code (Rust) and no bare `except:` (Python).
- No em-dashes (U+2014) anywhere committed.
- Diagrams use mermaid blocks; `text` blocks only for trees, JSON, log lines.
- One concern per commit; restructures split per concern; ~15 files / ~500 added lines as the
  hard rule of thumb.
- Latest stable deps; no pinned-patch versions on `cargo install` in CI.
- Astral stack on the Python side: `uv` for env management, `ruff check`/`ruff format`, `ty
  check` strict. `pytest` for tests.
- Mermaid diagrams in user docs auto-switch theme; no hand-picked colour variables.
- AGENTS.md Python rules: type hints on every public function, `from __future__ import
  annotations` at the top of every module, `@dataclass(frozen=True, slots=True)` for records
  where it fits, typed exceptions, `pathlib.Path` not `os.path.join`, f-strings only.

Hard rules from the project root:

- "Don't ever merge without my approval." Stop at the merge gate.
- "Don't ask questions until you finish." Proceed without further user interaction in this PR.
- "Don't write phases / oracle / pass references in the PR body or code." Plan content stays in
  `plans/`.
- "I intend to release also the finesse." finesse stays `publish = false` only because no second
  consumer needs it yet; the Python bindings do not turn finesse into a second consumer (they
  consume the `aviso` crate's `Stream`, not the parser directly).

---

## 3. Public Python API surface

This section fixes what Python users see. Names, shapes, and ergonomics are the contract; the
Rust-side implementation follows.

### 3.1 Module layout

```text
aviso/
├── __init__.py        # curated public surface, re-exports from _native
├── __init__.pyi       # type stubs for the curated surface
├── _native.<abi3>.so  # compiled Rust extension (built by maturin)
└── py.typed           # PEP 561 marker (already present)
```

`aviso._native` is the raw extension. Users never import it directly; the curated surface in
`aviso.__init__` is the documented entry point. The split keeps the option open to add
pure-Python helpers (e.g., a future async-context-manager wrapper) without forcing them to live
in Rust.

### 3.2 Top-level surface

```python
from aviso import (
    # Clients
    AvisoClient,           # sync client
    AsyncAvisoClient,      # async client

    # Value types
    Notification,          # received notification
    NotifyResponse,        # publish-side response
    SchemaCatalog,         # GET /api/v1/schema response
    SchemaResponse,        # GET /api/v1/schema/{event_type} response

    # Watch shaping
    WatchRequest,          # explicit construction for power users
    ResumeStart,           # from-id vs from-date discriminator
    WatchMode,             # WATCH or REPLAY_ONLY

    # Triggers
    Trigger,
    HttpMethod,

    # Auth providers
    Bearer,
    Basic,
    Env,
    ConfigFile,
    Chain,

    # State stores
    MemoryStore,
    JsonFileStore,

    # Exceptions
    AvisoError,            # base
    TransportError,
    HttpError,
    AuthError,
    DecodeError,
    MalformedEventError,
    HistoryGapError,
    StreamProtocolError,
    ConfigError,
    StateStoreError,
    TriggerError,

    # Version
    __version__,
)
```

`__all__` in `__init__.py` enumerates exactly this list. Nothing else is part of the public
surface.

### 3.3 `AvisoClient` (sync) and `AsyncAvisoClient` (async)

Naming follows httpx (`httpx.Client` is sync, `httpx.AsyncClient` is async). The sync class is
the default name because most ECMWF Python users will start with synchronous scripts and grow
into asyncio as their workloads demand it.

```python
class AvisoClient:
    def __init__(
        self,
        *,
        base_url: str,
        auth: AuthProvider | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        heartbeat_interval: float = 30.0,
        state_store: StateStore | None = None,
        ca_bundle: bytes | None = None,
        danger_accept_invalid_certs: bool = False,
        flush_cursor_on_exit: bool = False,
    ) -> None: ...

    @property
    def base_url(self) -> str: ...

    def notify(
        self,
        *,
        event_type: str,
        identifier: dict[str, str] | None = None,
        payload: object | None = None,
    ) -> NotifyResponse: ...

    def schema(self) -> SchemaCatalog: ...
    def schema_for(self, event_type: str) -> SchemaResponse: ...

    def wipe_stream(self, stream_name: str) -> None: ...
    def wipe_all(self) -> None: ...
    def delete_notification(self, notification_id: str) -> None: ...

    def listen(
        self,
        event_type: str | None = None,
        *,
        filter: dict[str, object] | None = None,
        from_: int | str | None = None,
        mode: WatchMode = WatchMode.WATCH,
        triggers: list[Trigger] | None = None,
        request: WatchRequest | None = None,
    ) -> NotificationIterator: ...

    def close(self) -> None: ...
    def __enter__(self) -> AvisoClient: ...
    def __exit__(self, exc_type, exc, tb) -> None: ...


class NotificationIterator:
    def __iter__(self) -> NotificationIterator: ...
    def __next__(self) -> Notification: ...
    def close(self) -> None:
        """Cancel the supervisor and wait for any pending checkpoint flush.
        Idempotent; safe to call from a `finally` block."""

class AsyncNotificationIterator:
    def __aiter__(self) -> AsyncNotificationIterator: ...
    async def __anext__(self) -> Notification: ...
    async def aclose(self) -> None:
        """Async equivalent of `NotificationIterator.close`."""
```

`AsyncAvisoClient` has the same shape with `async def` on `notify`, `schema`, `schema_for`,
`wipe_*`, `delete_notification`. The `listen` method on the async client is a regular `def`
(NOT `async def`) that returns an `AsyncNotificationIterator` directly, so
`async for n in client.listen(...)` reads naturally. Marking `listen` `async def` would force
`async for n in (await client.listen(...))`, which is wrong and surprises Python users.

Both classes support `with` / `async with` for explicit lifecycle. The iterator types are
returned from `listen`, not constructible directly; their `close`/`aclose` methods are the
only public surface.

`flush_cursor_on_exit=True` on the client only works as documented when the iterator is
closed via `iter.close()` / `await iter.aclose()` before the Python process exits. A bare
`return` from the loop body (no explicit close) drops the iterator, cancels the supervisor,
but does not wait for the post-loop flush. The docs page for this option spells out the
contract; the test `test_iter_close_flushes_cursor.py` asserts the cursor is durable after
explicit close and may be one notification behind after a bare drop.

#### `listen` ergonomics

Two ways to call `listen`:

1. **High level**: positional event type plus keyword shaping.

   ```python
   for n in client.listen("mars", filter={"class": "od", "stream": "oper"}):
       print(n.sequence, n.payload)
   ```

2. **Low level**: build a `WatchRequest` and pass it. Convenient when constructing the request
   somewhere else (config loader, CLI bridge) and consuming it in another place.

   ```python
   req = WatchRequest.watch_from("mars", ResumeStart.after_sequence(42))
   for n in client.listen(request=req):
       ...
   ```

The two forms are mutually exclusive. Argument validation runs before any HTTP round-trip:

- `event_type` and `request` are mutually exclusive: passing both raises `ConfigError` naming
  the conflict.
- Passing neither raises `ConfigError`.
- When `request` is set, every high-level shaping kwarg (`filter`, `from_`, `triggers`) is
  rejected with `ConfigError`. `mode` is rejected unless it equals its default
  (`WatchMode.WATCH`), since the request already carries its mode.
- `from_=True` and `from_=False` are rejected (Python's `bool` is an `int` subclass, so a bare
  `True` would silently route as `from_id=1`). The error message names the type.
- Negative `from_` integers are rejected (sequences are `u64` on the wire).
- `mode=WatchMode.REPLAY_ONLY` requires `from_` (or a `request` whose mode is replay-only),
  matching the server's precondition.
- `from_` accepts an `int` (treated as `from_id` / `ResumeStart::AfterSequence`) or a `str`
  (treated as `from_date` / `ResumeStart::Date`). The int/str dispatch mirrors the CLI's
  `--from` flag and keeps the common case (resume from sequence N) a one-liner.

Each rejection produces a `ConfigError` whose message names the offending argument; the test
`test_listen_validation.py` exercises every rejection path.

#### Cancellation

- Async: drop the iterator (exit the `async for` body) and the supervisor cancels cooperatively
  per the Rust `Stream::Drop` path.
- Sync: drop the iterator (exit the `for` body) and the supervisor cancels via the same path.
- Explicit close: `iter.close()` (sync) and `await iter.aclose()` (async) consume the iterator,
  cancel cooperatively, and (when `flush_cursor_on_exit=True`) wait for the supervisor's final
  checkpoint flush via the Rust `NotificationStream::close()` path. Without an explicit close,
  the supervisor's pending checkpoint may not land before the Python process exits, which the
  fact-check workspace test catches as a "second run starts from the same notification we just
  observed".
- KeyboardInterrupt:
  - **Async**: asyncio's standard signal handling delivers `KeyboardInterrupt` to the awaiting
    task; the iterator is dropped via the `async for` `__exit__` path; the supervisor cancels.
  - **Sync**: each `__next__` polls `recv` with a 100 ms bounded timeout and calls
    `py.check_signals()` between polls. An idle stream that has not emitted a notification
    still responds to Ctrl+C within ~100 ms because the polling cadence drives the signal
    check. Without this polling shape, `block_on(recv)` on a quiet stream would block
    indefinitely; Ctrl+C would only land when the next notification or EOF arrives, which
    can be hours away on a heartbeat-only stream.

The "two-Ctrl+C-in-five-seconds hard exit" behaviour is a CLI policy, not a library policy.
The Python library raises `KeyboardInterrupt` on the first Ctrl+C and lets the caller decide.

### 3.4 Value types

`Notification`, `NotifyResponse`, `SchemaCatalog`, and `SchemaResponse` are PyO3 classes with
`#[pyo3(get)]` accessors, frozen from Python (no setters), and equipped with `__repr__`,
`__eq__`, and `as_dict()` so users can pipe into `json.dumps` or
`pandas.DataFrame.from_records`. These classes deliberately do NOT define `__hash__`: they
carry `dict` / `list` payload fields after Python conversion, and a value type that contains
unhashable inner values is itself unhashable per Python convention. Each class declares
`__hash__ = None` so a misuse like `set([notification])` raises `TypeError` loudly instead of
falling back to an id-based hash that would silently treat equal notifications as distinct.

```python
@dataclass(frozen=True, slots=True)  # logical shape; actual impl is a PyO3 class
class Notification:
    event_type: str
    sequence: int
    identifier: Mapping[str, str]
    payload: Any  # JSON-decodable
    cloudevent: Mapping[str, Any] | None  # raw envelope, present in real watch path

    def as_dict(self) -> dict[str, Any]: ...
```

`as_dict()` returns a plain Python `dict` with JSON-compatible values, so users can serialize
without thinking about it.

### 3.5 Triggers

A direct port of the Rust `Trigger` builder, with Python-friendly keyword constructors layered
on top. Class methods accept the per-trigger fields as keyword arguments; chainable setters
remain available for users who prefer the builder shape.

```python
class Trigger:
    @classmethod
    def echo(
        cls,
        *,
        retries: int = 0,
        required: bool = True,
        label: str | None = None,
    ) -> Trigger: ...

    @classmethod
    def log(
        cls,
        path: str | os.PathLike[str],
        *,
        retries: int = 0,
        required: bool = True,
    ) -> Trigger: ...

    @classmethod
    def command(
        cls,
        cmd: str,
        *,
        env: dict[str, str] | None = None,
        working_dir: str | os.PathLike[str] | None = None,
        retries: int = 0,
        required: bool = True,
        timeout: float | None = None,
        fail_fast: bool = True,
    ) -> Trigger: ...
    # Present on every platform; raises `ConfigError` on non-Unix.

    @classmethod
    def webhook(
        cls,
        url: str,
        *,
        method: HttpMethod = HttpMethod.POST,
        headers: dict[str, str] | None = None,
        body_template: str | None = None,
        retries: int = 0,
        required: bool = True,
        timeout: float = 30.0,
        fail_fast: bool = True,
    ) -> Trigger: ...

    @classmethod
    def teams(
        cls,
        url: str,
        *,
        title_template: str | None = None,
        retries: int = 0,
        required: bool = True,
        timeout: float = 30.0,
        fail_fast: bool = True,
    ) -> Trigger: ...

    @classmethod
    def post(
        cls,
        url: str,
        *,
        headers: dict[str, str] | None = None,
        retries: int = 0,
        required: bool = True,
        timeout: float = 30.0,
        fail_fast: bool = True,
    ) -> Trigger: ...

    # Chainable setters (alternative to constructor kwargs):
    def retries(self, n: int) -> Trigger: ...
    def required(self, on: bool) -> Trigger: ...
    def timeout(self, seconds: float) -> Trigger: ...
    def fail_fast(self, on: bool) -> Trigger: ...
    def label(self, name: str) -> Trigger: ...
    def env(self, key: str, value: str) -> Trigger: ...
    def working_dir(self, path: str | os.PathLike[str]) -> Trigger: ...
    def method(self, m: HttpMethod) -> Trigger: ...
    def header(self, name: str, value: str) -> Trigger: ...
    def body_template(self, body: str) -> Trigger: ...
    def title_template(self, body: str) -> Trigger: ...  # teams only
```

A setter that does not apply to the current trigger kind is silently ignored, matching the
Rust core's behaviour (`Trigger.echo().method(HttpMethod.POST)` returns the trigger unchanged).

`HttpMethod` is defined in the Python wrapper as `class HttpMethod(str, Enum)` (not
`enum.StrEnum`, which is 3.11+). Members: `POST`, `GET`, `PUT`, `PATCH`, `DELETE`. The string
values are the uppercase forms the Rust side already deserialises. Python users get
`aviso.HttpMethod.POST`; the comparison `HttpMethod.POST == "POST"` is True because of the
`str` base class.

### 3.6 Auth providers

```python
class Bearer:
    def __init__(self, token: str) -> None: ...

class Basic:
    def __init__(self, username: str, password: str = "") -> None: ...

class Env:
    """Reads AVISO_TOKEN / AVISO_USERNAME / AVISO_PASSWORD from os.environ at construction."""
    def __init__(self) -> None: ...

class ConfigFile:
    def __init__(self, path: str | os.PathLike[str]) -> None: ...

class Chain:
    def __init__(self, *providers: AuthProvider) -> None: ...

# Type alias in the stubs (NOT a Protocol):
AuthProvider = Bearer | Basic | Env | ConfigFile | Chain
```

`AuthProvider` is a closed union alias of the five shipped wrapper classes, not a
`typing.Protocol`. The runtime only accepts those exact types (PyO3 type-check at the FFI
boundary); a Protocol declaration would lead static checkers to accept arbitrary Python
classes that the runtime would then reject with a confusing `TypeError`. The closed union
keeps the static and runtime models consistent.

Widening the alias to a Protocol is the documented upgrade path when custom Python-side
`AuthProvider` implementations are added (out of scope for v1, see §11).

### 3.7 State stores

```python
class MemoryStore:
    def __init__(self) -> None: ...

class JsonFileStore:
    def __init__(self, path: str | os.PathLike[str]) -> None: ...

# Type alias in the stubs (NOT a Protocol):
StateStore = MemoryStore | JsonFileStore
```

Both wrap their Rust counterparts via `Arc<dyn StateStore>` internally. The Python users see a
closed union alias for the same consistency reason as auth providers (§3.6): the runtime
accepts only the two shipped wrappers; a Protocol declaration would mislead static checkers.

The user passes the wrapped store to the client constructor:

```python
client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    state_store=aviso.JsonFileStore(pathlib.Path.home() / ".config" / "aviso" / "state.json"),
)
```

`~` expansion: `JsonFileStore` accepts any `os.PathLike` or `str`; the normalisation runs
Rust-side at the PyO3 boundary via the shared `normalize_path(py, &Bound<PyAny>)` helper
described in §4. The helper calls `os.fspath` then `pathlib.Path(p).expanduser()` under the
GIL, then hands a `PathBuf` to the core library. Rust `PathBuf::from("~/...")` does NOT
expand tilde; doing the expansion at the boundary, once, in one helper, ensures every
path-typed argument across the entire surface (`Trigger.log`, `Trigger.working_dir`,
`JsonFileStore`, `ConfigFile`) behaves identically. The docs prefer `pathlib.Path`
constructions in examples to avoid relying on tilde behaviour, but tilde strings work too.

`JsonFileStore` opens the file lazily on first `put`. Parent directories must exist; the
constructor does not create them. If the parent is missing, the first `put` raises
`StateStoreError`. The PyO3 method's docstring spells this out; the docs page includes a
recipe for `path.parent.mkdir(parents=True, exist_ok=True)` before constructing the store.

Custom Python-side impls (the "implement StateStore in Python" use case) defer to a later PR.

### 3.8 Exceptions

Each Rust error variant becomes a Python exception class with structured fields. Stringly
typed error payloads lose information that the Rust side already carries; the bindings expose
the structured fields so Python users can pattern-match on them.

```python
class AvisoError(Exception):
    """Base for every aviso-raised exception. Catch this to catch all library errors."""

class TransportError(AvisoError):
    """Network-level failure before the response begins: DNS, TCP, TLS."""

class HttpError(AvisoError):
    """Non-success HTTP response from the server."""
    status: int               # HTTP status code
    body: str                 # response body, verbatim
    request_id: str | None    # X-Request-ID for support correlation

class AuthError(AvisoError):
    """Auth source resolution or refresh failed."""

class DecodeError(AvisoError):
    """Response body could not be decoded as expected JSON shape."""

class MalformedEventError(AvisoError):
    """CloudEvent id field did not parse per <event_type>@<sequence>.
    Terminal per D9; reconnecting would re-receive the same bad event."""

class HistoryGapError(AvisoError):
    """A gap was detected in the watch stream. Terminal per D2."""
    # Discriminator: one of "replay_limit_reached" or "sequence_jump".
    reason: str
    # Set when reason == "replay_limit_reached":
    max_allowed: int | None
    # Set when reason == "sequence_jump":
    expected: int | None
    observed: int | None

class StreamProtocolError(AvisoError):
    """Wire-protocol-level fatal condition during streaming."""
    message: str
    request_id: str | None

class ConfigError(AvisoError):
    """Client configuration or argument validation failed."""

class StateStoreError(AvisoError):
    """Persistent state-store operation failed. Terminal during watch sessions."""

class TriggerError(AvisoError):
    """A required trigger failed after all retries. Terminal per D11."""
    # Discriminator: "echo" | "log" | "command" | "webhook" | "teams" | "post".
    trigger_kind: str
    # Sub-discriminator on the source error variant:
    # "io" | "encode" | "command" | "timeout" | "webhook" | "webhook_build" | "template".
    error_kind: str
    # Set when error_kind == "command" (Unix only):
    exit_code: int | None
    stderr_tail: str | None
    # Set when error_kind == "webhook":
    status: int | None        # HTTP status, None on transport error
    body_tail: str | None     # last 4 KiB of response body, lossy UTF-8
    # Set when error_kind == "webhook_build":
    reason: str | None
    # Set when error_kind == "timeout":
    timeout_seconds: float | None
    # Set when error_kind == "template":
    context: str | None       # safe static label: "command" / "webhook url" / ...
    field: str | None         # the failing JSON path or env var name
    template_kind: str | None # "missing" | "env_not_set" | "env_not_unicode" |
                              # "bad_syntax" | "notification_encode"
    # Set when error_kind == "log" or "io" without a more specific kind:
    path: str | None
```

Discriminator strings (`reason` on `HistoryGapError`, `trigger_kind` and `error_kind` on
`TriggerError`, `template_kind` on its sub-payload) come from hand-coded `match`-arm string
conversion inside `map_client_error`, NOT from `{:?}` debug formatting (debug-formatted Rust
enum names leak field syntax and are stable only by accident, and depend on derive macros).
The string values are part of the contract and have tests asserting them. No new dependency
is added for this; the conversion is one screen of straightforward Rust.

Every exception subclasses `AvisoError`, so `except AvisoError` catches the whole hierarchy.
`pyo3::create_exception!` builds the classes at module init; structured fields are set as
attributes on the instance during `map_client_error`.

The test `test_exception_attributes.py` provokes one instance of each Rust error variant
(against a wiremock server for `HttpError`, against a tampered SSE harness for
`MalformedEventError`, etc.) and asserts every documented field is present with the
documented type.

### 3.9 Version

`aviso.__version__` is a string, sourced from installed package metadata via
`importlib.metadata.version("aviso")`. The Rust extension also exposes
`aviso._native.VERSION` for symmetry; the Python wrapper prefers the package-metadata source.
Existing `python/aviso/__init__.py` already does this; no change needed.

---

## 4. Pythonic ergonomics

Specific Python conventions the API honours:

- **Keyword-only after the first argument** for any function with two-plus arguments. Python
  3.10's `*,` separator makes call sites self-documenting.
- **`with` and `async with`** as context managers on both client classes. `__exit__` calls
  `close()` which drops the underlying Rust client (parent-drop cascade fires every supervisor's
  cancellation).
- **`for` and `async for`** on `listen`. Iterators are first-class; `next(it)` and `await
  anext(it)` both work because the iterator types implement `__iter__`/`__next__` and
  `__aiter__`/`__anext__`.
- **`os.PathLike` accepted** anywhere a path is accepted (`Trigger.log`, `JsonFileStore`,
  `ConfigFile`, `Trigger.working_dir`). Path normalisation happens **Rust-side** inside the
  PyO3 method: the parameter is declared as `&Bound<PyAny>`, the method calls Python's
  `os.fspath(arg)` then `pathlib.Path(p).expanduser()` while holding the GIL, then converts
  the resolved string into a `PathBuf` before calling into the core library. This keeps the
  Python wrapper module free of facade duplication and ensures every path-typed argument
  receives the same normalisation regardless of which Python class it crosses. Rust's
  `PathBuf::from("~/...")` does NOT expand tilde, so tilde strings that reached Rust unchanged
  would silently produce relative paths under the cwd.
- **No mutable default arguments**; `identifier=None` then `identifier or {}` inside.
- **f-strings only** in tests and docs examples.
- **`class X(str, Enum)`** for `HttpMethod` and `WatchMode` (defined in the Python wrapper
  `python/aviso/__init__.py`, NOT `enum.StrEnum` which is 3.11+ and breaks the 3.10 lower
    bound). `HttpMethod.POST == "POST"` is True because of the `str` base class. The
  `.value` attribute is the canonical serialisation form (`HttpMethod.POST.value == "POST"`,
  `WatchMode.WATCH.value == "watch"`); `str()` of a `str+Enum` member is version-dependent
  (3.10 returns the value, 3.11+ returns the qualified name in some cases), so docs and
  Rust-side conversion always use `.value` for explicit string serialisation. Native PyO3
  methods declare the parameter as a Rust `String`; PyO3 extracts via the `str` base class
  (the enum instance IS a `str` whose payload is the value), so passing `HttpMethod.POST`
  yields `"POST"` in Rust regardless of Python version. A boundary test asserts this for
  both enums on Python 3.10 and 3.13.
- **`__repr__` and `__eq__`** on every value type. **No `__hash__`** on `Notification`,
  `NotifyResponse`, `SchemaCatalog`, `SchemaResponse`: their `payload` / `schema` fields hold
  `dict` / `list` Python objects after conversion, and a value type that contains unhashable
  inner values is itself unhashable per Python convention. The classes are marked
  `__hash__ = None` so attempting to use them as dict keys raises `TypeError` loudly rather
  than producing an id-based hash by accident.
- **`as_dict()` everywhere** so users can drop into pandas / json / dataclasses without bespoke
  serialisation.
- **Closed union aliases** for `AuthProvider` and `StateStore` in the stubs (see §3.6, §3.7),
  not `typing.Protocol`. The runtime accepts only the shipped wrapper classes; a Protocol
  declaration would mislead static checkers.

---

## 5. Implementation architecture

### 5.1 Crate scaffolding delta

`crates/aviso-py/Cargo.toml` changes:

```toml
[lib]
crate-type = ["cdylib", "rlib"]
# was: crate-type = ["rlib"]

[dependencies]
aviso = { path = "../aviso", version = "=0.1.0" }
pyo3 = { workspace = true, features = ["extension-module", "abi3-py310"] }
pyo3-async-runtimes = { workspace = true, features = ["tokio-runtime"] }
pyo3-log = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true, features = ["log"] }
futures-util = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
```

`crate-type = ["cdylib", "rlib"]` so the same crate ships both as a Python extension and an
in-process Rust dependency (matters for the workspace `cargo build`/`cargo test` to see the
crate's types in cross-crate tests, even though only the cdylib is published as a wheel).

The `tracing` dep enables the `log` feature so emitted `tracing` events propagate through the
`log` facade when no `tracing-subscriber` is initialised, which the Python extension does not
initialise (the supervisor in the core crate emits events; the Python extension just bridges
them to Python's logging via `pyo3-log`).

Workspace `Cargo.toml` additions (latest stable verified via `cargo search`):

```toml
[workspace.dependencies]
pyo3 = { version = "0.28", default-features = false, features = ["macros"] }
pyo3-async-runtimes = { version = "0.28", default-features = false, features = ["tokio-runtime"] }
pyo3-log = { version = "0.13", default-features = false }
futures-util = { version = "0.3", default-features = false }
```

`pyproject.toml` `[build-system]` updates to track latest stable maturin:

```toml
[build-system]
requires = ["maturin>=1.13,<2"]
build-backend = "maturin"
```

The `abi3-py310` feature on PyO3 gives one wheel that works for Python 3.10+;
`requires-python = ">=3.10"` in `pyproject.toml` aligns.

`deny.toml` allowed-licences list already covers Apache-2.0, MIT, BSD-*; PyO3 ships under
MIT-OR-Apache-2.0. No new licence to add.

### 5.2 Module init

```rust
use std::sync::OnceLock;
use pyo3::exceptions::PyRuntimeError;
use pyo3_log::{Caching, Logger};

static LOGGER_INSTALLED: OnceLock<()> = OnceLock::new();

#[pyo3::pymodule]
fn _native(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    install_logging_bridge(py)?;
    register_exceptions(py, m)?;
    register_value_types(m)?;
    register_clients(m)?;
    register_triggers(m)?;
    register_auth(m)?;
    register_state_stores(m)?;
    m.add("VERSION", aviso::VERSION)?;
    Ok(())
}

fn install_logging_bridge(py: Python<'_>) -> PyResult<()> {
    if LOGGER_INSTALLED.get().is_some() {
        return Ok(());
    }
    let logger = Logger::new(py, Caching::Loggers)
        .map_err(|e| PyRuntimeError::new_err(format!("pyo3-log Logger::new failed: {e}")))?;
    // reason: SetLoggerError means another bridge is already installed for
    // this process (a host application has its own log facade). The library
    // does not fight an existing installation; treating that one failure mode
    // as success is the documented behaviour.
    logger.install().ok();
    // reason: benign race between concurrent first-time imports. Both threads
    // can race through Logger::new and install; only one set() commits, the
    // other is a no-op because the cell already holds (). Either order is
    // correct because the work is idempotent.
    LOGGER_INSTALLED.set(()).ok();
    Ok(())
}
```

`get_or_init` is deliberately NOT used here. Its closure signature returns `T`, not
`Result<T, E>`, so a `Logger::new` failure would have to commit `()` to the cell anyway and
a later re-import would short-circuit without retrying. The check-then-construct shape above
keeps the retry semantics correct: a `Logger::new` failure aborts module init loudly, the
cell stays uninitialised, and a subsequent `import aviso` retries. `OnceLock::get_or_try_init`
is still unstable on the current toolchain (rust-lang/rust#109737) and not used.

Both `.ok()` calls carry the `// reason:` comment AGENTS.md requires for intentional error
discards. Every other failure path (logger construction failure, GIL acquisition during ctor)
propagates as a `PyRuntimeError` and aborts module init loudly.

The bridge chain is `tracing::info!` (in the core crate) -> `log` (via the `tracing` crate's
`log` feature) -> `pyo3-log` -> Python `logging`. `Caching::Loggers` (not the default
`Caching::LoggersAndLevels`) is used because Python users typically configure logging via
`logging.basicConfig(...)` AFTER importing `aviso`; caching levels would pin the levels at
their import-time value and Python-side `logger.setLevel(...)` changes would not take effect.

Each `register_*` function attaches the relevant classes / exceptions to the module. Splitting
keeps any one Rust source file under the 500-LOC limit.

A `tests/test_logging_bridge.py` asserts:

- `import aviso` does not raise even when called multiple times.
- After `logging.basicConfig(level=DEBUG)`, a publish call produces at least one log record on
  the `aviso` Python logger.
- The bridge does not crash when called from a non-main thread.

### 5.3 Tokio runtime

Use `pyo3_async_runtimes::tokio::get_runtime()` directly. The crate manages a single static
multi-thread runtime keyed to the Python process; the bindings never construct their own. This
matches the pattern used by icechunk, tensorzero, nautilus_trader, and other production PyO3
projects.

```rust
use pyo3_async_runtimes::tokio::get_runtime;
```

Sync call shape (sync client method delegating to async core):

```rust
fn py_notify(&self, py: Python<'_>, ...) -> PyResult<NotifyResponse> {
    let client = self.inner.clone();
    py.detach(|| {
        get_runtime().block_on(async move {
            client.notify(&req).await
                .map(NotifyResponse::from)
                .map_err(map_client_error)
        })
    })
}
```

`py.detach(|| ...)` releases the GIL for the duration of the `block_on`; the GIL is reacquired
when the closure returns. This is mandatory: holding the GIL across a `block_on` would deadlock
any other Python thread the runtime may need to call back into (logging bridge, callback).

Async call shape (async client method):

```rust
fn py_notify<'py>(&self, py: Python<'py>, ...) -> PyResult<Bound<'py, PyAny>> {
    let client = self.inner.clone();
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        client.notify(&req).await
            .map(NotifyResponse::from)
            .map_err(map_client_error)
    })
}
```

`future_into_py` wraps the Rust future as a Python awaitable; `await`-ing it on the Python side
runs the future on the shared runtime without blocking the Python event loop.

### 5.4 Async iteration over watch streams

The Rust `NotificationStream` is `impl Stream<Item = Result<Notification, ClientError>>` backed
by a `tokio::sync::mpsc::Receiver`. The Python wrapper exposes it as both an async iterator and
a sync iterator over the same underlying object. The opendal `AsyncLister` pattern is the
template:

```rust
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration};

#[pyclass(module = "aviso._native")]
pub struct AsyncNotificationStream {
    inner: Arc<AsyncMutex<Option<aviso::watch::NotificationStream>>>,
}

#[pymethods]
impl AsyncNotificationStream {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> { slf }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let inner = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let mut guard = inner.lock().await;
            let stream = guard
                .as_mut()
                .ok_or_else(|| PyRuntimeError::new_err("stream already closed"))?;
            match stream.recv().await {
                Some(Ok(notification)) => Python::attach(|py| {
                    PyNotification::from(notification)
                        .into_py_any(py)
                        .map(Some)
                }),
                Some(Err(e)) => Err(map_client_error(e)),
                None => Err(PyStopAsyncIteration::new_err(())),
            }
        })
    }
}
```

Key elements:

- The stream stays owned by `Self` between `__anext__` calls via `Arc<AsyncMutex<Option<Stream>>>`. The mutex is held briefly per recv and released; the option is only `take`n on explicit close.
- `__anext__` returns the awaitable directly (no `Option<...>` wrap; `Bound<PyAny>` IS the
  Python future).
- Exhaustion surfaces as `PyStopAsyncIteration` per Python's async iterator protocol.
- A client-side error during iteration ends iteration with that error (`map_client_error(e)`)
  rather than `PyStopAsyncIteration`, so users see the actual failure.

The sync variant uses `__iter__`/`__next__` with a bounded polling loop instead of a single
`block_on(recv)`. Each iteration polls the underlying mpsc with a 100 ms timeout, releases the
GIL via `py.detach` for the duration of the await, and (after the await returns and the GIL is
reacquired) calls `py.check_signals()`. A timeout from `tokio::time::timeout` just continues
the loop after the signal check; a real receive returns the value. The sync `__next__` raises
`PyStopIteration` (not the async variant) on stream exhaustion.

```rust
const SYNC_RECV_POLL: Duration = Duration::from_millis(100);

fn __next__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
    let stream = self.inner.clone();
    loop {
        let outcome = py.detach(|| {
            get_runtime().block_on(async {
                let mut guard = stream.lock().await;
                let stream = guard
                    .as_mut()
                    .ok_or(IterStatus::Closed)?;
                match tokio::time::timeout(SYNC_RECV_POLL, stream.recv()).await {
                    Ok(Some(Ok(n))) => Ok(IterStatus::Received(n)),
                    Ok(Some(Err(e))) => Err(IterStatus::Error(e)),
                    Ok(None) => Err(IterStatus::Exhausted),
                    Err(_) => Ok(IterStatus::Timeout),
                }
            })
        });
        py.check_signals()?;  // raises KeyboardInterrupt if pending
        match outcome {
            Ok(IterStatus::Received(n)) => {
                return Python::attach(|py| PyNotification::from(n).into_py_any(py));
            }
            Ok(IterStatus::Timeout) => continue,
            Err(IterStatus::Exhausted) => return Err(PyStopIteration::new_err(())),
            Err(IterStatus::Closed) => return Err(PyRuntimeError::new_err("stream closed")),
            Err(IterStatus::Error(e)) => return Err(map_client_error(e)),
        }
    }
}
```

**Cancellation**:

- Async: `Drop` on `AsyncNotificationStream` drops the inner Rust `NotificationStream`, which
  drops its oneshot sender, which signals the supervisor to exit cooperatively. Asyncio's
  signal handling propagates `KeyboardInterrupt` to the awaiting task.
- Sync: the polling loop above checks signals every 100 ms, so an idle stream still responds to
  Ctrl+C within one poll period. On signal, `py.check_signals()` raises the exception; Python
  surfaces it as `KeyboardInterrupt`; the iterator is dropped; the supervisor exits.
- Explicit close: `iter.close()` (sync) takes the iterator value, drops the inner stream, and
  blocks on `NotificationStream::close()` so any pending checkpoint flushes before return.
  Async equivalent is `await iter.aclose()`.

Tests cover three scenarios: (1) Ctrl+C during an idle sync iteration raises
`KeyboardInterrupt` from the iteration call site within ~200 ms, (2) Ctrl+C during async
iteration cancels the task cleanly, (3) `iter.close()` after a finite stream returns without
hanging.

### 5.5 GIL discipline

PyO3 0.28 uses `Python::detach` as the GIL-release method (older releases called it
`Python::allow_threads`). The bindings target 0.28 and use the `detach` name throughout.

- Any `await` on the Rust side from a sync method: wrap `block_on` in `py.detach(|| ...)` so
  the GIL is released while the runtime drives the future.
- Async methods that use `future_into_py`: the GIL is released implicitly while the awaitable
  is pending on the Python side; the inner Rust future executes on the shared runtime without
  the GIL.
- Construction of a Python value (a `Notification`, a `NotifyResponse`): acquire the GIL via
  `Python::attach(|py| { ... })` inside the future block, build the value, return it.
- Iteration: each `__next__` / `__anext__` is one await; the GIL is released for the duration
  of that await and reacquired to build the returned value. `py.check_signals()` runs inside
  the GIL window just before the value is returned so a pending `KeyboardInterrupt` cancels the
  iteration step rather than the next one.

### 5.6 Error mapping

Rust `ClientError` variants map onto Python exceptions with structured fields (per §3.8). A
single `map_client_error(err: ClientError) -> PyErr` function lives in
`crates/aviso-py/src/error.rs`; it constructs the exception class, attaches the structured
fields as instance attributes, and returns a `PyErr`. Discriminator strings (`reason`,
`trigger_kind`, `error_kind`, `template_kind`) are hand-coded `match`-arm conversions inside
this function; no new dependency, the strings live in one file and are exhaustively tested.

| Rust variant                                       | Python exception (with attributes set)                                                                                  |
|---------------------------------------------------:|:------------------------------------------------------------------------------------------------------------------------|
| `Transport(reqwest::Error)`                        | `TransportError(str(err))`                                                                                              |
| `Http { status, body, request_id }`                | `HttpError`; sets `.status`, `.body`, `.request_id`                                                                     |
| `Auth(String)`                                     | `AuthError(message)`                                                                                                    |
| `Decode(serde_json::Error)`                        | `DecodeError(str(err))`                                                                                                 |
| `MalformedEvent(String)`                           | `MalformedEventError(detail)`                                                                                           |
| `HistoryGap { ReplayLimitReached { max_allowed } }`| `HistoryGapError`; sets `.reason = "replay_limit_reached"`, `.max_allowed`                                              |
| `HistoryGap { SequenceJump { expected, observed } }`| `HistoryGapError`; sets `.reason = "sequence_jump"`, `.expected`, `.observed`                                          |
| `StreamProtocol { message, request_id }`           | `StreamProtocolError`; sets `.message`, `.request_id`                                                                   |
| `Config(String)`                                   | `ConfigError(message)`                                                                                                  |
| `StateStore(StoreError)`                           | `StateStoreError(str(err))` (sub-discrimination is a follow-up; the inner debug string is the v1 explanation)           |
| `TriggerFailed { kind: Echo, source: Io(_) }`      | `TriggerError`; sets `.trigger_kind = "echo"`, `.error_kind = "io"`                                                     |
| `TriggerFailed { kind: Log{path}, source: Io(_) }` | `TriggerError`; sets `.trigger_kind = "log"`, `.error_kind = "io"`, `.path`                                             |
| `TriggerFailed { kind: Command, source: Command{exit_code, stderr_tail} }` | `TriggerError`; sets `.trigger_kind = "command"`, `.error_kind = "command"`, `.exit_code`, `.stderr_tail`     |
| `TriggerFailed { kind: Webhook, source: Webhook{status, body_tail} }` | `TriggerError`; sets `.trigger_kind = "webhook"`, `.error_kind = "webhook"`, `.status`, `.body_tail`         |
| `TriggerFailed { kind: Webhook, source: WebhookBuild{reason} }` | `TriggerError`; sets `.trigger_kind = "webhook"`, `.error_kind = "webhook_build"`, `.reason`                       |
| `TriggerFailed { kind: any, source: Timeout(d) }`  | `TriggerError`; sets `.trigger_kind`, `.error_kind = "timeout"`, `.timeout_seconds = d.as_secs_f64()`                   |
| `TriggerFailed { kind: any, source: Template{context, field, kind} }` | `TriggerError`; sets `.trigger_kind`, `.error_kind = "template"`, `.context`, `.field`, `.template_kind`     |
| `TriggerFailed { kind: any, source: Encode(_) }`   | `TriggerError`; sets `.trigger_kind`, `.error_kind = "encode"`                                                          |
| `TriggerFailed { kind: Teams, source: ... }`       | same as Webhook for HTTP-shaped errors, `.trigger_kind = "teams"`                                                       |
| `TriggerFailed { kind: Post, source: ... }`        | same as Webhook for HTTP-shaped errors, `.trigger_kind = "post"`                                                        |

Tests cover one round-trip per `TriggerError` row (mocked variants via wiremock or test-only
constructors) plus one round-trip per top-level variant. The mapping is exhaustive: a future
Rust variant with no row in the table fails to compile because the `match` arm in
`map_client_error` handles every variant currently in the Rust source. The Rust source
enums (`ClientError`, `GapReason`, `TriggerKindLabel`, `TriggerError`, `TemplateErrorKind`)
are `#[non_exhaustive]`, so any future variant added in the core crate will NOT cause a
compile failure in `aviso-py` (the wildcard arm is mandatory under `#[non_exhaustive]`).
The mapping therefore includes an explicit `_ =>` arm that constructs the base
`AvisoError` with the message `format!("unhandled variant: {err}")`, so a future variant
is at least caught by `except AvisoError` while the binding catches up.

There is no auto-detection mechanism for a new variant in the core crate. Auto-detection
would require either macro expansion (which `#[non_exhaustive]` defeats by design) or a
build-time script that parses the core's source (brittle and out of scope). Instead, the
binding maintainer is responsible for keeping the table in `map_client_error` in sync with
the core's `ClientError` variants; the AGENTS.md plan-driven workflow already requires a
core-side variant addition to land alongside the binding update because they share a
review pass. A `tests/test_exception_attributes.py` round-trips every variant currently in
the table (one provoked failure per variant via wiremock or test fixtures), so a regression
that removes a row in the table also fails the test.

`StoreError`'s internal variants (`Io`, `Decode`, ...) collapse into `StateStoreError`'s
message string in v1; if Python users need finer-grained discrimination later (e.g., to retry
on `Io`-induced failures), the mapping expands without breaking the existing field set.

### 5.7 Python wrapper module

`python/aviso/__init__.py` becomes the user-facing entry. It re-exports the native classes,
defines the pure-Python enums (`HttpMethod`, `WatchMode`) so they stay 3.10-compatible, and
attaches the closed union aliases that the stubs reference. Nothing else:

```python
"""Python client for aviso-server."""
from __future__ import annotations

from enum import Enum
from importlib.metadata import PackageNotFoundError, version

from aviso._native import (
    AsyncAvisoClient,
    AsyncNotificationIterator,
    AvisoClient,
    AvisoError,
    AuthError,
    Basic,
    Bearer,
    Chain,
    ConfigError,
    ConfigFile,
    DecodeError,
    Env,
    HistoryGapError,
    HttpError,
    JsonFileStore,
    MalformedEventError,
    MemoryStore,
    Notification,
    NotificationIterator,
    NotifyResponse,
    ResumeStart,
    SchemaCatalog,
    SchemaResponse,
    StateStoreError,
    StreamProtocolError,
    Trigger,
    TransportError,
    TriggerError,
    WatchRequest,
)


class HttpMethod(str, Enum):
    """HTTP method for webhook / teams / post triggers."""

    POST = "POST"
    GET = "GET"
    PUT = "PUT"
    PATCH = "PATCH"
    DELETE = "DELETE"


class WatchMode(str, Enum):
    """Whether a watch session reconnects after `end_of_stream` (WATCH)
    or terminates after replay completes (REPLAY_ONLY)."""

    WATCH = "watch"
    REPLAY_ONLY = "replay_only"


# Closed union aliases: the runtime accepts only these concrete types.
AuthProvider = Bearer | Basic | Env | ConfigFile | Chain
StateStore = MemoryStore | JsonFileStore

try:
    __version__ = version("aviso")
except PackageNotFoundError:
    __version__ = "0.0.0+uninstalled"

__all__ = [
    "AsyncAvisoClient",
    "AsyncNotificationIterator",
    "AuthProvider",
    "AvisoClient",
    "AvisoError",
    "AuthError",
    "Basic",
    "Bearer",
    "Chain",
    "ConfigError",
    "ConfigFile",
    "DecodeError",
    "Env",
    "HistoryGapError",
    "HttpError",
    "HttpMethod",
    "JsonFileStore",
    "MalformedEventError",
    "MemoryStore",
    "Notification",
    "NotificationIterator",
    "NotifyResponse",
    "ResumeStart",
    "SchemaCatalog",
    "SchemaResponse",
    "StateStore",
    "StateStoreError",
    "StreamProtocolError",
    "Trigger",
    "TransportError",
    "TriggerError",
    "WatchMode",
    "WatchRequest",
    "__version__",
]
```

No magic, no auto-init, no `_native` leakage.

### 5.8 Type stubs

`python/aviso/__init__.pyi` declares every public symbol with full signatures. The stub file
gets `ty check` over it (strict mode); any drift between the runtime PyO3 surface and the
declared stubs surfaces as a `ty` failure.

Two viable options researched: hand-written stubs vs `pyo3-stub-gen` (used by opendal in
production). v1 uses hand-written stubs for these reasons:

- One file, ~250 lines; the maintenance cost is low.
- Explicit control over what the user sees: stub file IS the documented surface, no
  generator-driven surprises (auto-derived `Optional`/`Union` quirks, unwanted private symbols).
- No build dependency; the stub file ships verbatim from the repo and is not regenerated by
  CI.
- The shape is stable for the v1 surface; if the surface grows beyond ~500 stub lines or
  starts to drift, `pyo3-stub-gen` is the documented upgrade path.

Two stub-quality tests, not one:

- `tests/test_stub_completeness.py`: every public name in `aviso.__all__` has a corresponding
  declaration in the stub file. Catches symbol drift.
- `tests/test_stub_signatures.py`: for every PyO3 method declared on `AvisoClient`,
  `AsyncAvisoClient`, `Trigger`, the auth providers, and the state stores, the runtime
  signature (via `inspect.signature(...)`) matches the stub-declared signature in parameter
  names and keyword-only markers. Default-value comparison is scoped: the test compares
  `bool`, `int`, `float`, `str`, and `None` defaults directly, and for enum-backed parameters
  (`mode: WatchMode = WatchMode.WATCH`, `method: HttpMethod = HttpMethod.POST`) it compares
  the runtime default's string value against the stub default enum member's `.value`. The
  runtime side declares `#[pyo3(signature = (..., mode = "watch"))]` because PyO3 cannot
  reference a Python class as a default literal; the stub side declares the enum form for
  Python users. The test bridges the two representations explicitly. Return-type coverage
  is OUT of scope for this test: PyO3's `__text_signature__` (the basis for
  `inspect.signature` on Rust-defined methods) carries the call parameter list only, not the
  return annotation. Return types ride on the stub file and are validated by `ty check`
  against usage samples in the integration tests.

To make `inspect.signature` work on PyO3-defined methods, every public `#[pymethods]`
function carries an explicit `#[pyo3(signature = (...))]` annotation. PyO3 0.28 does not
auto-emit `__text_signature__` for methods without this annotation, so without the
annotation `inspect.signature` raises and the test cannot run. This is the load-bearing rule
that keeps the signature-test gate honest.

Plus an integration test that round-trips a representative public method through every
keyword-only / default position to assert the signature shape on the wire (calling with
positional args where the stub says keyword-only must raise `TypeError`).

---

## 6. Testing strategy

Layered, comprehensive, and reasonably fast.

### 6.1 Unit tests (Rust side)

The PyO3 module gets Rust-side unit tests in `crates/aviso-py/src/*/tests.rs`. They exercise:

- Type construction from Rust values (Notification, NotifyResponse, etc.).
- Error mapping (`map_client_error` per variant).
- WatchRequest builder accessors round-trip from Python kwargs.
- HttpMethod parse/render.
- ResumeStart conversion (int vs str).

These run under `cargo test -p aviso-py` and are part of the standard 11-gate set. No GIL
acquisition needed because they exercise plain Rust functions; Python integration sits in §6.2.

### 6.2 Pytest suite (Python side)

`python/tests/` mirrors `python/aviso/` layout. The shape:

```text
python/tests/
├── conftest.py
├── test_smoke.py                       # import, version, repr basics
├── test_clients.py                     # constructor validation, with/__exit__
├── test_notify.py                      # mock server, both sync and async
├── test_schema.py                      # mock server, both sync and async
├── test_admin.py                       # mock server, both sync and async
├── test_listen_sync.py                 # mock server, sync iteration
├── test_listen_async.py                # mock server, async iteration
├── test_listen_validation.py           # listen() arg validation: mixed kwargs, bool, negative
├── test_listen_resume.py               # state store resume, from_id, from_date
├── test_iter_close.py                  # iter.close() + aclose(); flush_cursor_on_exit semantics
├── test_triggers.py                    # builder shape, all six kinds, keyword vs chainable
├── test_auth.py                        # provider construction + 401 refresh
├── test_state_stores.py                # MemoryStore + JsonFileStore round-trip
├── test_path_handling.py               # str / Path / ~ / missing parent / non-utf8
├── test_errors.py                      # exception hierarchy + structured attributes
├── test_exception_attributes.py        # every variant produced + every attr present
├── test_stub_completeness.py           # __all__ vs .pyi (name drift)
├── test_stub_signatures.py             # inspect.signature vs .pyi (signature drift)
├── test_logging_bridge.py              # pyo3-log -> Python logging, multi-thread safe
├── test_json_payload_conversion.py     # nested dict / list / null / unicode / large
├── test_property_filters.py            # hypothesis: filter dict round-trip
├── test_property_resume_start.py       # hypothesis: int vs date parsing
├── test_keyboard_interrupt_sync.py     # idle sync stream responds to Ctrl+C within ~200ms
├── test_keyboard_interrupt_async.py    # async iteration cancels cleanly on KeyboardInterrupt
└── test_integration_live.py            # opt-in, real server, marker `integration`
```

Estimated 90-120 tests total across the files. The integration test file is the only one that
needs a real server; the rest run hermetically.

Mock server: `pytest-httpserver` (https://pytest-httpserver.readthedocs.io/) for the HTTP
endpoints and a small `aiohttp`-based SSE harness for the watch stream. Both are well-maintained
Python libraries; cleaner than rolling our own.

Async tests use `pytest-asyncio` (dev-dep). The conftest sets `asyncio_mode = "auto"` so test
functions whose names start with `test_` and are `async def` are picked up automatically.

Property-based tests via `hypothesis`. Two property tests in v1:

- **Filter dict round-trip**: a randomly-generated `dict[str, JsonValue]` survives Python -> Rust
  -> JSON -> back-to-Python with equality.
- **ResumeStart parsing**: any int routes to `AfterSequence`, any string of the seven
  date-shapes routes to `Date`, anything else raises `ConfigError`.

The CLI's `--from` parsing logic lives Rust-side already; the Python wrapper just routes int
vs str.

Specific risk-driven tests that earned their own files:

- `test_keyboard_interrupt_sync.py`: starts a sync `for n in client.listen(...)` loop against
  a mock server that NEVER emits a notification (heartbeat only). After ~150 ms in the loop,
  `signal.raise_signal(signal.SIGINT)` is invoked from a sibling thread. The test asserts the
  `for` body raises `KeyboardInterrupt` within ~250 ms total. Without the bounded polling
  shape in §5.4 this test would hang the suite.
- `test_iter_close.py`: builds a client with `flush_cursor_on_exit=True` and a tempfile
  `JsonFileStore`. Iterates over the stream, observes N notifications, calls `iter.close()`,
  reopens the store, asserts the cursor is at N (not N-1). A second variant exercises
  `await iter.aclose()` on the async client.
- `test_json_payload_conversion.py`: round-trips Python -> notify -> mock server captures
  body -> watcher receives -> Notification.payload. Exercises nested dicts, lists, Unicode
  identifiers, `None`, large strings, deep nesting (10 levels). Asserts no information loss.
- `test_logging_bridge.py`: imports `aviso`, calls `logging.basicConfig(level=DEBUG)`,
  triggers a publish through a mock 5xx, asserts at least one log record landed on the
  `aviso` Python logger. Second test imports `aviso` from a non-main thread (concurrent
  `import` race); the bridge installation must remain idempotent.

### 6.3 Integration tests (opt-in)

`python/tests/test_integration_live.py` is marked `@pytest.mark.integration` and reads
`AVISO_INTEGRATION_BASE_URL` from the environment. When the env var is set, the test:

1. Publishes a notification with `event_type=test_polygon` (the harmless ECMWF test stream).
2. Watches the same stream from the just-published sequence.
3. Asserts the notification arrives within 5 seconds.

When the env var is unset, the test is skipped. CI does not set the env var by default; a
follow-up wires the ECMWF aviso-server credentials in CI secrets when the matrix PR ships.

### 6.4 What's NOT tested in v1

- Cross-process state-store resume from Python (covered by the Rust integration tests for the
  underlying file store).
- Python-side custom `StateStore` / `AuthProvider` (out of scope).
- Wheel builds on every Python version 3.10-3.13 (separate PR, wheel matrix).
- Windows behaviour for `Trigger.command`. The Rust core gates this trigger behind
  `#[cfg(unix)]`, but the Python wrapper exposes the `Trigger.command` classmethod on every
  platform: on non-Unix builds the wrapper body is one line that raises `ConfigError`
  ("command trigger is Unix-only"). Keeping the method present everywhere gives Python users
  a consistent surface (no `AttributeError` on Windows), and a Python try/except around
  `Trigger.command(...)` works the same way it would on a Unix host. The behaviour is not
  CI-tested in v1 (the matrix has no Windows leg); the source-level `cfg!(not(unix))` gate
  inside the wrapper is the load-bearing guarantee. When the wheel-matrix PR ships Windows
  CI, a `@pytest.mark.skipif(sys.platform != "win32")` test asserts the `ConfigError`
  runtime behaviour.

---

## 7. Documentation

The existing `docs/src/python/status.md` is a placeholder pointing users at `subprocess`. It
gets replaced with a real Python tree.

### 7.1 New mdBook pages

```text
docs/src/python/
├── overview.md             # what aviso-py is, when to use sync vs async
├── install.md              # source install today, PyPI when the wheels PR ships
├── quickstart.md           # 30-second taste, three concrete recipes
├── publish.md              # notify, single + bulk patterns
├── listen.md               # sync and async iteration, state, resume, iter.close
├── triggers.md             # the six trigger kinds, when to use which
├── auth.md                 # five providers, layering
├── state-and-resume.md     # how at-least-once works, MemoryStore vs JsonFileStore
├── error-handling.md       # exception hierarchy with a worked example
├── api-reference.md        # exhaustive symbol list (this file IS the reference)
└── troubleshooting.md      # common operator failures and fixes
```

Eleven pages, each ~50-200 lines. Total prose budget: ~1200 lines of docs.

**Install messaging in v1**: PyPI publishing is out of scope for this PR (§11). The
`install.md` page describes the source-install path that actually works today:

```text
# From a checkout
git clone https://github.com/ecmwf/aviso-client.git
cd aviso-client
uv venv
uv sync
uv run maturin develop --release
```

The page includes a clearly-marked block explaining that `pip install aviso` will work once
the wheel-matrix PR ships, with a link to the issue / PR. Until then, source install is the
documented path. The fact-check workspace verifies the source-install snippet on a clean
machine. Any code block that documents `pip install aviso` carries a `<!-- not-runnable -->`
marker explaining the PyPI gate; the fact-check tool skips marked blocks.

### 7.2 SUMMARY.md update

Replace the single `[Python](python/status.md)` entry with a nested block:

```markdown
- [Python](python/overview.md)
  - [Install](python/install.md)
  - [Quickstart](python/quickstart.md)
  - [Publishing](python/publish.md)
  - [Listening](python/listen.md)
  - [Triggers](python/triggers.md)
  - [Auth](python/auth.md)
  - [State and resume](python/state-and-resume.md)
  - [Error handling](python/error-handling.md)
  - [API reference](python/api-reference.md)
  - [Troubleshooting](python/troubleshooting.md)
```

Old anchor (`python/status.md`) gets an mdBook redirect to `python/overview.md` so external
links keep working.

### 7.3 Other doc touches

- `docs/src/index.md`: the hero block currently has stacked code samples (Rust / Python / CLI).
  The Python tab is replaced with real working code (`import aviso; client =
  aviso.AvisoClient(...)`).
- `docs/src/getting-started/quickstart.md`: same treatment for the quickstart's Python tab.
- `docs/src/getting-started/install.md`: add a "Python" subsection describing the source
  install (`git clone` + `uv sync` + `uv run maturin develop --release`). A note explains
  that `pip install aviso` becomes the recommended path when the wheel-matrix PR ships;
  marked `<!-- not-runnable -->` so the fact-check tool skips it.
- `docs/src/concepts/streams.md`: add a one-paragraph Python aside showing the async iteration
  shape under the existing Rust example.

### 7.4 Diagrams

Two mermaid diagrams to add:

1. `docs/src/python/listen.md`: an async iteration flow showing supervisor -> mpsc channel ->
   Python `__anext__` -> user loop body -> next iteration.
2. `docs/src/python/state-and-resume.md`: the same checkpoint-on-next-send flow already
   documented in `docs/src/concepts/streams.md`, but with Python code on the consumer side.

Both auto-theme via the existing `mdbook-mermaid` wiring.

### 7.5 Fact-checking discipline (mandatory)

Every code example in the Python documentation runs end-to-end against
`https://aviso-server.ecmwf.int` before the corresponding docs commit lands. This is the same
discipline the docs-restructure PR followed; it caught four documented inaccuracies that would
have shipped otherwise. The Python docs have a higher inaccuracy risk than the CLI docs because
the API is new and the example space is larger, so the discipline is non-optional.

Concrete procedure:

1. **Workspace setup**: a durable scratch directory at `/tmp/aviso-py-fact-check/` (not
   committed) holds the verification project. Inside it: a fresh `uv venv`, `maturin develop
   --release` building the local wheel from the checked-out branch, a `~/.config/aviso/config.yaml`
   configured with the production base URL and operator credentials (already on the machine
   from the docs-restructure work).

2. **Per-example verification**: every fenced code block in `docs/src/python/*.md` whose
   language is `python` is copied verbatim into a runnable script in the scratch dir and
   executed. Expected behaviour observed against the production server:

   - **Publishing examples**: run, capture the returned `NotifyResponse`, assert
     `response.status == "success"` and that `response.request_id` is a non-empty string. The
     server-side audit log confirms the publish landed.
   - **Listening examples**: run, send a publish in parallel from a second process, assert the
     listener receives the published notification, assert the documented field shapes
     (`notification.sequence` is `int`, `notification.identifier` is a `dict[str, str]`,
     `notification.payload` is the published JSON).
   - **Schema examples**: run against the production catalog (which includes `mars`, `polygon`,
     `dissemination`, `test_polygon`); assert the documented field names exist.
   - **Trigger examples**: every shipped trigger kind (echo, log, command, webhook, teams,
     post) gets one end-to-end run with the production server emitting at least one real
     notification. Echo output captured and compared to the documented shape; log file
     inspected; command env vars confirmed (a small script that writes `env | grep AVISO_` to a
     file and is then invoked as the command); webhook receiver is a tiny aiohttp server
     started in the scratch dir on a non-privileged port; teams + post target the same local
     receiver and assert the body shape.
   - **State store examples**: a `JsonFileStore` example writes to a tempfile, the file is
     inspected to confirm the documented JSON shape, the example is run twice in sequence to
     confirm the resume behaviour (first run gets N notifications, second run picks up at
     N+1).
   - **Error handling examples**: every documented exception is provoked at least once
     (`HttpError` via a bad URL, `AuthError` via a bad token, `MalformedEventError` via a
     mocked server response, `HistoryGapError` via `from_=999999999`, `ConfigError` via an
     invalid argument).

3. **Output inspection**: for each example, the actual output is compared against the
   documented output. If the docs say "you'll see something like", the captured output is
   read for shape (not exact value); if the docs quote literal output, the comparison is
   strict.

4. **Defect handling**: every fact-check failure produces a small fix commit on the branch.
   The commit message names the docs file and the failure mode (`docs(python): correct
   listen.md filter scalar form`, mirroring the docs-restructure pattern).

5. **No code examples ship undated**: the docs commits in the §10 commit plan land AFTER the
   fact-check has been completed for the page in question. The plan's commit order (docs
   commits 20-23 land near the end of the PR) gives time for the bindings to be solid before
   docs are written against them.

6. **Mocked tests are not a substitute**: the `pytest` suite mocks the HTTP server for speed
   and hermetic CI, but mocked tests cannot prove the docs match the real server's wire
   contract. Real-server fact-checks catch shape drift that mocks cannot.

The fact-check workspace is not committed; the artefacts (test scripts, captured outputs,
listener YAMLs) live under `/tmp/aviso-py-fact-check/` and survive only until the machine is
rebooted, by design. Each PR sets it up fresh.

---

## 8. CI integration

`.github/workflows/ci.yml` gains a `python` job with a matrix that exercises both the lower
Python version bound (3.10, where ABI3 is pinned) and the current stable, on both Linux and
macOS:

```yaml
python:
  name: Python ${{ matrix.python }} ${{ matrix.os }}
  runs-on: ${{ matrix.os }}
  strategy:
    fail-fast: false
    matrix:
      include:
        - { os: ubuntu-latest, python: "3.10" }
        - { os: ubuntu-latest, python: "3.13" }
        - { os: macos-latest,  python: "3.13" }
  steps:
    - uses: actions/checkout@v4

    - name: Install Rust toolchain (from rust-toolchain.toml)
      run: |
        rustup show
        rustc --version

    - uses: Swatinem/rust-cache@v2
      with:
        shared-key: python-build-${{ matrix.python }}

    - name: Install uv
      uses: astral-sh/setup-uv@v4

    - name: Set Python version for uv
      run: uv python pin ${{ matrix.python }}

    - name: Sync env (locked, with dev deps)
      run: uv sync --locked --group dev

    - name: Build extension (debug for speed)
      run: uv run maturin develop --locked

    - name: Lint
      run: uv run ruff check python/

    - name: Format check
      run: uv run ruff format --check python/

    - name: Type check
      run: uv run ty check python/

    - name: Tests
      run: uv run pytest python/tests/ -v
```

The 3.10 leg specifically catches:

- `enum.StrEnum` regressions (3.11+ only, our enums must be `str+Enum`).
- `typing.Self`, `match` statement quirks, walrus, and any other 3.11+ syntax.
- `abi3-py310` linkage problems that would not surface against 3.13.

`uv` resolves the Python env from `pyproject.toml` + a committed `uv.lock`. `maturin develop
--locked` builds the extension in-tree (writes `python/aviso/_native*.so`); `--locked` ensures
the Cargo.lock-gate (#6) stays green. Tests run against the locally-built extension.

The pre-push hook gains the same five Python steps (against whatever the developer's local
Python is); AGENTS.md's local-gate rule covers them.
`tests/e2e/docker-compose.yml` is unchanged (Python tests use pytest-httpserver, not docker).

`cargo-deny` and `mdbook` jobs are unchanged.

---

## 9. File touch list (estimated)

This is the rough shape, used to size the commit plan in §10.

**Created** (~30 files):

- `crates/aviso-py/src/lib.rs` (rewritten from 15-line placeholder to ~80-line module entry)
- `crates/aviso-py/src/clients/sync.rs` (~250 LOC)
- `crates/aviso-py/src/clients/async_.rs` (~250 LOC)
- `crates/aviso-py/src/clients/mod.rs` (re-exports, ~30 LOC)
- `crates/aviso-py/src/clients/tests.rs` (~120 LOC)
- `crates/aviso-py/src/streams/sync_iter.rs` (~150 LOC)
- `crates/aviso-py/src/streams/async_iter.rs` (~150 LOC)
- `crates/aviso-py/src/streams/mod.rs` (~20 LOC)
- `crates/aviso-py/src/streams/tests.rs` (~80 LOC)
- `crates/aviso-py/src/values/notification.rs` (~150 LOC)
- `crates/aviso-py/src/values/notify_response.rs` (~60 LOC)
- `crates/aviso-py/src/values/schema.rs` (~100 LOC)
- `crates/aviso-py/src/values/watch_request.rs` (~200 LOC)
- `crates/aviso-py/src/values/resume_start.rs` (~100 LOC)
- `crates/aviso-py/src/values/watch_mode_conv.rs` (~30 LOC, private Rust-side conversion
  helpers between the Python string forms `"watch"`/`"replay_only"` and the core's
  `WatchMode`. NOT a public PyO3 class; the user-facing `WatchMode` lives in
  `python/aviso/__init__.py` as `class WatchMode(str, Enum)` to keep the type 3.10
  compatible.)
- `crates/aviso-py/src/values/mod.rs` (~30 LOC)
- `crates/aviso-py/src/values/tests.rs` (~150 LOC)
- `crates/aviso-py/src/triggers.rs` (~250 LOC, single-file is fine, six builders)
- `crates/aviso-py/src/triggers_tests.rs` (~100 LOC)
- `crates/aviso-py/src/auth.rs` (~150 LOC, five providers)
- `crates/aviso-py/src/auth_tests.rs` (~80 LOC)
- `crates/aviso-py/src/state_stores.rs` (~80 LOC, two stores)
- `crates/aviso-py/src/state_stores_tests.rs` (~50 LOC)
- `crates/aviso-py/src/error.rs` (~150 LOC, exception hierarchy + mapping)
- `crates/aviso-py/src/error_tests.rs` (~80 LOC)
- `crates/aviso-py/src/runtime.rs` (~30 LOC)
- `python/aviso/__init__.pyi` (~250 LOC type stubs)
- `python/tests/conftest.py` (~40 LOC)
- `python/tests/test_*.py` (16 files, ~50-150 LOC each, total ~1500 LOC)
- `python/tests/__init__.py` (empty)
- `docs/src/python/*.md` (11 files, total ~1200 lines of prose)
- `uv.lock` (generated)

**Modified** (~15 files):

- `crates/aviso-py/Cargo.toml` (rlib -> cdylib+rlib, add deps)
- `Cargo.toml` (workspace deps)
- `python/aviso/__init__.py` (re-exports from `_native`)
- `pyproject.toml` (add `[dependency-groups] dev = [...]` per PEP 735, NOT
  `[project.optional-dependencies]`; `uv sync --locked --group dev` reads the former. Add
  `[tool.uv]`, `[tool.pytest.ini_options]`, `[tool.ty]`)
- `.github/workflows/ci.yml` (add python job)
- `.githooks/pre-push` (add python steps)
- `CONTRIBUTING.md` (Python toolchain section gets actual commands)
- `README.md` (Python install snippet)
- `docs/src/SUMMARY.md` (nested Python block)
- `docs/src/index.md` (hero Python tab)
- `docs/src/getting-started/quickstart.md` (Python tab)
- `docs/src/getting-started/install.md` (Python section)
- `docs/src/concepts/streams.md` (Python aside)
- `docs/book.toml` (redirect from old `python/status.html`)
- `deny.toml` (no change expected, but verify pyo3-async-runtimes' license tree)

**Deleted** (1 file):

- `docs/src/python/status.md`

---

## 10. Commit plan

Per AGENTS.md: one concern per commit, ~15 files / ~500 added lines per commit (renames and
generated lockfiles excluded), each commit independently passes the 16-gate set (11 Rust + 5
Python).

The first commit is intentionally larger than the others because every gate must be honest
from commit 1 onwards: until the cdylib flip lands together with the PyO3 deps, the CI gate
config, the dev-deps in pyproject, the uv.lock, and at least one passing pytest, the new
Python gates would fail on intermediate commits and the AGENTS.md "every commit on `main`
builds" rule would be violated. The shape below is "scaffold once, then add one feature at a
time."

Estimated 19-22 commits, each gate-clean.

1. **`build(py): bootstrap PyO3 scaffold and Python CI`** (the gate-on commit; larger than
   subsequent commits, ~250 added LOC + uv.lock + Cargo.lock churn).
   - Workspace `Cargo.toml`: add `pyo3`, `pyo3-async-runtimes`, `pyo3-log`, `futures-util`,
     and `tracing` with the `log` feature.
   - `crates/aviso-py/Cargo.toml`: flip `crate-type` to `["cdylib", "rlib"]`; pull the new
     workspace deps; declare the `extension-module` and `abi3-py310` PyO3 features.
   - `crates/aviso-py/src/lib.rs`: rewrite from the 15-line VERSION-only placeholder to a
     proper `#[pymodule] fn _native(...)` that:
     - installs the logging bridge once via `OnceLock`,
     - registers `__version__` and `VERSION`.
   - `python/aviso/__init__.py`: keep `__version__` from importlib.metadata; import nothing
     from `_native` yet beyond the `VERSION` constant (the real surface fills in subsequent
     commits).
   - `python/aviso/__init__.pyi`: minimal stub matching the v1 `__init__.py` (just
     `__version__: str`).
   - `pyproject.toml`: bump `maturin>=1.13,<2` in `[build-system].requires`; add
     `[dependency-groups]` `dev = ["maturin>=1.13,<2", "ruff", "ty", "pytest",
     "pytest-asyncio", "pytest-httpserver", "hypothesis", "aiohttp"]`. The `dev` group lists
     `maturin`, `ruff`, and `ty` explicitly so `uv sync --locked --group dev` installs them
     into the project venv; relying on `build-system.requires` would only fetch maturin
     during a build and not provide a `uv run maturin` CLI in the dev environment. Add
     `[tool.pytest.ini_options]` with `asyncio_mode = "auto"`; add `[tool.uv]`; add
     `[tool.ty]`.
   - `uv.lock`: generated from `uv lock`, committed.
   - `python/tests/__init__.py` (empty), `python/tests/conftest.py` (~30 LOC), and
     `python/tests/test_smoke.py` (one test asserting `import aviso` works and
     `aviso.__version__` is a string).
   - `.github/workflows/ci.yml`: add the `python` job with the Linux 3.10 + Linux 3.13 + macOS
     3.13 matrix from §8.
   - `.githooks/pre-push`: add the five Python steps (ruff check + format check + ty check +
     maturin develop + pytest).
   - `CONTRIBUTING.md`: replace the "Python toolchain (once `python/aviso/` carries code)"
     placeholder with the real commands.
   - `README.md`: update the Python install snippet to the source-install commands.
   - File touch count: ~13 hand-written files + uv.lock + Cargo.lock churn. AGENTS.md's
     ~15-files rule passes because the lockfiles are generated; the hand-written diff is
     ~250 lines.

   After commit 1: all 16 gates green; `import aviso` works; the test suite has one test.

2. `feat(py): tokio runtime helper and ClientError -> PyErr mapping skeleton`. Adds
   `crates/aviso-py/src/runtime.rs` (the `get_runtime()` wrapper) and `error.rs` with the
   exception-class registration plus a `map_client_error` stub that handles `Transport` and
   `Http` to start. One pytest covers each mapped variant.

3. `feat(py): expand exception mapping to every ClientError variant + structured fields`.
   Adds the remaining variants (Auth, Decode, MalformedEvent, HistoryGap, StreamProtocol,
   Config, StateStore, TriggerFailed) and their structured attributes. `test_exception_attributes.py`
   covers each.

4. `feat(py): Notification, NotifyResponse, SchemaCatalog, SchemaResponse value types`.
   Adds `crates/aviso-py/src/values/notification.rs`, `notify_response.rs`, `schema.rs`,
   `mod.rs`. PyO3 classes with `#[pyo3(get)]` and explicit `#[pyo3(signature = (...))]`.
   `test_values.py` round-trips construction, repr, equality, `as_dict()`.

5. `feat(py): WatchRequest, ResumeStart, WatchMode value types`. Adds
   `watch_request.rs`, `resume_start.rs`, `watch_mode_conv.rs` (the last is a private Rust
   conversion helper, not a public PyO3 class; the user-facing `WatchMode` and `HttpMethod`
   are added as `class X(str, Enum)` in `python/aviso/__init__.py` so they stay 3.10
   compatible). PyO3 methods that accept these enums declare the parameter as `String` and
   accept either the enum value or the bare string at runtime (the enum auto-extracts to
   its string value because it inherits from `str`).

6. `feat(py): Trigger builder (echo, log, command, webhook, teams, post)`. Adds
   `crates/aviso-py/src/triggers.rs` and `triggers_tests.rs`. Each constructor accepts
   keyword args; chainable setters delegate to the same underlying Rust setters. Adds the
   shared `normalize_path` helper (`crates/aviso-py/src/paths.rs`) used by every path-typed
   PyO3 parameter (Rust-side `os.fspath` + `pathlib.Path.expanduser` under the GIL, then
   `PathBuf`).

7. `feat(py): auth providers (Bearer, Basic, Env, ConfigFile, Chain)`. Adds `auth.rs` and
   `auth_tests.rs`. `ConfigFile` consumes `normalize_path`. Type alias `AuthProvider =
   Bearer | Basic | Env | ConfigFile | Chain` in the stubs.

8. `feat(py): state stores (MemoryStore, JsonFileStore)`. Adds `state_stores.rs` and tests.
   `JsonFileStore` consumes `normalize_path`. Type alias `StateStore = MemoryStore |
   JsonFileStore` in stubs. `test_path_handling.py` covers str, Path, ~, missing-parent.

9. `feat(py): AvisoClient (sync) - notify, schema, schema_for, admin`. Adds
   `crates/aviso-py/src/clients/sync.rs`, the synchronous methods. `test_clients.py`,
   `test_notify.py`, `test_schema.py`, `test_admin.py` for the sync paths.

10. `feat(py): AsyncAvisoClient (async) - notify, schema, schema_for, admin`. Adds
    `clients/async_.rs` (uses `future_into_py`). Tests extended to cover async paths.

11. `feat(py): NotificationIterator (sync) + listen() arg validation`. Adds
    `streams/sync_iter.rs` with the polling-loop shape from §5.4. Adds
    `test_listen_sync.py`, `test_listen_validation.py`, `test_keyboard_interrupt_sync.py`,
    `test_iter_close.py`.

12. `feat(py): AsyncNotificationIterator + AsyncAvisoClient.listen`. Adds
    `streams/async_iter.rs` and `streams/mod.rs`. Adds `test_listen_async.py`,
    `test_keyboard_interrupt_async.py`.

13. `feat(py): __init__.py re-exports + __init__.pyi full stubs`. Wires every public name
    through the curated `python/aviso/__init__.py` re-export list; lands the full stub file;
    adds `test_stub_completeness.py` and `test_stub_signatures.py`.

14. `test(py): listen_resume + state_store integration`. Adds `test_listen_resume.py`
    (state-store-backed resume across two iterations). Touches no production code.

15. `test(py): hypothesis property tests + logging bridge`. Adds `test_property_filters.py`,
    `test_property_resume_start.py`, `test_logging_bridge.py`,
    `test_json_payload_conversion.py`.

16. `docs(py): overview + install + quickstart pages`. Adds the first three docs pages plus
    the redirect from the old `status.md`. SUMMARY.md gets the nested block now (so all
    later docs commits' pages are reachable from mdbook build).

17. `docs(py): publish + listen + triggers pages`.

18. `docs(py): auth + state-and-resume + error-handling pages`.

19. `docs(py): api-reference + troubleshooting pages`.

20. `docs(py): index + getting-started Python tabs + concepts/streams aside`. Touches
    `docs/src/index.md`, `docs/src/getting-started/quickstart.md`,
    `docs/src/getting-started/install.md`, `docs/src/concepts/streams.md`.

21. `chore(py): delete docs/src/python/status.md placeholder`. (book.toml redirect was added
    in commit 16; this commit just removes the source file.)

If commits 13-15 (test suite) come out heavier than the 500-line guidance, split per-test-file
batch. Commit ordering: tests depend on features, docs depend on the feature surface being
stable, CI was already wired in commit 1.

Each fact-check pass (§7.5) runs after the corresponding docs commit. A fact-check failure
produces a small fix commit on the branch with a `docs(py)` scope.

Mid-stream rebases acceptable: oracle plan/code reviews may ask for an earlier commit to grow
or shrink, an `interactive rebase` to amend it is the standard discipline.

---

## 11. Out of scope (carried forward)

The following are explicitly NOT in this PR. Each is listed so the omission is intentional, not
an oversight.

- **Python CLI wrapper**. The Rust CLI is the canonical CLI. A Python `argparse` / `typer` shim
  duplicates surface; users who need a CLI run `aviso ...` or `subprocess.run(["aviso", ...])`.
- **Wheel matrix for PyPI**. Linux + macOS in CI is enough to prove the bindings compile and the
  test suite passes. A follow-up PR adds `cibuildwheel` for manylinux x86_64/aarch64, musllinux,
  macOS universal2, Windows x86_64.
- **PyPI publish**. Same follow-up PR adds a `release` workflow that builds wheels on tag push
  and pushes them to PyPI under the `aviso` distribution name.
- **Custom Python `StateStore` impl**. Inheriting an `async` Rust trait from Python requires
  PyO3 plumbing the project does not need yet. The two shipped Rust impls (memory + JSON file)
  cover every current operator workflow. Lands when a real demand surfaces.
- **Custom Python `AuthProvider` impl**. Same reasoning. The five shipped providers (Bearer,
  Basic, Env, ConfigFile, Chain) cover the documented auth modes. OAuth2 / OIDC flows that need
  custom token-refresh land as a future provider.
- **Python callable trigger**. The async iteration loop body already does what a function
  trigger would do. Adding a `Trigger.python_callable(fn)` form pulls in tricky GIL semantics
  and a second iteration mechanism for the same task. Defer until somebody asks.
- **`pyo3-stub-gen`**. Hand-written stubs are sufficient at this size; auto-generated stubs add
  a build dependency and a synchronisation discipline for low payoff.
- **Async context manager for the watch stream itself**. The client classes are `with`/`async
  with`-able; the iterator returned by `listen` is currently a plain iterator. A future
  `async with client.listen(...) as stream:` form is feasible but adds a second cancellation
  surface; the current `for n in client.listen(...)` shape covers cancellation via iterator
  drop. Revisit if operators ask.

---

## 12. Open questions (none gate the PR)

- Should `Notification.payload` be deep-converted from `serde_json::Value` to a Python `dict` /
  `list` / `str` / `int` / `float` / `bool` / `None` once at construction, or lazily on access?
  Lazy preserves the original bytes; eager is what users expect when they `print(n.payload)`.
  Eager is the v1 plan; revisit if a real benchmark shows it hurts.

- Should the v1 Python API expose `client.watch_with_handler(req, fn)` even though there's no
  Python callable trigger? Probably no in v1: anyone using a handler is already in the async
  iteration loop body. Lands together with the callable trigger if it ever comes.

---

## 13. Definition of done

- Every commit on `feat/python-api` passes the full local gate (11 Rust gates + 5 Python
  gates): `uv run ruff check python/`, `uv run ruff format --check python/`,
  `uv run ty check python/`, `uv run maturin develop --locked`,
  `uv run pytest python/tests/`. The Python-side stub-completeness and stub-signature checks
  run inside the pytest gate.
- `pytest python/tests/` runs in under 30 seconds without a network connection.
- `mdbook test docs` continues to pass (Rust code blocks unchanged in the docs).
- A fresh `uv sync --locked && uv run maturin develop --locked && uv run pytest python/tests/`
  on a clean checkout produces an importable `aviso` module and a green test suite.
- A user can run the quickstart code block from `docs/src/python/quickstart.md` against the
  production aviso-server and receive a notification (verified through the §7.5 fact-check
  workspace before the docs commit lands).
- The eleven docs pages render correctly in `mdbook serve docs --open`; all mermaid diagrams
  switch theme cleanly between light and dark.
- `cargo deny check` continues to pass (no new licence categories).
- The CI Python matrix passes on all three cells (ubuntu/3.10, ubuntu/3.13, macos/3.13).
- Oracle code review issues PROCEED-TO-MERGE.
- Copilot review loop terminates per AGENTS.md.

The PR sits at the merge gate until the user says merge.
