"""What is installed inside a sandbox, as a snapshot.

The probe is a fixed script (`app/inventory.sh`) that ships with the runner and
is fed to `sh` inside the container. It takes no arguments: there is no place to
put a parameter, so there is nothing an agent inside the container or a caller
outside it can steer. The parser below turns its sections into the JSON the
control plane stores.

Two facts this module is careful about:

* a value of an environment variable never appears — the script prints names
  only, and this parser has no field that could carry a value;
* a partial answer is reported as partial (`note`), because "no packages" and
  "`dpkg-query` is missing" are different facts and the console must not show the
  first when the second is true.
"""
from __future__ import annotations
from app.workload import get_container, list_workloads, sandbox_name

import os
import time
from typing import Any

import docker
from app.gateway_exec import exec_run
from docker.errors import APIError, NotFound
from fastapi import APIRouter, HTTPException

router = APIRouter()
client = docker.from_env()

SCRIPT_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "inventory.sh")
# A package list on a fat image is large; 2 MiB is generous and still bounded by
# something this process can hold without thinking about it.
OUTPUT_LIMIT_BYTES = 2 * 1024 * 1024
PACKAGE_LIMIT = 4000


def container(name: str) -> Any:
    try:
        return get_container(client, name)
    except NotFound as exc:
        raise HTTPException(404, f"no such sandbox: {name}") from exc


def script_text() -> str:
    """The probe script, read once from the runner's own files.

    Executing it through `sh -c` is the one place a shell string appears. It is a
    constant read from a file that ships with this image, and no request value is
    ever interpolated into it; the alternative, requiring every sandbox image to
    contain the script, would let the image decide what the inventory says.
    """
    with open(SCRIPT_PATH, encoding="utf-8") as handle:
        return handle.read()


def parse(raw: str) -> dict[str, Any]:
    """Sections to JSON. Unknown sections are ignored, not guessed at."""
    sections: dict[str, list[str]] = {}
    current = ""
    for line in raw.splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            current = stripped[1:-1]
            sections.setdefault(current, [])
            continue
        if current and line.strip():
            sections[current].append(line.rstrip("\n"))

    os_release: dict[str, str] = {}
    for line in sections.get("os", []):
        if "=" in line:
            key, value = line.split("=", 1)
            os_release[key] = value.strip().strip('"')
    kernel = sections.get("kernel", [])
    if kernel:
        os_release["kernel"] = kernel[0]
    for line in kernel[1:]:
        if line.startswith("arch\t"):
            os_release["arch"] = line.split("\t", 1)[1]

    packages = []
    for line in sections.get("packages", []):
        parts = line.split("\t", 1)
        packages.append({"name": parts[0], "version": parts[1] if len(parts) > 1 else ""})
        if len(packages) >= PACKAGE_LIMIT:
            break

    agents: dict[str, str] = {}
    for line in sections.get("agents", []):
        parts = line.split("\t", 1)
        agents[parts[0]] = parts[1].strip() if len(parts) > 1 else ""

    listening = []
    for line in sections.get("listening", []):
        parts = line.split("\t", 1)
        listening.append(
            {
                "address": parts[0],
                "protocol": parts[1] if len(parts) > 1 else "tcp",
            }
        )

    mounts = []
    for line in sections.get("mounts", []):
        parts = line.split("\t")
        if len(parts) == 3:
            mounts.append({"target": parts[0], "fstype": parts[1], "source": parts[2]})

    return {
        "os": os_release,
        "packages": packages,
        "agents": agents,
        "listening": listening,
        "mounts": mounts,
        "mcp_servers": sections.get("mcp_servers", []),
        "env_keys": sorted(set(sections.get("env_keys", []))),
    }


def image_facts(item: Any) -> dict[str, Any]:
    """Image identity from the container, which is a fact rather than a probe."""
    item.reload()
    attrs = item.attrs
    config = attrs.get("Config") or {}
    return {
        "reference": config.get("Image") or "",
        "id": attrs.get("Image") or "",
        "created": attrs.get("Created") or "",
        "labels": {
            key: value
            for key, value in (config.get("Labels") or {}).items()
            if key.startswith("nestbox.")
        },
    }


def probe(name: str) -> dict[str, Any]:
    """Run the fixed script in the container and parse the answer."""
    item = container(name)
    try:
        exit_code, output = exec_run(item, ["/bin/sh", "-c", script_text()], timeout=60)
    except APIError as exc:
        raise HTTPException(409, f"the sandbox cannot be inspected: {exc}") from exc

    truncated = len(output) > OUTPUT_LIMIT_BYTES
    text = output[:OUTPUT_LIMIT_BYTES].decode("utf-8", errors="replace")
    inventory = parse(text)
    inventory["image"] = image_facts(item)

    note = None
    if exit_code not in (0, None):
        note = f"the probe exited with status {exit_code}; the snapshot may be partial"
    elif truncated:
        note = f"the probe output was cut at {OUTPUT_LIMIT_BYTES} bytes"
    elif not inventory["packages"]:
        note = "no package database answered inside the container"

    return {
        "sandbox": name,
        "taken_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "inventory": inventory,
        "note": note,
    }


@router.get("/v1/sandboxes/{name}/inventory")
def inventory(name: str) -> dict[str, Any]:
    """A fresh inventory snapshot. The control plane stores it; this is the probe."""
    return probe(name)

