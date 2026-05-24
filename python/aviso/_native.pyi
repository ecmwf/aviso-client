"""Type stubs for the PyO3 extension module `aviso._native`.

Hand-written. Users never import this module directly; the public Python
surface lives in `aviso.__init__`. These stubs describe the compiled
extension's exports so `ty check` can resolve imports from the wrapper
package.
"""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any

VERSION: str

class Notification:
    def __init__(
        self,
        event_type: str,
        sequence: int,
        identifier: Mapping[str, str],
        payload: Any,
        cloudevent: Mapping[str, Any] | None = None,
    ) -> None: ...
    @property
    def event_type(self) -> str: ...
    @property
    def sequence(self) -> int: ...
    @property
    def identifier(self) -> dict[str, str]: ...
    @property
    def payload(self) -> Any: ...
    @property
    def cloudevent(self) -> dict[str, Any] | None: ...
    def as_dict(self) -> dict[str, Any]: ...

class NotifyResponse:
    def __init__(self, status: str, request_id: str, processed_at: str) -> None: ...
    @property
    def status(self) -> str: ...
    @property
    def request_id(self) -> str: ...
    @property
    def processed_at(self) -> str: ...
    def as_dict(self) -> dict[str, Any]: ...

class SchemaCatalog:
    @property
    def status(self) -> str: ...
    @property
    def schema(self) -> dict[str, Any]: ...
    @property
    def event_types(self) -> list[str]: ...
    @property
    def total_schemas(self) -> int: ...
    def as_dict(self) -> dict[str, Any]: ...

class SchemaResponse:
    @property
    def status(self) -> str: ...
    @property
    def event_type(self) -> str: ...
    @property
    def schema(self) -> dict[str, Any]: ...
    def as_dict(self) -> dict[str, Any]: ...

class AvisoClient:
    def __init__(
        self,
        *,
        base_url: str,
        token: str | None = None,
        timeout: float | None = None,
        user_agent: str | None = None,
    ) -> None: ...
    @property
    def base_url(self) -> str: ...
    def notify(
        self,
        *,
        event_type: str,
        identifier: Mapping[str, str] | None = None,
        payload: Any | None = None,
    ) -> NotifyResponse: ...
    def schema(self) -> SchemaCatalog: ...
    def schema_for(self, event_type: str) -> SchemaResponse: ...
    def wipe_stream(self, stream_name: str) -> None: ...
    def wipe_all(self) -> None: ...
    def delete_notification(self, notification_id: str) -> None: ...
    def __enter__(self) -> AvisoClient: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: Any | None,
    ) -> bool: ...

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
