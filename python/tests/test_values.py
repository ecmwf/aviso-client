# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

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

import json
from types import MappingProxyType
from typing import Any

import pyaviso
import pytest


@pytest.mark.parametrize("payload", [None, False, 0, "", [], {}, {"é": [None, 2**63, [True]]}])
def test_notification_str_without_cloudevent(payload: object) -> None:
    notification = pyaviso.Notification("mars", 42, {"class": "od"}, payload)
    assert json.loads(str(notification)) == notification.as_dict()
    assert str(notification).startswith('{\n  "event_type": "mars",')
    assert "cloudevent" not in json.loads(str(notification))


@pytest.mark.parametrize("cloudevent", [False, 0, "", [], {}, {"é": [None, 2**63, [True]]}])
def test_notification_str_preserves_supplied_cloudevent(cloudevent: Any) -> None:
    # Any exercises runtime JSON scalars beyond the mapping-only constructor stub.
    notification = pyaviso.Notification("mars", 42, {}, None, cloudevent)
    assert json.loads(str(notification)) == cloudevent
    assert notification.as_dict()["cloudevent"] == cloudevent
    assert repr(notification).startswith("Notification(event_type=")


def test_notification_str_explicit_none_uses_fields() -> None:
    notification = pyaviso.Notification("mars", 0, {}, None, None)
    assert json.loads(str(notification)) == {
        "event_type": "mars",
        "sequence": 0,
        "identifier": {},
        "payload": None,
    }


def test_notification_constructs_and_exposes_fields() -> None:
    n = pyaviso.Notification(
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


def test_notification_preserves_structured_spatial_identifier() -> None:
    point_cloud = [[46.0, 8.0], [47.0, 9.0]]
    notification = pyaviso.Notification(
        event_type="observations",
        sequence=42,
        identifier={"point_cloud": point_cloud},
        payload=None,
    )

    assert notification.identifier == {"point_cloud": point_cloud}
    assert notification.as_dict()["identifier"] == {"point_cloud": point_cloud}


@pytest.mark.parametrize("value", [float("nan"), float("inf"), float("-inf")])
def test_notification_rejects_nested_non_finite_identifier(value: float) -> None:
    with pytest.raises(TypeError, match="NaN or infinity"):
        pyaviso.Notification(
            event_type="observations",
            sequence=42,
            identifier={"point_cloud": [[46.0, value]]},
            payload=None,
        )


def test_notification_rejects_non_finite_value_in_general_mapping() -> None:
    identifier = MappingProxyType({"point_cloud": [[46.0, float("nan")]]})

    with pytest.raises(TypeError, match="NaN or infinity"):
        pyaviso.Notification(
            event_type="observations",
            sequence=7,
            identifier=identifier,
            payload=None,
        )


def _notification_with_identifier(identifier: dict[str, Any]) -> pyaviso.Notification:
    return pyaviso.Notification(
        event_type="observations",
        sequence=7,
        identifier=identifier,
        payload=None,
    )


def test_notification_rejects_self_referential_list() -> None:
    cycle: list[Any] = []
    cycle.append(cycle)

    with pytest.raises(TypeError, match="cyclic containers"):
        _notification_with_identifier({"value": cycle})


def test_notification_rejects_self_referential_dict() -> None:
    cycle: dict[str, Any] = {}
    cycle["self"] = cycle

    with pytest.raises(TypeError, match="cyclic containers"):
        _notification_with_identifier(cycle)


def test_notification_rejects_indirect_container_cycle() -> None:
    first: list[Any] = []
    second: dict[str, Any] = {"first": first}
    first.append(second)

    with pytest.raises(TypeError, match="cyclic containers"):
        _notification_with_identifier({"value": first})


def test_notification_rejects_excessive_identifier_nesting() -> None:
    nested: Any = "leaf"
    for _ in range(101):
        nested = [nested]

    with pytest.raises(ValueError, match="100 nested containers"):
        _notification_with_identifier({"value": nested})


def test_notification_accepts_shared_acyclic_containers() -> None:
    shared = [46.0, 8.0]
    notification = _notification_with_identifier({"first": shared, "second": shared})

    assert notification.identifier == {"first": [46.0, 8.0], "second": [46.0, 8.0]}


def test_notification_as_dict_carries_documented_keys() -> None:
    n = pyaviso.Notification(
        event_type="mars",
        sequence=42,
        identifier={"class": "od"},
        payload={"k": "v"},
    )
    d = n.as_dict()
    assert set(d.keys()) == {"event_type", "sequence", "identifier", "payload"}
    assert d["payload"] == {"k": "v"}


def test_notification_equality_is_value_based() -> None:
    a = pyaviso.Notification(event_type="m", sequence=1, identifier={"k": "v"}, payload=None)
    b = pyaviso.Notification(event_type="m", sequence=1, identifier={"k": "v"}, payload=None)
    c = pyaviso.Notification(event_type="m", sequence=2, identifier={"k": "v"}, payload=None)
    assert a == b
    assert a != c


def test_notification_is_unhashable() -> None:
    n = pyaviso.Notification(
        event_type="m",
        sequence=1,
        identifier={"k": "v"},
        payload={"x": 1},
    )
    with pytest.raises(TypeError):
        hash(n)


def test_notification_repr_includes_key_fields() -> None:
    n = pyaviso.Notification(
        event_type="mars",
        sequence=42,
        identifier={"class": "od"},
        payload=None,
    )
    rendered = repr(n)
    assert "mars" in rendered
    assert "42" in rendered


def test_notify_response_round_trips() -> None:
    r = pyaviso.NotifyResponse(status="success", request_id="req-1", processed_at="2026-05-17")
    assert r.status == "success"
    assert r.request_id == "req-1"
    assert r.processed_at == "2026-05-17"
    assert r.as_dict() == {
        "status": "success",
        "request_id": "req-1",
        "processed_at": "2026-05-17",
    }


def test_notify_response_is_unhashable() -> None:
    r = pyaviso.NotifyResponse(status="success", request_id="req-1", processed_at="t")
    with pytest.raises(TypeError):
        hash(r)
