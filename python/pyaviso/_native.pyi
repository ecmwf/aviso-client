"""Type stubs for the PyO3 extension module `pyaviso._native`.

Hand-written. Users never import this module directly; the public Python
surface lives in `pyaviso.__init__`. These stubs describe the compiled
extension's exports so `ty check` can resolve imports from the wrapper
package.
"""

from __future__ import annotations

import os
from collections.abc import Awaitable, Mapping, Sequence

# reason: Notification payload and identifier/filter values are JSON-shaped values
# (dict, list, str, int, float, bool, or None), so the stubs use `Any`
# at those positions deliberately.
from typing import Any

VERSION: str

class Notification:
    def __init__(
        self,
        event_type: str,
        sequence: int,
        identifier: Mapping[str, Any],
        payload: Any,
        cloudevent: Mapping[str, Any] | None = None,
    ) -> None: ...
    @property
    def event_type(self) -> str: ...
    @property
    def sequence(self) -> int: ...
    @property
    def identifier(self) -> dict[str, Any]: ...
    @property
    def payload(self) -> Any: ...
    @property
    def cloudevent(self) -> dict[str, Any] | None: ...
    def as_dict(self) -> dict[str, Any]: ...

class NotifyResponse:
    def __init__(self, status: str, request_id: str, processed_at: str) -> None: ...
    @property
    def status(self) -> str: ...
    @property
    def request_id(self) -> str: ...
    @property
    def processed_at(self) -> str: ...
    def as_dict(self) -> dict[str, Any]: ...

class NotifyResult:
    @property
    def index(self) -> int: ...
    @property
    def ok(self) -> bool: ...
    @property
    def response(self) -> NotifyResponse | None: ...
    @property
    def error(self) -> AvisoError | None: ...

class SchemaCatalog:
    @property
    def status(self) -> str: ...
    @property
    def schema(self) -> dict[str, Any]: ...
    @property
    def event_types(self) -> list[str]: ...
    @property
    def total_schemas(self) -> int: ...
    def as_dict(self) -> dict[str, Any]: ...

class SchemaResponse:
    @property
    def status(self) -> str: ...
    @property
    def event_type(self) -> str: ...
    @property
    def schema(self) -> dict[str, Any]: ...
    def as_dict(self) -> dict[str, Any]: ...

class Trigger:
    @staticmethod
    def echo(*, retries: int = 0, required: bool = True, label: str | None = None) -> Trigger: ...
    @staticmethod
    def log(
        path: str | os.PathLike[str], *, retries: int = 0, required: bool = True
    ) -> Trigger: ...
    @staticmethod
    def command(
        cmd: str,
        *,
        env: dict[str, str] | None = None,
        working_dir: str | os.PathLike[str] | None = None,
        retries: int = 0,
        required: bool = True,
        timeout: float | None = None,
        fail_fast: bool = True,
    ) -> Trigger: ...
    @staticmethod
    def webhook(
        url: str,
        *,
        method: str | None = None,
        headers: dict[str, str] | None = None,
        body_template: str | None = None,
        retries: int = 0,
        required: bool = True,
        timeout: float = 30.0,
        fail_fast: bool = True,
    ) -> Trigger: ...
    @staticmethod
    def teams(
        url: str,
        *,
        retries: int = 0,
        required: bool = True,
        timeout: float = 30.0,
        fail_fast: bool = True,
    ) -> Trigger: ...
    @staticmethod
    def post(
        url: str,
        *,
        retries: int = 0,
        required: bool = True,
        timeout: float = 30.0,
        fail_fast: bool = True,
    ) -> Trigger: ...
    def retries(self, n: int) -> Trigger: ...
    def required(self, on: bool) -> Trigger: ...
    def timeout(self, seconds: float) -> Trigger: ...
    def fail_fast(self, on: bool) -> Trigger: ...
    def label(self, name: str) -> Trigger: ...

class Bearer:
    def __init__(self, token: str) -> None: ...

class Basic:
    def __init__(self, username: str, password: str = "") -> None: ...

class Env:
    def __init__(self) -> None: ...

class ConfigFile:
    def __init__(self, path: str | os.PathLike[str]) -> None: ...

class Chain:
    def __init__(self, *providers: Bearer | Basic | Env | ConfigFile | Chain) -> None: ...

class MemoryStore:
    def __init__(self) -> None: ...

class JsonFileStore:
    def __init__(self, path: str | os.PathLike[str]) -> None: ...

class WatchRequest:
    @staticmethod
    def watch(event_type: str) -> WatchRequest: ...
    @staticmethod
    def watch_from(event_type: str, from_: int | str) -> WatchRequest: ...
    @staticmethod
    def replay_only(event_type: str, from_: int | str) -> WatchRequest: ...
    def with_filter(self, filter: dict[str, Any]) -> WatchRequest: ...
    def with_triggers(self, triggers: list[Trigger]) -> WatchRequest: ...
    @property
    def event_type(self) -> str: ...
    @property
    def mode(self) -> str: ...

class NotificationIterator:
    def __iter__(self) -> NotificationIterator: ...
    def __next__(self) -> Notification: ...
    def close(self) -> None: ...
    def __enter__(self) -> NotificationIterator: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: object | None,
    ) -> bool: ...

class AsyncNotificationIterator:
    def __aiter__(self) -> AsyncNotificationIterator: ...
    def __anext__(self) -> Awaitable[Notification]: ...
    def aclose(self) -> Awaitable[None]: ...
    def __aenter__(self) -> Awaitable[AsyncNotificationIterator]: ...
    def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: object | None,
    ) -> Awaitable[None]: ...

class AvisoClient:
    def __init__(
        self,
        *,
        base_url: str,
        auth: Bearer | Basic | Env | ConfigFile | Chain | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: MemoryStore | JsonFileStore | None = None,
        heartbeat_interval: float | None = None,
        danger_accept_invalid_certs: bool = False,
        flush_cursor_on_exit: bool = False,
    ) -> None: ...
    @property
    def base_url(self) -> str: ...
    def notify(
        self,
        *,
        event_type: str,
        identifier: Mapping[str, Any] | None = None,
        payload: Any | None = None,
    ) -> NotifyResponse: ...
    def notify_many(
        self,
        notifications: Sequence[Mapping[str, Any]],
        *,
        concurrency: int = 0,
    ) -> list[NotifyResult]: ...
    def schema(self) -> SchemaCatalog: ...
    def schema_for(self, event_type: str) -> SchemaResponse: ...
    def wipe_stream(self, stream_name: str) -> None: ...
    def wipe_all(self) -> None: ...
    def delete_notification(self, notification_id: str) -> None: ...
    def listen(
        self,
        event_type: str | None = None,
        *,
        filter: dict[str, Any] | None = None,
        from_: int | str | None = None,
        mode: str | None = None,
        triggers: Sequence[Trigger] | None = None,
        request: WatchRequest | None = None,
    ) -> NotificationIterator: ...
    def __enter__(self) -> AvisoClient: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: object | None,
    ) -> bool: ...

class AsyncAvisoClient:
    def __init__(
        self,
        *,
        base_url: str,
        auth: Bearer | Basic | Env | ConfigFile | Chain | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: MemoryStore | JsonFileStore | None = None,
        heartbeat_interval: float | None = None,
        danger_accept_invalid_certs: bool = False,
        flush_cursor_on_exit: bool = False,
    ) -> None: ...
    @property
    def base_url(self) -> str: ...
    def notify(
        self,
        *,
        event_type: str,
        identifier: Mapping[str, Any] | None = None,
        payload: Any | None = None,
    ) -> Awaitable[NotifyResponse]: ...
    def notify_many(
        self,
        notifications: Sequence[Mapping[str, Any]],
        *,
        concurrency: int = 0,
    ) -> Awaitable[list[NotifyResult]]: ...
    def schema(self) -> Awaitable[SchemaCatalog]: ...
    def schema_for(self, event_type: str) -> Awaitable[SchemaResponse]: ...
    def wipe_stream(self, stream_name: str) -> Awaitable[None]: ...
    def wipe_all(self) -> Awaitable[None]: ...
    def delete_notification(self, notification_id: str) -> Awaitable[None]: ...
    def listen(
        self,
        event_type: str | None = None,
        *,
        filter: dict[str, Any] | None = None,
        from_: int | str | None = None,
        mode: str | None = None,
        triggers: Sequence[Trigger] | None = None,
        request: WatchRequest | None = None,
    ) -> AsyncNotificationIterator: ...

class AvisoError(Exception): ...
class TransportError(AvisoError): ...

class HttpError(AvisoError):
    status: int
    body: str
    request_id: str | None

class AuthError(AvisoError): ...
class DecodeError(AvisoError): ...
class MalformedEventError(AvisoError): ...

class HistoryGapError(AvisoError):
    reason: str
    max_allowed: int | None
    expected: int | None
    observed: int | None

class StreamProtocolError(AvisoError):
    message: str
    request_id: str | None

class ConfigError(AvisoError): ...
class StateStoreError(AvisoError): ...

class TriggerError(AvisoError):
    trigger_kind: str
    error_kind: str
    path: str | None
    exit_code: int | None
    stderr_tail: str | None
    status: int | None
    body_tail: str | None
    reason: str | None
    timeout_seconds: float | None
    context: str | None
    field: str | None
    template_kind: str | None

def _provoke_error(
    kind: str,
    *,
    status: int | None = None,
    body: str | None = None,
    request_id: str | None = None,
    message: str | None = None,
    detail: str | None = None,
    sequence: int | None = None,
    max_allowed: int | None = None,
    expected: int | None = None,
    observed: int | None = None,
    log_path: str | None = None,
) -> None: ...
def _run_cli(argv: list[str]) -> int: ...
