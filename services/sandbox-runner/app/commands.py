"""What gets started inside a sandbox, and by what way in.

Every process that appears after the first look is an event: its command line,
its user, and *how it got there* — typed in a terminal, run over SSH, or started
by an agent or a script. The answer comes from the parent chain: a process under
`sshd` came in over SSH, one under the `tmux` server came from a console
terminal, the rest were started by something already inside.

It is a sample taken whenever somebody looks (the console polls while the tab is
open), not a recording: a command that starts and ends between two looks is not
seen, and nothing is kept for a sandbox nobody watches. The terminal service
still records only who attached and for how long (ADR 0023); this view reads the
process table, which the operator may already see.
"""
from __future__ import annotations

import threading
import time
from collections import deque
from typing import Any

from docker.errors import NotFound
from fastapi import APIRouter, HTTPException

from app.processes import client
from app.gateway_exec import exec_run
from app.workload import get_container

router = APIRouter()

RING = 300
COMMAND_LIMIT = 300
PS = ["ps", "-eo", "pid=,ppid=,user=,comm=,args="]

_lock = threading.Lock()
# sandbox -> {"known": {pid: start_key}, "events": deque, "seq": int}
_state: dict[str, dict[str, Any]] = {}


def _read(container: Any) -> dict[int, dict[str, Any]]:
    result = exec_run(container, PS)
    if result.exit_code != 0:
        raise HTTPException(409, "the sandbox process table is unavailable")
    table: dict[int, dict[str, Any]] = {}
    for line in (result.output or b"").decode(errors="replace").splitlines():
        fields = line.strip().split(None, 4)
        if len(fields) < 4 or not fields[0].isdigit():
            continue
        table[int(fields[0])] = {
            "pid": int(fields[0]), "ppid": int(fields[1]) if fields[1].isdigit() else 0,
            "user": fields[2], "comm": fields[3],
            "command": (fields[4] if len(fields) > 4 else fields[3])[:COMMAND_LIMIT],
        }
    return table


def way_in(table: dict[int, dict[str, Any]], pid: int) -> str:
    """terminal, ssh, or inside, from the names of the process's ancestors."""
    seen = 0
    current = table.get(pid)
    while current and seen < 32:
        name = current["comm"]
        if name.startswith("sshd") or name == "sftp-server":
            return "ssh"
        if name.startswith("tmux"):
            return "terminal"
        current = table.get(current["ppid"])
        seen += 1
    return "inside"


# Helper programs of the platform itself; listing them would bury the commands a
# person typed under the supervisor's own polling.
NOISE = ("nestbox-agent-supervisor", "nestbox-supervisor", "ps -eo", "sh -c for f in tcp")


@router.get("/v1/sandboxes/{name}/live/commands")
def commands(name: str) -> dict[str, Any]:
    """Processes that started since this sandbox was first looked at."""
    try:
        container = get_container(client, name)
    except NotFound as exc:
        raise HTTPException(404, f"no such sandbox: {name}") from exc
    table = _read(container)
    now = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    with _lock:
        state = _state.setdefault(name, {"known": None, "events": deque(maxlen=RING), "seq": 0})
        first_look = state["known"] is None
        known: dict[int, str] = state["known"] or {}
        current: dict[int, str] = {}
        for pid, row in table.items():
            key = f"{row['ppid']}:{row['comm']}"
            current[pid] = key
            if first_look or known.get(pid) == key:
                continue
            if any(mark in row["command"] for mark in NOISE):
                continue
            state["seq"] += 1
            state["events"].append({
                "seq": state["seq"], "ts": now, "pid": pid, "ppid": row["ppid"],
                "user": row["user"], "command": row["command"], "via": way_in(table, pid),
            })
        state["known"] = current
        events = list(state["events"])
    return {
        "sandbox": name, "taken_at": now, "items": events,
        "watching_since": events[0]["ts"] if events else now,
        "note": "Commands that start and end between two looks are not seen; nothing is kept while nobody watches.",
    }
