# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Python client for aviso-server, ECMWF's notification service.

The package wraps the Rust `aviso` crate via PyO3 bindings. The compiled
extension lives at ``pyaviso._native``; user-facing names are re-exported
from this module so callers always ``import pyaviso`` and never reach for
``pyaviso._native`` directly.

The public surface covers the publish, schema-discovery, and listen
verbs on both ``AvisoClient`` (sync) and ``AsyncAvisoClient`` (async),
the value types they return (``Notification``, ``NotifyResponse``,
``SchemaCatalog``, ``SchemaResponse``), the trigger builder, the five
auth providers, the two state stores, and the exception hierarchy
rooted at ``AvisoError``.
"""

from __future__ import annotations

from enum import Enum
from importlib.metadata import PackageNotFoundError, version

from pyaviso._native import (
    VERSION,
    Anonymous,
    AsyncAvisoClient,
    AsyncNotificationIterator,
    AuthError,
    AvisoClient,
    AvisoError,
    Basic,
    Bearer,
    Chain,
    ConfigError,
    ConfigFile,
    DecodeError,
    Env,
    HistoryGapError,
    HttpError,
    JsonFileStore,
    MalformedEventError,
    MemoryStore,
    Notification,
    NotificationIterator,
    NotifyResponse,
    NotifyResult,
    SchemaCatalog,
    SchemaResponse,
    StateStoreError,
    StreamProtocolError,
    TransportError,
    Trigger,
    TriggerError,
    WatchRequest,
)

AuthProvider = Bearer | Basic | Env | ConfigFile | Chain
StateStore = MemoryStore | JsonFileStore


class HttpMethod(str, Enum):
    """HTTP method for webhook / teams / post triggers."""

    POST = "POST"
    GET = "GET"
    PUT = "PUT"
    PATCH = "PATCH"
    DELETE = "DELETE"


class WatchMode(str, Enum):
    """Whether a watch reconnects after end_of_stream (WATCH) or terminates
    once replay completes (REPLAY_ONLY)."""

    WATCH = "watch"
    REPLAY_ONLY = "replay_only"


try:
    __version__ = version("pyaviso")
except PackageNotFoundError:
    __version__ = "0.0.0+uninstalled"

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
