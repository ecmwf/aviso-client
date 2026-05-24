"""Type stubs for the curated public surface of the aviso Python package.

Hand-written. Kept in sync with the runtime ``__all__`` via
``python/tests/test_stub_completeness.py``. Subsequent commits expand the
surface as the bindings ship; this file lists only the symbols available
in the current commit.
"""

from __future__ import annotations

from collections.abc import Mapping
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

class AvisoClient:
    def __init__(
        self,
        *,
        base_url: str,
        token: str | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
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
    def __enter__(self) -> AvisoClient: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: Any | None,
    ) -> bool: ...

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
    "AuthError",
    "AvisoClient",
    "AvisoError",
    "ConfigError",
    "DecodeError",
    "HistoryGapError",
    "HttpError",
    "MalformedEventError",
    "Notification",
    "NotifyResponse",
    "SchemaCatalog",
    "SchemaResponse",
    "StateStoreError",
    "StreamProtocolError",
    "TransportError",
    "TriggerError",
    "__version__",
]
