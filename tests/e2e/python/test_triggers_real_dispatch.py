# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

from __future__ import annotations

import json
import time
from pathlib import Path

import pyaviso
import pytest
from _helpers import receive_within

POLYGON = "40,0,41,0,41,1,40,0"
EVENT_TYPE = "test_polygon"


def _publish(client: pyaviso.AvisoClient, seq: int) -> None:
    client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": POLYGON, "date": "20260606", "time": f"{seq:04d}"},
        payload={"seq": seq, "src": "test_triggers"},
    )


def test_echo_log_webhook_triggers_fire_on_each_notification(
    producer_client: pyaviso.AvisoClient,
    httpserver,  # pytest-httpserver provides this; its public type is unstable
    capfd: pytest.CaptureFixture[str],
    tmp_path: Path,
) -> None:
    log_path = tmp_path / "trigger.log"
    httpserver.expect_request("/hook").respond_with_data("ok", status=200)

    triggers = [
        pyaviso.Trigger.echo(),
        pyaviso.Trigger.log(str(log_path)),
        pyaviso.Trigger.webhook(httpserver.url_for("/hook")),
    ]

    received: list[int] = []
    with producer_client.listen(
        EVENT_TYPE,
        filter={"polygon": POLYGON},
        triggers=triggers,
    ) as iterator:
        time.sleep(0.5)
        _publish(producer_client, 1)
        received.append(receive_within(iterator, timeout=5).payload["seq"])

    assert received == [1]

    out, _err = capfd.readouterr()
    stdout_lines = [json.loads(line) for line in out.splitlines() if line.strip()]
    assert any(line.get("payload", {}).get("seq") == 1 for line in stdout_lines), (
        f"echo trigger should emit NDJSON to stdout; got {out!r}"
    )

    log_lines = [json.loads(line) for line in log_path.read_text().splitlines() if line.strip()]
    assert any(line.get("payload", {}).get("seq") == 1 for line in log_lines)

    deadline = time.time() + 5.0
    while time.time() < deadline and not httpserver.log:
        time.sleep(0.05)
    assert httpserver.log, "webhook trigger should have hit the local HTTP server"
