# Seed a DestinE demo Aviso server with clearly labelled example notifications.
#
# Usage:
#     AVISO_BASE_URL=https://<server> AVISO_TOKEN=<token> python seed_demo_events.py [--tag TAG] [--count N]
#
# The token may also come from ~/.config/aviso/config.yaml (auth.bearer_token) or ~/.polytopeapirc (user_key).
import argparse
import json
import os
import pathlib
import sys
from datetime import datetime, timedelta, timezone

import pyaviso


def resolve_token():
    if os.environ.get("AVISO_TOKEN"):
        return os.environ["AVISO_TOKEN"]
    cfg = pathlib.Path.home() / ".config" / "aviso" / "config.yaml"
    if cfg.exists():
        import yaml
        tok = ((yaml.safe_load(cfg.read_text()) or {}).get("auth") or {}).get("bearer_token")
        if tok:
            return tok
    rc = pathlib.Path.home() / ".polytopeapirc"
    if rc.exists():
        return json.loads(rc.read_text()).get("user_key")
    sys.exit("no token: set AVISO_TOKEN")


def build_batch(event_types, tag, count):
    now = datetime.now(timezone.utc)
    batch = []
    for i in range(count):
        t = now - timedelta(hours=6 * i)
        if "de-scm-data" in event_types:
            batch.append({"event_type": "de-scm-data",
                          "identifier": {"experiment": tag[:10], "date": t.strftime("%Y%m%d"), "time": t.strftime("%H%M")},
                          "payload": {"output_path": f"ec:/RDX/prepIFS/{tag}/{t:%Y%m%d%H%M}/",
                                      "note": "DestinE Annual meeting demo data (seed_demo_events.py)"}})
        if "destine-demo" in event_types:
            batch.append({"event_type": "destine-demo",
                          "identifier": {"team": tag[:20], "date": t.strftime("%Y%m%d"), "time": t.strftime("%H%M")},
                          "payload": {"message": f"hello from {tag}", "note": "DestinE Annual meeting demo"}})
    return batch


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", default="despdemo")
    ap.add_argument("--count", type=int, default=3)
    args = ap.parse_args()
    base = os.environ.get("AVISO_BASE_URL", "http://localhost:8000")
    client = pyaviso.AvisoClient(base_url=base, auth=pyaviso.Bearer(resolve_token()))
    available = set(client.schema().event_types)
    batch = build_batch(available, args.tag, args.count)
    if not batch:
        sys.exit(f"{base} has none of the demo streams (found {sorted(available)})")
    results = client.notify_many(batch)
    ok = sum(1 for r in results if r.ok)
    for r in results:
        item = batch[r.index]
        status = r.response.status if r.ok else f"FAILED {r.error}"
        print(f"[{r.index}] {item['event_type']} {item['identifier']} -> {status}")
    print(f"{ok}/{len(batch)} published to {base}")
    return 0 if ok == len(batch) else 1


if __name__ == "__main__":
    sys.exit(main())
