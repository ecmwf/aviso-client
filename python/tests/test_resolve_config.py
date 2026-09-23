# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""A client built with no arguments, and the report of what it resolved.

``AvisoClient()`` takes the address from code, then ``AVISO_BASE_URL``, then
the config file. ``client.config`` and ``pyaviso.resolve_config()`` say which
won and where every other setting came from, without the secret. The
``conftest.py`` fixture clears the developer's own variables and files first.
"""

from __future__ import annotations

import json
import pathlib

import pyaviso
import pytest
from pytest_httpserver import HTTPServer


def write_config(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path, body: str
) -> pathlib.Path:
    path = tmp_path / "config.yaml"
    path.write_text(body)
    monkeypatch.setenv("AVISO_CLIENT_CONFIG_FILE", str(path))
    return path


def serve_schema(httpserver: HTTPServer) -> None:
    httpserver.expect_request("/api/v1/schema").respond_with_json(
        {"status": "success", "schema": {}, "event_types": [], "total_schemas": 0}
    )


def test_the_address_comes_from_code_then_the_environment_then_the_file(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    path = write_config(monkeypatch, tmp_path, "base_url: https://file.example.org\n")

    from_file = pyaviso.resolve_config().base_url
    assert from_file is not None
    assert from_file.value == "https://file.example.org/"
    assert from_file.source == f"config file {path}"

    monkeypatch.setenv("AVISO_BASE_URL", "https://env.example.org")
    from_env = pyaviso.resolve_config().base_url
    assert from_env is not None
    assert from_env.value == "https://env.example.org/"
    assert from_env.source == "environment AVISO_BASE_URL"

    from_code = pyaviso.resolve_config(base_url="https://code.example.org").base_url
    assert from_code is not None
    assert from_code.value == "https://code.example.org/"
    assert from_code.source == "code"


def test_a_client_with_no_arguments_connects_to_the_resolved_address(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(monkeypatch, tmp_path, "base_url: https://unreachable.example.org\n")
    monkeypatch.setenv("AVISO_BASE_URL", httpserver.url_for("/"))
    serve_schema(httpserver)

    client = pyaviso.AvisoClient()
    client.schema()

    url = client.config.base_url
    assert url is not None
    assert url.source == "environment AVISO_BASE_URL"
    assert httpserver.url_for("/") in url.value


def test_no_address_anywhere_is_a_config_error_that_names_the_places(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    assert pyaviso.resolve_config().base_url is None
    with pytest.raises(pyaviso.ConfigError, match="AVISO_BASE_URL"):
        pyaviso.AvisoClient()


def test_the_report_never_carries_the_secret(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(
        monkeypatch,
        tmp_path,
        "base_url: https://alice:hunter2@file.example.org\n"
        "timeout: 7s\n"
        "auth:\n  bearer_token: from-file\n",
    )
    config = pyaviso.resolve_config()
    shown = repr(config) + json.dumps(config.as_dict())
    assert "hunter2" not in shown
    assert "alice" not in shown
    assert "from-file" not in shown

    assert config.auth is not None
    assert config.auth.kind == "bearer"
    assert config.auth.source.startswith("config file ")
    assert config.auth.refused is None
    assert config.timeout.value == 7.0
    assert config.heartbeat_interval.value is None
    assert config.heartbeat_interval.source == "default"


def test_the_report_says_when_a_found_credential_is_refused(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(
        monkeypatch,
        tmp_path,
        "base_url: http://public.example.org\nauth:\n  bearer_token: from-file\n",
    )
    config = pyaviso.resolve_config()
    assert config.auth is not None
    assert config.auth.refused is not None
    assert "refused" in config.auth.refused
    assert "public.example.org" in config.auth.refused
    # The client itself refuses to build in that state.
    with pytest.raises(pyaviso.AuthError):
        pyaviso.AvisoClient()
    # A credential named in code is never refused.
    named = pyaviso.resolve_config(auth=pyaviso.Bearer("named"))
    assert named.auth is not None
    assert named.auth.source == "code"
    assert named.auth.refused is None


def test_an_argument_wins_over_the_file_and_is_reported_as_code(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(monkeypatch, tmp_path, "base_url: https://file.example.org\ntimeout: 7s\n")
    client = pyaviso.AvisoClient(timeout=3)
    assert client.config.timeout.value == 3.0
    assert client.config.timeout.source == "code"
    url = client.config.base_url
    assert url is not None
    assert url.source.startswith("config file ")


def test_anonymous_is_reported_as_a_choice_made_in_code(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(
        monkeypatch,
        tmp_path,
        "base_url: https://file.example.org\nauth:\n  bearer_token: from-file\n",
    )
    config = pyaviso.resolve_config(auth=pyaviso.Anonymous())
    assert config.auth is not None
    assert config.auth.kind == "anonymous"
    assert config.auth.source == "code"


def test_from_file_reports_the_named_file(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    named = tmp_path / "other.yaml"
    named.write_text("base_url: https://named.example.org\n")
    client = pyaviso.AvisoClient.from_file(named)
    assert client.config.config_file == str(named)
    url = client.config.base_url
    assert url is not None
    assert url.value == "https://named.example.org/"


def test_the_async_client_has_the_same_report(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(monkeypatch, tmp_path, "base_url: https://file.example.org\n")
    client = pyaviso.AsyncAvisoClient()
    url = client.config.base_url
    assert url is not None
    assert url.value == "https://file.example.org/"


def test_from_file_reads_the_address_from_the_file_not_the_environment(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(monkeypatch, tmp_path, "base_url: https://file.example.org\n")
    monkeypatch.setenv("AVISO_BASE_URL", "https://env.example.org")

    plain = pyaviso.AvisoClient()
    assert plain.config.base_url is not None
    assert plain.config.base_url.source == "environment AVISO_BASE_URL"

    from_file = pyaviso.AvisoClient.from_file()
    assert from_file.config.base_url is not None
    assert from_file.config.base_url.value == "https://file.example.org/"
    assert from_file.config.base_url.source.startswith("config file ")


def test_a_half_set_environment_is_an_auth_error_even_for_the_report(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(monkeypatch, tmp_path, "base_url: https://file.example.org\n")
    monkeypatch.setenv("AVISO_USERNAME", "alice")
    with pytest.raises(pyaviso.AuthError):
        pyaviso.resolve_config()
