# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

from __future__ import annotations

import pyaviso


def test_schema_lists_both_configured_event_types(reader_client: pyaviso.AvisoClient) -> None:
    catalog = reader_client.schema()
    assert "test_event" in catalog.event_types
    assert "test_polygon" in catalog.event_types
    assert catalog.total_schemas == 2


def test_schema_for_test_polygon_returns_expected_shape(
    reader_client: pyaviso.AvisoClient,
) -> None:
    response = reader_client.schema_for("test_polygon")
    assert response.event_type == "test_polygon"
    identifier = response.schema["identifier"]
    assert identifier["polygon"]["type"] == "PolygonHandler"
    assert identifier["polygon"]["required"] is True
    assert identifier["date"]["type"] == "DateHandler"
    assert identifier["time"]["type"] == "TimeHandler"
    assert response.schema["payload"]["required"] is True
