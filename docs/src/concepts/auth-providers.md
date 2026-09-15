# Authentication providers

<div class="reference-guide">

Authentication tells the server who you are. Ask your service operator for the
server address and the credentials to use: a token, or a username and password.
Some servers allow you to receive public notifications without credentials.

Receiving notifications needs read permission. Publishing needs write
permission. Managing schemas and deleting notifications are operator tasks.
Valid credentials do not necessarily grant all of these permissions.

An authentication provider supplies credentials to the client. This use of
"provider" is different from a data provider, who publishes notifications.
For setup, follow [CLI authentication](../cli/configuration.md#authentication)
or [Python authentication](../python/auth.md).

## Which one the CLI picks for you

The CLI checks credentials in this order:

1. Command-line flags: `--token`, or `--username` with `--password`.
2. Environment variables: `AVISO_TOKEN`, or `AVISO_USERNAME` with
   `AVISO_PASSWORD`.
3. The config file: `auth.bearer_token`, or `auth.basic` with both `username`
   and `password`.
4. If none are set, an anonymous connection with no credentials.

Choose one authentication method at a time. Conflicting credential flags are
rejected. Incomplete environment credentials cause an error rather than silently
falling back to the file. When a token and a complete username/password pair
are both in the environment, the token takes precedence. The file rejects a
configuration containing both methods.

Python clients are anonymous by default. They use environment credentials only
when you explicitly supply `pyaviso.Env()` as their authentication provider.

## What happens on a 401

`401 Unauthorized` means the server rejected the credentials. aviso asks the
selected authentication provider to refresh them, then retries once. A second
401 in the same attempt cycle stops the request with an authentication error.

- `Bearer` and `Basic` hold fixed credentials. Refresh does not change them.
- `Env` reads the process's environment once, when the provider is created.
  Refresh does not change those credentials. Create a new provider and client
  after updating the environment, or restart the listener with the new values.
- `ConfigFile` re-reads its credentials file, so a replaced credential can be
  picked up on refresh.
- `Chain` refreshes the member that supplied the rejected header.

The CLI turns credentials from its main config file into a fixed `Bearer` or
`Basic` provider. Editing that file does not give it `ConfigFile` refresh
behaviour. Restart the CLI to use the changed main configuration.

`403 Forbidden` usually means the account is not allowed to perform the
requested operation. Ask the service operator about the needed permission.

## The five providers at a glance

These program names appear in logs and library documentation. Bearer sends a
token; Basic sends a username and password encoded in an HTTP header. Encoding
is not encryption, so use your service's HTTPS address for credentials.

| Provider | Credential source |
|---|---|
| `Bearer` | A token supplied in code. |
| `Basic` | A username and password supplied in code. |
| `Env` | The process's environment variables. |
| `ConfigFile` | A separate credentials file. |
| `Chain` | An ordered list of providers. |

All five mark the `Authorization` request header as sensitive so the HTTP
libraries can hide its value in logs.

## When you need `ConfigFile`

This is a library option for credentials kept separately from the main client
configuration. A Rust caller uses `ConfigFile::from_path`. The CLI does not
select this provider for its main config file.

The separate credentials file contains one of these shapes, with your own
credentials in place of the example values:

```yaml
bearer:
  token: "opaque-or-jwt-token"
```

or:

```yaml
basic:
  username: "alice"
  password: "wonderland"
```

Use exactly one block. The parser rejects both, neither, or unknown keys.
These are credentials-file examples, not main CLI configuration files.

## When you need `Chain`

Library callers can use `Chain` to try several credential sources in order.
The first member that successfully supplies a request header wins. This is a
fallback between sources, not an attempt to log in with every credential until
the server accepts one. A 401 refreshes the selected member.

The CLI already checks flags, environment and configuration in order, so you do
not need to construct a chain for ordinary command-line use. See the
[library guide](../developers/lib-guide.md) for programmatic setup.

## Writing a custom provider

If your login service is not covered by the built-in providers, an application
developer can add support for it. See the
[Rust API reference](../reference/rust-api.md) for the developer interface.

## What next

- [CLI configuration: authentication](../cli/configuration.md#authentication):
  set credentials for commands.
- [Python authentication](../python/auth.md): choose credentials in Python.
- [Library guide](../developers/lib-guide.md): configure Rust applications.

</div>
