"""Type stubs for the curated public surface of the aviso Python package.

Hand-written. Kept in sync with the runtime ``__all__`` via
``python/tests/test_stub_completeness.py``. Subsequent commits expand the
surface as the bindings ship; this file lists only the symbols available
in the current commit.
"""

from __future__ import annotations

__version__: str
VERSION: str

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
    "AvisoError",
    "ConfigError",
    "DecodeError",
    "HistoryGapError",
    "HttpError",
    "MalformedEventError",
    "StateStoreError",
    "StreamProtocolError",
    "TransportError",
    "TriggerError",
    "__version__",
]
