"""Shared pytest configuration for the pyaviso Python suite.

Tests run against the locally-built `pyaviso._native` extension produced by
``uv run maturin develop``. The conftest stays minimal until later
commits introduce fixtures.
"""

from __future__ import annotations
