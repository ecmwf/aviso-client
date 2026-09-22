# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Credential discovery tests for the Python client.

A client built without an ``auth`` argument looks for a credential: the
environment first, then the config file, then the credentials file. These
tests assert the ``Authorization`` header that actually reaches a server,
rather than the provider object, because the header is what matters.
"""

from __future__ import annotations

import pathlib

import pyaviso
import pytest
from pytest_httpserver import HTTPServer
from werkzeug.wrappers import Request, Response

CREDENTIAL_VARS = ("AVISO_TOKEN", "AVISO_USERNAME", "AVISO_PASSWORD")


@pytest.fixture(autouse=True)
def isolate_sources(monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path) -> None:
    """Points every source at a path that does not exist.

    Without this the tests would read the developer's own credentials and
    pass or fail depending on the machine.
    """
    for name in CREDENTIAL_VARS:
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("AVISO_CLIENT_CONFIG_FILE", str(tmp_path / "absent-config.yaml"))
    monkeypatch.setenv("AVISO_CREDENTIALS_FILE", str(tmp_path / "absent-credentials.yaml"))


def capture_authorization(httpserver: HTTPServer) -> list[str | None]:
    """Records the Authorization header of each schema request."""
    seen: list[str | None] = []

    def handler(request: Request) -> Response:
        seen.append(request.headers.get("Authorization"))
        body = '{"status": "success", "schema": {}, "event_types": [], "total_schemas": 0}'
        return Response(body, content_type="application/json")

    httpserver.expect_request("/api/v1/schema").respond_with_handler(handler)
    return seen


def test_credentials_file_is_found_without_an_auth_argument(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    credentials = tmp_path / "credentials.yaml"
    credentials.write_text("bearer:\n  token: from-credentials-file\n")
    monkeypatch.setenv("AVISO_CREDENTIALS_FILE", str(credentials))
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient(base_url=httpserver.url_for("/")).schema()

    assert seen == ["Bearer from-credentials-file"]


def test_config_file_auth_block_is_found(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    config = tmp_path / "config.yaml"
    config.write_text("auth:\n  bearer_token: from-config-file\n")
    monkeypatch.setenv("AVISO_CLIENT_CONFIG_FILE", str(config))
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient(base_url=httpserver.url_for("/")).schema()

    assert seen == ["Bearer from-config-file"]


def test_config_file_wins_over_credentials_file(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    config = tmp_path / "config.yaml"
    config.write_text("auth:\n  bearer_token: from-config-file\n")
    credentials = tmp_path / "credentials.yaml"
    credentials.write_text("bearer:\n  token: from-credentials-file\n")
    monkeypatch.setenv("AVISO_CLIENT_CONFIG_FILE", str(config))
    monkeypatch.setenv("AVISO_CREDENTIALS_FILE", str(credentials))
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient(base_url=httpserver.url_for("/")).schema()

    assert seen == ["Bearer from-config-file"]


def test_environment_wins_over_both_files(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    config = tmp_path / "config.yaml"
    config.write_text("auth:\n  bearer_token: from-config-file\n")
    credentials = tmp_path / "credentials.yaml"
    credentials.write_text("bearer:\n  token: from-credentials-file\n")
    monkeypatch.setenv("AVISO_CLIENT_CONFIG_FILE", str(config))
    monkeypatch.setenv("AVISO_CREDENTIALS_FILE", str(credentials))
    monkeypatch.setenv("AVISO_TOKEN", "from-environment")
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient(base_url=httpserver.url_for("/")).schema()

    assert seen == ["Bearer from-environment"]


def test_explicit_auth_wins_over_every_source(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    monkeypatch.setenv("AVISO_TOKEN", "from-environment")
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient(base_url=httpserver.url_for("/"), auth=pyaviso.Bearer("explicit")).schema()

    assert seen == ["Bearer explicit"]


def test_anonymous_sends_no_header_even_with_a_credential_present(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    credentials = tmp_path / "credentials.yaml"
    credentials.write_text("bearer:\n  token: from-credentials-file\n")
    monkeypatch.setenv("AVISO_CREDENTIALS_FILE", str(credentials))
    monkeypatch.setenv("AVISO_TOKEN", "from-environment")
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient(base_url=httpserver.url_for("/"), auth=pyaviso.Anonymous()).schema()

    assert seen == [None]


def test_no_credential_anywhere_sends_no_header(httpserver: HTTPServer) -> None:
    seen = capture_authorization(httpserver)

    pyaviso.AvisoClient(base_url=httpserver.url_for("/")).schema()

    assert seen == [None]


def test_malformed_credentials_file_is_reported(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    credentials = tmp_path / "credentials.yaml"
    credentials.write_text("bearer:\n  toke: typo\n")
    monkeypatch.setenv("AVISO_CREDENTIALS_FILE", str(credentials))

    with pytest.raises(pyaviso.ConfigError):
        pyaviso.AvisoClient(base_url="https://aviso.example.org")


def test_partial_environment_credentials_are_reported(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("AVISO_USERNAME", "alice")

    with pytest.raises(pyaviso.AuthError):
        pyaviso.AvisoClient(base_url="https://aviso.example.org")


def test_async_client_discovers_the_same_credential(
    httpserver: HTTPServer, monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    import asyncio

    credentials = tmp_path / "credentials.yaml"
    credentials.write_text("bearer:\n  token: from-credentials-file\n")
    monkeypatch.setenv("AVISO_CREDENTIALS_FILE", str(credentials))
    seen = capture_authorization(httpserver)

    async def run() -> None:
        client = pyaviso.AsyncAvisoClient(base_url=httpserver.url_for("/"))
        await client.schema()

    asyncio.run(run())

    assert seen == ["Bearer from-credentials-file"]


def test_a_discovered_credential_is_refused_for_a_plaintext_address(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    credentials = tmp_path / "credentials.yaml"
    credentials.write_text("bearer:\n  token: from-credentials-file\n")
    monkeypatch.setenv("AVISO_CREDENTIALS_FILE", str(credentials))

    with pytest.raises(pyaviso.AuthError) as excinfo:
        pyaviso.AvisoClient(base_url="http://aviso.example.org")

    assert "from-credentials-file" not in str(excinfo.value)


def test_an_explicit_credential_may_go_to_a_plaintext_address(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    monkeypatch.setenv("AVISO_TOKEN", "from-environment")

    pyaviso.AvisoClient(base_url="http://aviso.example.org", auth=pyaviso.Bearer("chosen"))


def test_a_discovered_credential_may_go_to_an_https_address(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    monkeypatch.setenv("AVISO_TOKEN", "from-environment")

    pyaviso.AvisoClient(base_url="https://aviso.example.org")


def test_an_invalid_address_is_reported_the_same_with_or_without_a_credential(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # The plaintext rule must not get in front of the builder's own check:
    # the caller should see "invalid base_url", not an auth refusal.
    with pytest.raises(pyaviso.ConfigError):
        pyaviso.AvisoClient(base_url="ftp://aviso.example.org")
    monkeypatch.setenv("AVISO_TOKEN", "from-environment")
    with pytest.raises(pyaviso.ConfigError):
        pyaviso.AvisoClient(base_url="ftp://aviso.example.org")
    with pytest.raises(pyaviso.ConfigError):
        pyaviso.AvisoClient(base_url="not a url")
