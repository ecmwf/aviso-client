# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

"""Type stubs for the PyO3 extension module `pyaviso._native`.

Hand-written. Users never import this module directly; the public Python
surface lives in `pyaviso.__init__`. These stubs describe the compiled
extension's exports so `ty check` can resolve imports from the wrapper
package.
"""

from __future__ import annotations

import os
from collections.abc import Awaitable, Callable, Mapping, Sequence

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
    def __str__(self) -> str: ...

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
    @staticmethod
    def function(
        func: Callable[[Notification], object],
        *,
        retries: int = 0,
        required: bool = True,
        label: str | None = None,
    ) -> Trigger:
        """Calls ``func`` with each notification.

        It runs in the thread that reads the notification (or on its event
        loop, where ``async def`` functions are awaited), one notification at
        a time, after the built-in triggers. ``retries`` calls it again when
        it raises. A required function that still fails raises
        ``TriggerError``, or goes to ``on_error`` with ``listen_many``; an
        optional one is logged and skipped. ``timeout`` and ``fail_fast`` do
        not apply to functions.
        """
        ...
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

class Anonymous:
    def __init__(self) -> None: ...

class MemoryStore:
    def __init__(self) -> None: ...

class JsonFileStore:
    def __init__(self, path: str | os.PathLike[str]) -> None: ...

class WatchRequest:
    @staticmethod
    def watch(event_type: str) -> WatchRequest: ...
    @staticmethod
    def watch_from(event_type: str, start_from: int | str) -> WatchRequest: ...
    @staticmethod
    def replay_only(event_type: str, start_from: int | str) -> WatchRequest: ...
    def with_filter(self, filter: dict[str, Any]) -> WatchRequest: ...
    def with_triggers(self, triggers: list[Trigger]) -> WatchRequest: ...
    @property
    def event_type(self) -> str: ...
    @property
    def mode(self) -> str: ...

class NotificationIterator:
    def __iter__(self) -> NotificationIterator: ...
    def __next__(self) -> Notification: ...
    def run(self) -> None:
        """Reads every notification until the stream ends, running its
        triggers, then closes it."""
        ...
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
    def run(self) -> Awaitable[None]:
        """Reads every notification until the stream ends, running its
        triggers, then closes it."""
        ...
    def aclose(self) -> Awaitable[None]: ...
    def __aenter__(self) -> Awaitable[AsyncNotificationIterator]: ...
    def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: object | None,
    ) -> Awaitable[None]: ...

class SourcedValue:
    @property
    def value(self) -> Any: ...
    @property
    def source(self) -> str: ...

class ResolvedAuth:
    @property
    def kind(self) -> str: ...
    @property
    def source(self) -> str: ...
    @property
    def refused(self) -> str | None: ...

class ResolvedConfig:
    @property
    def base_url(self) -> SourcedValue | None: ...
    @property
    def timeout(self) -> SourcedValue: ...
    @property
    def heartbeat_interval(self) -> SourcedValue: ...
    @property
    def ca_bundle(self) -> SourcedValue: ...
    @property
    def danger_accept_invalid_certs(self) -> SourcedValue: ...
    @property
    def auth(self) -> ResolvedAuth | None: ...
    @property
    def config_file(self) -> str | None: ...
    @property
    def credentials_file(self) -> str | None: ...
    def as_dict(self) -> dict[str, Any]: ...

def resolve_config(
    *,
    base_url: str | None = None,
    auth: Bearer | Basic | Env | ConfigFile | Chain | Anonymous | None = None,
    timeout: float | None = None,
    heartbeat_interval: float | None = None,
    danger_accept_invalid_certs: bool | None = None,
) -> ResolvedConfig: ...

class AvisoClient:
    def __init__(
        self,
        *,
        base_url: str | None = None,
        auth: Bearer | Basic | Env | ConfigFile | Chain | Anonymous | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: MemoryStore | JsonFileStore | None = None,
        heartbeat_interval: float | None = None,
        danger_accept_invalid_certs: bool | None = None,
        flush_cursor_on_exit: bool | None = None,
    ) -> None: ...
    @staticmethod
    def from_file(
        path: str | os.PathLike[str] | None = None,
        *,
        base_url: str | None = None,
        auth: Bearer | Basic | Env | ConfigFile | Chain | Anonymous | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: MemoryStore | JsonFileStore | None = None,
        heartbeat_interval: float | None = None,
        danger_accept_invalid_certs: bool | None = None,
        flush_cursor_on_exit: bool | None = None,
    ) -> AvisoClient: ...
    @property
    def base_url(self) -> str: ...
    @property
    def config(self) -> ResolvedConfig: ...
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
    # reason: the wrapper types live in pyaviso._many, which this native stub
    # does not import; pyaviso/__init__.pyi declares the public types.
    def listen(
        self,
        event_type: str | None = None,
        *,
        filter: dict[str, Any] | None = None,
        start_from: int | str | None = None,
        mode: str | None = None,
        triggers: Sequence[Trigger] | None = None,
        request: WatchRequest | None = None,
    ) -> NotificationIterator | Any: ...
    # reason: the wrapper types live in pyaviso._many, which this native stub
    # does not import; pyaviso/__init__.pyi declares the public types.
    def listen_many(
        self,
        listeners: Mapping[str, Mapping[str, Any] | WatchRequest],
        *,
        start_from: int | str | None = None,
        mode: str | None = None,
        on_error: str | Callable[[str, BaseException], object] | None = None,
    ) -> Any:
        """Listens to several things at once, through one loop.

        ``listeners`` maps a name of your choosing to the ``listen()``
        keywords for that listener (``event_type``, ``filter``,
        ``start_from``, ``mode``, ``triggers``), or to a ``WatchRequest``.
        ``start_from`` and ``mode`` given here apply to every dict entry that
        does not set its own. The loop yields ``(name, notification)``.

        ``on_error`` decides what a failure does: ``"raise"`` stops every
        listener and raises; ``"continue"`` drops a failed listener, or skips
        the notification a failed ``Trigger.function`` was called for, and
        keeps going; a function is called with ``(name, error)`` and stops
        everything if it raises. If every listener fails, the loop raises
        ``AvisoError``.
        """
        ...
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
        base_url: str | None = None,
        auth: Bearer | Basic | Env | ConfigFile | Chain | Anonymous | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: MemoryStore | JsonFileStore | None = None,
        heartbeat_interval: float | None = None,
        danger_accept_invalid_certs: bool | None = None,
        flush_cursor_on_exit: bool | None = None,
    ) -> None: ...
    @staticmethod
    def from_file(
        path: str | os.PathLike[str] | None = None,
        *,
        base_url: str | None = None,
        auth: Bearer | Basic | Env | ConfigFile | Chain | Anonymous | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: MemoryStore | JsonFileStore | None = None,
        heartbeat_interval: float | None = None,
        danger_accept_invalid_certs: bool | None = None,
        flush_cursor_on_exit: bool | None = None,
    ) -> AsyncAvisoClient: ...
    @property
    def base_url(self) -> str: ...
    @property
    def config(self) -> ResolvedConfig: ...
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
    # reason: the wrapper types live in pyaviso._many, which this native stub
    # does not import; pyaviso/__init__.pyi declares the public types.
    def listen(
        self,
        event_type: str | None = None,
        *,
        filter: dict[str, Any] | None = None,
        start_from: int | str | None = None,
        mode: str | None = None,
        triggers: Sequence[Trigger] | None = None,
        request: WatchRequest | None = None,
    ) -> AsyncNotificationIterator | Any: ...
    # reason: the wrapper types live in pyaviso._many, which this native stub
    # does not import; pyaviso/__init__.pyi declares the public types.
    def listen_many(
        self,
        listeners: Mapping[str, Mapping[str, Any] | WatchRequest],
        *,
        start_from: int | str | None = None,
        mode: str | None = None,
        on_error: str | Callable[[str, BaseException], object] | None = None,
    ) -> Any:
        """Listens to several things at once, through one loop.

        ``listeners`` maps a name of your choosing to the ``listen()``
        keywords for that listener (``event_type``, ``filter``,
        ``start_from``, ``mode``, ``triggers``), or to a ``WatchRequest``.
        ``start_from`` and ``mode`` given here apply to every dict entry that
        does not set its own. The loop yields ``(name, notification)``.

        ``on_error`` decides what a failure does: ``"raise"`` stops every
        listener and raises; ``"continue"`` drops a failed listener, or skips
        the notification a failed ``Trigger.function`` was called for, and
        keeps going; a function is called with ``(name, error)`` and stops
        everything if it raises. If every listener fails, the loop raises
        ``AvisoError``.
        """
        ...

class AvisoError(Exception):
    listener: str | None
    # reason: the elements are pyaviso._many.ListenFailure, which this
    # native stub does not import.
    failures: list[Any] | None

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
