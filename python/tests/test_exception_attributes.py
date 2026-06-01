"""Round-trip tests for every ClientError variant the binding maps.

For each documented variant the binding's ``_provoke_error`` test helper
synthesises a Rust ``ClientError``, runs it through ``map_client_error``,
and lets the Python side observe the resulting exception. The tests
assert the class and every documented attribute is set.

Adding a new variant in the core crate requires extending the table here
plus the matching arm in ``crates/aviso-py/src/error.rs::map_client_error``.
"""

from __future__ import annotations

import pyaviso
import pytest
from pyaviso import _native


def test_auth_error_carries_message() -> None:
    with pytest.raises(pyaviso.AuthError) as excinfo:
        _native._provoke_error("auth", message="bad credentials")
    assert "bad credentials" in str(excinfo.value)


def test_decode_error_carries_inner_message() -> None:
    with pytest.raises(pyaviso.DecodeError) as excinfo:
        _native._provoke_error("decode")
    rendered = str(excinfo.value)
    assert rendered, "decode error should render a non-empty message"


def test_malformed_event_error_carries_detail() -> None:
    with pytest.raises(pyaviso.MalformedEventError) as excinfo:
        _native._provoke_error("malformed_event", detail="missing '@' separator: 'mars'")
    assert "missing '@' separator" in str(excinfo.value)


def test_history_gap_replay_limit_attributes() -> None:
    with pytest.raises(pyaviso.HistoryGapError) as excinfo:
        _native._provoke_error("history_gap_replay_limit", max_allowed=10_000)
    err = excinfo.value
    assert err.reason == "replay_limit_reached"
    assert err.max_allowed == 10_000
    assert err.expected is None
    assert err.observed is None


def test_history_gap_sequence_jump_attributes() -> None:
    with pytest.raises(pyaviso.HistoryGapError) as excinfo:
        _native._provoke_error("history_gap_sequence_jump", expected=42, observed=99)
    err = excinfo.value
    assert err.reason == "sequence_jump"
    assert err.expected == 42
    assert err.observed == 99
    assert err.max_allowed is None


def test_stream_protocol_error_attributes() -> None:
    with pytest.raises(pyaviso.StreamProtocolError) as excinfo:
        _native._provoke_error(
            "stream_protocol",
            message="stream_processing_failed",
            request_id="req-stream-001",
        )
    err = excinfo.value
    assert err.message == "stream_processing_failed"
    assert err.request_id == "req-stream-001"


def test_stream_protocol_error_with_no_request_id() -> None:
    with pytest.raises(pyaviso.StreamProtocolError) as excinfo:
        _native._provoke_error("stream_protocol", message="boom")
    assert excinfo.value.request_id is None


def test_config_error_carries_message() -> None:
    with pytest.raises(pyaviso.ConfigError) as excinfo:
        _native._provoke_error("config", message="missing base_url")
    assert "missing base_url" in str(excinfo.value)


def test_state_store_error_carries_inner_message() -> None:
    with pytest.raises(pyaviso.StateStoreError) as excinfo:
        _native._provoke_error("state_store_io", message="disk full")
    assert "disk full" in str(excinfo.value)


def test_trigger_failed_echo_io_attributes() -> None:
    with pytest.raises(pyaviso.TriggerError) as excinfo:
        _native._provoke_error("trigger_failed_echo_io", message="broken pipe")
    err = excinfo.value
    assert err.trigger_kind == "echo"
    assert err.error_kind == "io"
    assert err.path is None
    assert err.exit_code is None
    assert err.template_kind is None


def test_trigger_failed_log_io_attributes() -> None:
    with pytest.raises(pyaviso.TriggerError) as excinfo:
        _native._provoke_error(
            "trigger_failed_log_io",
            log_path="/var/log/aviso.log",
            message="permission denied",
        )
    err = excinfo.value
    assert err.trigger_kind == "log"
    assert err.error_kind == "io"
    assert err.path == "/var/log/aviso.log"


def test_trigger_failed_webhook_4xx_attributes() -> None:
    with pytest.raises(pyaviso.TriggerError) as excinfo:
        _native._provoke_error(
            "trigger_failed_webhook_4xx",
            status=429,
            body="rate limited",
        )
    err = excinfo.value
    assert err.trigger_kind == "webhook"
    assert err.error_kind == "webhook"
    assert err.status == 429
    assert err.body_tail == "rate limited"


def test_trigger_failed_template_missing_attributes() -> None:
    with pytest.raises(pyaviso.TriggerError) as excinfo:
        _native._provoke_error(
            "trigger_failed_template_missing",
            detail="webhook url",
            message="notification.payload.target",
        )
    err = excinfo.value
    assert err.trigger_kind == "webhook"
    assert err.error_kind == "template"
    assert err.context == "webhook url"
    assert err.field == "notification.payload.target"
    assert err.template_kind == "missing"


@pytest.mark.parametrize(
    "exception_class",
    [
        pyaviso.AuthError,
        pyaviso.ConfigError,
        pyaviso.DecodeError,
        pyaviso.HistoryGapError,
        pyaviso.HttpError,
        pyaviso.MalformedEventError,
        pyaviso.StateStoreError,
        pyaviso.StreamProtocolError,
        pyaviso.TransportError,
        pyaviso.TriggerError,
    ],
)
def test_every_specific_class_subclasses_aviso_error(exception_class: type) -> None:
    assert issubclass(exception_class, pyaviso.AvisoError)
