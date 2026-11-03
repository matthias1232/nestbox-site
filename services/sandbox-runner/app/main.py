"""Docker execution adapter for the phase-1 Nestbox development runtime.

This is deliberately an adapter boundary: the control API never talks to Docker directly.
The adapter exposes observable OCI sandbox facts now and can be replaced by the official
OpenShell SDK adapter without changing the public control-plane contract.
"""
from __future__ import annotations

import logging
import os
import re
from datetime import UTC, datetime
from typing import Any

import docker
from docker.errors import DockerException, NotFound
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field

from app import inventory as inventory_probe
from app import remote_ssh
from app import processes as process_table
from app import connections as socket_table
from app import vnc as desktop_stream
from app import commands as command_feed
from app import stats as resource_stats
from app import terminal as terminal_transport
from app import forwards as published_ports
from app import system as appliance_components
from app import telemetry_shipper
from app import host as host_load
from app import backups as root_backups
from app import catalogue as package_catalogue
from app import registry_images
from app import snapshots as root_snapshots
from app import ca_bundle
from app.model_metrics import parse_prometheus
from app import openshell_backend as osh
from app.images import image_allowed
from app.workload import get_container, list_workloads, openshell_name, sandbox_name

app = FastAPI(title="Nestbox Sandbox Runtime", version="1.0.0")
client = docker.from_env()


def _expected_token() -> str:
    path = os.getenv("NESTBOX_NODE_TOKEN_FILE", "")
    if path:
        try:
            with open(path, encoding="utf-8") as handle:
                return handle.read().strip()
        except OSError:
            return ""
    return os.getenv("NESTBOX_NODE_TOKEN", "").strip()


@app.middleware("http")
async def require_node_token(request, call_next):
    """On a node reached over the network, every request carries the node token.

    The appliance's own runner sits on a private compose network and does not set this. A joined
    node does (ADR 0047): its runner holds the Docker socket, so an unauthenticated port on any
    network is a root shell. No token configured is a refusal, never an open door. WebSockets
    (terminal, desktop) check the token themselves in their handshake.
    """
    if os.getenv("NESTBOX_RUNNER_REQUIRE_TOKEN") == "1" and request.url.path != "/healthz":
        import hmac
        from fastapi.responses import JSONResponse

        expected = _expected_token()
        header = request.headers.get("authorization", "")
        presented = header[7:].strip() if header.lower().startswith("bearer ") else ""
        if not expected or not presented or not hmac.compare_digest(expected, presented):
            return JSONResponse({"detail": "The node token was refused."}, status_code=401)
    return await call_next(request)

# Uvicorn configures its own loggers and leaves the root logger bare, so a module
# logger's `info` would be dropped and only its warnings would reach the console
# through the last-resort handler. The egress wiring is exactly the thing an
# operator needs to see in the log next to the access lines, so give the root one
# handler if nobody else did.
if not logging.getLogger().handlers:
    logging.basicConfig(
        level=os.getenv("NESTBOX_LOG_LEVEL", "INFO"),
        format="%(levelname)s:     %(name)s - %(message)s",
    )
logger = logging.getLogger("nestbox.runner")

# Admin Center adapters. Each module owns its routes and its own Docker client,
# so the packages that fill them in can change one file without touching the
# request path that already ships. `terminal` answers 1011 until the PTY pump
# exists; `inventory`, `processes`, and `stats` answer real data today.
app.include_router(resource_stats.router)
app.include_router(process_table.router)
app.include_router(socket_table.router)
app.include_router(desktop_stream.router)
app.include_router(command_feed.router)
app.include_router(inventory_probe.router)
app.include_router(terminal_transport.router)
app.include_router(published_ports.router)
app.include_router(appliance_components.router)
app.include_router(host_load.router)
app.include_router(root_backups.router)
app.include_router(package_catalogue.router)
app.include_router(remote_ssh.router)
app.include_router(registry_images.router)
app.include_router(root_snapshots.router)


def adopt_restart_policy() -> None:
    """Give sandboxes created before the policy existed the same policy.

    Only containers that are running now: a stopped one was stopped on purpose.
    """
    try:
        for container in client.containers.list(filters={"label": "nestbox.managed=true"}):
            labels = container.labels or {}
            if labels.get("nestbox.kind") == "model":
                continue
            policy = (container.attrs.get("HostConfig", {}).get("RestartPolicy") or {}).get("Name")
            if policy in (None, "", "no"):
                container.update(restart_policy={"Name": "unless-stopped"})
    except DockerException as exc:
        logger.warning("could not adopt the restart policy: %s", exc)


@app.on_event("startup")
async def restore_published_ports() -> None:
    """Re-open the listeners the control plane last asked for.

    The runner is recreated on every deploy; without this every sandbox would be
    unreachable until something pushed its forwards again.
    """
    await published_ports.manager.restore()
    adopt_restart_policy()
    # The proxy's decisions are only visible once something reads its log.
    import asyncio

    asyncio.create_task(telemetry_shipper.run())
NETWORK = os.getenv("NESTBOX_RUNTIME_NETWORK", "nestbox-runtime")
# The only way out of a sandbox. It is joined to the sandbox network by this
# adapter rather than by compose, because this adapter is what creates that
# network; see `attach_egress_proxy`.
EGRESS_PROXY_CONTAINER = os.getenv("NESTBOX_EGRESS_PROXY_CONTAINER", "nestbox-egress-proxy-1")
# The name a sandbox resolves the proxy under, and the port it listens on. The
# control API writes `http://<host>:<port>` into the sandbox environment, so the
# two defaults have to agree; `NESTBOX_EGRESS_PROXY_URL` there is the other half.
EGRESS_PROXY_HOST = os.getenv("NESTBOX_EGRESS_PROXY_HOST", "egress-proxy")
EGRESS_PROXY_PORT = 3128
# Ports a tool would try when it ignores the proxy environment. Redirecting them
# inside the proxy container can only narrow what a sandbox reaches, never widen
# it, which is why a failure to install the rules is reported and not fatal.
REDIRECT_PORTS = (80, 443, 8080)
# The only directory tree a bind mount may come from. Validated as a real path
# so `..` and symlinks cannot escape it. Empty disables mounts entirely.
PERSISTENT_BASE = os.path.realpath(os.getenv("NESTBOX_PERSISTENT_BASE", "/var/lib/nestbox/persistent"))
HOST_PROC = os.getenv("NESTBOX_HOST_PROC", "/proc")
# The network a sandbox with `direct` egress joins: an ordinary bridge with a
# default route, which is what "no proxy" means. It is separate from the stack's
# `control` network on purpose — a sandbox must not reach the database or the
# control API just because it may reach the internet.
DIRECT_NETWORK = os.getenv("NESTBOX_DIRECT_NETWORK", "nestbox-sandbox-direct")
# The deployment CA, as this container sees it. Mounted into a sandbox in every
# mode that uses the proxy; the file itself is never in an image. The target is
# owned by `app/ca_bundle.py`, which mounts it together with the bundle.
EGRESS_CA_HOST = os.getenv("NESTBOX_EGRESS_CA", "/run/egress/ca.crt")
EGRESS_CA_TARGET = ca_bundle.EGRESS_CA_TARGET



class MountSpec(BaseModel):
    source: str
    target: str = Field(pattern=r"^/.+")
    read_only: bool = False


def validate_mount(mount: MountSpec) -> dict[str, Any]:
    """Refuse any source outside the persistent base.

    The control plane computed the source from a validated slug; this is the
    second line, for the case where the control plane is wrong or compromised.
    """
    real = os.path.realpath(mount.source)
    if not PERSISTENT_BASE or real != PERSISTENT_BASE and not real.startswith(PERSISTENT_BASE + os.sep):
        raise HTTPException(422, f"mount source is outside the persistent base: {mount.source}")
    if ".." in mount.target.split("/"):
        raise HTTPException(422, "mount target must not contain '..'")
    os.makedirs(real, exist_ok=True)
    # The directory is made by this process (root), but the sandbox user that writes
    # into it is not root and differs between images, so it must be writable by any
    # of them. It is reachable only through this bind: its parents are not
    # world-readable, so no other process on the host gets to it by path.
    try:
        os.chmod(real, 0o777)
    except OSError:
        pass
    return {"type": "bind", "source": real, "target": mount.target, "read_only": mount.read_only}


class SandboxRequest(BaseModel):
    name: str = Field(pattern=r"^[a-z0-9-]{1,63}$")
    image: str = "nestbox-ubuntu-dev:local"
    command: list[str] = Field(default_factory=lambda: ["sleep", "infinity"])
    runtime: str = "docker-development"
    pack_key: str = "ubuntu-dev"
    tool_profile: str = "development-default"
    environment: dict[str, str] = Field(default_factory=dict)
    memory_limit: str = "1g"
    # A reservation is not a limit: it is what the scheduler may count on, so a
    # sandbox is placed where that much memory is really free without being
    # capped at it. `cpu_shares` is relative weight, not a quota, and it only
    # matters when the node is contended.
    memory_reservation: str | None = None
    cpu_shares: int | None = None
    cpu_limit: float = Field(default=1.0, gt=0, le=8)
    pids_limit: int = Field(default=256, ge=32, le=4096)
    ports: dict[str, int] = Field(default_factory=dict)
    mounts: list[MountSpec] = Field(default_factory=list)
    gpu_count: int = Field(default=0, ge=0, le=8)
    # `direct`, `forward_proxy`, `transparent` or `hybrid`. Two of them change
    # how the container is placed: `direct` needs a route out, the TLS-reading
    # ones need the node CA.
    egress_mode: str = "forward_proxy"


def container_name(name: str) -> str:
    return f"nestbox-sandbox-{name}"


def ensure_network() -> Any:
    """Return the sandbox network, or refuse one that has a route out.

    Docker has no per-container "no default route", so the network carries the
    property instead: `internal: true` means the bridge has no gateway, no NAT
    and no DNS beyond container names. A container on it gets a route to its own
    subnet and nothing else — no default route, no way to reach the host, and an
    external name answers `SERVFAIL`.

    A network that already exists with `Internal=false` is exactly the state
    this replaces: every sandbox on it has a default route and full egress. It is
    refused with 503 rather than reused, because a create that quietly succeeds
    on an open network is the failure that made the old setup look policed while
    nothing was. `internal` cannot be flipped on an existing bridge, so the fix
    is an operator action (remove the network, let this adapter recreate it) and
    it is named in the error.
    """
    try:
        network = client.networks.get(NETWORK)
    except NotFound:
        return client.networks.create(
            NETWORK,
            driver="bridge",
            internal=True,
            labels={"nestbox.managed": "true"},
        )
    if not network.attrs.get("Internal", False):
        raise HTTPException(
            503,
            f"sandbox network '{NETWORK}' has a route out (internal=false). "
            "Remove it and create a sandbox again so the adapter recreates it "
            "with no default route; nothing on an open network is policed.",
        )
    return network


EGRESS_CA_PUBLIC_DIR = os.getenv("NESTBOX_EGRESS_CA_PUBLIC", "/var/lib/nestbox/egress")


def publish_egress_ca() -> str | None:
    """Copy the public CA to a world-readable file a sandbox can bind-mount.

    The deployment keeps `secrets/` at 0600 for its own user, which a sandbox
    user cannot read, and a deploy resets that mode. The certificate is public;
    the key is never copied. The same path inside and out, like every bind
    source the runner hands to Docker.
    """
    if not os.path.exists(EGRESS_CA_HOST):
        return None
    os.makedirs(EGRESS_CA_PUBLIC_DIR, mode=0o755, exist_ok=True)
    target = os.path.join(EGRESS_CA_PUBLIC_DIR, "ca.crt")
    with open(EGRESS_CA_HOST, "rb") as source, open(target, "wb") as out:
        out.write(source.read())
    os.chmod(target, 0o644)
    return target


def ensure_direct_network() -> Any:
    """Return the bridge a `direct` sandbox leaves through, creating it once.

    The sandbox network is `internal`: no gateway, no NAT, no external DNS. That
    is what makes the egress proxy the only way out. A sandbox whose operator
    chose `direct` is supposed to have a route of its own, so it gets this
    second bridge instead — a normal one, with Docker's NAT and the node's
    resolver — and is then attached to the sandbox network as well so it keeps
    reaching its model and the MCP hub by name.

    Deliberately not the stack's `control` network: a sandbox that may reach the
    internet has no business reaching PostgreSQL.
    """
    try:
        return client.networks.get(DIRECT_NETWORK)
    except NotFound:
        return client.networks.create(
            DIRECT_NETWORK,
            driver="bridge",
            labels={"nestbox.managed": "true", "nestbox.egress": "direct"},
        )


def find_egress_proxy() -> Any | None:
    """The node's egress proxy container, by configured name or compose label."""
    try:
        return client.containers.get(EGRESS_PROXY_CONTAINER)
    except NotFound:
        pass
    try:
        containers = client.containers.list(
            all=True, filters={"label": "com.docker.compose.service=egress-proxy"}
        )
    except DockerException:
        return None
    return containers[0] if containers else None


def egress_proxy_endpoint(network: Any, name: str) -> dict[str, Any] | None:
    """The network endpoint of a container, or `None` when it is not attached."""
    try:
        network.reload()
    except DockerException:
        return None
    for endpoint in (network.attrs.get("Containers") or {}).values():
        if endpoint.get("Name") == name:
            return endpoint
    return None


def egress_proxy_aliases(proxy: Any) -> list[str]:
    """The names the proxy answers to on the sandbox network.

    Read from the container, not from the network: `network inspect` lists the
    endpoint but not its aliases, so a check against that view reports "no alias"
    for a proxy that has one and would reconnect it on every create.
    """
    try:
        proxy.reload()
    except DockerException:
        return []
    endpoint = (proxy.attrs.get("NetworkSettings", {}).get("Networks") or {}).get(NETWORK) or {}
    return list(endpoint.get("Aliases") or [])


def attach_egress_proxy(network: Any, proxy: Any) -> str:
    """Put the proxy on the sandbox network under the name the env names it.

    The proxy is the only container besides this adapter that belongs on the
    sandbox network, and it is joined here because this adapter is what creates
    the network. The alias is not cosmetic: a manual `connect` gives the
    container only its container name, so a sandbox asking for
    `http://egress-proxy:3128` would not resolve and every request would fail
    with a proxy that is right there. `aliases` is what makes the name the
    control API writes into the environment true.

    Idempotent, and a failure is reported rather than fatal: an unreachable proxy
    leaves the sandbox isolated, which is the safe direction.
    """
    endpoint = egress_proxy_endpoint(network, proxy.name)
    if endpoint is not None and EGRESS_PROXY_HOST in egress_proxy_aliases(proxy):
        return "attached"
    if endpoint is not None:
        # Present without the name a sandbox asks for: reconnect so the alias
        # exists. Envoy keeps its listener; only the endpoint is re-added.
        try:
            network.disconnect(proxy)
        except DockerException:
            pass
    try:
        network.connect(proxy, aliases=[EGRESS_PROXY_HOST])
        return "attached-now"
    except DockerException as exc:
        if "already exists" in str(exc) or "already attached" in str(exc):
            return "attached"
        return f"failed: {exc}"


def redirect_rule_status(proxy: Any) -> str:
    """Install the NAT redirect for ports a proxy-unaware tool would use.

    Idempotent: each port is checked with `-C` before `-A`, so a restart of this
    adapter does not stack duplicate rules. Installed *inside* the proxy
    container because that is the one component with `NET_ADMIN` (deploy README);
    a sandbox has every capability dropped and cannot install anything.

    This is a convenience, never the boundary. A sandbox has no default route, so
    a raw socket is already unroutable; the rule only helps traffic that does
    reach the proxy container — a client that addresses the proxy as a
    gateway-like host on 80/443/8080 instead of 3128. That is why an image
    without `iptables` is reported as `unavailable` and not treated as a hole:
    the sandbox is exactly as isolated either way.
    """
    try:
        for port in REDIRECT_PORTS:
            match = [
                "-t", "nat", "-p", "tcp", "--dport", str(port),
                "-j", "REDIRECT", "--to-ports", str(EGRESS_PROXY_PORT),
            ]
            check = proxy.exec_run(["iptables", "-C", *match], demux=False)
            if check.exit_code == 0:
                continue
            if check.exit_code in (126, 127):
                return "unavailable: the proxy image has no iptables"
            added = proxy.exec_run(["iptables", "-A", *match], demux=False)
            if added.exit_code in (126, 127):
                return "unavailable: the proxy image has no iptables"
            if added.exit_code != 0:
                detail = (added.output or b"").decode(errors="replace").strip()
                if "executable file not found" in detail:
                    return "unavailable: the proxy image has no iptables"
                return f"failed: {detail or added.exit_code}"
    except DockerException as exc:
        return f"failed: {exc}"
    return f"prepared: {','.join(str(port) for port in REDIRECT_PORTS)} -> {EGRESS_PROXY_PORT}"


def prepare_egress(network: Any) -> dict[str, str]:
    """Make the proxy reachable from the sandbox network, and say what happened.

    Called before a container is created, so the proxy is on the network the
    moment the first sandbox is. Every value is a fact — `attached`, `missing`,
    `unavailable` — because this is what an operator has to check when a sandbox
    "has no internet", and a cheerful `ok` would hide the interesting case.
    """
    proxy = find_egress_proxy()
    if proxy is None:
        return {
            "proxy": "missing",
            "attached": "no",
            "redirects": "no proxy container",
        }
    return {
        "proxy": proxy.name,
        "attached": attach_egress_proxy(network, proxy),
        "redirects": redirect_rule_status(proxy),
    }



def utc(value: str | None) -> str | None:
    if not value or value.startswith("0001-"):
        return None
    return value


def supervisor_facts(item: Any) -> dict[str, str]:
    """The OpenShell supervisor next to a workload: its own container.

    Empty for a sandbox of our own, which has no supervisor container. The status is
    the container's, with Docker's health when the supervisor image defines one.
    """
    if not openshell_name(item.name):
        return {}
    try:
        companion = client.containers.get(f"{item.name}-supervisor")
    except (NotFound, DockerException):
        return {"supervisor_status": "missing", "supervisor_image": ""}
    state = companion.attrs.get("State", {})
    health = (state.get("Health") or {}).get("Status", "")
    status = "running" if state.get("Running") else "stopped"
    if status == "running" and health in {"unhealthy", "starting"}:
        status = "degraded" if health == "unhealthy" else "starting"
    # The supervisor ships with the gateway: its version is the gateway's.
    version = osh.gateway_version()
    return {"supervisor_status": status, "supervisor_image": f"openshell-supervisor {version}" if version else ""}


def details(item: Any) -> dict[str, Any]:
    item.reload()
    attrs = item.attrs
    state = attrs.get("State", {})
    networks = attrs.get("NetworkSettings", {}).get("Networks", {})
    labels = attrs.get("Config", {}).get("Labels") or {}
    ports = attrs.get("NetworkSettings", {}).get("Ports") or {}
    return {
        "id": item.id,
        "name": sandbox_name(item),
        "container_name": item.name,
        "status": item.status,
        "runtime": "openshell" if openshell_name(item.name) else labels.get("nestbox.runtime", "docker-development"),
        "pack_key": labels.get("nestbox.pack", ""),
        "tool_profile": labels.get("nestbox.tool_profile", ""),
        "image": attrs.get("Config", {}).get("Image", ""),
        "created_at": utc(attrs.get("Created")),
        "started_at": utc(state.get("StartedAt")),
        "finished_at": utc(state.get("FinishedAt")),
        "uptime_seconds": max(0, int((datetime.now(UTC) - datetime.fromisoformat(state["StartedAt"].replace("Z", "+00:00"))).total_seconds())) if state.get("Running") and state.get("StartedAt") else 0,
        "exit_code": state.get("ExitCode"),
        "health": (state.get("Health") or {}).get("Status", "unknown"),
        "error": state.get("Error", ""),
        "network": [
            {"name": key, "ip": value.get("IPAddress", ""), "gateway": value.get("Gateway", ""), "aliases": value.get("Aliases", [])}
            for key, value in networks.items()
        ],
        "ports": ports,
        "resources": {
            "memory_limit": attrs.get("HostConfig", {}).get("Memory", 0),
            "nano_cpus": attrs.get("HostConfig", {}).get("NanoCpus", 0),
            "pids_limit": attrs.get("HostConfig", {}).get("PidsLimit", 0),
        },
        "labels": {key: value for key, value in labels.items() if key.startswith("nestbox.")},
        **supervisor_facts(item),
    }


@app.get("/healthz")
def healthz() -> dict[str, str]:
    """Docker reachable, and what the way out of a sandbox currently looks like."""
    try:
        client.ping()
    except DockerException as exc:
        raise HTTPException(503, f"Docker runtime unavailable: {exc}") from exc
    try:
        internal = client.networks.get(NETWORK).attrs.get("Internal", False)
        network_state = "internal" if internal else "open"
    except NotFound:
        network_state = "absent"
    except DockerException:
        network_state = "unknown"
    proxy = find_egress_proxy()
    return {
        "status": "ok",
        "adapter": "docker-development",
        "network": NETWORK,
        "network_isolation": network_state,
        "egress_proxy": proxy.name if proxy else "missing",
    }


class OpenShellPolicy(BaseModel):
    document: dict[str, Any]


@app.put("/v1/sandboxes/{name}/openshell-policy")
def set_openshell_policy(name: str, body: OpenShellPolicy) -> dict[str, Any]:
    """Load a policy document into a running OpenShell sandbox (live fields only
    change without a restart: the network policies)."""
    if not re.fullmatch(r"[a-z0-9-]{1,63}", name):
        raise HTTPException(422, "invalid sandbox name")
    try:
        container = get_container(client, name)
    except NotFound as exc:
        raise HTTPException(404, "sandbox not found") from exc
    if not openshell_name(container.name):
        # Nothing to push: a Docker-runtime sandbox is governed by the egress
        # proxy, not by a gateway policy. Answering 200 keeps the sync quiet.
        return {"name": name, "applied": False, "reason": "not on the OpenShell gateway"}
    try:
        osh.set_policy(name, body.document)
    except osh.OpenShellError as exc:
        raise HTTPException(400, f"policy rejected: {exc}") from exc
    return {"name": name, "applied": True}


@app.get("/v1/sandboxes/{name}/openshell")
def openshell_view(name: str) -> dict[str, Any]:
    """What the gateway and the supervisor say about one sandbox, for the console.

    The boundary state (phase, conditions), the policy it enforces, and the recent log
    of the supervisor and the gateway. Read-only.
    """
    if not re.fullmatch(r"[a-z0-9-]{1,63}", name):
        raise HTTPException(422, "invalid sandbox name")
    try:
        container = get_container(client, name)
    except NotFound as exc:
        raise HTTPException(404, "sandbox not found") from exc
    if not openshell_name(container.name):
        raise HTTPException(409, "this sandbox is not on the OpenShell gateway")
    try:
        sandbox = osh.get(name)
        logs = osh.run(["logs", name, "-n", "120", "--source", "all"], timeout=30)
    except osh.OpenShellError as exc:
        raise HTTPException(502, str(exc)) from exc
    if sandbox is None:
        raise HTTPException(404, "the gateway does not know this sandbox")
    keep = (
        "id", "name", "phase", "created_at", "conditions", "restart_policy", "restart_count",
        "current_policy_version", "policy_source", "configuration_admission", "annotations",
    )
    return {
        "gateway_version": osh.gateway_version(),
        "sandbox": {key: sandbox.get(key) for key in keep},
        "policy": sandbox.get("policy") or {},
        "logs": logs.splitlines()[-120:],
    }


@app.get("/v1/openshell/events")
def openshell_events(since: int = 120) -> dict[str, Any]:
    """Network decisions the OpenShell supervisors logged in the last `since` seconds."""
    from app import openshell_events as events

    since = max(10, min(since, 3600))
    found: list[dict[str, Any]] = []
    for item in list_workloads(client):
        name = openshell_name(item.name)
        if not name or item.status != "running":
            continue
        try:
            text = osh.run(["logs", name, "--source", "sandbox", "--since", f"{since}s", "-n", "1000"], timeout=30)
        except osh.OpenShellError:
            continue
        for event in events.parse(text):
            found.append({**event, "sandbox": name})
    return {"events": found}


@app.get("/v1/openshell")
def openshell_status() -> dict[str, Any]:
    """Whether the OpenShell gateway answers, and what version it is."""
    return osh.status()


@app.get("/v1/egress")
def egress() -> dict[str, Any]:
    """The sandbox path out, as facts.

    Read-only on purpose: it inspects the network and the proxy and reports what
    it found. The redirect check asks the proxy container whether each rule is
    present (`iptables -C`) instead of installing it, because a GET that changes
    firewall state is a GET an operator cannot trust to be harmless.
    """
    try:
        network = client.networks.get(NETWORK)
    except NotFound:
        return {
            "network": NETWORK,
            "network_isolation": "absent",
            "proxy": None,
            "attached": False,
            "alias": EGRESS_PROXY_HOST,
            "redirects": "no network",
        }
    except DockerException as exc:
        raise HTTPException(503, f"Docker runtime unavailable: {exc}") from exc
    proxy = find_egress_proxy()
    endpoint = egress_proxy_endpoint(network, proxy.name) if proxy else None
    missing = redirect_missing(proxy) if endpoint is not None else "proxy not attached"
    aliases = egress_proxy_aliases(proxy) if proxy else []
    return {
        "network": NETWORK,
        "network_isolation": "internal" if network.attrs.get("Internal", False) else "open",
        "proxy": proxy.name if proxy else None,
        "attached": endpoint is not None,
        "alias": EGRESS_PROXY_HOST,
        "alias_present": EGRESS_PROXY_HOST in aliases,
        "proxy_url": f"http://{EGRESS_PROXY_HOST}:{EGRESS_PROXY_PORT}",
        "redirects": "prepared" if not missing else missing,
    }


def redirect_missing(proxy: Any) -> str:
    """The first port with no redirect rule, or an empty string when all exist.

    The mirror of `redirect_rule_status` for a read-only report: it only asks,
    never appends.
    """
    for port in REDIRECT_PORTS:
        match = [
            "-t", "nat", "-p", "tcp", "--dport", str(port),
            "-j", "REDIRECT", "--to-ports", str(EGRESS_PROXY_PORT),
        ]
        try:
            check = proxy.exec_run(["iptables", "-C", *match], demux=False)
        except DockerException as exc:
            return f"unavailable ({exc})"
        if check.exit_code in (126, 127):
            return "unavailable: the proxy image has no iptables"
        if check.exit_code != 0:
            return f"port {port} has no redirect"
    return ""


def sandbox_runtime_of(container: Any) -> str:
    """Which runtime a container or gateway workload belongs to."""
    if openshell_name(container.name):
        return "openshell"
    return container.labels.get("nestbox.runtime") or "docker-development"


def drop_if_other_runtime(name: str, requested: str) -> None:
    """Remove a sandbox that exists on the *other* runtime than the one asked for.

    A create for a name that already exists returns the existing sandbox, which is
    what a retried create should do. The exception is a change of runtime: an egress
    mode that reads TLS runs on Docker, `forward_proxy` on the gateway, and the
    stored mode may switch from one to the other. Keeping the old workload would
    leave the sandbox on the runtime its mode no longer uses.
    """
    try:
        existing = get_container(client, name)
    except NotFound:
        return
    if sandbox_runtime_of(existing) == requested:
        return
    logger.info("sandbox %s moves from %s to %s; removing the old workload",
                name, sandbox_runtime_of(existing), requested)
    if sandbox_runtime_of(existing) == "openshell":
        try:
            osh.lifecycle(name, "delete")
        except osh.OpenShellError as exc:
            logger.warning("could not delete gateway sandbox %s: %s", name, exc)
    else:
        try:
            existing.remove(force=True)
        except DockerException as exc:
            logger.warning("could not remove container for %s: %s", name, exc)


@app.post("/v1/sandboxes", status_code=201)
def create_sandbox(body: SandboxRequest) -> dict[str, Any]:
    if not image_allowed(body.image):
        raise HTTPException(422, "image is not in the approved phase-1 catalog")
    network = ensure_network()
    requested = "openshell" if body.runtime == "openshell" and body.egress_mode != "direct" else "docker-development"
    drop_if_other_runtime(body.name, requested)
    # `direct` means "no proxy, own route to the internet". A gateway sandbox has no
    # route of its own (its supervisor decides every connection), so that one mode
    # stays on the Docker runtime instead of being silently turned into another.
    if body.runtime == "openshell" and body.egress_mode != "direct":
        return create_openshell_sandbox(body)
    registry_images.ensure_image(client, body.image)
    name = container_name(body.name)
    try:
        return details(client.containers.get(name))
    except NotFound:
        pass
    mounts = [validate_mount(m) for m in body.mounts]
    direct = body.egress_mode == "direct"
    if direct:
        # Before the container exists: `network=` with a name Docker does not
        # know is a create that fails with a message about a network.
        ensure_direct_network()
    # Every mode that uses the proxy gets the CA and the bundle beside it; the
    # file is public, but trusting it is a decision, and the decision is the
    # mode. `direct` has no proxy, so it is given neither.
    public_ca = publish_egress_ca()
    if not direct and not public_ca:
        logger.warning(
            "egress mode %s needs the node CA, but %s does not exist",
            body.egress_mode,
            EGRESS_CA_HOST,
        )
    mounts.extend(ca_bundle.ca_mounts(body.egress_mode, public_ca, ca_bundle.publish_ca_bundle(public_ca)))
    extra: dict[str, Any] = {}
    if mounts:
        from docker.types import Mount as DockerMount

        extra["mounts"] = [
            DockerMount(target=m["target"], source=m["source"], type="bind", read_only=m["read_only"])
            for m in mounts
        ]
    # A reservation lets the scheduler count this sandbox without capping it, and
    # shares decide who wins when the node is contended. Both are optional; a
    # request without them keeps Docker's defaults, which is what every sandbox
    # created before this field existed already has.
    if body.memory_reservation:
        extra["mem_reservation"] = body.memory_reservation
    if body.cpu_shares:
        extra["cpu_shares"] = body.cpu_shares
    if body.gpu_count > 0:
        if not container_gpu_available():
            raise HTTPException(422, "this node cannot give a container a GPU: no NVIDIA container runtime")
        from docker.types import DeviceRequest

        extra["device_requests"] = [DeviceRequest(count=body.gpu_count, capabilities=[["gpu"]])]
    # Before the container exists, so the proxy is on the network the moment a
    # sandbox can send its first request. The result is logged and served by
    # `/v1/egress`; it is not fatal, because a sandbox without a reachable proxy
    # is isolated rather than exposed.
    egress_state = prepare_egress(network)
    if egress_state["proxy"] == "missing" or not egress_state["attached"].startswith("attached"):
        logger.warning("egress proxy not reachable from the sandbox network: %s", egress_state)
    else:
        logger.info("egress ready for %s: %s", body.name, egress_state)
    try:
        container = client.containers.run(
            body.image,
            body.command,
            name=name,
            detach=True,
            # The first network is the one Docker takes the default route from.
            # A direct sandbox gets it from the bridge with NAT; every other
            # sandbox is on the internal network, which has no gateway at all.
            network=DIRECT_NETWORK if direct else NETWORK,
            hostname=body.name,
            environment=body.environment,
            mem_limit=body.memory_limit,
            nano_cpus=int(body.cpu_limit * 1_000_000_000),
            pids_limit=body.pids_limit,
            cap_drop=["ALL"],
            security_opt=["no-new-privileges:true"],
            # A host reboot must not leave the sandbox down; a stop through the
            # console still holds, because Docker honours a manual stop.
            restart_policy={"Name": "unless-stopped"},
            **extra,
            labels={
                "nestbox.managed": "true",
                "nestbox.runtime": body.runtime,
                "nestbox.pack": body.pack_key,
                "nestbox.tool_profile": body.tool_profile,
            },
        )
        if direct:
            # Keep the internal network too: the model on this node and the MCP
            # hub are only reachable there, and neither is egress.
            try:
                ensure_network().connect(container)
            except DockerException as exc:
                logger.warning("direct sandbox %s could not join %s: %s", body.name, NETWORK, exc)
        return details(container)
    except DockerException as exc:
        raise HTTPException(400, f"sandbox creation failed: {exc}") from exc


def create_openshell_sandbox(body: SandboxRequest) -> dict[str, Any]:
    """Create the sandbox on the OpenShell gateway and report it like any other."""
    try:
        existing = get_container(client, body.name)
        return details(existing)
    except NotFound:
        pass
    mounts = [validate_mount(mount) for mount in body.mounts]
    # A mode that uses the proxy needs the node CA inside the sandbox, as on
    # Docker: every such mode, and only then, and only the public half plus the
    # bundle built from it.
    public_ca = publish_egress_ca()
    if body.egress_mode != "direct" and not public_ca:
        logger.warning("egress mode %s needs the node CA, but %s does not exist", body.egress_mode, EGRESS_CA_HOST)
    mounts.extend(ca_bundle.ca_mounts(body.egress_mode, public_ca, ca_bundle.publish_ca_bundle(public_ca)))
    # The gateway runs the command it is given *instead of* the image's entrypoint
    # (Docker would pass it as arguments to it). A recipe image's entrypoint is the
    # script that starts its supervisor and agents, so it has to be named, or the
    # sandbox comes up with nothing running in it.
    command = body.command
    if body.image.startswith("nestbox-recipe/"):
        command = ["/usr/local/bin/nestbox-entrypoint"]
    # `memory_reservation` and `cpu_shares` are not passed on: OpenShell 0.1.6 has
    # no reservation or shares, only the hard `--memory` and `--cpu` limits below.
    try:
        osh.create(
            name=body.name,
            image=body.image,
            command=command,
            environment=body.environment,
            memory_limit=body.memory_limit,
            cpu_limit=body.cpu_limit,
            mounts=mounts,
            labels={"nestbox.pack": body.pack_key, "nestbox.tool_profile": body.tool_profile},
        )
    except osh.OpenShellError as exc:
        raise HTTPException(400, f"sandbox creation failed: {exc}") from exc
    found = osh.wait_for_container(lambda n: _find_or_none(n), body.name)
    if found is None:
        raise HTTPException(504, "the gateway accepted the sandbox but no workload appeared")
    return details(found)


def _find_or_none(name: str) -> Any | None:
    try:
        return get_container(client, name)
    except NotFound:
        return None


def container_gpu_available() -> bool:
    """Whether the Docker daemon has an NVIDIA runtime registered."""
    try:
        runtimes = client.info().get("Runtimes") or {}
    except DockerException:
        return False
    return "nvidia" in runtimes


def read_text(path: str) -> str:
    try:
        with open(path, encoding="utf-8", errors="replace") as handle:
            return handle.read()
    except OSError:
        return ""


def run_capture(args: list[str], timeout: float = 5.0) -> str | None:
    """Run a fixed argument vector and return stdout, or None if it is absent."""
    import subprocess

    try:
        done = subprocess.run(args, capture_output=True, text=True, timeout=timeout, check=False)
    except (OSError, subprocess.SubprocessError):
        return None
    if done.returncode != 0:
        return None
    return done.stdout


@app.get("/v1/hardware")
def hardware() -> dict[str, Any]:
    """What this host has, as raw text for the control plane to parse.

    Raw rather than parsed on purpose: the parsers are tested Rust, and a
    second implementation here would be a second opinion. ``/proc`` is read
    from ``NESTBOX_HOST_PROC`` so a containerised runner can be given the host's
    ``/proc`` and report the machine, not itself.
    """
    cpuinfo = read_text(os.path.join(HOST_PROC, "cpuinfo"))
    meminfo = read_text(os.path.join(HOST_PROC, "meminfo"))
    csv = run_capture([
        "nvidia-smi",
        "--query-gpu=index,name,memory.total,memory.free",
        "--format=csv,noheader,nounits",
    ])
    header = run_capture(["nvidia-smi"]) if csv is not None else None
    note = None
    if csv is None:
        note = "nvidia-smi is not available on this node; GPU models need a GPU host."
    return {
        "hostname": read_text("/etc/hostname").strip() or None,
        "cpuinfo": cpuinfo,
        "meminfo": meminfo,
        "nvidia_smi_csv": csv,
        "nvidia_smi_header": header,
        "container_gpu": container_gpu_available(),
        "note": note,
    }


# The Dockerfile is generated by the control API, never typed by a person, and
# the desktop pack carries its start, display and desktop-control scripts inside
# it as base64 (about 120 KB together, and they grow with every new tool). The
# cap is therefore a guard against a runaway body, not against a real one: a
# smaller cap silently refused the desktop image while the pack still built.
MAX_DOCKERFILE_BYTES = 1_048_576


class BuildRequest(BaseModel):
    tag: str = Field(pattern=r"^nestbox-recipe/[a-z0-9-]{1,63}:[a-f0-9]{8,64}$")
    dockerfile: str = Field(min_length=1, max_length=MAX_DOCKERFILE_BYTES)
    files: list[tuple[str, str]] = Field(default_factory=list)


@app.post("/v1/builds")
def build_image(body: BuildRequest) -> dict[str, Any]:
    """Build an image from a generated Dockerfile.

    The tag pattern is the only way a build can name itself, so a recipe image
    always carries the prefix the allowlist trusts. Extra files are written
    into the build context by relative name; a name with a path separator is
    refused.
    """
    import io
    import tarfile

    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as tar:
        def add(name: str, content: str) -> None:
            data = content.encode()
            info = tarfile.TarInfo(name=name)
            info.size = len(data)
            info.mode = 0o644
            tar.addfile(info, io.BytesIO(data))

        add("Dockerfile", body.dockerfile)
        for name, content in body.files:
            if "/" in name or name.startswith(".") or not name:
                raise HTTPException(422, f"invalid build file name: {name!r}")
            add(name, content)
    buffer.seek(0)

    log_lines: list[str] = []
    try:
        image, stream = client.images.build(fileobj=buffer, custom_context=True, tag=body.tag, rm=True, forcerm=True)
        for chunk in stream:
            if isinstance(chunk, dict) and "stream" in chunk:
                log_lines.append(chunk["stream"])
        return {"tag": body.tag, "image_id": image.id, "log": "".join(log_lines)[-65536:], "success": True}
    except docker.errors.BuildError as exc:
        for chunk in exc.build_log:
            if isinstance(chunk, dict) and "stream" in chunk:
                log_lines.append(chunk["stream"])
        return {"tag": body.tag, "image_id": None, "log": ("".join(log_lines) + str(exc))[-65536:], "success": False}
    except DockerException as exc:
        raise HTTPException(400, f"build failed: {exc}") from exc


# ---------------------------------------------------------------------------
# Model serving: fetch one artefact, then run one server container per install.
#
# The control plane decides *what* to install; this process decides *how*, on
# the machine that has the disk and the Docker socket. Two rules keep a buggy
# caller from turning that into host access:
#
#   1. the image must be on the allowlist, by digest, and
#   2. the file names are validated, and none of them can leave the model base.
#
# The model container joins the same internal runtime network as the sandboxes
# that select its profile, and nothing else.
# ---------------------------------------------------------------------------

MODEL_BASE = os.path.realpath(os.getenv("NESTBOX_MODEL_BASE", "/var/lib/nestbox/models"))
MODEL_IMAGES = set(filter(None, os.getenv("NESTBOX_MODEL_IMAGES", "").split(",")))
MODEL_CONTAINER_PREFIX = "model-"
MODEL_KEY = re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")
MODEL_REPO = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9][A-Za-z0-9._-]*$")
MODEL_FILE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*\.gguf$")
# A download may not be larger than this, whatever the caller asks for: the
# node's disk is shared with everything else.
MODEL_DOWNLOAD_CEILING = int(os.getenv("NESTBOX_MODEL_MAX_BYTES", str(32 * 1024**3)))
# CUDA-X components are pulled by exact, catalogue-provided image reference.
# The allowlist is what keeps an operator request from becoming an arbitrary
# registry fetch on the node.
CUDAX_IMAGES = set(filter(None, os.getenv("NESTBOX_CUDAX_IMAGES", "").split(",")))
CUDAX_COMPONENT = re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")
MODEL_CONTAINER_PATH = "/models"
# The port each served model listens on. `serve_model` records it, `stop_model`
# forgets it; the metrics endpoint uses it to reach the server inside the
# runtime network, where the control plane cannot go.
MODEL_PORTS: dict[str, int] = {}


def model_key(value: str) -> str:
    if not MODEL_KEY.match(value):
        raise HTTPException(422, "model key is not a lowercase slug")
    return value


def model_directory(key: str) -> str:
    """The directory for one model, refused if it is not inside the base.

    `realpath` after the join means `..` and a symlink are both resolved before
    the prefix check, so neither can name a directory outside the base.
    """
    directory = os.path.realpath(os.path.join(MODEL_BASE, key))
    if os.path.commonpath([MODEL_BASE, directory]) != MODEL_BASE:
        raise HTTPException(422, "model directory escapes the model base")
    return directory


def container_port(item) -> int | None:
    """The port a served model listens on, recovered from its own command.

    `MODEL_PORTS` only knows what this process started; after a runner restart
    the container is still running, and the port is in its `--port` argument.
    The image's exposed ports are the last resort.
    """
    args = item.attrs.get("Config", {}).get("Cmd") or []
    for index, argument in enumerate(args):
        if argument == "--port" and index + 1 < len(args):
            try:
                return int(args[index + 1])
            except (TypeError, ValueError):
                pass
        if isinstance(argument, str) and argument.startswith("--port="):
            try:
                return int(argument.split("=", 1)[1])
            except (TypeError, ValueError):
                pass
    exposed = item.attrs.get("Config", {}).get("ExposedPorts") or {}
    for spec in exposed:
        candidate = spec.split("/")[0]
        if candidate.isdigit():
            return int(candidate)
    return None


def model_container(key: str) -> str:
    # The same transform the control plane uses for the provider endpoint, so
    # `model-<host>` in the profile is the container that actually runs.
    return MODEL_CONTAINER_PREFIX + key.replace(".", "-").replace(":", "-").replace("/", "-").lower()


class ModelDownloadRequest(BaseModel):
    model_key: str
    repo: str
    file: str
    max_bytes: int = Field(gt=0)


@app.post("/v1/models/download")
def download_model(body: ModelDownloadRequest) -> dict[str, Any]:
    """Fetch one file from Hugging Face into the model base.

    Written to `<file>.part` and renamed only after the last byte, so an
    interrupted download is never mistaken for a finished artefact. A file that
    is already there is not fetched again: an install retried after a crash
    resumes at "serve", not at "download".
    """
    import shutil
    import urllib.error
    import urllib.request

    key = model_key(body.model_key)
    if not MODEL_REPO.match(body.repo):
        raise HTTPException(422, "repository must be 'org/name'")
    if not MODEL_FILE.match(body.file):
        raise HTTPException(422, "only a *.gguf file from the repository root is accepted")
    limit = min(body.max_bytes, MODEL_DOWNLOAD_CEILING)

    directory = model_directory(key)
    os.makedirs(directory, exist_ok=True)
    target = os.path.join(directory, body.file)
    partial = target + ".part"

    if os.path.exists(target):
        return {"path": target, "bytes": os.path.getsize(target), "cached": True}

    url = f"https://huggingface.co/{body.repo}/resolve/main/{body.file}"
    request = urllib.request.Request(url, headers={"User-Agent": "nestbox-runner"})
    written = 0
    try:
        with urllib.request.urlopen(request, timeout=60) as response, open(partial, "wb") as sink:
            declared = response.headers.get("Content-Length")
            if declared is not None and int(declared) > limit:
                raise HTTPException(
                    422,
                    f"{body.file} is {int(declared)} bytes, above the {limit}-byte limit for this node",
                )
            while True:
                chunk = response.read(1024 * 1024)
                if not chunk:
                    break
                written += len(chunk)
                if written > limit:
                    raise HTTPException(422, f"{body.file} grew past the {limit}-byte limit")
                sink.write(chunk)
            sink.flush()
            os.fsync(sink.fileno())
    except HTTPException:
        if os.path.exists(partial):
            os.remove(partial)
        raise
    except urllib.error.HTTPError as exc:
        if os.path.exists(partial):
            os.remove(partial)
        raise HTTPException(404, f"Hugging Face answered {exc.code} for {body.repo}/{body.file}") from exc
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        if os.path.exists(partial):
            os.remove(partial)
        raise HTTPException(502, f"download failed: {exc}") from exc

    shutil.move(partial, target)
    return {"path": target, "bytes": written, "cached": False}


class ModelServeRequest(BaseModel):
    model_key: str
    image: str = Field(min_length=1, max_length=256)
    command: list[str] = Field(min_length=1, max_length=32)
    memory_limit_bytes: int = Field(gt=0)
    cpu_cores: float = Field(gt=0, le=64)
    port: int = Field(ge=1024, le=65535)


class CudaxInstallRequest(BaseModel):
    component: str
    images: list[str] = []

@app.post("/v1/cudax/install")
def install_cudax(body: CudaxInstallRequest) -> dict[str, Any]:
    """Pull the declared images of one CUDA-X component.

    Only references on `NESTBOX_CUDAX_IMAGES` are accepted, and only as an exact
    string: the control plane sends what the catalogue stored, and the runner
    refuses anything else instead of turning an operator action into an open
    registry proxy. An image already present is not pulled again.
    """
    if not CUDAX_COMPONENT.match(body.component):
        raise HTTPException(422, "component key is not a lowercase slug")
    pulled: list[str] = []
    for image in body.images[:4]:
        if image not in CUDAX_IMAGES:
            raise HTTPException(422, f"{image} is not an approved CUDA-X image")
        try:
            client.images.get(image)
        except NotFound:
            client.images.pull(image)
        pulled.append(image)
    return {"component": body.component, "pulled": pulled}

MODEL_SERVER_CONTAINER_PORT = 8080


def publish_target() -> tuple[str, int, int] | None:
    """Where and in which range this runner publishes a model server, if at all.

    A joined node serves a model on its own overlay address, because the control
    plane is on another machine and reaches it through WireGuard; it never joins
    the node's runtime network, so the container name means nothing there. The
    appliance's own runner leaves the publish variables unset and keeps the
    server reachable by name, as before. A malformed value is also "do not
    publish" rather than a crash: a model that serves only locally is still
    usable, and the install reports that it has no published address.
    """
    bind = os.getenv("NESTBOX_FORWARD_BIND", "").strip()
    raw = os.getenv("NESTBOX_MODEL_PUBLISH_RANGE", "").strip()
    if not bind or not raw:
        return None
    try:
        start, end = (int(part) for part in raw.split("-", 1))
    except ValueError:
        return None
    if not 1 <= start <= end <= 65535:
        return None
    return bind, start, end


def pick_port(bind: str, start: int, end: int, is_free) -> int | None:
    """The first port in `start..end` for which `is_free(bind, port)` holds.

    The choice is a pure function of the predicate, so a test can drive it
    without a socket, and `port_is_free` is what the runner passes in. `None`
    means the range is exhausted, which the caller reports instead of silently
    choosing a port the node does not publish.
    """
    for port in range(start, end + 1):
        if is_free(bind, port):
            return port
    return None


def port_is_free(bind: str, port: int) -> bool:
    """Whether this process can bind `bind:port` right now.

    Binding and closing is the only answer Docker itself would accept, and it
    catches a port the runner's own forward listeners already hold as well as
    one another process took on the host.
    """
    import socket

    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        try:
            probe.bind((bind, port))
        except OSError:
            return False
    return True


def published_address(item) -> str | None:
    """The overlay address a running model container is published on, if any.

    Read from Docker's record rather than from memory: a re-serve after a runner
    restart finds a container whose port this process never chose, and a
    container started before a node gained a publish range has no mapping at
    all, which is `None` and not an invented address.
    """
    ports = (item.attrs.get("NetworkSettings") or {}).get("Ports") or {}
    for spec, bindings in ports.items():
        if not spec.startswith(f"{MODEL_SERVER_CONTAINER_PORT}/") or not bindings:
            continue
        binding = bindings[0] or {}
        host_ip = str(binding.get("HostIp") or "")
        host_port = str(binding.get("HostPort") or "")
        if host_ip and host_ip not in ("0.0.0.0", "::") and host_port.isdigit():
            return f"{host_ip}:{host_port}"
    return None


@app.post("/v1/models/serve")
def serve_model(body: ModelServeRequest) -> dict[str, Any]:
    """Start the server container for one installed artefact.

    The image must be on `NESTBOX_MODEL_IMAGES`, an exact digest reference. The
    command is an argument vector, never a shell string; every absolute path in
    it must be the model file this runner mounted, so the argument list cannot
    be used to read something else.
    """
    key = model_key(body.model_key)
    if body.image not in MODEL_IMAGES:
        raise HTTPException(422, "image is not an approved model server image")

    directory = model_directory(key)
    if not os.path.isdir(directory):
        raise HTTPException(404, f"no artefact for {key}: {directory} does not exist")
    artefacts = sorted(name for name in os.listdir(directory) if name.endswith(".gguf"))
    if len(artefacts) != 1:
        raise HTTPException(409, f"expected exactly one .gguf for {key}, found {len(artefacts)}")
    model_path = f"{MODEL_CONTAINER_PATH}/{artefacts[0]}"

    for argument in body.command:
        if "\x00" in argument or len(argument) > 256:
            raise HTTPException(422, "invalid argument in the model server command")
        if argument.startswith("/") and argument != model_path:
            raise HTTPException(422, f"only {model_path} may be named as a path")

    name = model_container(key)
    try:
        existing = client.containers.get(name)
        existing.reload()
        if existing.status == "running":
            return {
                "name": name,
                "container_id": existing.id,
                "status": existing.status,
                "published": published_address(existing),
            }
        existing.remove(force=True)
    except NotFound:
        pass

    # A joined node publishes the server on its overlay address, because that is
    # the only way the control plane can reach it. The mapping goes into the
    # container at creation, so the first answer can already carry the address
    # the provider profile needs; an appliance-local server is not published and
    # stays reachable by container name on the runtime network.
    published: str | None = None
    ports: dict[str, Any] | None = None
    target = publish_target()
    if target is not None:
        bind, low, high = target
        chosen = pick_port(bind, low, high, port_is_free)
        if chosen is None:
            raise HTTPException(409, f"no free model port in {low}-{high} on {bind}")
        ports = {f"{MODEL_SERVER_CONTAINER_PORT}/tcp": (bind, chosen)}
        published = f"{bind}:{chosen}"

    ensure_network()
    try:
        client.images.get(body.image)
    except NotFound:
        client.images.pull(body.image)

    try:
        container = client.containers.run(
            body.image,
            body.command,
            name=name,
            detach=True,
            network=NETWORK,
            volumes={directory: {"bind": MODEL_CONTAINER_PATH, "mode": "ro"}},
            ports=ports,
            mem_limit=str(body.memory_limit_bytes),
            nano_cpus=int(body.cpu_cores * 1_000_000_000),
            pids_limit=256,
            cap_drop=["ALL"],
            security_opt=["no-new-privileges"],
            restart_policy={"Name": "unless-stopped"},
            labels={
                "nestbox.managed": "true",
                "nestbox.kind": "model",
                "nestbox.model": key,
            },
        )
    except DockerException as exc:
        raise HTTPException(400, f"the model server did not start: {exc}") from exc
    MODEL_PORTS[key] = body.port
    return {
        "name": name,
        "container_id": container.id,
        "status": container.status,
        "published": published,
    }


@app.get("/v1/models/serve/{key}")
def inspect_model(key: str) -> dict[str, Any]:
    name = model_container(model_key(key))
    try:
        item = client.containers.get(name)
    except NotFound as exc:
        raise HTTPException(404, f"no model server {name}") from exc
    item.reload()
    state = item.attrs.get("State", {})
    return {
        "name": name,
        "container_id": item.id,
        "status": item.status,
        "exit_code": state.get("ExitCode"),
        "error": state.get("Error", ""),
    }


@app.get("/v1/models/{key}/logs")
def model_logs(key: str, tail: int = 200) -> dict[str, Any]:
    """The last lines of a served model's container.

    A model server is not a sandbox, so this is its own endpoint: asking the
    sandbox route for `model-<key>` answers "sandbox not found", which reads
    like a missing model even when it is running.
    """
    key = model_key(key)
    name = model_container(key)
    try:
        item = client.containers.get(name)
    except NotFound:
        raise HTTPException(404, f"no model server {name}") from None
    tail = max(1, min(int(tail), 2000))
    try:
        body = item.logs(tail=tail)
    except DockerException as exc:
        raise HTTPException(400, f"the logs of {name} could not be read: {exc}") from exc
    text = body.decode("utf-8", "replace") if isinstance(body, bytes) else str(body)
    return {"name": name, "tail": tail, "logs": text}


@app.get("/v1/models/{key}/metrics")
def model_metrics(key: str) -> dict[str, Any]:
    """Read the served model's own metrics.

    llama.cpp exposes them on the port the container listens on when it was
    started with `--metrics`. The runner is on the same network, so it asks the
    container directly; a model that is not running, or that was started
    without metrics, is a 503 with the reason instead of an empty graph.
    """
    import urllib.error
    import urllib.request

    key = model_key(key)
    name = model_container(key)
    try:
        item = client.containers.get(name)
        item.reload()
    except NotFound:
        raise HTTPException(404, f"no model server {name}") from None
    port = MODEL_PORTS.get(key) or container_port(item)
    if port is None:
        raise HTTPException(503, f"{name} does not report a port to read metrics from")
    # The runner reaches model containers through the runtime network, not by
    # name: it is not attached to that network itself, so Docker's DNS does not
    # answer for it. The container's address on that network does.
    networks = (item.attrs.get("NetworkSettings") or {}).get("Networks") or {}
    address = (networks.get(NETWORK) or {}).get("IPAddress") or ""
    host = address or name
    url = f"http://{host}:{port}/metrics"
    try:
        with urllib.request.urlopen(url, timeout=5) as response:  # noqa: S310
            body = response.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            raise HTTPException(
                503,
                f"{name} answers {url} with 404: it was started without --metrics. "
                "Save a tuning change (or restart the model) to start it with metrics on.",
            ) from exc
        raise HTTPException(503, f"the model server answered {url} with {exc.code}") from exc
    except (urllib.error.URLError, OSError) as exc:
        raise HTTPException(
            503,
            f"the model server did not answer {url}: {exc}",
        ) from exc
    return {"model_key": key, "container": name, "metrics": parse_prometheus(body)}


@app.delete("/v1/models/serve/{key}")
def stop_model(key: str) -> dict[str, Any]:
    key = model_key(key)
    name = model_container(key)
    MODEL_PORTS.pop(key, None)
    try:
        item = client.containers.get(name)
    except NotFound:
        raise HTTPException(404, f"no model server {name}") from None
    try:
        item.remove(force=True)
    except DockerException as exc:
        raise HTTPException(400, f"the model server did not stop: {exc}") from exc
    return {"name": name, "stopped": True}


@app.get("/v1/sandboxes")
def list_sandboxes() -> list[dict[str, Any]]:
    return [details(item) for item in list_workloads(client)]


@app.get("/v1/sandboxes/{name}")
def inspect_sandbox(name: str) -> dict[str, Any]:
    try:
        return details(get_container(client, name))
    except NotFound as exc:
        raise HTTPException(404, "sandbox not found") from exc


# Registered before the `/{name}/{action}` lifecycle route on purpose: FastAPI
# matches routes in declaration order, and the catch-all would otherwise claim
# `/exec` and answer "unsupported lifecycle action".

class ExecRequest(BaseModel):
    """One command to run inside a sandbox.

    The command is an argument vector, never a shell string. A caller that
    wants a shell asks for ``["bash", "-lc", "..."]`` explicitly, which is
    visible in the audit record. The output is capped so a runaway command
    cannot fill the API's memory with a single response.
    """

    command: list[str] = Field(min_length=1, max_length=64)
    workdir: str | None = None
    timeout_seconds: int = Field(default=60, ge=1, le=600)
    max_output_bytes: int = Field(default=65536, ge=1024, le=1_048_576)


@app.post("/v1/sandboxes/{name}/exec")
def exec_in_sandbox(name: str, body: ExecRequest) -> dict[str, Any]:
    """Run one command in a running sandbox and return its output.

    Phase 1 is a request/response exec rather than an interactive PTY: it is
    what the smoke test, the console's "run" box, and a scheduled health probe
    need, and it keeps the transport to plain JSON. A streaming PTY is a
    separate, later endpoint.
    """
    for part in body.command:
        if "\x00" in part:
            raise HTTPException(422, "command arguments must not contain NUL")
    try:
        container = get_container(client, name)
    except NotFound as exc:
        raise HTTPException(404, "sandbox not found") from exc
    if container.status != "running":
        raise HTTPException(409, "sandbox is not running")

    if openshell_name(container.name):
        # Through the gateway, so the command runs inside the boundary.
        try:
            code, text = osh.exec_command(name, body.command, workdir=body.workdir, timeout=body.timeout_seconds)
        except osh.OpenShellError as exc:
            raise HTTPException(504 if "finish" in str(exc) else 400, str(exc)) from exc
        return {"name": name, "exit_code": code, "output": text[: body.max_output_bytes], "truncated": len(text) > body.max_output_bytes}

    import threading

    result: dict[str, Any] = {}

    def run() -> None:
        try:
            exit_code, output = container.exec_run(
                body.command,
                workdir=body.workdir,
                demux=False,
                tty=False,
                environment=None,
            )
            result["exit_code"] = exit_code
            result["output"] = output or b""
        except DockerException as exc:  # pragma: no cover - surfaced below
            result["error"] = str(exc)

    worker = threading.Thread(target=run, daemon=True)
    worker.start()
    worker.join(body.timeout_seconds)
    if worker.is_alive():
        raise HTTPException(504, f"command did not finish within {body.timeout_seconds}s")
    if "error" in result:
        raise HTTPException(400, f"exec failed: {result['error']}")

    raw: bytes = result.get("output", b"")
    truncated = len(raw) > body.max_output_bytes
    text = raw[: body.max_output_bytes].decode(errors="replace")
    return {
        "name": name,
        "exit_code": result.get("exit_code"),
        "output": text,
        "truncated": truncated,
    }


@app.post("/v1/sandboxes/{name}/{action}")
def lifecycle(name: str, action: str) -> dict[str, Any]:
    if action not in {"start", "stop", "restart", "remove"}:
        raise HTTPException(422, "unsupported lifecycle action")
    if action in {"start", "restart"}:
        # A deploy that recreates the proxy container drops its endpoint on the
        # sandbox network, and a sandbox that was working before the deploy would
        # come back with an unreachable proxy. A start is the natural moment to
        # repair the wiring, and the call is idempotent. Before the container
        # runs, so it never has a moment without a proxy.
        egress_state = prepare_egress(ensure_network())
        logger.info("egress for %s before %s: %s", name, action, egress_state)
    try:
        container = get_container(client, name)
        if openshell_name(container.name):
            try:
                osh.lifecycle(name, action)
            except osh.OpenShellError as exc:
                raise HTTPException(400, f"lifecycle action failed: {exc}") from exc
            if action == "remove":
                return {"name": name, "status": "deleted"}
            return details(osh.wait_for_container(_find_or_none, name) or container)
        if action == "remove":
            container.remove(force=True)
            return {"name": name, "status": "deleted"}
        getattr(container, action)()
        return details(container)
    except NotFound as exc:
        raise HTTPException(404, "sandbox not found") from exc
    except DockerException as exc:
        raise HTTPException(400, f"lifecycle action failed: {exc}") from exc


@app.get("/v1/sandboxes/{name}/logs")
def logs(name: str, tail: int = 200) -> dict[str, str | int]:
    if not 1 <= tail <= 1000:
        raise HTTPException(422, "tail must be between 1 and 1000")
    try:
        container = get_container(client, name)
        agent = container.logs(tail=tail, timestamps=True).decode(errors="replace")
        if not openshell_name(container.name):
            return {"name": name, "tail": tail, "logs": agent}
        # An OpenShell workload usually prints little itself; what happened to it is in the
        # supervisor's and the gateway's log. One list, oldest first, each line tagged.
        try:
            shell = osh.run(["logs", name, "-n", str(tail), "--source", "all"], timeout=20)
        except osh.OpenShellError as exc:
            shell = ""
            logger.warning("could not read the OpenShell log of %s: %s", name, exc)
        from app import log_merge

        return {"name": name, "tail": tail, "logs": log_merge.merge(agent, shell, tail)}
    except NotFound as exc:
        raise HTTPException(404, "sandbox not found") from exc
