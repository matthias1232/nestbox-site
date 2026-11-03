"""The process table of one sandbox.

`docker top` is the honest answer to "what is running in there": it is the same
list `ps` inside the container would show, without running a command through the
agent's shell. The column set is fixed; `ps` arguments come from this file and
never from the request, because `ps` accepts a format string and that is enough
to read a file the caller should not see.
"""
from __future__ import annotations
from app.gateway_exec import exec_run
from app.workload import get_container

import time
from typing import Any

import docker
from docker.errors import APIError, NotFound
from fastapi import APIRouter, HTTPException
from pydantic import BaseModel

router = APIRouter()
client = docker.from_env()

# Fixed on purpose: a caller cannot add a column, an `-o` flag, or a sort key.
PS_ARGS = "-eo pid,ppid,user,%cpu,%mem,etime,args"
SAMPLE_LIMIT = 500
ALLOWED_ACTIONS = {"terminate": "TERM", "kill": "KILL", "pause": "STOP", "resume": "CONT"}


class ProcessAction(BaseModel):
    action: str
    nice: int | None = None


def container(name: str) -> Any:
    try:
        return get_container(client, name)
    except NotFound as exc:
        raise HTTPException(404, f"no such sandbox: {name}") from exc


def _to_number(value: str) -> float | None:
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _rows(item: Any) -> list[dict[str, Any]]:
    """Read PIDs from inside the container's PID namespace.

    `docker top` reports host PIDs. Those cannot safely be signalled from a
    container exec, especially for OpenShell workloads. A fixed `ps` command
    yields the PID namespace the operator sees and the action endpoint targets.
    """
    result = exec_run(item, ["ps", "-eo", "pid=,ppid=,user=,%cpu=,%mem=,etime=,args="])
    if result.exit_code != 0:
        raise HTTPException(409, "the sandbox process table is unavailable")
    entries = []
    for line in (result.output or b"").decode(errors="replace").splitlines()[:SAMPLE_LIMIT]:
        fields = line.strip().split(None, 6)
        if len(fields) < 6:
            continue
        entries.append({
            "pid": int(_to_number(fields[0]) or 0), "ppid": int(_to_number(fields[1]) or 0) or None,
            "user": fields[2], "cpu_pct": _to_number(fields[3]), "mem_pct": _to_number(fields[4]),
            "elapsed": fields[5], "command": fields[6] if len(fields) > 6 else "",
        })
    return entries


@router.get("/v1/sandboxes/{name}/processes")
def processes(name: str) -> dict[str, Any]:
    """Rows of `docker top`, capped and typed, with the raw header kept.

    A stopped container has no process table; that is a 409 with a reason, not an
    empty list, so the console can say "the sandbox is stopped" instead of
    drawing a table with no rows and leaving the operator to guess.
    """
    item = container(name)
    entries = _rows(item)
    return {
        "sandbox": name,
        "taken_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "columns": ["PID", "PPID", "USER", "%CPU", "%MEM", "ELAPSED", "COMMAND"],
        "items": entries,
        "truncated": len(entries) >= SAMPLE_LIMIT,
    }


@router.post("/v1/sandboxes/{name}/processes/{pid}/action")
def process_action(name: str, pid: int, request: ProcessAction) -> dict[str, Any]:
    """Apply one fixed signal or a lower CPU priority to a live child process.

    Docker's exec API is used with an argument vector, never a request-supplied
    shell command. PID 1 is deliberately protected: it is the sandbox workload,
    so its lifecycle belongs to the sandbox controls instead.
    """
    if pid < 2:
        raise HTTPException(422, "PID 1 is the sandbox entrypoint; use the sandbox controls instead")
    item = container(name)
    live = _rows(item)
    if not any(row["pid"] == pid for row in live):
        raise HTTPException(404, f"no live process {pid} in sandbox {name}")
    if request.action == "set_nice":
        if request.nice is None or not -20 <= request.nice <= 19:
            raise HTTPException(422, "set_nice requires a nice value between -20 and 19")
        command = ["renice", "-n", str(request.nice), "-p", str(pid)]
        message = f"Priority of PID {pid} set to nice {request.nice}."
    else:
        signal = ALLOWED_ACTIONS.get(request.action)
        if not signal or request.nice is not None:
            raise HTTPException(422, "action must be terminate, kill, pause, resume, or set_nice")
        command = ["kill", f"-{signal}", str(pid)]
        message = f"Sent SIG{signal} to PID {pid}."
    result = exec_run(item, command)
    if result.exit_code != 0:
        raise HTTPException(409, (result.output or b"process action failed").decode(errors="replace")[:512])
    return {"pid": pid, "action": request.action, "message": message}
