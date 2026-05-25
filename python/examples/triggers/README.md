# triggers

The five trigger kinds the library exposes, each attached via the `triggers=` kwarg on `client.listen()`. The supervisor dispatches every configured trigger per notification before the iterator yields it to your code; required triggers terminate the watch on failure, optional triggers log a warning and let it continue.

- **01_echo.py** -- prints one compact JSON line per notification to stdout.
- **02_log.py** -- appends one compact JSON line per notification to a file under a temp directory.
- **03_multiple.py** -- combines `echo` and `log` to demonstrate that triggers compose in declaration order.
- **04_command.py** -- runs `/bin/sh -c <rendered_command>` per notification, with notification fields exposed as `AVISO_*` env vars and `{{ notification.<dotted.path> }}` templates. Unix only; skipped on Windows with a clear message.
- **05_webhook.py** -- illustration only (placeholder URL, not runnable end-to-end). For the runnable counterpart that spins up an in-process HTTP server, see `advanced/03_webhook_with_local_server.py`.
