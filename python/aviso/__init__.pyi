"""Type stubs for the curated public surface of the aviso Python package.

Hand-written. Kept in sync with the runtime ``__all__`` via
``python/tests/test_stub_completeness.py``.
"""

from __future__ import annotations

import os
from collections.abc import Awaitable, Mapping
from enum import Enum

# reason: Notification.payload and filter values are JSON-shaped values
# (dict, list, str, int, float, bool, or None), so the stubs use `Any`
# at those positions deliberately.
from typing import Any

__version__: str
VERSION: str

class Notification:
    def __init__(
        self,
        event_type: str,
        sequence: int,
        identifier: Mapping[str, str],
        payload: Any,
        cloudevent: Mapping[str, Any] | None = None,
    ) -> None: ...
    @property
    def event_type(self) -> str: ...
    @property
    def sequence(self) -> int: ...
    @property
    def identifier(self) -> dict[str, str]: ...
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

class HttpMethod(str, Enum):
    POST = "POST"
    GET = "GET"
    PUT = "PUT"
    PATCH = "PATCH"
    DELETE = "DELETE"

class WatchMode(str, Enum):
    WATCH = "watch"
    REPLAY_ONLY = "replay_only"

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
        method: str | HttpMethod | None = None,
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
    def __init__(self, *providers: Any) -> None: ...

class MemoryStore:
    def __init__(self) -> None: ...

class JsonFileStore:
    def __init__(self, path: str | os.PathLike[str]) -> None: ...

AuthProvider = Bearer | Basic | Env | ConfigFile | Chain
StateStore = MemoryStore | JsonFileStore

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

class AsyncNotificationIterator:
    def __aiter__(self) -> AsyncNotificationIterator: ...
    def __anext__(self) -> Awaitable[Notification]: ...
    def aclose(self) -> Awaitable[None]: ...

class AvisoClient:
    def __init__(
        self,
        *,
        base_url: str,
        auth: AuthProvider | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: StateStore | None = None,
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
        identifier: Mapping[str, str] | None = None,
        payload: Any | None = None,
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
        filter: dict[str, Any] | None = None,
        from_: int | str | None = None,
        mode: WatchMode | str = "watch",
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
        auth: AuthProvider | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: StateStore | None = None,
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
        identifier: Mapping[str, str] | None = None,
        payload: Any | None = None,
    ) -> Awaitable[NotifyResponse]: ...
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
        mode: WatchMode | str = "watch",
        request: WatchRequest | None = None,
    ) -> AsyncNotificationIterator: ...

class AvisoError(Exception):
    """Base class for every exception raised by the aviso library."""

class TransportError(AvisoError):
    """Network-level failure before the server response begins."""

class HttpError(AvisoError):
    """Server returned a non-success HTTP status."""

    status: int
    body: str
    request_id: str | None

class AuthError(AvisoError):
    """Auth source resolution or refresh failed."""

class DecodeError(AvisoError):
    """Response body could not be decoded as the expected JSON shape."""

class MalformedEventError(AvisoError):
    """CloudEvent id field did not parse per <event_type>@<sequence>. Terminal per D9."""

class HistoryGapError(AvisoError):
    """A gap was detected in the watch stream. Terminal per D2."""

    reason: str
    max_allowed: int | None
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

__all__ = [
    "VERSION",
    "AsyncAvisoClient",
    "AsyncNotificationIterator",
    "AuthError",
    "AuthProvider",
    "AvisoClient",
    "AvisoError",
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
    "SchemaCatalog",
    "SchemaResponse",
    "StateStore",
    "StateStoreError",
    "StreamProtocolError",
    "TransportError",
    "Trigger",
    "TriggerError",
    "WatchMode",
    "WatchRequest",
    "__version__",
]
