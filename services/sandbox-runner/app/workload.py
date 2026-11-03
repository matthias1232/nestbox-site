"""Find the container behind a sandbox, whoever created it.

A sandbox is either one of our own Docker containers (`nestbox-sandbox-<name>`) or
an OpenShell sandbox, whose workload container the OpenShell Docker driver names
`openshell-default--<name>-<id>` next to a `...-supervisor` container. Everything
that only *observes* a sandbox (processes, inventory, metrics, the terminal, the
SSH door) goes through this one lookup, so it does not care which one it is.
"""

from __future__ import annotations

from typing import Any

from docker.errors import NotFound

OWN_PREFIX = "nestbox-sandbox-"
OPENSHELL_PREFIX = "openshell-default--"
SUPERVISOR_SUFFIX = "-supervisor"


def openshell_name(container_name: str) -> str | None:
    """`openshell-default--demo-<uuid>` -> `demo`; None for anything else."""
    if not container_name.startswith(OPENSHELL_PREFIX) or container_name.endswith(SUPERVISOR_SUFFIX):
        return None
    rest = container_name[len(OPENSHELL_PREFIX):]
    # The id is a 36-character uuid after the last dash group: name-<8>-<4>-<4>-<4>-<12>.
    parts = rest.rsplit("-", 5)
    return parts[0] if len(parts) == 6 else None


def get_container(client: Any, name: str) -> Any:
    """The container for a sandbox name, or `NotFound`.

    A name can briefly have two containers: the Docker one from before a move to
    the gateway runtime, and the gateway's workload. The one that is running is the
    sandbox; only when neither runs does the order decide (ours first, as before).
    """
    found = []
    try:
        found.append(client.containers.get(f"{OWN_PREFIX}{name}"))
    except NotFound:
        pass
    for item in client.containers.list(all=True, filters={"name": f"{OPENSHELL_PREFIX}{name}-"}):
        if openshell_name(item.name) == name:
            found.append(item)
    if not found:
        raise NotFound(f"no sandbox {name}")
    for item in found:
        if item.status == "running":
            return item
    return found[0]


def list_workloads(client: Any) -> list[Any]:
    """Every sandbox container, own and OpenShell, supervisors excluded."""
    own = client.containers.list(all=True, filters={"label": "nestbox.managed=true"})
    own = [item for item in own if item.name.startswith(OWN_PREFIX)]
    shell = [
        item
        for item in client.containers.list(all=True, filters={"name": OPENSHELL_PREFIX})
        if openshell_name(item.name)
    ]
    return own + shell


def sandbox_name(container: Any) -> str:
    """The Nestbox name of a sandbox container."""
    shell = openshell_name(container.name)
    return shell if shell else container.name.removeprefix(OWN_PREFIX)
