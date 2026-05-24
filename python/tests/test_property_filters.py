"""Property tests for the filter dict round-trip.

Filters are passed to `client.listen(..., filter=dict)` and to
`WatchRequest.with_filter(dict)`. The Python wrapper accepts any
JSON-encodable value as a filter value and round-trips it through serde
on the Rust side. These tests assert the binding accepts a broad set
of valid filter shapes without raising.
"""

from __future__ import annotations

from typing import Any

import aviso
from hypothesis import given
from hypothesis import strategies as st

# JSON-compatible recursive value strategy. Keys are non-empty ASCII
# identifiers because the server's identifier names follow that shape.
_json_value: st.SearchStrategy[Any] = st.recursive(
    st.one_of(
        st.text(max_size=20),
        st.integers(min_value=-(2**31), max_value=2**31 - 1),
        st.floats(allow_nan=False, allow_infinity=False, width=32),
        st.booleans(),
        st.none(),
    ),
    lambda inner: st.one_of(
        st.lists(inner, max_size=4),
        st.dictionaries(
            keys=st.from_regex(r"\A[a-z][a-z0-9_]{0,7}\Z"),
            values=inner,
            max_size=4,
        ),
    ),
    max_leaves=6,
)


_filter_dict = st.dictionaries(
    keys=st.from_regex(r"\A[a-z][a-z0-9_]{0,15}\Z"),
    values=_json_value,
    max_size=6,
)


@given(filt=_filter_dict)
def test_watch_request_with_filter_accepts_any_json_compatible_dict(filt: dict[str, Any]) -> None:
    req = aviso.WatchRequest.watch("mars").with_filter(filt)
    assert req.event_type == "mars"


@given(filt=_filter_dict)
def test_listen_accepts_any_json_compatible_filter(filt: dict[str, Any]) -> None:
    client = aviso.AvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen("mars", filter=filt)
    iterator.close()


@given(
    sequence=st.integers(min_value=0, max_value=2**63 - 1),
)
def test_watch_from_accepts_any_u64(sequence: int) -> None:
    req = aviso.WatchRequest.watch_from("mars", sequence)
    assert req.event_type == "mars"


@given(
    sequence=st.integers(min_value=2**64, max_value=2**256),
)
def test_watch_from_rejects_int_exceeding_u64(sequence: int) -> None:
    import pytest

    with pytest.raises(ValueError):
        aviso.WatchRequest.watch_from("mars", sequence)
