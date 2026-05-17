"""Python client for aviso-server, ECMWF's notification service.

The public surface grows as features land in the underlying Rust core and
PyO3 binding crate; the design rationale lives in
``docs/src/internals/decisions.md`` and is referenced by stable ADR id.

The package version is sourced from installed package metadata, which itself
is sourced from ``Cargo.toml`` via maturin's ``dynamic = ["version"]``. There
is exactly one source of truth for the version: the Rust workspace.
"""

from __future__ import annotations

from importlib.metadata import PackageNotFoundError, version

__all__ = ["__version__"]

try:
    __version__ = version("aviso")
except PackageNotFoundError:
    __version__ = "0.0.0+uninstalled"
