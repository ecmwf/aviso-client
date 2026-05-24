"""Python client for aviso-server, ECMWF's notification service.

The package wraps the Rust `aviso` crate via PyO3 bindings. The compiled
extension lives at ``aviso._native``; user-facing names are re-exported
from this module so callers always ``import aviso`` and never reach for
``aviso._native`` directly.

The public surface grows as features land. Until the bindings ship the
full API, only ``__version__`` and ``VERSION`` (the Rust crate version)
are available.
"""

from __future__ import annotations

from importlib.metadata import PackageNotFoundError, version

from aviso._native import VERSION

try:
    __version__ = version("aviso")
except PackageNotFoundError:
    __version__ = "0.0.0+uninstalled"

__all__ = ["VERSION", "__version__"]
