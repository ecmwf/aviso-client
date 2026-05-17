"""Python client for aviso-server, ECMWF's notification service.

Phase 0 ships only the package skeleton. The async client, blocking wrapper,
and triggers land in Phase 5; see ``docs/src/internals/decisions.md``.

The package version is sourced from installed package metadata, which itself
is sourced from ``Cargo.toml`` via maturin's ``dynamic = ["version"]``. There
is exactly one source of truth for the version: the Rust workspace.
"""

from __future__ import annotations

from importlib.metadata import PackageNotFoundError, version

__all__ = ["__version__"]

try:
    __version__ = version("aviso-client")
except PackageNotFoundError:
    __version__ = "0.0.0+uninstalled"
