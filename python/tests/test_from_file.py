# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Building a client from the config file.

``AvisoClient.from_file()`` reads the same file the ``aviso`` binary uses and
applies keyword arguments on top. These tests point ``AVISO_CLIENT_CONFIG_FILE``
at a file they write, and assert what actually reaches a server. The other
credential sources are neutralised by the fixture in ``conftest.py``.
"""

from __future__ import annotations

import asyncio
import pathlib

import pyaviso
import pytest
from pytest_httpserver import HTTPServer
from werkzeug.wrappers import Request, Response


def capture_authorization(httpserver: HTTPServer) -> list[str | None]:
    seen: list[str | None] = []

    def handler(request: Request) -> Response:
        seen.append(request.headers.get("Authorization"))
        body = '{"status": "success", "schema": {}, "event_types": [], "total_schemas": 0}'
        return Response(body, content_type="application/json")

    httpserver.expect_request("/api/v1/schema").respond_with_handler(handler)
    return seen


def write_config(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path, body: str
) -> pathlib.Path:
    path = tmp_path / "config.yaml"
    path.write_text(body)
    monkeypatch.setenv("AVISO_CLIENT_CONFIG_FILE", str(path))
    return path


def test_the_file_supplies_the_address_and_the_credential(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(
        monkeypatch,
        tmp_path,
        f"base_url: {httpserver.url_for('/')}\nauth:\n  bearer_token: from-file\n",
    )
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient.from_file().schema()

    assert seen == ["Bearer from-file"]


def test_keyword_arguments_replace_what_the_file_said(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(
        monkeypatch,
        tmp_path,
        "base_url: https://nowhere.example.org\nauth:\n  bearer_token: from-file\n",
    )
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient.from_file(
        base_url=httpserver.url_for("/"), auth=pyaviso.Bearer("from-code")
    ).schema()

    assert seen == ["Bearer from-code"]


def test_anonymous_removes_the_credential_the_file_supplied(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(
        monkeypatch,
        tmp_path,
        f"base_url: {httpserver.url_for('/')}\nauth:\n  bearer_token: from-file\n",
    )
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient.from_file(auth=pyaviso.Anonymous()).schema()

    assert seen == [None]


def test_a_named_file_is_read_instead_of_the_default(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(monkeypatch, tmp_path, "base_url: https://default.example.org\n")
    other = tmp_path / "other.yaml"
    other.write_text(f"base_url: {httpserver.url_for('/')}\nauth:\n  bearer_token: from-other\n")
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient.from_file(other).schema()

    assert seen == ["Bearer from-other"]


def test_a_missing_default_file_sets_nothing(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    monkeypatch.setenv("AVISO_CLIENT_CONFIG_FILE", str(tmp_path / "absent.yaml"))
    seen = capture_authorization(httpserver)

    # Nothing from the file, so base_url has to come from the call.
    pyaviso.AvisoClient.from_file(base_url=httpserver.url_for("/")).schema()

    assert seen == [None]


def test_a_missing_default_file_without_a_base_url_is_the_ordinary_error(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    monkeypatch.setenv("AVISO_CLIENT_CONFIG_FILE", str(tmp_path / "absent.yaml"))

    with pytest.raises(pyaviso.ConfigError):
        pyaviso.AvisoClient.from_file()


def test_a_named_file_that_does_not_exist_is_an_error(tmp_path: pathlib.Path) -> None:
    with pytest.raises(pyaviso.ConfigError):
        pyaviso.AvisoClient.from_file(tmp_path / "absent.yaml")


def test_a_broken_file_is_reported(monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path) -> None:
    write_config(monkeypatch, tmp_path, "tls:\n  ca_bundel: [x.pem]\n")

    with pytest.raises(pyaviso.ConfigError, match="ca_bundel"):
        pyaviso.AvisoClient.from_file()


def test_a_found_credential_is_refused_for_a_plaintext_address_in_the_file(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(
        monkeypatch,
        tmp_path,
        "base_url: http://aviso.example.org\nauth:\n  bearer_token: from-file\n",
    )

    with pytest.raises(pyaviso.AuthError) as excinfo:
        pyaviso.AvisoClient.from_file()

    assert "from-file" not in str(excinfo.value)


def test_the_async_client_reads_the_same_file(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    write_config(
        monkeypatch,
        tmp_path,
        f"base_url: {httpserver.url_for('/')}\nauth:\n  bearer_token: from-file\n",
    )
    seen = capture_authorization(httpserver)

    async def run() -> None:
        await pyaviso.AsyncAvisoClient.from_file().schema()

    asyncio.run(run())

    assert seen == ["Bearer from-file"]
