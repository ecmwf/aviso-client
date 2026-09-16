# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Both iterator surfaces inherit core endpoint and opening validation."""

from __future__ import annotations

import pyaviso
import pytest
from pytest_httpserver import HTTPServer


@pytest.mark.parametrize("client_type", [pyaviso.AvisoClient, pyaviso.AsyncAvisoClient])
def test_unsupported_scheme_is_a_config_error(client_type: type) -> None:
    with pytest.raises(pyaviso.ConfigError, match="http or https") as error:
        client_type(base_url="ftp://user:SECRET@example.org/private?token=SECRET")
    assert "SECRET" not in str(error.value)


@pytest.mark.parametrize("client_type", [pyaviso.AvisoClient, pyaviso.AsyncAvisoClient])
@pytest.mark.parametrize(
    "content_type,body",
    [
        ("text/html", "<html>SECRET</html>"),
        ("application/json", '{"token":"SECRET"}'),
        ("text/event-stream", 'event: replay-control\ndata: {"type":"replay_started"}\n\n'),
        ("text/event-stream", 'event: live-notification\ndata: {"id":"mars@1"}\n\n'),
    ],
)
async def test_bad_stream_fails_before_delivery(
    httpserver: HTTPServer, client_type: type, content_type: str, body: str
) -> None:
    httpserver.expect_request("/api/v1/watch", method="POST").respond_with_data(
        body, content_type=content_type
    )
    client = client_type(base_url=httpserver.url_for("/"))
    stream = client.listen("mars", filter={})
    with pytest.raises(pyaviso.StreamProtocolError) as error:
        if isinstance(stream, pyaviso.AsyncNotificationIterator):
            async with stream:
                await anext(stream)
        else:
            with stream:
                next(stream)
    assert "SECRET" not in str(error.value)
