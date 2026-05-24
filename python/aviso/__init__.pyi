"""Type stubs for the curated public surface of the aviso Python package.

Hand-written. Kept in sync with the runtime ``__all__`` via
``python/tests/test_stub_completeness.py``. Subsequent commits expand the
surface as the bindings ship; this file lists only the symbols available
in the current commit.
"""

from __future__ import annotations

__version__: str
VERSION: str

__all__ = ["VERSION", "__version__"]
