# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""State store construction tests."""

from __future__ import annotations

import pathlib
from typing import Any

import pyaviso
import pytest


def test_memory_store_constructs() -> None:
    store = pyaviso.MemoryStore()
    assert "MemoryStore" in repr(store)


def test_json_file_store_constructs(tmp_path: pathlib.Path) -> None:
    store = pyaviso.JsonFileStore(tmp_path / "state.json")
    rendered = repr(store)
    assert "JsonFileStore" in rendered
    assert "state.json" in rendered


def test_json_file_store_accepts_str_path(tmp_path: pathlib.Path) -> None:
    pyaviso.JsonFileStore(str(tmp_path / "state.json"))


def test_json_file_store_expands_tilde(
    tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    store = pyaviso.JsonFileStore("~/state.json")
    rendered = repr(store)
    assert "~" not in rendered
    assert str(tmp_path) in rendered


def test_client_accepts_auth_and_state_store(tmp_path: pathlib.Path) -> None:
    client = pyaviso.AvisoClient(
        base_url="http://127.0.0.1:1",
        auth=pyaviso.Bearer("jwt"),
        state_store=pyaviso.MemoryStore(),
    )
    assert client is not None


def test_client_rejects_invalid_auth_type() -> None:
    bogus: Any = "just-a-string"
    with pytest.raises(TypeError):
        pyaviso.AvisoClient(base_url="http://127.0.0.1:1", auth=bogus)


def test_client_rejects_invalid_state_store_type() -> None:
    bogus: Any = "not-a-store"
    with pytest.raises(TypeError):
        pyaviso.AvisoClient(base_url="http://127.0.0.1:1", state_store=bogus)
