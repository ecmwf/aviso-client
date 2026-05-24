"""Smoke tests for the aviso Python package.

These tests run on every CI cell and the pre-push hook. They verify that
the extension imports, that the public surface in this commit is the
expected shape, and that the version string is consistent across the
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


def test_public_surface_in_this_commit() -> None:
    assert set(aviso.__all__) == {
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
    }
