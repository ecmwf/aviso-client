"""Python client for aviso-server, ECMWF's notification service.

The package wraps the Rust `aviso` crate via PyO3 bindings. The compiled
extension lives at ``aviso._native``; user-facing names are re-exported
from this module so callers always ``import aviso`` and never reach for
``aviso._native`` directly.

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

from aviso._native import (
    VERSION,
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
    __version__ = version("aviso")
except PackageNotFoundError:
    __version__ = "0.0.0+uninstalled"

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
