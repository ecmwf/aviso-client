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

__all__ = [
    "VERSION",
    "AvisoError",
    "HttpError",
    "TransportError",
    "__version__",
]
