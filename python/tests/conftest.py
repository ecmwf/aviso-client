"""Shared pytest configuration for the aviso Python suite.

Tests run against the locally-built `aviso._native` extension produced by
``uv run maturin develop``. The conftest stays minimal until later
commits introduce fixtures.
"""

from __future__ import annotations
