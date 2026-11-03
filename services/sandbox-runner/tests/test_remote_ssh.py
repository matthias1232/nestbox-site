"""Tests for the remote SSH adapter, without a machine to reach.

`app.remote_ssh` imports FastAPI, asyncssh and pydantic at module scope, and only
pydantic is guaranteed on a machine that just wants to run this test. The two
stand-ins below are install only if the real packages are missing, so the module
under test is always the real file.

What is worth testing here is the part that can corrupt a command: how an
environment value is quoted for the shell it is pasted into, and which keys are
refused. The SSH call itself is one library call and is not re-implemented.
"""
from __future__ import annotations

import asyncio
import sys
import types
import unittest

# --- framework stand-ins, only when the real ones are absent --------------


try:
    import asyncssh  # noqa: F401
except ModuleNotFoundError:  # pragma: no cover - the runner image has it
    _asyncssh = types.ModuleType("asyncssh")

    class _Error(Exception):
        """Stand-in for the library's base error."""

    _asyncssh.Error = _Error
    _asyncssh.import_private_key = lambda _text: object()
    _asyncssh.connect = None
    sys.modules["asyncssh"] = _asyncssh

try:
    import fastapi  # noqa: F401
except ModuleNotFoundError:  # pragma: no cover - the runner image has it
    _fastapi = types.ModuleType("fastapi")

    class _HTTPException(Exception):
        """Stand-in carrying the status the real exception would answer with."""

        def __init__(self, status_code: int, detail: str = "") -> None:
            super().__init__(detail)
            self.status_code = status_code
            self.detail = detail

    class _APIRouter:
        def post(self, _path: str):
            return lambda handler: handler

    _fastapi.APIRouter = _APIRouter
    _fastapi.HTTPException = _HTTPException
    sys.modules["fastapi"] = _fastapi

try:
    import pydantic  # noqa: F401
except ModuleNotFoundError:  # pragma: no cover - the runner image has it
    _pydantic = types.ModuleType("pydantic")

    class _Field:
        """Stand-in for a field declaration with a default."""

        def __init__(self, default=None, **_bounds) -> None:
            self.default = default

    class _BaseModel:
        """Stand-in that takes the fields as keywords and keeps them."""

        def __init__(self, **values) -> None:
            for name in dir(type(self)):
                if not name.startswith("_") and hasattr(getattr(type(self), name), "default"):
                    setattr(self, name, values.get(name, getattr(type(self), name).default))
            for name, value in values.items():
                setattr(self, name, value)

    _pydantic.BaseModel = _BaseModel
    _pydantic.Field = _Field
    sys.modules["pydantic"] = _pydantic

from app import remote_ssh  # noqa: E402  (after the stand-ins, on purpose)


class EnvPrefixTests(unittest.TestCase):
    def test_a_quote_in_a_value_is_escaped_for_the_shell(self):
        self.assertEqual(remote_ssh.env_prefix({"NB_X": "a'b"}), "export NB_X='a'\\''b'\n")

    def test_a_value_with_a_dollar_and_a_newline_stays_literal(self):
        text = remote_ssh.env_prefix({"NB_X": "$HOME\nline two"})
        self.assertEqual(text, "export NB_X='$HOME\nline two'\n")

    def test_several_keys_keep_their_order(self):
        text = remote_ssh.env_prefix({"NB_A": "1", "NB_B": "2"})
        self.assertEqual(text.splitlines(), ["export NB_A='1'", "export NB_B='2'"])

    def test_an_empty_env_is_empty_text(self):
        self.assertEqual(remote_ssh.env_prefix({}), "")

    def test_a_key_that_is_not_a_shell_name_is_refused(self):
        for bad in ("nb_x", "1X", "A;B", "A-B", "", "A B", "A\nB"):
            with self.assertRaises(ValueError, msg=bad):
                remote_ssh.env_prefix({bad: "v"})


class SshRunRequestTests(unittest.TestCase):
    def test_a_password_or_a_key_is_required(self):
        body = remote_ssh.SshRun(host="192.0.2.10", script="true")
        with self.assertRaises(remote_ssh.HTTPException) as raised:
            asyncio.run(remote_ssh.ssh_run(body))
        self.assertEqual(raised.exception.status_code, 422)

    def test_a_bad_environment_key_is_a_422_and_not_a_500(self):
        body = remote_ssh.SshRun(host="192.0.2.10", password="x", script="true", env={"A;B": "v"})
        with self.assertRaises(remote_ssh.HTTPException) as raised:
            asyncio.run(remote_ssh.ssh_run(body))
        self.assertEqual(raised.exception.status_code, 422)


if __name__ == "__main__":
    unittest.main()
