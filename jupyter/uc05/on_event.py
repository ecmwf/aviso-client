#!/usr/bin/env python3
# Aviso command-trigger handler: retrieve the announced Extremes-DT field, plot it, email it.
#
# Called by `aviso listen`/`aviso replay` through a `command` trigger, e.g.
#   on_event.py --date 20260923 --time 0000 --step 0 --stream oper --levtype sfc --sequence 42
# Environment: EMAIL_DRY_RUN=1 | SMTP_HOST[,SMTP_PORT,SMTP_USER,SMTP_PASSWORD,SMTP_STARTTLS] | sendmail fallback.
import argparse
import json
import os
import pathlib
import shutil
import smtplib
import subprocess
import sys
import time
import traceback
from datetime import datetime, timezone
from email.message import EmailMessage
from email.utils import formatdate, make_msgid

ADDRESSES = {"lumi": "polytope.lumi.apps.dte.destination-earth.eu", "mn5": "polytope.mn5.apps.dte.destination-earth.eu"}
DATASET = "extremes-dt"


def parse_args():
    p = argparse.ArgumentParser(description="Aviso trigger handler: retrieve, plot and email an Extremes-DT field.")
    p.add_argument("--date", required=True, help="YYYYMMDD")
    p.add_argument("--time", required=True, help="HHMM or HH")
    p.add_argument("--step", required=True)
    p.add_argument("--stream", default="oper")
    p.add_argument("--levtype", default="sfc")
    p.add_argument("--type", dest="type_", default="fc")
    p.add_argument("--expver", default="0001")
    p.add_argument("--param", default="167", help="paramId, default 167 (2 m temperature)")
    p.add_argument("--levelist", default=None, help="needed for levtype pl/hl")
    p.add_argument("--area", default=None, help="server-side sub-area N/W/S/E, e.g. 72/-25/30/45 (Europe)")
    p.add_argument("--grid", default=None, help="server-side interpolation target grid, e.g. O320 (O, F, N grids)")
    p.add_argument("--address", default="lumi", help="data bridge: lumi | mn5 | full hostname")
    p.add_argument("--domain", default="Europe", help="earthkit-plots domain name, or 'Global'")
    p.add_argument("--to", default="ulrike.falk@ecmwf.int")
    p.add_argument("--sender", default=os.environ.get("EMAIL_FROM"))
    p.add_argument("--out-dir", default="output")
    p.add_argument("--sequence", default=os.environ.get("AVISO_SEQUENCE", "?"))
    p.add_argument("--event-type", default=os.environ.get("AVISO_EVENT_TYPE", "data"))
    p.add_argument("--no-email", action="store_true")
    p.add_argument("--no-fail-email", action="store_true", help="do not email when the retrieval failed")
    return p.parse_args()


def build_request(a):
    req = {"class": "d1", "dataset": DATASET, "expver": a.expver, "stream": a.stream, "type": a.type_,
           "levtype": a.levtype, "date": a.date, "time": a.time, "step": str(a.step), "param": a.param}
    if a.levelist:
        req["levelist"] = a.levelist
    if a.area:
        n, w, s_, e = (float(x) for x in a.area.split("/"))
        req["area"] = [n, w, s_, e]                 # MARS convention: North/West/South/East
    if a.grid:
        req["grid"] = a.grid
    return req


def file_stem(req):
    stem = "{stream}_{levtype}_{param}_{date}_{time}_step{step}".format(**req)
    if "area" in req:
        stem += "_area" + "_".join(f"{v:g}" for v in req["area"]).replace("-", "m")
    if "grid" in req:
        stem += "_" + req["grid"]
    return stem


def retrieve_and_plot(req, domain, out_dir, address):
    import matplotlib
    matplotlib.use("Agg")
    import earthkit.data as ekd
    import earthkit.plots as ekp

    stem = file_stem(req)
    data = ekd.from_source("polytope", "destination-earth", req, address=address, stream=False)
    fields = data.to_fieldlist() if hasattr(data, "to_fieldlist") else data
    grib = out_dir / f"{stem}.grib"
    fields.to_target("file", str(grib))
    chart = ekp.Map(domain=None if domain == "Global" else domain)
    chart.quickplot(fields[0])
    chart.coastlines()
    chart.gridlines()
    chart.legend()
    chart.title("Extremes-DT {variable_name}, valid {valid_time:%Y-%m-%d %H:%M} (T+{step})")
    png = out_dir / f"{stem}_{domain.replace(' ', '')}.png"
    chart.save(png)
    return grib, png, f"{fields[0].metadata('name')} ({grib.stat().st_size / 1024:.0f} kB GRIB)"


def compose(a, req, png, error):
    ok = error is None
    subject = ("[Aviso] Extremes-DT {stream}/{levtype} {date} {time}Z step {step} "
               + ("available" if ok else "announced (retrieval failed)")).format(**req)
    lines = [f"Aviso '{a.event_type}' notification (sequence {a.sequence}) from the LUMI data bridge.", "",
             "identifier:", *[f"  {k:<8} {req[k]}" for k in ("class", "stream", "type", "levtype", "date", "time", "step")],
             "", "Polytope request:", "  " + json.dumps(req), ""]
    if ok:
        lines += [f"Map attached: {png.name}"]
    else:
        lines += ["Retrieval/plot failed:", "  " + error]
    lines += ["", f"sent by on_event.py at {datetime.now(timezone.utc):%Y-%m-%d %H:%M:%S} UTC"]
    msg = EmailMessage()
    msg["Subject"], msg["From"], msg["To"] = subject, a.sender or a.to, a.to
    msg["Date"], msg["Message-ID"] = formatdate(localtime=True), make_msgid()
    msg.set_content("\n".join(lines))
    if ok and png and png.exists():
        msg.add_attachment(png.read_bytes(), maintype="image", subtype="png", filename=png.name)
    return msg


def send(msg):
    if os.environ.get("EMAIL_DRY_RUN", "0") == "1":
        print("EMAIL_DRY_RUN=1 -> not sending. Message (headers + text part):")
        for h in ("From", "To", "Subject", "Date"):
            print(f"  {h}: {msg[h]}")
        body = msg.get_body(preferencelist=("plain",))
        print("  " + (body.get_content() if body else "").replace("\n", "\n  "))
        print("  attachments:", [p.get_filename() for p in msg.iter_attachments()])
        return "dry-run"
    host = os.environ.get("SMTP_HOST")
    if host:
        port = int(os.environ.get("SMTP_PORT", "587"))
        user, password = os.environ.get("SMTP_USER"), os.environ.get("SMTP_PASSWORD")
        with smtplib.SMTP(host, port, timeout=30) as smtp:
            if os.environ.get("SMTP_STARTTLS", "1") == "1":
                smtp.starttls()
            if user:
                smtp.login(user, password or "")
            smtp.send_message(msg)
        return f"smtp://{host}:{port}"
    sendmail = shutil.which("sendmail") or "/usr/sbin/sendmail"
    if pathlib.Path(sendmail).exists():
        subprocess.run([sendmail, "-t", "-oi"], input=msg.as_bytes(), check=True, timeout=60)
        return f"{sendmail} (local MTA; delivery depends on its relay configuration)"
    raise RuntimeError("no mail transport: set SMTP_HOST or EMAIL_DRY_RUN=1, or install sendmail")


def main():
    a = parse_args()
    out_dir = pathlib.Path(a.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    log = out_dir / "on_event.log"
    req = build_request(a)
    t0 = time.monotonic()
    png, error = None, None
    address = ADDRESSES.get(a.address, a.address)
    try:
        grib, png, name = retrieve_and_plot(req, a.domain, out_dir, address)
        print(f"retrieved {name} -> {grib.name}, {png.name} in {time.monotonic() - t0:.1f}s")
    except Exception as exc:                        # noqa: BLE001 - report anything, keep the listener alive
        error = f"{type(exc).__name__}: {str(exc).splitlines()[0][:300]}"
        print("retrieval/plot failed:", error, file=sys.stderr)
    status = "ok" if error is None else "retrieval-failed"
    if not a.no_email and (error is None or not a.no_fail_email):
        try:
            via = send(compose(a, req, png, error))
            print(f"email -> {a.to} via {via}")
        except Exception as exc:                    # noqa: BLE001
            status += "+email-failed"
            print("email failed:", f"{type(exc).__name__}: {exc}", file=sys.stderr)
    with log.open("a", encoding="utf-8") as fh:
        fh.write(json.dumps({"ts": datetime.now(timezone.utc).isoformat(timespec="seconds"), "sequence": a.sequence,
                             "status": status, "request": req, "png": str(png) if png else None, "error": error}) + "\n")
    sys.exit(0 if "email-failed" not in status else 1)


if __name__ == "__main__":
    main()
