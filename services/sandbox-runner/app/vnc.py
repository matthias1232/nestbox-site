"""The desktop of one sandbox, as a WebSocket carrying the VNC byte stream.

A sandbox with the desktop pack runs a VNC server on `127.0.0.1:5900` *inside*
its own network namespace. Nothing publishes that port and nothing outside the
container can reach it. This module is the only way out, and it works the way
the terminal does:

* the control API connects here with the node token (the same one the terminal
  uses) and has already checked who the user is and whether they may use this
  sandbox;
* a fixed program is started in the container with `docker exec`; it connects
  to `127.0.0.1:5900` and copies bytes in both directions;
* the runner moves those bytes to and from the WebSocket. Screen content is
  never recorded here; the only bytes it looks at are the small client-to-server
  messages, and only for a view-only stream, where every input message is
  dropped before it reaches the desktop.

The in-container program is Python because the desktop image already has it.
It is a constant, never built from the request.
"""
from __future__ import annotations

import asyncio
import logging
import struct
import uuid
from typing import Any

from fastapi import APIRouter, WebSocket, WebSocketDisconnect

from app.gateway_exec import GatewayStream
from app.terminal import CLOSE_INTERNAL, CLOSE_UNAUTHORIZED, get_container, node_token
from app.workload import openshell_name

router = APIRouter()
logger = logging.getLogger("nestbox.runner.vnc")

SUBPROTOCOL = "nestbox-vnc"
CLOSE_NORMAL = 1000
READ_CHUNK = 65_536
VNC_PORT = 5900

BRIDGE = (
    "import os,socket,threading\n"
    f"s=socket.create_connection(('127.0.0.1',{VNC_PORT}),timeout=5)\n"
    "s.settimeout(None)\n"
    "def up():\n"
    "    while True:\n"
    "        d=os.read(0,65536)\n"
    "        if not d: break\n"
    "        s.sendall(d)\n"
    "    s.shutdown(socket.SHUT_WR)\n"
    "threading.Thread(target=up,daemon=True).start()\n"
    "while True:\n"
    "    d=s.recv(65536)\n"
    "    if not d: break\n"
    "    os.write(1,d)\n"
)


def bridge_command(tag: str) -> list[str]:
    """The argv that starts one bridge inside the sandbox.

    The tag rides on the command line (Python leaves it in `sys.argv`), so the
    runner can end exactly this bridge when the viewer goes away. Without the
    tag a closed viewer left its bridge behind: after about seven connections
    the sandbox refused further upgrades (`node.runtime_unavailable`).
    """
    return ["python3", "-c", BRIDGE, f"nestbox-vnc-bridge-{tag}"]


def bridge_kill_command(tag: str) -> list[str]:
    """The argv that ends one bridge by its tag, and only that one."""
    return ["pkill", "-f", f"nestbox-vnc-bridge-{tag}"]


def authorised(websocket: WebSocket) -> bool:
    """Whether the handshake carries the node token (see the terminal)."""
    expected = node_token()
    if not expected:
        return False
    for protocol in websocket.headers.get("sec-websocket-protocol", "").split(","):
        candidate = protocol.strip()
        if candidate and candidate != SUBPROTOCOL and candidate == expected:
            return True
    return False


class RfbClientFilter:
    """Forward only the messages a viewer needs; drop input when view-only.

    Client handshake: 12 bytes version, 1 byte security type, 1 byte ClientInit = 14 bytes.
    Then messages: 0 SetPixelFormat (20), 2 SetEncodings (4 + 4n), 3 FramebufferUpdateRequest (10),
    4 KeyEvent (8), 5 PointerEvent (6), 6 ClientCutText (8 + |len|), 150 EnableContinuousUpdates (10),
    248 ClientFence (9 + len), 251 SetDesktopSize (8 + 16n).
    """

    DROP_WHEN_VIEW_ONLY = {4, 5, 6, 251}

    def __init__(self, view_only: bool) -> None:
        self.view_only = view_only
        self.handshake_left = 14
        self.buffer = b""
        self.broken = False

    def _length(self, data: bytes) -> int | None:
        kind = data[0]
        fixed = {0: 20, 3: 10, 4: 8, 5: 6, 150: 10}
        if kind in fixed:
            return fixed[kind]
        if kind == 2:
            return None if len(data) < 4 else 4 + 4 * int.from_bytes(data[2:4], "big")
        if kind == 6:
            return None if len(data) < 8 else 8 + abs(int.from_bytes(data[4:8], "big", signed=True))
        if kind == 248:
            return None if len(data) < 9 else 9 + data[8]
        if kind == 251:
            return None if len(data) < 8 else 8 + 16 * data[6]
        self.broken = True
        return None

    def feed(self, data: bytes) -> bytes:
        """Bytes in from the browser, bytes out to the desktop."""
        if not self.view_only:
            return data
        out = b""
        if self.handshake_left:
            take = data[: self.handshake_left]
            self.handshake_left -= len(take)
            out += take
            data = data[len(take):]
        self.buffer += data
        while self.buffer and not self.broken:
            length = self._length(self.buffer)
            if length is None or len(self.buffer) < length:
                break
            message, self.buffer = self.buffer[:length], self.buffer[length:]
            if message[0] not in self.DROP_WHEN_VIEW_ONLY:
                out += message
        return out


class ExecStream:
    """The raw, hijacked socket of a non-TTY exec, demultiplexed.

    Without a TTY Docker frames the output as `[stream, 0, 0, 0, size:u32]` plus
    payload. Only stdout carries the VNC stream; stderr is logged and dropped.
    """

    def __init__(self, raw: Any) -> None:
        self._socket = getattr(raw, "_sock", raw)
        settimeout = getattr(self._socket, "settimeout", None)
        if settimeout is not None:
            settimeout(None)
        self._buffer = b""

    def _take(self, size: int) -> bytes:
        while len(self._buffer) < size:
            chunk = self._socket.recv(READ_CHUNK)
            if not chunk:
                raise EOFError
            self._buffer += chunk
        out, self._buffer = self._buffer[:size], self._buffer[size:]
        return out

    def read(self) -> bytes:
        """The next stdout payload; raises `EOFError` when the exec ends."""
        while True:
            header = self._take(8)
            stream, size = header[0], struct.unpack(">I", header[4:])[0]
            payload = self._take(size) if size else b""
            if stream == 1:
                return payload
            if stream == 2 and payload:
                logger.warning("vnc bridge: %s", payload.decode(errors="replace").strip()[:200])

    def write(self, data: bytes) -> None:
        self._socket.sendall(data)

    def close(self) -> None:
        try:
            self._socket.close()
        except OSError:
            pass


def open_bridge(container: Any, tag: str) -> Any:
    gateway = openshell_name(container.name)
    if gateway:
        # The desktop's loopback port lives inside the sandbox's own boundary, so
        # the bridge is started through the gateway, not beside it.
        return GatewayStream.open(gateway, bridge_command(tag))
    client = container.client
    created = client.api.exec_create(
        container.id, bridge_command(tag), tty=False, stdin=True, stdout=True, stderr=True
    )
    return ExecStream(client.api.exec_start(created["Id"], tty=False, socket=True))


def close_bridge(container: Any, tag: str) -> None:
    """End the bridge of one viewer, whatever state the socket is in.

    Closing the exec socket does not end the process inside the sandbox, so the
    tag from `bridge_command` is killed explicitly. A failure here is logged and
    ignored: the viewer is already gone, and a leftover is better than an
    exception in the teardown path.
    """
    try:
        gateway = openshell_name(container.name)
        if gateway:
            GatewayStream.open(gateway, bridge_kill_command(tag)).close()
            return
        client = container.client
        created = client.api.exec_create(container.id, bridge_kill_command(tag), tty=False)
        client.api.exec_start(created["Id"])
    except Exception as exc:  # noqa: BLE001 - teardown must never raise
        logger.warning("vnc: could not end the bridge %s: %s", tag, exc)


@router.websocket("/v1/sandboxes/{name}/vnc")
async def vnc(websocket: WebSocket, name: str) -> None:
    """Carry the sandbox's VNC stream to the control API."""
    if not authorised(websocket):
        await websocket.close(code=CLOSE_UNAUTHORIZED, reason="node token required")
        return

    # `view=1` means the control API decided this caller may watch but not
    # drive. The decision is repeated here, so a bug or a stale console cannot
    # turn a view-only ticket into a keyboard.
    view_only = websocket.query_params.get("view") == "1"
    flt = RfbClientFilter(view_only)

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

    logger.info("ws vnc: accepting %s", name)
    await websocket.accept(subprotocol=SUBPROTOCOL)
    logger.info("ws vnc: accepted %s", name)
    tag = uuid.uuid4().hex[:12]
    try:
        stream = await asyncio.to_thread(open_bridge, container, tag)
    except DockerException as exc:
        await websocket.close(code=CLOSE_INTERNAL, reason=f"could not open the desktop: {exc}")
        return

    async def to_browser() -> None:
        while True:
            try:
                data = await asyncio.to_thread(stream.read)
            except (EOFError, OSError):
                return
            await websocket.send_bytes(data)

    async def to_desktop() -> None:
        while True:
            message = await websocket.receive()
            if message.get("type") == "websocket.disconnect":
                return
            data = message.get("bytes")
            if data:
                forwarded = flt.feed(data)
                if flt.broken:
                    # An unknown client message in view-only mode: the safest
                    # answer is to end the stream, not to guess what it meant.
                    logger.warning("vnc: unknown client message in view-only mode; closing")
                    await websocket.close(code=CLOSE_NORMAL, reason="unknown client message")
                    return
                if forwarded:
                    await asyncio.to_thread(stream.write, forwarded)

    tasks = [asyncio.create_task(to_browser()), asyncio.create_task(to_desktop())]
    try:
        await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
    except WebSocketDisconnect:
        pass
    finally:
        for task in tasks:
            task.cancel()
        stream.close()
        await asyncio.to_thread(close_bridge, container, tag)
        try:
            await websocket.close(code=CLOSE_NORMAL)
        except RuntimeError:
            pass
