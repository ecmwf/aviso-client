# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Shared pytest configuration for the pyaviso Python suite.

Tests run against the locally-built `pyaviso._native` extension produced by
``uv run maturin develop``.

A client created without an ``auth`` argument searches the environment and
two files for a credential, so every test would otherwise depend on whatever
the machine running it happens to have. The autouse fixture below points all
three sources at paths that do not exist.
"""

from __future__ import annotations

import pathlib

import pytest

CREDENTIAL_ENV_VARS = ("AVISO_TOKEN", "AVISO_USERNAME", "AVISO_PASSWORD")


@pytest.fixture(autouse=True)
def isolate_credential_sources(
    monkeypatch: pytest.MonkeyPatch, tmp_path_factory: pytest.TempPathFactory
) -> None:
    """Hides any credential the developer or CI runner has on disk."""
    absent = pathlib.Path(tmp_path_factory.mktemp("no-credentials"))
    for name in CREDENTIAL_ENV_VARS:
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("AVISO_CLIENT_CONFIG_FILE", str(absent / "config.yaml"))
    monkeypatch.setenv("AVISO_CREDENTIALS_FILE", str(absent / "credentials.yaml"))
