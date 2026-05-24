# Python

A Python package for aviso is on the roadmap. Until it ships, the recommended way to use aviso from Python is to call the `aviso` command-line tool with [`subprocess`](https://docs.python.org/3/library/subprocess.html) and pipe its NDJSON output.

For today's workflows, the command-line tool covers the main jobs: publish, listen, replay, inspect schemas, and run every built-in trigger.

## Publish a notification from Python

```python
import json, subprocess

result = subprocess.run(
    [
        "aviso", "--base-url", "https://aviso.example",
        "notify",
        'event=mars,class=od,stream=oper,date=20260601,'
        'domain=g,expver=0001,step=0,time=1200,'
        'data={"location":"s3://bucket/path"}',
    ],
    capture_output=True, text=True, check=True,
)
print(result.stdout)
```

Pass the bearer token through `--token` or set `AVISO_TOKEN` in the environment.

## Listen for notifications from Python

`aviso listen` streams one JSON object per line to stdout when its output is piped, which is exactly the shape Python wants:

```python
import json, subprocess

proc = subprocess.Popen(
    [
        "aviso", "--base-url", "https://aviso.example",
        "listen",
        "--event", "mars",
        "--identifiers", '{"class":"od"}',
    ],
    stdout=subprocess.PIPE, text=True,
)

for line in proc.stdout:
    notification = json.loads(line)
    print(notification["sequence"], notification["payload"])
```

Press Ctrl+C in the parent process to stop the listener cleanly.

## What you get

- The same delivery guarantees and resume behaviour the CLI provides.
- The same triggers (write to a file, run a shell command, post to a webhook, send to Microsoft Teams).
- The same authentication options.

## When the Python package arrives

The [getting-started quickstart](../getting-started/quickstart.md) page will be updated with a native Python example. Existing scripts that wrap the CLI will keep working; the package is additive, not a replacement.

Until then, the [CLI overview](../cli/overview.md) is the right entry point.
