"""The SSH door into a sandbox.

Sandbox images carry no `sshd`, and giving every image one would mean a daemon,
host keys, capabilities and a user database inside each sandbox. Instead the
runner *is* the SSH server for the port a sandbox was given: a key-authenticated
`ssh -p <port> nestbox@<node>` lands in `docker exec` of the same sandbox, with a
real PTY when the client asks for one. That covers an interactive shell,
`ssh host <command>` (so `ssh host claude` works), and agent CLIs that only need a
terminal.

What it is **not**: an SFTP or port-forwarding server. There is no SFTP
subsystem, and `-L`, `-R` and `-D` are refused, so this door cannot be used to
tunnel traffic around the egress policy. VS Code Remote-SSH needs both and does
not work here; the console says so rather than hiding it. Legacy `scp -O` still
works when the image has `scp`, because it is only a command run inside the
sandbox (`scp -t`), the same as typing it in a shell.

Authentication is public-key only. The authorised keys are the sandbox's own
public key (the pair the control plane generated), delivered with the forward
set; the private half never reaches the runner.

Where a login *lands* is tmux, so that SSH and the browser terminal are two
windows on the same work: a plain login joins the shared session `nestbox-ssh`,
`ssh -p <port> nestbox@<node> attach <tab>` joins the browser tab `<tab>`, and a
dropped connection only detaches from tmux instead of ending the shell.
"""
from __future__ import annotations
from app.workload import get_container, list_workloads, sandbox_name

import asyncio
import hashlib
import hmac
import logging
import os
import re
import struct
import time
from pathlib import Path
from typing import Any

logger = logging.getLogger("nestbox.runner.ssh")

CONTAINER_PREFIX = "nestbox-sandbox-"
READ_CHUNK = 65_536
SSH_USER = "nestbox"
TERM_ENV = {"LANG": "C.UTF-8"}
# A tab name is also a tmux session name, so it is the same shape the browser
# terminal is held to: short, ASCII, no path and no shell metacharacter.
TAB_RE = re.compile(r"^[A-Za-z0-9_-]{1,64}$")


def demux(buffer: bytes) -> tuple[list[tuple[int, bytes]], bytes]:
    """Split Docker's multiplexed exec stream into `(stream, payload)` frames.

    Without a TTY, Docker prefixes every chunk with an 8-byte header: one byte
    for the stream (1 stdout, 2 stderr), three zero bytes, and a big-endian
    length. A chunk can arrive split anywhere, so what is left over is returned
    and joined to the next read.
    """
    frames: list[tuple[int, bytes]] = []
    offset = 0
    while len(buffer) - offset >= 8:
        stream = buffer[offset]
        (length,) = struct.unpack(">I", buffer[offset + 4 : offset + 8])
        end = offset + 8 + length
        if len(buffer) < end:
            break
        frames.append((stream, buffer[offset + 8 : end]))
        offset = end
    return frames, buffer[offset:]


def password_matches(credentials: list[dict[str, Any]], username: str, password: str, now: float | None = None) -> bool:
    """Whether `username`/`password` is a temporary login that is still good.

    The control plane stores `sha256(salt || password)` and sends only that; the
    password is never here. The expiry is checked on every attempt against this
    machine's clock, so a login ends on time even if the control plane is gone.
    Every entry is compared, so the time taken does not say which user names exist.
    """
    moment = time.time() if now is None else now
    ok = False
    for entry in credentials:
        digest = hashlib.sha256(f"{entry.get('salt', '')}{password}".encode()).hexdigest()
        same = hmac.compare_digest(digest, str(entry.get("hash", "")))
        if same and entry.get("username") == username and float(entry.get("expires_at", 0)) > moment:
            ok = True
    return ok


def tmux_tab(command: str | None) -> str | None:
    """The tmux tab an SSH login asks for: none = the shared 'ssh' tab; 'attach X' / 'tab X' = X."""
    if not command or not command.strip():
        return "ssh"
    parts = command.split()
    if len(parts) == 2 and parts[0] in ("attach", "tab") and TAB_RE.match(parts[1]):
        return parts[1]
    return None


def shell_command(command: str | None, has_tmux: bool = False) -> list[str]:
    """The argument vector for one SSH session; never a spliced shell string.

    With tmux in the image a login is a *tab* rather than a shell: `-A` attaches
    to the tab when it exists and creates it otherwise, so the browser terminal
    and a second SSH client share one session. Without tmux (an older image)
    the login is an ordinary shell, and a requested command always stays a
    command: only the bare login and `attach`/`tab` are turned into a tab.
    """
    tab = tmux_tab(command) if has_tmux else None
    if tab:
        return ["tmux", "new-session", "-A", "-s", f"nestbox-{tab}"]
    if command:
        return ["bash", "-lc", command]
    return ["bash", "-l"]


def host_key_path(state_dir: str) -> Path:
    return Path(state_dir) / "ssh_host_ed25519_key"


def ensure_host_key(state_dir: str) -> Path:
    """Create the runner's SSH host key once and keep it across restarts.

    A new key at every start would make every client warn about a changed host
    key, which teaches people to ignore that warning.
    """
    import asyncssh

    path = host_key_path(state_dir)
    if path.exists():
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    key = asyncssh.generate_private_key("ssh-ed25519", comment="nestbox-runner")
    key.write_private_key(str(path))
    os.chmod(path, 0o600)
    return path


class _Exec:
    """One shell of a sandbox, as a byte stream.

    A Docker sandbox is a `docker exec`. A gateway (OpenShell) sandbox goes
    through `openshell sandbox exec --tty`, so the shell has the proxy, the CA
    and the boundary the agent has (see `gateway_exec.py`); a `docker exec`
    there would run outside that boundary and be a different machine.
    """

    def __init__(self, client: Any, sandbox: str, command: str | None, tty: bool, size: tuple[int, int]):
        from app.terminal import PtySocket
        from app.workload import openshell_name

        container = get_container(client, sandbox)
        if container.status != "running":
            raise RuntimeError(f"sandbox '{sandbox}' is {container.status}, not running")
        self._client = client
        self.bytes_in = 0
        self.bytes_out = 0
        self.tty = tty
        # tmux is what makes an SSH login land in a tab the browser terminal
        # shares, so it is only worth a container round-trip for a terminal.
        has_tmux = False
        if tty:
            code, _ = container.exec_run(["sh", "-c", "command -v tmux >/dev/null"], stdout=False, stderr=False)
            has_tmux = code == 0
        self.tab = tmux_tab(command) if has_tmux else None
        argv = shell_command(command, has_tmux)
        self.gateway = openshell_name(container.name) or ""
        if self.gateway:
            from app.gateway_exec import GatewayPty

            self._gateway: Any = GatewayPty.open_command(self.gateway, argv)
            self.exec_id = ""
            self.sock: Any = self._gateway
        else:
            self._gateway = None
            created = client.api.exec_create(
                container.id,
                argv,
                tty=tty,
                stdin=True,
                stdout=True,
                stderr=True,
                environment={**TERM_ENV, "TERM": "xterm-256color"} if tty else TERM_ENV,
            )
            self.exec_id = created["Id"]
            self.sock = PtySocket(client.api.exec_start(self.exec_id, tty=tty, socket=True))
        if tty:
            self.resize(*size)

    def resize(self, cols: int, rows: int) -> None:
        if self.gateway:
            self._gateway.resize(cols, rows)
            return
        self._client.api.exec_resize(self.exec_id, height=max(rows, 1), width=max(cols, 1))

    def end_input(self) -> None:
        """Tell the command that the client sent its last byte.

        A pipe ends with a half-close. A PTY has no half-close, so the byte a
        terminal means by "end of input" is Ctrl-D (a gateway sandbox pipes a
        non-terminal command through a PTY, so it needs that form too).
        """
        if self.tty:
            return  # a terminal session is ended by `_pump`, not by an end-of-file
        if self.gateway:
            self.sock.write(b"\x04")
            return
        try:
            self.sock._socket.shutdown(1)  # SHUT_WR
        except OSError:
            pass

    def exit_code(self) -> int:
        # The gateway CLI is a local process talking to a PTY and reports no exit
        # status for what ran inside; a detached terminal ends cleanly.
        if self.gateway:
            return 0
        info = self._client.api.exec_inspect(self.exec_id)
        code = info.get("ExitCode")
        return int(code) if code is not None else 0

    def close(self) -> None:
        self.sock.close()


async def _pump(process: Any, run: _Exec) -> None:
    """Move bytes both ways until the command or the client ends."""
    import asyncssh

    async def to_client() -> None:
        pending = b""
        while True:
            chunk = await asyncio.to_thread(run.sock.read, READ_CHUNK)
            if not chunk:
                return
            run.bytes_out += len(chunk)
            # A PTY carries one bare stream, and so does the gateway CLI (stdout
            # and stderr arrive together); only a plain `docker exec` without a
            # TTY frames its two streams with Docker's 8-byte headers.
            if run.tty or run.gateway:
                process.stdout.write(chunk)
                continue
            pending += chunk
            frames, pending = demux(pending)
            for stream, payload in frames:
                (process.stderr if stream == 2 else process.stdout).write(payload)

    async def to_sandbox() -> None:
        while True:
            try:
                data = await process.stdin.read(READ_CHUNK)
            except asyncssh.TerminalSizeChanged as change:
                await asyncio.to_thread(run.resize, change.width, change.height)
                continue
            except (asyncssh.BreakReceived, asyncssh.SignalReceived):
                continue
            if not data:
                # The client closed its input: end the command's stdin so it can
                # finish. A terminal session is ended by the wait below.
                await asyncio.to_thread(run.end_input)
                return
            run.bytes_in += len(data)
            await asyncio.to_thread(run.sock.write, data)

    reader = asyncio.create_task(to_client())
    writer = asyncio.create_task(to_sandbox())
    try:
        if run.tty:
            # A terminal ends when either side does. If the client vanished first (a dropped
            # network, a closed PuTTY window) nothing would ever end the shell, `reader` would
            # wait on it for ever, and the session and its `bash` would leak. Ctrl-D twice ends
            # an idle shell; closing the socket releases the reader either way.
            await asyncio.wait({reader, writer}, return_when=asyncio.FIRST_COMPLETED)
            if not reader.done():
                if run.tab is None:
                    # No tmux, so the shell is the session: a dropped client must not
                    # leave an orphan `bash` behind. With a tab, closing the door is
                    # the whole point — the session keeps running for the next login.
                    try:
                        await asyncio.to_thread(run.sock.write, b"\x04\x04")
                    except Exception:  # noqa: BLE001 - the shell may already be gone
                        pass
                run.close()
                reader.cancel()
        else:
            await reader
    finally:
        writer.cancel()
        reader.cancel()


def make_process_handler(client: Any, sandbox: str):
    async def handle(process: Any) -> None:
        peer = process.get_extra_info("peername")
        tty = process.get_terminal_type() is not None
        command = process.command
        logger.info("ssh session for %s from %s (tty=%s, command=%s)", sandbox, peer, tty, bool(command))
        cols, rows, _, _ = process.get_terminal_size() if tty else (80, 24, 0, 0)
        try:
            run = await asyncio.to_thread(_Exec, client, sandbox, command, tty, (cols or 80, rows or 24))
        except Exception as failure:  # noqa: BLE001 - every cause ends the session with a sentence
            process.stderr.write(f"nestbox: {failure}\r\n".encode())
            process.exit(1)
            return
        # Who, when, how long and how many bytes go to the control plane's audit
        # table; what was typed or printed never does. The tab says which window
        # of the sandbox the login joined.
        from app.telemetry_shipper import post_json

        opened = await asyncio.to_thread(
            post_json, "/api/v1/internal/ssh-sessions", {"event": "open", "sandbox": sandbox, "tab": run.tab}
        )
        session_id = (opened or {}).get("session_id")
        reason = "user"
        try:
            await _pump(process, run)
            process.exit(await asyncio.to_thread(run.exit_code))
        except Exception:  # noqa: BLE001
            logger.exception("ssh session for %s ended with an error", sandbox)
            reason = "error"
            process.exit(1)
        finally:
            run.close()
            logger.info("ssh session for %s closed", sandbox)
            if session_id:
                await asyncio.to_thread(
                    post_json,
                    "/api/v1/internal/ssh-sessions",
                    {"event": "close", "sandbox": sandbox, "session_id": session_id, "reason": reason,
                     "bytes_in": run.bytes_in, "bytes_out": run.bytes_out},
                )

    return handle


async def serve(
    *,
    client: Any,
    sandbox: str,
    bind: str,
    port: int,
    keys_for: Any,
    host_key: Path,
    credentials_for: Any = lambda _sandbox: [],
) -> Any:
    """Start an SSH listener on `port` that opens `sandbox`.

    `keys_for(sandbox)` returns the current authorised-keys text each time a
    client connects, so a rotated key applies to the next login without
    restarting the listener.
    """
    import asyncssh

    class Door(asyncssh.SSHServer):
        def connection_made(self, conn: Any) -> None:
            self._conn = conn

        def begin_auth(self, username: str) -> bool:
            # `True` means "authentication is required". Returning `False` would
            # accept the connection without it, so every path returns `True`
            # and a wrong user simply gets an empty key list.
            text = keys_for(sandbox) if username == SSH_USER else ""
            self._conn.set_authorized_keys(asyncssh.import_authorized_keys(text))
            return True

        def public_key_auth_supported(self) -> bool:
            return True

        def password_auth_supported(self) -> bool:
            # Only a still-valid temporary login passes `validate_password`; with
            # none issued every attempt fails, so offering it costs nothing.
            return True

        async def validate_password(self, username: str, password: str) -> bool:
            good = password_matches(credentials_for(sandbox), username, password)
            if not good:
                # Slows a guesser without making a typo painful.
                await asyncio.sleep(1.0)
            logger.info("password login for %s as %s: %s", sandbox, username, "accepted" if good else "refused")
            return good

        def kbdint_auth_supported(self) -> bool:
            return False

    return await asyncssh.listen(
        bind,
        port,
        server_factory=Door,
        server_host_keys=[str(host_key)],
        process_factory=make_process_handler(client, sandbox),
        encoding=None,
        agent_forwarding=False,
        x11_forwarding=False,
        allow_pty=True,
        # A dead client is noticed within a minute instead of never.
        keepalive_interval=20,
        keepalive_count_max=3,
        line_editor=False,
        # No sftp_factory: SFTP is not offered. `-L`/`-R`/`-D` are
        # refused because `Door` does not override `connection_requested` or
        # `server_requested`, whose default is to say no. This door opens a
        # shell, not a tunnel around the egress policy.
    )
