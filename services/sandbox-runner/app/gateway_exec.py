"""Running things inside a sandbox, whichever runtime holds it.

A gateway (OpenShell) sandbox is a Docker container with a boundary inside it:
the supervisor puts the workload in its own network namespace, gives it the
proxy and the environment the policy names, and decides every connection. A
process started with `docker exec` is inside the container but *outside* that
boundary — no proxy variables, no CA, no route — so a terminal, a desktop
bridge or a probe started that way sees a different machine than the agent does
(`curl` answers 000, the CA file is there but nothing points at it).

For a gateway sandbox everything therefore goes through the gateway
(`openshell sandbox exec`). For a Docker sandbox nothing changes. The callers
ask this module and do not care which one they got.
"""
from __future__ import annotations

import fcntl
import logging
import os
import pty
import signal
import struct
import subprocess
import termios
from typing import Any, NamedTuple

from app import openshell_backend as osh
from app.workload import openshell_name

logger = logging.getLogger("nestbox.runner.gateway")

TERMINAL_ENV = ("TERM=xterm-256color", "COLORTERM=truecolor", "LANG=C.UTF-8")


class ExecResult(NamedTuple):
    """The shape Docker's `exec_run` returns, so callers use either unchanged."""

    exit_code: int
    output: bytes


def gateway_name(container: Any) -> str | None:
    """The sandbox name when `container` is a gateway workload, else `None`."""
    return openshell_name(container.name)


def exec_run(container: Any, argv: list[str], *, user: str | None = None, timeout: int = 25) -> ExecResult:
    """Run `argv` to completion inside the sandbox and return its output.

    Docker sandboxes use `docker exec` (as `user` when given). Gateway sandboxes
    use the gateway: the command runs as the sandbox user, inside the boundary.
    """
    name = gateway_name(container)
    if name is None:
        result = container.exec_run(argv, stdout=True, stderr=True, user=user or "")
        return ExecResult(result.exit_code if result.exit_code is not None else 0, result.output or b"")
    try:
        code, text = osh.exec_command(name, argv, workdir=None, timeout=timeout)
    except osh.OpenShellError as exc:
        return ExecResult(255, str(exc).encode())
    return ExecResult(code, text.encode())


class GatewayPty:
    """A terminal in a gateway sandbox: `openshell sandbox exec --tty` on a local PTY.

    The local PTY exists only because the CLI wants a terminal on its own stdin and
    stdout. The shell it starts is `tmux new-session -A`, so closing the browser
    ends this CLI process and leaves the session running in the sandbox, to be
    picked up again by the same tab.
    """

    def __init__(self, process: subprocess.Popen, master: int) -> None:
        self._process = process
        self._master = master

    @classmethod
    def open(cls, name: str, tab: str) -> "GatewayPty":
        has_tmux = osh.exec_command(name, ["sh", "-c", "command -v tmux >/dev/null"], workdir=None, timeout=15)[0] == 0
        command = ["tmux", "new-session", "-A", "-s", f"nestbox-{tab}"] if has_tmux else ["bash", "-l"]
        args = [osh.CLI, "sandbox", "exec", "-n", name, "--tty"]
        for item in TERMINAL_ENV:
            args += ["--env", item]
        args += ["--", *command]
        master, slave = pty.openpty()
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        process = subprocess.Popen(  # noqa: S603 - fixed argument vector, tab validated by the caller
            args,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            start_new_session=True,
            close_fds=True,
            env=os.environ.copy(),
        )
        os.close(slave)
        return cls(process, master)

    @classmethod
    def open_command(cls, name: str, command: list[str]) -> "GatewayPty":
        """Open an interactive command (not only a terminal tab) inside a gateway sandbox.

        The SSH door needs what a terminal tab cannot express: one command
        instead of `tmux new-session`, or an explicit tab the caller chose. The
        argv is built by the caller and is already an argument vector, so
        nothing here decides what runs.
        """
        args = [osh.CLI, "sandbox", "exec", "-n", name, "--tty"]
        for item in TERMINAL_ENV:
            args += ["--env", item]
        args += ["--", *command]
        master, slave = pty.openpty()
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        process = subprocess.Popen(  # noqa: S603 - fixed argument vector, argv validated by the caller
            args,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            start_new_session=True,
            close_fds=True,
            env=os.environ.copy(),
        )
        os.close(slave)
        return cls(process, master)

    def read(self, size: int = 8192) -> bytes:
        try:
            return os.read(self._master, size)
        except OSError:
            return b""

    def write(self, data: bytes) -> None:
        view = memoryview(data)
        while view:
            written = os.write(self._master, view)
            view = view[written:]

    def resize(self, cols: int, rows: int) -> None:
        fcntl.ioctl(self._master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        try:
            self._process.send_signal(signal.SIGWINCH)
        except ProcessLookupError:
            pass

    def close(self) -> None:
        try:
            self._process.send_signal(signal.SIGHUP)
        except ProcessLookupError:
            pass
        try:
            self._process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self._process.kill()
        try:
            os.close(self._master)
        except OSError:
            pass


class GatewayStream:
    """A byte stream to a program started in a gateway sandbox, over its pipes.

    Used for the desktop: the program inside copies bytes between its own
    `127.0.0.1:5900` and these pipes. No TTY, so nothing rewrites the bytes.
    """

    def __init__(self, process: subprocess.Popen) -> None:
        self._process = process

    @classmethod
    def open(cls, name: str, argv: list[str]) -> "GatewayStream":
        args = [osh.CLI, "sandbox", "exec", "-n", name, "--no-tty", "--no-login-shell", "--", *argv]
        process = subprocess.Popen(  # noqa: S603 - fixed argument vector
            args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, close_fds=True
        )
        return cls(process)

    def read(self) -> bytes:
        """The next bytes; raises `EOFError` when the program ended."""
        assert self._process.stdout is not None
        data = os.read(self._process.stdout.fileno(), 65536)
        if not data:
            raise EOFError
        return data

    def write(self, data: bytes) -> None:
        assert self._process.stdin is not None
        self._process.stdin.write(data)
        self._process.stdin.flush()

    def close(self) -> None:
        try:
            self._process.kill()
        except ProcessLookupError:
            pass
        for pipe in (self._process.stdin, self._process.stdout):
            try:
                if pipe:
                    pipe.close()
            except OSError:
                pass
