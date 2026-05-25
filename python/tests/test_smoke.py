"""Smoke tests for the aviso Python package.

These tests run on every CI cell and the pre-push hook. They verify that
the extension imports, that the documented public surface is the expected
shape, and that the version string is consistent across the Python
package and the underlying Rust crate.
"""

from __future__ import annotations

import re

import aviso


def test_package_imports() -> None:
    assert aviso is not None


def test_version_is_a_string() -> None:
    assert isinstance(aviso.__version__, str)
    assert aviso.__version__


def test_native_version_matches_rust_crate() -> None:
    assert isinstance(aviso.VERSION, str)
    assert re.match(r"^\d+\.\d+\.\d+", aviso.VERSION)
    assert aviso.__version__ == aviso.VERSION


def test_public_surface_matches_documented_set() -> None:
    assert set(aviso.__all__) == {
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
    }
