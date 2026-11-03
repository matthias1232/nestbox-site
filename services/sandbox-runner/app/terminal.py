"""The PTY side of the browser terminal.

`GET /v1/sandboxes/{name}/terminal` upgrades to a WebSocket and attaches a real
pseudo-terminal to the container: `bash` with a TTY, so `vim`, `htop`, and every
full-screen tool work, because they need a terminal and not a pipe.

The wire protocol is fixed, and the control API and the console are built
against it:

* a text frame from the client is keyboard input;
* one text frame from the client is a control frame — `{"type":"resize",
  "cols":80,"rows":24}` or `{"type":"close"}`;
* a text frame from the server is terminal output;
* before the server closes, it sends one control frame naming the reason:
  `{"type":"close","reason":"idle" | "user" | "sandbox_stopped" | "error"}`.

Three rules shape this file:

1. **No content is recorded.** The bytes cross the socket and are gone; the
   audit row lives in the control API and holds who, when, how long, and how
   many bytes (ADR 0023). A log line here with a keystroke in it would undo that.
2. **The PTY is closed on every path.** A `docker exec` with a TTY keeps the
   process and the socket alive until the socket is closed, so every exit —
   idle, client gone, container gone, an exception — goes through the same
   cleanup.
3. **Authentication is the node token.** The runner also sits on the sandbox
   network, so a container could reach this endpoint; without a token any
   sandbox could open a shell in another. No configured token refuses every
   connection rather than allowing them, because a missing secret must never
   mean "no check".
"""
from __future__ import annotations
from app.gateway_exec import GatewayPty
from app.workload import get_container, list_workloads, openshell_name, sandbox_name

import asyncio
import logging
import codecs
import itertools
import json
import os
import threading
import time

logger = logging.getLogger("nestbox.runner.terminal")
from typing import Any

from fastapi import APIRouter, WebSocket, WebSocketDisconnect

router = APIRouter()

CLOSE_UNAUTHORIZED = 1008
CLOSE_INTERNAL = 1011
CLOSE_NORMAL = 1000

# Mirrors `main.container_name`. Importing the module would be a cycle, and the
# prefix is part of the runner's contract with the control plane either way.
CONTAINER_PREFIX = "nestbox-sandbox-"

# The shell a terminal opens. An argument vector, never a shell string: nothing
# from the request reaches this list.
SHELL = ("bash",)
TERMINAL_ENV = {"TERM": "xterm-256color", "LANG": "C.UTF-8"}

DEFAULT_IDLE_SECONDS = 1800.0  # thirty minutes, ADR 0023
MIN_IDLE_SECONDS = 5.0
MAX_IDLE_SECONDS = 7200.0
DEFAULT_COLS = 80
DEFAULT_ROWS = 24
MIN_DIMENSION = 1
MAX_DIMENSION = 500
READ_CHUNK = 8192
# A keystroke frame is small. A megabyte "keystroke" is a client bug or an
# attempt to make the pump hold memory, and neither deserves a PTY.
MAX_INPUT_BYTES = 65_536
# Bounded, so a browser that stops reading cannot make the runner buffer a
# session's output in memory. When it is full, the PTY reader waits.
EVENT_QUEUE = 64

SUBPROTOCOL = "nestbox-terminal"

# Who is attached to which tab, per sandbox: {sandbox: {tab: {viewer_id: {...}}}}.
# In memory on purpose: it describes sockets this process holds right now, and a
# restart of the runner ends them anyway. Nothing is stored.
_PRESENCE: dict[str, dict[str, dict[str, dict[str, Any]]]] = {}
_PRESENCE_LOCK = threading.Lock()
_VIEWER_SEQ = itertools.count(1)


def presence_add(sandbox: str, tab: str, who: str) -> int:
    """Record one attached viewer and return its id (to remove it later)."""
    viewer = next(_VIEWER_SEQ)
    entry = {"who": who or "someone", "since": time.time()}
    with _PRESENCE_LOCK:
        _PRESENCE.setdefault(sandbox, {}).setdefault(tab, {})[str(viewer)] = entry
    return viewer


def presence_remove(sandbox: str, tab: str, viewer: int) -> None:
    with _PRESENCE_LOCK:
        tabs = _PRESENCE.get(sandbox)
        if not tabs:
            return
        viewers = tabs.get(tab)
        if viewers:
            viewers.pop(str(viewer), None)
            if not viewers:
                tabs.pop(tab, None)
        if not tabs:
            _PRESENCE.pop(sandbox, None)


def presence_of(sandbox: str) -> dict[str, Any]:
    """The tabs of one sandbox and who is in them, as the console draws it."""
    with _PRESENCE_LOCK:
        tabs = {tab: list(viewers.values()) for tab, viewers in _PRESENCE.get(sandbox, {}).items()}
    return {
        "sandbox": sandbox,
        "items": [
            {
                "tab": tab,
                "viewers": [
                    {
                        "who": viewer["who"],
                        "since": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(viewer["since"])),
                        "seconds": int(time.time() - viewer["since"]),
                    }
                    for viewer in viewers
                ],
            }
            for tab, viewers in tabs.items()
        ],
    }


def container_name(name: str) -> str:
    """The container a sandbox name belongs to."""
    return f"{CONTAINER_PREFIX}{name}"


def node_token() -> str:
    """The token every upgrade must carry, from a file when one is configured.

    The file wins over the variable because an environment variable is visible
    in `docker inspect` and in a crash dump, and this token is the only thing
    between a compromised sandbox and a shell in its neighbour (ADR 0005).
    """
    path = os.getenv("NESTBOX_NODE_TOKEN_FILE", "")
    if path:
        try:
            with open(path, encoding="utf-8") as handle:
                return handle.read().strip()
        except OSError:
            # A configured file that cannot be read is not a licence to skip
            # the check; it is the same as no token, which refuses everything.
            return ""
    return os.getenv("NESTBOX_NODE_TOKEN", "").strip()


def authorised(websocket: WebSocket) -> bool:
    """Whether the handshake carries the node token.

    Both forms are accepted because a browser cannot set headers on a WebSocket
    and the control API is not a browser: the API offers
    `Sec-WebSocket-Protocol: nestbox-terminal, <token>` and the token travels in
    a header rather than in a URL that a proxy would log.
    """
    expected = node_token()
    if not expected:
        return False
    offered = websocket.query_params.get("token")
    if not offered:
        for protocol in websocket.headers.get("sec-websocket-protocol", "").split(","):
            candidate = protocol.strip()
            if candidate and candidate != SUBPROTOCOL:
                offered = candidate
                break
    return offered == expected


def idle_seconds() -> float:
    """How long a terminal may go without input before it is closed.

    Configurable so a test can watch the idle path in seconds instead of
    waiting half an hour, and clamped so a typo cannot disable the timeout or
    close a terminal before anyone can type.
    """
    raw = os.getenv("NESTBOX_TERMINAL_IDLE_SECONDS", "")
    try:
        value = float(raw)
    except ValueError:
        return DEFAULT_IDLE_SECONDS
    return min(max(value, MIN_IDLE_SECONDS), MAX_IDLE_SECONDS)


def control_frame(text: str) -> dict[str, Any] | None:
    """The control frame in a text message, or `None` for terminal input.

    Only a text frame that is a JSON object with a string `type` can be a
    control frame. The cheap `{` guard keeps ordinary typing out of the JSON
    parser, and a shell line that happens to be valid JSON without a `type` is
    input, not a command to the pump.
    """
    if not text.startswith("{"):
        return None
    try:
        value = json.loads(text)
    except ValueError:
        return None
    if isinstance(value, dict) and isinstance(value.get("type"), str):
        return value
    return None


def _dimension(value: Any, default: int) -> int:
    """One terminal dimension, clamped to what a PTY accepts."""
    try:
        number = int(value)
    except (TypeError, ValueError):
        return default
    return min(max(number, MIN_DIMENSION), MAX_DIMENSION)


def resize_dimensions(frame: dict[str, Any]) -> tuple[int, int]:
    """The (cols, rows) a resize frame asks for, clamped."""
    cols = _dimension(frame.get("cols"), DEFAULT_COLS)
    rows = _dimension(frame.get("rows"), DEFAULT_ROWS)
    return cols, rows


def close_frame(reason: str, message: str | None = None) -> str:
    """The control frame that tells the browser why the terminal ended."""
    payload: dict[str, Any] = {"type": "close", "reason": reason}
    if message:
        payload["message"] = message
    return json.dumps(payload)


class PtySocket:
    """The byte stream of a `docker exec` TTY.

    `exec_start(socket=True)` hands back a `socket.SocketIO` whose underlying
    socket carries the raw stream (no demultiplexing header, because the exec
    has a TTY). This wrapper hides that detail, and covers the pipe case where
    the object has `read`/`write` instead of `recv`/`sendall`, so the pump is
    written against bytes and not against docker-py internals.
    """

    def __init__(self, raw: Any) -> None:
        self._raw = raw
        self._socket = getattr(raw, "_sock", raw)
        # docker-py's client carries a 60 s timeout into the hijacked socket.
        # Left in place, `recv` raises after one minute of silence and the pump
        # reads that as the shell exiting, so an idle terminal would die after a
        # minute no matter what the idle policy says. Blocking is what a PTY
        # needs: there is nothing to time out, the session ends when a side ends
        # it.
        clearer = getattr(self._socket, "settimeout", None)
        if clearer is not None:
            clearer(None)

    def read(self, size: int = READ_CHUNK) -> bytes:
        """Block until bytes arrive; empty bytes mean the stream ended."""
        reader = getattr(self._socket, "recv", None)
        if reader is not None:
            return reader(size)
        return self._raw.read(size) or b""

    def write(self, data: bytes) -> None:
        """Send bytes to the PTY."""
        sender = getattr(self._socket, "sendall", None)
        if sender is not None:
            sender(data)
        else:
            self._raw.write(data)
            self._raw.flush()

    def close(self) -> None:
        """Release the socket. Closing it is also what unblocks a pending read."""
        for target in (self._socket, self._raw):
            closer = getattr(target, "close", None)
            if closer is None:
                continue
            try:
                closer()
            except OSError:
                return


async def finish(websocket: WebSocket, reason: str, message: str | None = None) -> None:
    """Say why the session ended, then close it.

    A browser that already left is not an error. Starlette raises
    `WebSocketDisconnect` when a frame is written to a socket the client closed,
    and a traceback for a session that ended correctly is noise that hides the
    ones that did not — so this is the only place that writes the last frame,
    and it swallows exactly that case.
    """
    try:
        await websocket.send_text(close_frame(reason, message))
    except (WebSocketDisconnect, RuntimeError, OSError):
        return
    try:
        await websocket.close(code=CLOSE_NORMAL, reason=reason)
    except (WebSocketDisconnect, RuntimeError, OSError):
        pass


async def pump(
    websocket: WebSocket,
    pty: PtySocket,
    resize: Any,
    idle_timeout: float,
) -> str:
    """Move bytes between the browser and the PTY until one side ends it.

    Returns the close reason, which the caller puts in the close frame and the
    control API puts in the audit row. `resize` is a callable rather than a
    docker client so the pump is testable without a container.

    There is exactly **one** reader for the PTY, and it lives as long as the
    session. A reader per loop iteration looks simpler and is wrong: cancelling
    an `asyncio.to_thread` does not stop the thread, so a second reader would
    race the abandoned one for the next chunk and silently drop it.

    The idle clock resets on *input* only. That is what "thirty minutes without
    input" means in ADR 0023: an `htop` that repaints forever is still an
    unattended session, and a session nobody watches is a shell nobody closes.
    """
    loop = asyncio.get_running_loop()
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    events: asyncio.Queue[tuple[str, Any]] = asyncio.Queue(maxsize=EVENT_QUEUE)
    deadline = loop.time() + idle_timeout

    async def read_pty() -> None:
        """Everything the shell prints, until the stream ends."""
        while True:
            try:
                chunk = await asyncio.to_thread(pty.read, READ_CHUNK)
            except TimeoutError:
                # A socket that brought its own timeout is not a closed shell.
                # `PtySocket` clears one, so this is a second line rather than
                # the mechanism.
                continue
            except (OSError, ValueError):
                chunk = b""
            if not chunk:
                await events.put(("closed", "sandbox_stopped"))
                return
            await events.put(("output", chunk))

    async def read_browser() -> None:
        """Keyboard and control frames, one message at a time, in order."""
        while True:
            try:
                message = await websocket.receive()
            except (RuntimeError, OSError):
                await events.put(("closed", "user"))
                return
            if message.get("type") == "websocket.disconnect":
                await events.put(("closed", "user"))
                return
            if message.get("type") != "websocket.receive":
                continue

            raw_text = message.get("text")
            if raw_text is None:
                data = message.get("bytes") or b""
                if data:
                    await asyncio.to_thread(pty.write, data)
                    await events.put(("input", 0))
                continue

            if len(raw_text) > MAX_INPUT_BYTES:
                await events.put(("closed", "error"))
                return

            frame = control_frame(raw_text)
            if frame is not None:
                if frame["type"] == "resize":
                    cols, rows = resize_dimensions(frame)
                    await asyncio.to_thread(resize, cols, rows)
                    continue
                if frame["type"] == "close":
                    await events.put(("closed", "user"))
                    return
                # An unknown control frame is ignored rather than typed into
                # the shell: a future frame type must not corrupt a session.
                continue

            await asyncio.to_thread(pty.write, raw_text.encode("utf-8"))
            await events.put(("input", len(raw_text)))

    workers = [asyncio.create_task(read_pty()), asyncio.create_task(read_browser())]
    reason = "user"
    try:
        while True:
            remaining = deadline - loop.time()
            if remaining <= 0:
                reason = "idle"
                break
            try:
                event, payload = await asyncio.wait_for(events.get(), timeout=remaining)
            except asyncio.TimeoutError:
                reason = "idle"
                break

            if event == "closed":
                reason = payload
                break
            if event == "input":
                deadline = loop.time() + idle_timeout
                continue

            text = decoder.decode(payload)
            if text:
                await websocket.send_text(text)
    finally:
        for worker in workers:
            worker.cancel()
        # The reader thread stays blocked in the socket until the PTY closes,
        # which the caller does on its way out; cancelling the task alone would
        # leak it.

    trailing = decoder.decode(b"", final=True)
    if trailing:
        await websocket.send_text(trailing)
    return reason


def opened_exec(container: Any, tab: str) -> tuple[str, PtySocket]:
    """Attach to a durable tmux tab or create it the first time.

    The browser is only a viewer. Disconnecting its WebSocket detaches from
    tmux, while the shell keeps running until the user explicitly closes its
    tab. `tab` has already been constrained by the control API and is checked
    again here because the runner is a separate security boundary.
    """
    client = container.client
    # Existing sandboxes keep working while their image is awaiting a controlled
    # recreate. New images carry tmux and gain durable tabs; an old image gets a
    # normal shell with an honest, non-persistent fallback rather than a broken
    # terminal after this control-plane deploy.
    has_tmux, _ = container.exec_run(["sh", "-c", "command -v tmux >/dev/null"], stdout=False, stderr=False)
    command = ["tmux", "new-session", "-A", "-s", f"nestbox-{tab}"] if has_tmux == 0 else list(SHELL)
    created = client.api.exec_create(
        container.id,
        command,
        tty=True,
        stdin=True,
        stdout=True,
        stderr=True,
        environment=TERMINAL_ENV,
    )
    exec_id = created["Id"]
    raw = client.api.exec_start(exec_id, tty=True, socket=True)
    return exec_id, PtySocket(raw)


@router.get("/v1/sandboxes/{name}/terminal/sessions")
def terminal_sessions(name: str) -> dict[str, Any]:
    """Which terminal tabs of this sandbox are open, and who is in each."""
    return presence_of(name)


@router.websocket("/v1/sandboxes/{name}/terminal")
async def terminal(websocket: WebSocket, name: str) -> None:
    """Attach a PTY to the container and pump it.

    The refusal for a missing token happens before `accept`, so an unauthorised
    client gets an HTTP 403 on the handshake instead of a WebSocket it must
    parse to learn it was refused.
    """
    logger.info("ws terminal: enter %s", name)
    if not authorised(websocket):
        logger.info("ws terminal: refused (no token) %s", name)
        await websocket.close(code=CLOSE_UNAUTHORIZED, reason="node token required")
        return
    logger.info("ws terminal: authorised %s", name)

    import docker
    from docker.errors import DockerException, NotFound

    client = docker.from_env()
    try:
        container = await asyncio.to_thread(get_container, client, name)
    except NotFound:
        await websocket.close(code=CLOSE_INTERNAL, reason="sandbox not found")
        return
    except DockerException as exc:
        await websocket.close(code=CLOSE_INTERNAL, reason=f"docker unavailable: {exc}")
        return

    if container.status != "running":
        await websocket.close(code=CLOSE_INTERNAL, reason="sandbox is not running")
        return

    logger.info("ws terminal: accepting %s", name)
    await websocket.accept(subprotocol=SUBPROTOCOL)
    logger.info("ws terminal: accepted %s", name)

    tab = websocket.query_params.get("tab", "")
    if not tab or len(tab) > 64 or not all(ch.isascii() and (ch.isalnum() or ch in "_-") for ch in tab):
        await websocket.close(code=CLOSE_INTERNAL, reason="invalid terminal tab")
        return
    viewer = presence_add(name, tab, websocket.query_params.get("who", "").strip()[:80])
    gateway = openshell_name(container.name)
    exec_id = None
    try:
        if gateway:
            # Through the gateway, so the shell has the proxy, the CA and the
            # boundary the agent has (see gateway_exec.py).
            pty = await asyncio.to_thread(GatewayPty.open, gateway, tab)
        else:
            exec_id, pty = await asyncio.to_thread(opened_exec, container, tab)
    except (DockerException, KeyError, OSError) as exc:
        presence_remove(name, tab, viewer)
        await finish(websocket, "error", f"could not open a shell: {exc}")
        return

    def resize(cols: int, rows: int) -> None:
        if gateway:
            pty.resize(cols, rows)
        else:
            client.api.exec_resize(exec_id, height=rows, width=cols)

    try:
        # A PTY starts at whatever the kernel defaults to; setting the size
        # explicitly means the first full-screen tool draws at 80x24 rather
        # than at 0x0 until the browser sends its first resize.
        await asyncio.to_thread(resize, DEFAULT_COLS, DEFAULT_ROWS)
        reason = await pump(websocket, pty, resize, idle_seconds())
    finally:
        pty.close()
        presence_remove(name, tab, viewer)

    await finish(websocket, reason)
