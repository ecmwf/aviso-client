from __future__ import annotations

import aviso


def test_schema_lists_both_configured_event_types(reader_client: aviso.AvisoClient) -> None:
    catalog = reader_client.schema()
    assert "test_event" in catalog.event_types
    assert "test_polygon" in catalog.event_types
    assert catalog.total_schemas == 2


def test_schema_for_test_polygon_returns_expected_shape(
    reader_client: aviso.AvisoClient,
) -> None:
    response = reader_client.schema_for("test_polygon")
    assert response.event_type == "test_polygon"
    identifier = response.schema["identifier"]
    assert identifier["polygon"]["type"] == "PolygonHandler"
    assert identifier["polygon"]["required"] is True
    assert identifier["date"]["type"] == "DateHandler"
    assert identifier["time"]["type"] == "TimeHandler"
    assert response.schema["payload"]["required"] is True
