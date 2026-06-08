# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Auth provider construction tests.

The five shipped providers (Bearer, Basic, Env, ConfigFile, Chain) wrap
their Rust counterparts. Tests cover construction success, validation
errors, and that the providers compose into a Chain.
"""

from __future__ import annotations

import pathlib
from typing import Any

import pyaviso
import pytest


def test_bearer_constructs_with_token() -> None:
    b = pyaviso.Bearer("opaque-jwt-here")
    assert "Bearer" in repr(b)
    assert "redacted" in repr(b)


def test_bearer_empty_token_raises_config_error() -> None:
    with pytest.raises(pyaviso.ConfigError):
        pyaviso.Bearer("")


def test_basic_constructs_with_user_and_pass() -> None:
    b = pyaviso.Basic("alice", "wonderland")
    assert "Basic" in repr(b)


def test_basic_empty_user_raises_config_error() -> None:
    with pytest.raises(pyaviso.ConfigError):
        pyaviso.Basic("", "pw")


def test_basic_username_with_colon_raises_config_error() -> None:
    with pytest.raises(pyaviso.ConfigError):
        pyaviso.Basic("alice:bob", "pw")


def test_env_uses_token_when_set(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("AVISO_TOKEN", "tok-from-env")
    e = pyaviso.Env()
    assert "Env" in repr(e)


def test_env_falls_back_to_basic(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("AVISO_TOKEN", raising=False)
    monkeypatch.setenv("AVISO_USERNAME", "alice")
    monkeypatch.setenv("AVISO_PASSWORD", "pw")
    pyaviso.Env()


def test_env_with_no_credentials_raises_auth_error(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("AVISO_TOKEN", raising=False)
    monkeypatch.delenv("AVISO_USERNAME", raising=False)
    monkeypatch.delenv("AVISO_PASSWORD", raising=False)
    with pytest.raises(pyaviso.AuthError):
        pyaviso.Env()


def test_config_file_parses_bearer_yaml(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "auth.yaml"
    path.write_text("bearer:\n  token: opaque-jwt\n")
    cfg = pyaviso.ConfigFile(path)
    assert "ConfigFile" in repr(cfg)


def test_config_file_accepts_str_path(tmp_path: pathlib.Path) -> None:
    path = tmp_path / "auth.yaml"
    path.write_text("basic:\n  username: alice\n  password: pw\n")
    pyaviso.ConfigFile(str(path))


def test_config_file_missing_file_raises_config_error(tmp_path: pathlib.Path) -> None:
    with pytest.raises(pyaviso.ConfigError):
        pyaviso.ConfigFile(tmp_path / "no-such-file.yaml")


def test_chain_composes_providers() -> None:
    chain = pyaviso.Chain(pyaviso.Bearer("first"), pyaviso.Bearer("second"))
    assert "Chain" in repr(chain)


def test_chain_accepts_empty_args() -> None:
    chain = pyaviso.Chain()
    assert chain is not None


def test_chain_rejects_non_provider_object() -> None:
    bogus: Any = "not-a-provider"
    with pytest.raises(TypeError):
        pyaviso.Chain(bogus)
