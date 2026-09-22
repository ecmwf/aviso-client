# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Smoke tests for the pyaviso Python package.

These tests run on every CI cell and the pre-push hook. They verify that
the extension imports, that the documented public surface is the expected
shape, and that the version string is consistent across the Python
package and the underlying Rust crate.
"""

from __future__ import annotations

import re

import pyaviso
from packaging.version import Version


def test_package_imports() -> None:
    assert pyaviso is not None


def test_version_is_a_string() -> None:
    assert isinstance(pyaviso.__version__, str)
    assert pyaviso.__version__


def test_native_version_matches_rust_crate() -> None:
    assert isinstance(pyaviso.VERSION, str)
    assert re.match(r"^\d+\.\d+\.\d+", pyaviso.VERSION)
    # The distribution version is PEP 440-normalized (2.0.0-rc.1 becomes
    # 2.0.0rc1) while the crate keeps the semver form, so the comparison
    # must normalize both sides.
    assert Version(pyaviso.__version__) == Version(pyaviso.VERSION)


def test_public_surface_matches_documented_set() -> None:
    assert set(pyaviso.__all__) == {
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
