from __future__ import annotations

import pyaviso
import pytest

EVENT_TYPE = "test_polygon"


def test_invalid_credentials_get_401(base_url: str) -> None:
    with (
        pyaviso.AvisoClient(
            base_url=base_url, auth=pyaviso.Basic("reader-user", "definitely-wrong-pass")
        ) as client,
        pytest.raises(pyaviso.HttpError) as exc_info,
    ):
        client.notify(
            event_type=EVENT_TYPE,
            identifier={
                "polygon": "85,0,86,0,86,1,85,0",
                "date": "20260603",
                "time": "0000",
            },
            payload={"test": "bad-creds"},
        )
    assert exc_info.value.status == 401


def test_role_mismatch_gets_403(reader_client: pyaviso.AvisoClient) -> None:
    with pytest.raises(pyaviso.HttpError) as exc_info:
        reader_client.notify(
            event_type=EVENT_TYPE,
            identifier={
                "polygon": "87,0,88,0,88,1,87,0",
                "date": "20260603",
                "time": "0001",
            },
            payload={"test": "wrong-role"},
        )
    assert exc_info.value.status == 403
