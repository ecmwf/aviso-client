# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

"""Type stubs for the curated public surface of the pyaviso Python package.

Hand-written. Kept in sync with the runtime ``__all__`` via
``python/tests/test_stub_completeness.py``.
"""

from __future__ import annotations

import os
from collections.abc import Awaitable, Mapping, Sequence
from enum import Enum

# reason: Notification payload and identifier/filter values are JSON-shaped values
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
    def __init__(self, *providers: AuthProvider) -> None: ...

class Anonymous:
    """Marker that turns off credential discovery.

    A client built with ``auth=Anonymous()`` sends no ``Authorization``
    header even when a credential is present in the environment or in a
    file. It carries no credential, so it cannot go into a ``Chain``.
    """

    def __init__(self) -> None: ...

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

class SourcedValue:
    """One resolved setting: its value and where it came from.

    ``source`` is one of ``code``, ``environment <NAME>``,
    ``config file <path>``, ``credentials file <path>`` or ``default``.
    """

    @property
    def value(self) -> Any: ...
    @property
    def source(self) -> str: ...

class ResolvedAuth:
    """The credential a client sends, described without the secret."""

    @property
    def kind(self) -> str:
        """``bearer``, ``basic``, ``anonymous`` when the caller asked for
        none, ``chain``, or a custom provider's name."""
        ...
    @property
    def source(self) -> str: ...
    @property
    def refused(self) -> str | None:
        """Why the credential is not being sent, when it is not.

        Set when a credential that was found rather than passed would go to
        a plain ``http://`` address on a host that is not loopback. A client
        built in that state raises ``AuthError``; this says why.
        """
        ...

class ResolvedConfig:
    """Everything a client's connection depends on, each with its source.

    Returned by ``resolve_config()`` and ``client.config``. ``repr()`` lays it
    out one setting per line and is written to be pasted into a ticket;
    ``as_dict()`` gives the same as plain values for JSON logging. Nothing in
    it is a secret.
    """

    @property
    def base_url(self) -> SourcedValue | None:
        """The address with credentials removed, or ``None`` if nothing set one."""
        ...
    @property
    def timeout(self) -> SourcedValue:
        """Seconds allowed for an ordinary request such as ``notify`` or
        ``schema``, or ``None`` for no limit. It does not apply to ``listen``.
        """
        ...
    @property
    def heartbeat_interval(self) -> SourcedValue:
        """Seconds, or ``None`` for the library default."""
        ...
    @property
    def ca_bundle(self) -> SourcedValue:
        """Extra CA bundle paths."""
        ...
    @property
    def danger_accept_invalid_certs(self) -> SourcedValue: ...
    @property
    def auth(self) -> ResolvedAuth | None:
        """The credential, or ``None`` when no source supplied one.

        ``Anonymous()`` passed in code is reported as kind ``anonymous``
        from ``code``, not as ``None``.
        """
        ...
    @property
    def config_file(self) -> str | None:
        """The config file that was read, if one existed."""
        ...
    @property
    def credentials_file(self) -> str | None:
        """The credentials file that was consulted, if any."""
        ...
    def as_dict(self) -> dict[str, Any]: ...

def resolve_config(
    *,
    base_url: str | None = None,
    auth: AuthProvider | Anonymous | None = None,
    timeout: float | None = None,
    heartbeat_interval: float | None = None,
    danger_accept_invalid_certs: bool | None = None,
) -> ResolvedConfig:
    """Reports what a client built with these arguments would use, without
    connecting.

    Takes the ``AvisoClient()`` arguments that take part in the lookup, with
    the same order of precedence: code, then the environment, then the
    config file, then the default. ``user_agent``, ``state_store`` and
    ``flush_cursor_on_exit`` are never looked up and are not accepted here.

    Works even when a client could not be built, for example with no
    address anywhere (``base_url`` is then ``None``) or with a found
    credential that would be refused (``auth.refused`` says why). Raises
    ``ConfigError`` when the config or credentials file exists but cannot be
    read, and ``AuthError`` when a credential source is present but
    unusable.
    """
    ...

class AvisoClient:
    def __init__(
        self,
        *,
        base_url: str | None = None,
        auth: AuthProvider | Anonymous | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: StateStore | None = None,
        heartbeat_interval: float | None = None,
        danger_accept_invalid_certs: bool | None = None,
        flush_cursor_on_exit: bool | None = None,
    ) -> None:
        """Builds a client.

        Every argument is optional. What is not given is taken from the
        environment, then the aviso config file, then the library default:
        ``base_url`` from ``AVISO_BASE_URL`` or the file's ``base_url``;
        ``auth`` from ``AVISO_TOKEN`` or ``AVISO_USERNAME`` with
        ``AVISO_PASSWORD``, the file's ``auth:`` block, or the credentials
        file; ``timeout``, ``heartbeat_interval`` and the TLS settings from
        the file. ``config`` shows what was chosen and from where.

        A credential that was found rather than passed is not sent to a
        plain ``http://`` address unless it is loopback; ``AuthError`` is
        raised instead. ``auth=Anonymous()`` sends none. With no address in
        code, the environment or the file, ``ConfigError`` is raised.
        """
        ...
    @staticmethod
    def from_file(
        path: str | os.PathLike[str] | None = None,
        *,
        base_url: str | None = None,
        auth: AuthProvider | Anonymous | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: StateStore | None = None,
        heartbeat_interval: float | None = None,
        danger_accept_invalid_certs: bool | None = None,
        flush_cursor_on_exit: bool | None = None,
    ) -> AvisoClient:
        """Builds a client from the aviso config file.

        Reads ``~/.config/aviso/config.yaml`` (or ``AVISO_CLIENT_CONFIG_FILE``),
        or ``path`` when given, for ``base_url``, ``timeout``,
        ``heartbeat_interval`` and ``tls``, and finds a credential the way the
        ``aviso`` binary does. Keyword arguments given here replace what the
        file said. ``auth=Anonymous()`` removes a found credential.

        A missing default file sets nothing. A ``path`` that does not exist,
        or a file that cannot be read, raises ``ConfigError``.
        """
        ...
    @property
    def base_url(self) -> str:
        """The base URL exactly as configured, ``user:password@`` included.

        ``repr(client)`` and ``client.config`` show it without the
        credentials, and are the forms to log.
        """
        ...
    @property
    def config(self) -> ResolvedConfig:
        """The settings this client uses and where each came from.

        Safe to log: the credential is described by kind and source, never
        by value, and the address has any ``user:password@`` removed.
        """
        ...
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
        start_from: int | str | None = None,
        mode: WatchMode | str | None = None,
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
        base_url: str | None = None,
        auth: AuthProvider | Anonymous | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: StateStore | None = None,
        heartbeat_interval: float | None = None,
        danger_accept_invalid_certs: bool | None = None,
        flush_cursor_on_exit: bool | None = None,
    ) -> None:
        """Builds a client.

        Every argument is optional. What is not given is taken from the
        environment, then the aviso config file, then the library default:
        ``base_url`` from ``AVISO_BASE_URL`` or the file's ``base_url``;
        ``auth`` from ``AVISO_TOKEN`` or ``AVISO_USERNAME`` with
        ``AVISO_PASSWORD``, the file's ``auth:`` block, or the credentials
        file; ``timeout``, ``heartbeat_interval`` and the TLS settings from
        the file. ``config`` shows what was chosen and from where.

        A credential that was found rather than passed is not sent to a
        plain ``http://`` address unless it is loopback; ``AuthError`` is
        raised instead. ``auth=Anonymous()`` sends none. With no address in
        code, the environment or the file, ``ConfigError`` is raised.
        """
        ...
    @staticmethod
    def from_file(
        path: str | os.PathLike[str] | None = None,
        *,
        base_url: str | None = None,
        auth: AuthProvider | Anonymous | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
        state_store: StateStore | None = None,
        heartbeat_interval: float | None = None,
        danger_accept_invalid_certs: bool | None = None,
        flush_cursor_on_exit: bool | None = None,
    ) -> AsyncAvisoClient:
        """Builds a client from the aviso config file.

        Reads ``~/.config/aviso/config.yaml`` (or ``AVISO_CLIENT_CONFIG_FILE``),
        or ``path`` when given, for ``base_url``, ``timeout``,
        ``heartbeat_interval`` and ``tls``, and finds a credential the way the
        ``aviso`` binary does. Keyword arguments given here replace what the
        file said. ``auth=Anonymous()`` removes a found credential.

        A missing default file sets nothing. A ``path`` that does not exist,
        or a file that cannot be read, raises ``ConfigError``.
        """
        ...
    @property
    def base_url(self) -> str:
        """The base URL exactly as configured, ``user:password@`` included.

        ``repr(client)`` and ``client.config`` show it without the
        credentials, and are the forms to log.
        """
        ...
    @property
    def config(self) -> ResolvedConfig:
        """The settings this client uses and where each came from.

        Safe to log: the credential is described by kind and source, never
        by value, and the address has any ``user:password@`` removed.
        """
        ...
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
        start_from: int | str | None = None,
        mode: WatchMode | str | None = None,
        triggers: Sequence[Trigger] | None = None,
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
    "Anonymous",
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
    "NotifyResult",
    "ResolvedAuth",
    "ResolvedConfig",
    "SchemaCatalog",
    "SchemaResponse",
    "SourcedValue",
    "StateStore",
    "StateStoreError",
    "StreamProtocolError",
    "TransportError",
    "Trigger",
    "TriggerError",
    "WatchMode",
    "WatchRequest",
    "__version__",
    "resolve_config",
]
