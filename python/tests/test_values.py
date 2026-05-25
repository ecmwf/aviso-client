"""Value-type round-trip tests.

The classes under test here are the two value types that have a public
Python constructor and can be exercised hermetically: ``Notification`` and
``NotifyResponse``. The tests verify property access, equality, ``as_dict()``
shape, ``__repr__`` shape, and the unhashable contract that keeps misuse
loud.

``SchemaCatalog`` and ``SchemaResponse`` are also value types but are
deliberately not user-constructible: they only exist as outputs of
``client.schema()`` and ``client.schema_for(...)``. They are exercised
indirectly through any integration path that calls those methods; a mock-
server harness for those calls lands in a follow-up.
"""

from __future__ import annotations

import aviso
import pytest


def test_notification_constructs_and_exposes_fields() -> None:
    n = aviso.Notification(
        event_type="mars",
        sequence=42,
        identifier={"class": "od", "stream": "oper"},
        payload={"location": "s3://bucket/path"},
    )
    assert n.event_type == "mars"
    assert n.sequence == 42
    assert n.identifier == {"class": "od", "stream": "oper"}
    assert n.payload == {"location": "s3://bucket/path"}
    assert n.cloudevent is None


def test_notification_as_dict_carries_documented_keys() -> None:
    n = aviso.Notification(
        event_type="mars",
        sequence=42,
        identifier={"class": "od"},
        payload={"k": "v"},
    )
    d = n.as_dict()
    assert set(d.keys()) == {"event_type", "sequence", "identifier", "payload"}
    assert d["payload"] == {"k": "v"}


def test_notification_equality_is_value_based() -> None:
    a = aviso.Notification(event_type="m", sequence=1, identifier={"k": "v"}, payload=None)
    b = aviso.Notification(event_type="m", sequence=1, identifier={"k": "v"}, payload=None)
    c = aviso.Notification(event_type="m", sequence=2, identifier={"k": "v"}, payload=None)
    assert a == b
    assert a != c


def test_notification_is_unhashable() -> None:
    n = aviso.Notification(
        event_type="m",
        sequence=1,
        identifier={"k": "v"},
        payload={"x": 1},
    )
    with pytest.raises(TypeError):
        hash(n)


def test_notification_repr_includes_key_fields() -> None:
    n = aviso.Notification(
        event_type="mars",
        sequence=42,
        identifier={"class": "od"},
        payload=None,
    )
    rendered = repr(n)
    assert "mars" in rendered
    assert "42" in rendered


def test_notify_response_round_trips() -> None:
    r = aviso.NotifyResponse(status="success", request_id="req-1", processed_at="2026-05-17")
    assert r.status == "success"
    assert r.request_id == "req-1"
    assert r.processed_at == "2026-05-17"
    assert r.as_dict() == {
        "status": "success",
        "request_id": "req-1",
        "processed_at": "2026-05-17",
    }


def test_notify_response_is_unhashable() -> None:
    r = aviso.NotifyResponse(status="success", request_id="req-1", processed_at="t")
    with pytest.raises(TypeError):
        hash(r)
