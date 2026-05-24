"""Type stubs for the PyO3 extension module `aviso._native`.

Hand-written. Users never import this module directly; the public Python
surface lives in `aviso.__init__`. These stubs describe the compiled
extension's exports so `ty check` can resolve imports from the wrapper
package.
"""

from __future__ import annotations

VERSION: str

class AvisoError(Exception): ...
class TransportError(AvisoError): ...

class HttpError(AvisoError):
    status: int
    body: str
    request_id: str | None

class AuthError(AvisoError): ...
class DecodeError(AvisoError): ...
class MalformedEventError(AvisoError): ...

class HistoryGapError(AvisoError):
    reason: str
    max_allowed: int | None
    expected: int | None
    observed: int | None

class StreamProtocolError(AvisoError):
    message: str
    request_id: str | None

class ConfigError(AvisoError): ...
class StateStoreError(AvisoError): ...

class TriggerError(AvisoError):
    trigger_kind: str
    error_kind: str
    path: str | None
    exit_code: int | None
    stderr_tail: str | None
    status: int | None
    body_tail: str | None
    reason: str | None
    timeout_seconds: float | None
    context: str | None
    field: str | None
    template_kind: str | None

def _provoke_error(
    kind: str,
    *,
    status: int | None = None,
    body: str | None = None,
    request_id: str | None = None,
    message: str | None = None,
    detail: str | None = None,
    sequence: int | None = None,
    max_allowed: int | None = None,
    expected: int | None = None,
    observed: int | None = None,
    log_path: str | None = None,
) -> None: ...
