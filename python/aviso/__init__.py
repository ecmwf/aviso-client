"""Python client for aviso-server, ECMWF's notification service.

The package wraps the Rust `aviso` crate via PyO3 bindings. The compiled
extension lives at ``aviso._native``; user-facing names are re-exported
from this module so callers always ``import aviso`` and never reach for
``aviso._native`` directly.

The public surface grows as features land. This commit ships the full
exception hierarchy rooted at ``AvisoError`` covering every
``ClientError`` variant from the core crate. Clients, value types,
triggers, auth providers, and state stores land in subsequent commits.
"""

from __future__ import annotations

from importlib.metadata import PackageNotFoundError, version

from aviso._native import (
    VERSION,
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
    NotifyResponse,
    SchemaCatalog,
    SchemaResponse,
    StateStoreError,
    StreamProtocolError,
    TransportError,
    TriggerError,
)

AuthProvider = Bearer | Basic | Env | ConfigFile | Chain
StateStore = MemoryStore | JsonFileStore

try:
    __version__ = version("aviso")
except PackageNotFoundError:
    __version__ = "0.0.0+uninstalled"

__all__ = [
    "VERSION",
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
    "JsonFileStore",
    "MalformedEventError",
    "MemoryStore",
    "Notification",
    "NotifyResponse",
    "SchemaCatalog",
    "SchemaResponse",
    "StateStore",
    "StateStoreError",
    "StreamProtocolError",
    "TransportError",
    "TriggerError",
    "__version__",
]
