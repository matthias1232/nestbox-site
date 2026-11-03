"""The sockets of one sandbox, with the process that owns each.

`ps` answers "what runs", this answers "who is talking to whom". The probe is a
fixed shell script run inside the container as the sandbox's own user (root has no more reach: the sandbox drops the capability to read other users' descriptors, so processes of other users show without an owner): it reads `/proc/net/*` and
maps each socket inode to the process whose file descriptor holds it. Nothing
from the request reaches the script, and nothing is installed in the image
(`sh`, `cat`, `readlink` and `sed` are enough, busybox included).

It is a sample, not a trace: a socket that opens and closes between two polls is
not seen. That limit is stated in the answer so the console can say it.
"""
from __future__ import annotations

import ipaddress
import socket
import struct
import time
from typing import Any

from fastapi import APIRouter, HTTPException
from docker.errors import NotFound

from app.gateway_exec import exec_run
from app.workload import get_container
from app.processes import client

router = APIRouter()

SAMPLE_LIMIT = 800
COMMAND_LIMIT = 160

# Fixed on purpose. `$p` and `$fd` come from the container's own /proc.
PROBE = r"""
for f in tcp tcp6 udp udp6; do sed "1d;s/^/NET $f /" /proc/net/$f 2>/dev/null; done
for p in /proc/[0-9]*; do
  pid=${p#/proc/}
  comm=$(cat $p/comm 2>/dev/null) || continue
  cmd=$(tr '\0' ' ' < $p/cmdline 2>/dev/null | cut -c1-160)
  echo "PROC $pid $comm|$cmd"
  for fd in $p/fd/*; do
    l=$(readlink "$fd" 2>/dev/null)
    case "$l" in "socket:["*) l=${l#socket:[}; echo "FD $pid ${l%]}";; esac
  done
done
"""

TCP_STATES = {
    "01": "established", "02": "syn_sent", "03": "syn_recv", "04": "fin_wait1",
    "05": "fin_wait2", "06": "time_wait", "07": "close", "08": "close_wait",
    "09": "last_ack", "0A": "listen", "0B": "closing",
}


def _address(hex_ip: str, hex_port: str) -> tuple[str, int]:
    """Decode the kernel's little-endian hex address into text and a port."""
    port = int(hex_port, 16)
    raw = bytes.fromhex(hex_ip)
    if len(raw) == 4:
        return socket.inet_ntoa(raw[::-1]), port
    words = struct.unpack("<4I", raw)
    packed = b"".join(struct.pack(">I", word) for word in words)
    address = ipaddress.IPv6Address(packed)
    return str(address.ipv4_mapped or address), port


def _scope(address: str) -> str:
    """loopback, internal (private/link-local), internet, or any."""
    try:
        ip = ipaddress.ip_address(address)
    except ValueError:
        return "unknown"
    if ip.is_unspecified:
        return "any"
    if ip.is_loopback:
        return "loopback"
    if ip.is_private or ip.is_link_local:
        return "internal"
    return "internet"


def _parse(output: str) -> dict[str, Any]:
    sockets: list[dict[str, Any]] = []
    processes: dict[str, tuple[str, str]] = {}
    owners: dict[str, str] = {}
    for line in output.splitlines():
        parts = line.split()
        if not parts:
            continue
        if parts[0] == "NET" and len(parts) >= 12:
            proto, local, remote, state, inode = parts[1], parts[3], parts[4], parts[5], parts[11]
            try:
                local_ip, local_port = _address(*local.split(":"))
                remote_ip, remote_port = _address(*remote.split(":"))
            except (ValueError, struct.error):
                continue
            sockets.append({
                "proto": proto.rstrip("6"), "inode": inode,
                "state": TCP_STATES.get(state, state.lower()) if proto.startswith("tcp") else "udp",
                "local_ip": local_ip, "local_port": local_port,
                "remote_ip": remote_ip, "remote_port": remote_port,
            })
        elif parts[0] == "PROC" and len(parts) >= 3:
            head, _, command = line.partition("|")
            pid_comm = head.split(None, 2)
            processes[pid_comm[1]] = (pid_comm[2] if len(pid_comm) > 2 else "", command.strip())
        elif parts[0] == "FD" and len(parts) == 3:
            owners.setdefault(parts[2], parts[1])

    listening = {s["local_port"] for s in sockets if s["state"] == "listen"}
    items = []
    for s in sockets:
        pid = owners.get(s["inode"])
        comm, command = processes.get(pid or "", ("", ""))
        if s["state"] == "listen":
            direction = "listen"
        elif s["proto"] == "udp" and s["remote_port"] == 0:
            direction = "listen"
        elif s["local_port"] in listening:
            direction = "inbound"
        else:
            direction = "outbound"
        scope = _scope(s["remote_ip"]) if direction != "listen" else _scope(s["local_ip"])
        items.append({
            "proto": s["proto"], "state": s["state"], "direction": direction, "scope": scope,
            "local": f"{s['local_ip']}:{s['local_port']}",
            "remote": "" if direction == "listen" else f"{s['remote_ip']}:{s['remote_port']}",
            "remote_ip": s["remote_ip"], "remote_port": s["remote_port"],
            "local_port": s["local_port"],
            "pid": int(pid) if pid else None, "process": comm, "command": command[:COMMAND_LIMIT],
            # An SSH connection is recognised by the program or the well-known
            # port; it is a label for the operator, not a protocol sniff.
            "ssh": comm in {"ssh", "sshd", "scp", "sftp-server"}
            or 22 in {s["remote_port"], s["local_port"]},
            # A name lookup is port 53 outbound, whichever process sent it.
            "dns": direction == "outbound" and s["remote_port"] == 53,
        })
    items = items[:SAMPLE_LIMIT]
    return {
        "items": items,
        "counts": {
            "total": len(items),
            "internet": sum(1 for i in items if i["scope"] == "internet" and i["direction"] != "listen"),
            "ssh": sum(1 for i in items if i["ssh"]),
            "dns": sum(1 for i in items if i["dns"]),
            "listening": sum(1 for i in items if i["direction"] == "listen"),
        },
    }


@router.get("/v1/sandboxes/{name}/live/connections")
def connections(name: str) -> dict[str, Any]:
    """Every socket in the sandbox's network namespace, with its owning process."""
    try:
        item = get_container(client, name)
    except NotFound as exc:
        raise HTTPException(404, f"no such sandbox: {name}") from exc
    result = exec_run(item, ["sh", "-c", PROBE])
    if result.exit_code not in (0, None):
        raise HTTPException(409, "the sandbox connection table is unavailable")
    answer = _parse((result.output or b"").decode(errors="replace"))
    answer["sandbox"] = name
    answer["taken_at"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    answer["note"] = "A sample: a socket opened and closed between two polls is not seen."
    return answer
