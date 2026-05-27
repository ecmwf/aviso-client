from __future__ import annotations

import json
import threading
import time
from pathlib import Path

import aviso
import pytest

POLYGON = "40,0,41,0,41,1,40,0"
EVENT_TYPE = "test_polygon"


def _publish(client: aviso.AvisoClient, seq: int) -> None:
    client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": POLYGON, "date": "20260606", "time": f"{seq:04d}"},
        payload={"seq": seq, "src": "test_triggers"},
    )


def test_echo_log_webhook_triggers_fire_on_each_notification(
    producer_client: aviso.AvisoClient,
    httpserver,  # pytest-httpserver provides this; its public type is unstable
    capfd: pytest.CaptureFixture[str],
    tmp_path: Path,
) -> None:
    log_path = tmp_path / "trigger.log"
    httpserver.expect_request("/hook").respond_with_data("ok", status=200)

    triggers = [
        aviso.Trigger.echo(),
        aviso.Trigger.log(str(log_path)),
        aviso.Trigger.webhook(httpserver.url_for("/hook")),
    ]

    received: list[int] = []
    with producer_client.listen(
        EVENT_TYPE,
        filter={"polygon": POLYGON},
        triggers=triggers,
    ) as iterator:
        time.sleep(0.5)
        threading.Thread(target=_publish, args=(producer_client, 1), daemon=True).start()
        received.append(next(iterator).payload["seq"])

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
