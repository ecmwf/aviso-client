<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# Authentication

Ask your server operator for its URL and credentials. Users need permission to
receive notifications; providers need permission to publish. An auth provider
tells the client where to get credentials. It does not grant permissions.

## Environment

Start with [pyaviso installed](./install.md) and the
[quickstart environment](./quickstart.md#set-the-environment). Set
`AVISO_BASE_URL` and `AVISO_TOKEN`, or unset the token and set both
`AVISO_USERNAME` and `AVISO_PASSWORD`.

Save this as `check_auth.py` and run `python check_auth.py`:

```python
import os

import pyaviso

client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env()
)
print(client.schema().event_types)
```

It prints the server's configured event types, for example `['mars']` for the
[quickstart schema](./quickstart.md#what-is-on-your-server). Schema discovery
does not prove you can listen or publish: those operations have their own
permission checks, and discovery may be public.

`Env()` reads credentials when constructed:

- A non-empty `AVISO_TOKEN` takes precedence over username and password.
- Otherwise it uses a non-empty `AVISO_USERNAME` with `AVISO_PASSWORD` set.
  The password may be an empty string if your service permits that.
- Without either combination, construction raises `pyaviso.AuthError`.

There is no anonymous fallback. For an anonymous server, omit
`auth=pyaviso.Env()` from the client initialization. Updating environment
variables does not change an existing `Env` provider; create it again or restart
the script.

## Bearer token

To select a token explicitly, replace the client initialization in
`check_auth.py` with this block. Keep the imports and schema call:

```python
client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=pyaviso.Bearer(os.environ["AVISO_TOKEN"]),
)
```

This sends the token in the HTTP `Authorization` header. `Bearer` redacts it in
`repr()`. Keep credentials out of source files and do not print them.

## Basic auth

With `AVISO_USERNAME` and `AVISO_PASSWORD` set, use this replacement instead:

```python
client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=pyaviso.Basic(os.environ["AVISO_USERNAME"], os.environ["AVISO_PASSWORD"]),
)
```

This explicitly selects username/password authentication even if `AVISO_TOKEN`
is also set. Basic authentication encodes credentials; HTTPS protects them in
transit. `Basic` redacts its password in `repr()`.

## Config file

`ConfigFile` reads an **auth-only YAML file**, not the CLI's general
configuration file. Create `~/.config/aviso/auth.yaml` with exactly one of the
following shapes, replacing the example values with your credentials:

```yaml
bearer:
  token: your-bearer-token
```

Or, for Basic authentication:

```yaml
basic:
  username: your-username
  password: your-password
```

Restrict file access to the account running your script. Keep the URL in the
client initialization, not this file. Unknown fields, both sections, neither
section, an unreadable file or malformed YAML raise `pyaviso.ConfigError` at
construction.

Use this replacement client initialization in `check_auth.py`:

```python
client = pyaviso.AvisoClient(
    base_url=os.environ["AVISO_BASE_URL"],
    auth=pyaviso.ConfigFile("~/.config/aviso/auth.yaml"),
)
```

The path accepts a string or `Path`; `~` is expanded. The file is read at
construction and reread when the client requests a credential refresh after
HTTP 401. A file change alone does not immediately replace the cached value.

## Refresh on 401

With an auth provider, the client attempts refresh after a 401 and retries the
request once if refresh succeeds. A second 401 is still an error: `HttpError`
for a one-shot request, or `AuthError` when watch authentication remains
rejected.

- `Bearer`, `Basic` and `Env` keep their original credentials; refresh does not
  change them.
- `ConfigFile` rereads its source file. An invalid replacement file raises
  `AuthError` during refresh.
- `Chain` refreshes the first member currently able to produce a header.

For static credentials, fix the source and construct a new provider/client or
restart the script. Inspect
[HTTP errors](./error-handling.md#httperror-exposes-the-servers-response)
to distinguish a server rejection from a local credential setup failure.

## Chain

This is an advanced option for already-constructed providers. `Chain` asks each
member for an authorization header in order and uses the first successful
result. If all fail, it propagates the last provider error. An empty chain
raises `AuthError` when a request needs credentials.

It is not a list of credentials to try against a server. A 401 does not switch
to the next member. `Env()` and `ConfigFile(...)` are constructed before being
passed to `Chain`; a missing environment or invalid file raises immediately,
before the chain can provide fallback. Choose and validate your available
credential source during setup rather than relying on a chain to skip those
construction errors.

## With `AsyncAvisoClient`

The same providers work with the async client. Credential setup remains
synchronous; HTTP methods are awaited. See [Async](./async.md) if your
application already uses `asyncio`.
