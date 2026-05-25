"""Trigger builder tests."""

from __future__ import annotations

import pathlib
import sys

import aviso
import pytest


def test_echo_constructs_with_defaults() -> None:
    t = aviso.Trigger.echo()
    assert "echo" in repr(t).lower()


def test_echo_with_kwargs() -> None:
    aviso.Trigger.echo(retries=2, required=False, label="mars-od")


def test_log_constructs(tmp_path: pathlib.Path) -> None:
    aviso.Trigger.log(tmp_path / "out.log")


def test_log_accepts_str(tmp_path: pathlib.Path) -> None:
    aviso.Trigger.log(str(tmp_path / "out.log"))


def test_webhook_constructs() -> None:
    aviso.Trigger.webhook("http://localhost:8000/hook")


def test_webhook_with_full_kwargs() -> None:
    aviso.Trigger.webhook(
        "http://localhost:8000/hook",
        method=aviso.HttpMethod.POST,
        headers={"Authorization": "Bearer x"},
        body_template='{"sequence": "{{ notification.sequence }}"}',
        retries=3,
        timeout=10.0,
        fail_fast=False,
    )


def test_webhook_rejects_unknown_method() -> None:
    with pytest.raises(ValueError):
        aviso.Trigger.webhook("http://localhost:8000", method="BREW")


def test_teams_constructs() -> None:
    aviso.Trigger.teams("https://teams.example.com/hook")


def test_post_constructs() -> None:
    aviso.Trigger.post("http://localhost:8000/post")


@pytest.mark.skipif(sys.platform == "win32", reason="command trigger is Unix-only")
def test_command_constructs_on_unix() -> None:
    aviso.Trigger.command("/bin/echo hello", env={"X": "1"}, retries=2)


def test_chainable_setters_return_new_instance() -> None:
    base = aviso.Trigger.echo()
    tuned = base.retries(5).required(False).timeout(2.0).fail_fast(False).label("x")
    assert tuned is not base
    assert "Trigger" in repr(tuned)


def test_http_method_value_matches_string() -> None:
    assert aviso.HttpMethod.POST.value == "POST"
    assert aviso.HttpMethod.GET.value == "GET"


def test_watch_mode_value() -> None:
    assert aviso.WatchMode.WATCH.value == "watch"
    assert aviso.WatchMode.REPLAY_ONLY.value == "replay_only"
