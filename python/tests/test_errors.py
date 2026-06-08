# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Exception hierarchy and ClientError -> PyErr round-trip tests.

The Rust ``map_client_error`` function maps every variant of
``aviso::ClientError`` onto a Python exception class. Tests here provoke
each variant via the private ``_provoke_error`` helper and assert the
resulting Python exception has the right class and the documented
structured attributes.
"""

from __future__ import annotations

import pyaviso
import pytest
from pyaviso import _native


def test_aviso_error_is_the_root_class() -> None:
    assert issubclass(pyaviso.AvisoError, Exception)


def test_transport_error_subclasses_aviso_error() -> None:
    assert issubclass(pyaviso.TransportError, pyaviso.AvisoError)


def test_http_error_subclasses_aviso_error() -> None:
    assert issubclass(pyaviso.HttpError, pyaviso.AvisoError)


def test_http_error_carries_structured_attributes() -> None:
    with pytest.raises(pyaviso.HttpError) as excinfo:
        _native._provoke_error(
            "http",
            status=418,
            body="I'm a teapot",
            request_id="req-test-001",
        )
    err = excinfo.value
    assert err.status == 418
    assert err.body == "I'm a teapot"
    assert err.request_id == "req-test-001"


def test_http_error_with_no_request_id_has_none_attribute() -> None:
    with pytest.raises(pyaviso.HttpError) as excinfo:
        _native._provoke_error("http", status=500, body="boom")
    assert excinfo.value.request_id is None


def test_http_error_message_includes_status_and_body() -> None:
    with pytest.raises(pyaviso.HttpError) as excinfo:
        _native._provoke_error("http", status=404, body="not found", request_id="req-abc")
    rendered = str(excinfo.value)
    assert "404" in rendered
    assert "not found" in rendered
    assert "req-abc" in rendered


def test_catching_http_via_aviso_error_works() -> None:
    with pytest.raises(pyaviso.AvisoError):
        _native._provoke_error("http", status=400, body="bad")


def test_unknown_kind_raises_value_error() -> None:
    with pytest.raises(ValueError):
        _native._provoke_error("not-a-real-kind")
