"""State store construction tests."""

from __future__ import annotations

import pathlib
from typing import Any

import aviso
import pytest


def test_memory_store_constructs() -> None:
    store = aviso.MemoryStore()
    assert "MemoryStore" in repr(store)


def test_json_file_store_constructs(tmp_path: pathlib.Path) -> None:
    store = aviso.JsonFileStore(tmp_path / "state.json")
    rendered = repr(store)
    assert "JsonFileStore" in rendered
    assert "state.json" in rendered


def test_json_file_store_accepts_str_path(tmp_path: pathlib.Path) -> None:
    aviso.JsonFileStore(str(tmp_path / "state.json"))


def test_json_file_store_expands_tilde() -> None:
    store = aviso.JsonFileStore("~/aviso-tmp-test-state.json")
    rendered = repr(store)
    assert "~" not in rendered


def test_client_accepts_auth_and_state_store(tmp_path: pathlib.Path) -> None:
    client = aviso.AvisoClient(
        base_url="http://127.0.0.1:1",
        auth=aviso.Bearer("jwt"),
        state_store=aviso.MemoryStore(),
    )
    assert client is not None


def test_client_rejects_invalid_auth_type() -> None:
    bogus: Any = "just-a-string"
    with pytest.raises(TypeError):
        aviso.AvisoClient(base_url="http://127.0.0.1:1", auth=bogus)


def test_client_rejects_invalid_state_store_type() -> None:
    bogus: Any = "not-a-store"
    with pytest.raises(TypeError):
        aviso.AvisoClient(base_url="http://127.0.0.1:1", state_store=bogus)
