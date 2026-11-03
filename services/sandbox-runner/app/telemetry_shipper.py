"""Carry the egress proxy's access log to the control API.

Envoy writes one JSON line per request to its standard output. Nothing read that
output, so every decision the proxy made was invisible: the Traffic tab was empty
while sandboxes worked. This module is the missing reader. Every few seconds it
reads what the proxy printed since the last read and posts it as one batch to
`/api/v1/internal/telemetry/batch`, which redacts it and stores it.

The runner does this because it already holds the Docker socket and the node
token, and the proxy has neither a socket nor a reason to have one. A read that
fails is retried from the same point, so a control API restart loses nothing; a
line that is not a JSON access-log record (Envoy's own startup chatter) is
skipped.

Parsing is a pure function (`record_from_line`) so the mapping from Envoy's
field names to the control API's is tested without Docker or a network.
"""
from __future__ import annotations

import asyncio
import json
import logging
import re
import os
import time
import urllib.error
import urllib.request
import uuid
from typing import Any

logger = logging.getLogger("nestbox.runner.telemetry")

POLL_SECONDS = float(os.getenv("NESTBOX_TELEMETRY_POLL_SECONDS", "2"))
MAX_BATCH = 400
# The control API refuses a request body over 1 MiB (it closes the connection, which
# the shipper sees as a broken pipe). Bodies of chat requests are large, so a batch
# is cut by size as well as by count.
MAX_BATCH_BYTES = 800_000
SEEN_LIMIT = 2000

#: Envoy prints "-" for a field it has no value for.
EMPTY = {"", "-", "null", "None"}


def _text(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return None if text in EMPTY else text


def _int(value: Any) -> int | None:
    text = _text(value)
    if text is None:
        return None
    try:
        return int(float(text))
    except ValueError:
        return None


def _uuid(value: Any) -> str | None:
    text = _text(value)
    if text is None:
        return None
    try:
        return str(uuid.UUID(text))
    except ValueError:
        return None


def split_host(authority: str | None) -> tuple[str | None, int | None]:
    """`example.com:443` -> (`example.com`, 443); a bare host has no port."""
    text = _text(authority)
    if text is None:
        return None, None
    if text.startswith("["):  # [2001:db8::1]:443
        host, _, rest = text[1:].partition("]")
        return host, _int(rest.lstrip(":"))
    host, _, port = text.rpartition(":") if text.count(":") == 1 else (text, "", "")
    return (host or text), _int(port)


TIMESTAMP_INSIDE = re.compile(r"(?<=[A-Za-z0-9+/=])\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z ?")


def record_from_line(line: str) -> dict[str, Any] | None:
    """One access-log line as the control API's traffic entry, or `None`.

    Docker prefixes each line with a timestamp when asked for them; the JSON
    starts at the first `{`.
    """
    start = line.find("{")
    if start < 0:
        return None
    # Docker cuts a long line into 16 KiB pieces and puts a timestamp in front of
    # each piece, so a recorded body arrives with timestamps inside its base64.
    # Neither base64 nor JSON ever contains this shape, so removing it restores
    # the line exactly as the proxy wrote it.
    body = TIMESTAMP_INSIDE.sub("", line[start:])
    try:
        raw = json.loads(body)
    except ValueError:
        return None
    if not isinstance(raw, dict) or "target_host" not in raw and "host" not in raw:
        return None

    host = _text(raw.get("target_host"))
    port = _int(raw.get("target_port"))
    if host is None:
        host, authority_port = split_host(raw.get("host"))
        port = port if port is not None else authority_port
    if host is None:
        return None

    entry: dict[str, Any] = {
        "ts": _text(raw.get("ts")),
        "sandbox_id": _uuid(raw.get("sandbox")),
        "host": host,
        "port": port,
        "protocol": _text(raw.get("protocol")),
        "method": _text(raw.get("method")),
        "path": _text(raw.get("path")),
        "status": _int(raw.get("status")),
        "duration_ms": _int(raw.get("duration_ms")),
        "bytes_in": _int(raw.get("bytes_in")) or 0,
        "bytes_out": _int(raw.get("bytes_out")) or 0,
        "listener": _text(raw.get("listener")),
        "connect": _text(raw.get("connect")),
        "sni": _text(raw.get("sni")),
        "app_protocol": _text(raw.get("app_protocol")),
        "user_agent": _text(raw.get("user_agent")),
        "request_body": _text(raw.get("request_body")),
        "response_body": _text(raw.get("response_body")),
        "usage": _text(raw.get("usage")),
        "cache_markers": _text(raw.get("cache_markers")),
        "llm_auth": _text(raw.get("llm_auth")),
        "request_id": _text(raw.get("request_id")),
        "decision": _text(raw.get("decision")),
        "would_deny": _text(raw.get("would_deny")),
        "reason": _text(raw.get("reason")),
        "rule_id": _uuid(raw.get("rule_id")),
        "mcp_session_id": _text(raw.get("mcp_session_id")),
        "mcp_method": _text(raw.get("mcp_method")),
        "mcp_tool": _text(raw.get("mcp_tool")),
    }
    return {key: value for key, value in entry.items() if value is not None}


def node_token() -> str:
    path = os.getenv("NESTBOX_NODE_TOKEN_FILE", "")
    if path:
        try:
            with open(path, encoding="utf-8") as handle:
                return handle.read().strip()
        except OSError:
            return ""
    return os.getenv("NESTBOX_NODE_TOKEN", "").strip()


def post_batch(entries: list[dict[str, Any]]) -> str:
    """Send one batch: `ok`, `refused` (a 4xx: this content will never be accepted), or `retry`."""
    base = os.getenv("NESTBOX_CONTROL_API_URL", "http://control-api:8000")
    body = json.dumps(
        {
            "environment_id": os.getenv("NESTBOX_ENVIRONMENT_ID", "00000000-0000-0000-0000-000000000002"),
            "traffic": entries,
        }
    ).encode()
    request = urllib.request.Request(
        f"{base}/api/v1/internal/telemetry/batch",
        data=body,
        method="POST",
        headers={
            "content-type": "application/json",
            "x-nestbox-node": os.getenv("NESTBOX_NODE_ID", ""),
            "authorization": f"Bearer {node_token()}",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:  # noqa: S310 - internal URL
            return "ok" if 200 <= response.status < 300 else "retry"
    except urllib.error.HTTPError as failure:
        logger.warning("telemetry batch refused: %s %s", failure.code, failure.read()[:200])
        # A 4xx says the content is wrong (an unknown sandbox, a bad body), not
        # that the server is down; sending it again changes nothing. 429 and 408
        # are the exceptions: those are "later".
        return "refused" if 400 <= failure.code < 500 and failure.code not in (408, 429) else "retry"
    except OSError as failure:
        logger.warning("telemetry batch not delivered: %s", failure)
    return "retry"


def take_batch(pending: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """The next entries to send: up to `MAX_BATCH`, and up to `MAX_BATCH_BYTES` of JSON.

    At least one entry is always taken, so a single large one still goes out alone.
    """
    batch: list[dict[str, Any]] = []
    size = 0
    for entry in pending[:MAX_BATCH]:
        cost = len(json.dumps(entry))
        if batch and size + cost > MAX_BATCH_BYTES:
            break
        batch.append(entry)
        size += cost
    return batch


def ship(entries: list[dict[str, Any]]) -> int:
    """Deliver as much of `entries` as can be delivered; how many were dealt with.

    A refused batch is split in halves until the entry the control API objects to
    stands alone, and only that one is dropped. Without this, a single line about
    a sandbox that no longer exists is resent forever and holds back everything
    behind it, and the console shows no traffic at all.
    """
    if not entries:
        return 0
    result = post_batch(entries)
    if result == "ok":
        return len(entries)
    if result == "retry":
        return 0
    if len(entries) == 1:
        logger.warning("dropping a telemetry entry the control API refuses: host=%s", entries[0].get("host"))
        return 1
    middle = len(entries) // 2
    done = ship(entries[:middle])
    if done < middle:
        return done
    return middle + ship(entries[middle:])


def post_json(path: str, payload: dict[str, Any]) -> dict[str, Any] | None:
    """POST one JSON document to the control API as this node; the answer, or `None`."""
    base = os.getenv("NESTBOX_CONTROL_API_URL", "http://control-api:8000")
    request = urllib.request.Request(
        f"{base}{path}",
        data=json.dumps(payload).encode(),
        method="POST",
        headers={
            "content-type": "application/json",
            "x-nestbox-node": os.getenv("NESTBOX_NODE_ID", ""),
            "x-nestbox-environment": os.getenv("NESTBOX_ENVIRONMENT_ID", "00000000-0000-0000-0000-000000000002"),
            "authorization": f"Bearer {node_token()}",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=8) as response:  # noqa: S310 - internal URL
            return json.loads(response.read() or b"{}")
    except (OSError, ValueError) as failure:
        logger.warning("could not report to %s: %s", path, failure)
        return None


def read_new_lines(since: float, seen: set[str]) -> tuple[list[str], float]:
    """The proxy's log lines newer than `since`, minus ones already shipped."""
    from app import system

    client = system._client()  # noqa: SLF001 - same package, same Docker client rule
    container = system._find(client, system.own_project(client), "egress-proxy")  # noqa: SLF001
    if container is None:
        return [], since
    started = time.time()
    raw = container.logs(since=since, timestamps=True).decode(errors="replace")
    fresh = []
    for line in raw.splitlines():
        if line in seen:
            continue
        seen.add(line)
        fresh.append(line)
    if len(seen) > SEEN_LIMIT:
        for item in list(seen)[: len(seen) - SEEN_LIMIT]:
            seen.discard(item)
    # Overlap one second so a line written while this read ran is not missed; the
    # `seen` set is what keeps the overlap from shipping a line twice.
    return fresh, started - 1.0


async def run() -> None:
    """Forever: read, ship, wait. Never raises: a broken tick must not end the loop."""
    since = time.time() - 60.0
    seen: set[str] = set()
    pending: list[dict[str, Any]] = []
    logger.info("shipping egress proxy logs every %.1fs", POLL_SECONDS)
    while True:
        try:
            lines, since = await asyncio.to_thread(read_new_lines, since, seen)
            for line in lines:
                record = record_from_line(line)
                if record is not None:
                    pending.append(record)
            while pending:
                batch = take_batch(pending)
                done = await asyncio.to_thread(ship, batch)
                del pending[:done]
                if done < len(batch):
                    break
            # A control API that stays down must not grow memory without bound.
            if len(pending) > 20_000:
                del pending[: len(pending) - 20_000]
        except Exception:  # noqa: BLE001
            logger.exception("telemetry tick failed")
        await asyncio.sleep(POLL_SECONDS)
