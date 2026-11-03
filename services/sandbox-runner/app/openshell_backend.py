"""Sandboxes on the NVIDIA OpenShell gateway.

The gateway owns the sandbox: it creates the workload and its supervisor, applies
the policy, and decides every network request. This module is the thin part of the
runner that speaks to it, through the pinned `openshell` CLI (`-o json` where the
CLI offers it). It holds no policy and makes no decision of its own.

Secrets never travel in argv: values are handed to the CLI through its own
environment (`--env-from`), because argv is visible in `ps`.
"""

from __future__ import annotations

import json
import logging
import os
import subprocess
import time
from typing import Any

logger = logging.getLogger("nestbox.openshell")

CLI = os.getenv("OPENSHELL_BIN", "openshell")
DEFAULT_TIMEOUT = 120.0


class OpenShellError(RuntimeError):
    """The gateway refused or could not be reached; the message is the CLI's."""


def run(args: list[str], *, timeout: float = DEFAULT_TIMEOUT, env: dict[str, str] | None = None) -> str:
    merged = {**os.environ, **(env or {})}
    try:
        done = subprocess.run(
            [CLI, *args], capture_output=True, text=True, timeout=timeout, env=merged, check=False
        )
    except FileNotFoundError as exc:
        raise OpenShellError("the openshell CLI is not installed in the runner") from exc
    except subprocess.TimeoutExpired as exc:
        raise OpenShellError(f"openshell {args[0]} did not finish within {timeout:.0f}s") from exc
    if done.returncode != 0:
        text = " ".join(part.strip(" │├─╰×") for part in (done.stderr or done.stdout).strip().splitlines())
        raise OpenShellError(" ".join(text.split())[:900] or f"openshell exited {done.returncode}")
    return done.stdout


def status() -> dict[str, Any]:
    """Gateway reachability, version and authentication, as facts."""
    try:
        out = run(["status", "-o", "json"], timeout=15)
        return {"connected": True, **json.loads(out)}
    except (OpenShellError, ValueError) as exc:
        return {"connected": False, "error": str(exc)}


def get(name: str) -> dict[str, Any] | None:
    try:
        return json.loads(run(["sandbox", "get", name, "-o", "json"], timeout=20))
    except OpenShellError as exc:
        if "not found" in str(exc).lower():
            return None
        raise


def phase_to_status(phase: str) -> str:
    """OpenShell's phase, as the container-style status the control plane reads."""
    return {"Ready": "running", "Provisioning": "created", "Starting": "created"}.get(phase, "exited")


def memory_quantity(value: str) -> str:
    """A Docker-style limit (`1073741824b`, `1g`, `512m`) as a Kubernetes quantity."""
    text = value.strip().lower().removesuffix("b")
    if text.isdigit():
        return text
    unit = text[-1:]
    if unit in {"k", "m", "g", "t"} and text[:-1].isdigit():
        return f"{text[:-1]}{unit.upper()}i"
    return value


# The filesystem rules of every sandbox this runner creates. The gateway's own
# default allows writing only /tmp and /dev/null, which is too little for what
# Nestbox puts in a sandbox: a terminal that survives a reload needs a
# pseudo-terminal (/dev/ptmx, /dev/pts), Chrome and the VNC server need shared
# memory (/dev/shm), and /dev/tty and /dev/zero are read by ordinary tools. This
# section is read once, when the sandbox starts, so it has to be given here.
FILESYSTEM_POLICY: dict[str, Any] = {
    "include_workdir": True,
    "read_only": [
        "/bin", "/usr", "/lib", "/proc", "/dev/urandom", "/dev/random", "/etc", "/var/log",
        # Tools installed under /opt, the keyboard data and dbus machine id, what
        # Chrome reads from /sys, and the node CA Nestbox mounts for inspecting modes.
        "/opt", "/var/lib", "/sys", "/run/secrets",
    ],
    "read_write": [
        "/tmp", "/var/tmp", "/dev/null", "/dev/zero", "/dev/tty",
        "/dev/ptmx", "/dev/pts", "/dev/shm",
        # The X server compiles the keyboard map here; without it Xtigervnc dies
        # with "Failed to activate virtual core keyboard".
        "/var/lib/xkb",
    ],
}


def base_policy_file() -> str:
    """Write the startup policy to a private temp file and return its path."""
    import tempfile

    document = {
        "version": 1,
        "filesystem_policy": FILESYSTEM_POLICY,
        "landlock": {"compatibility": "best_effort"},
        "network_policies": {},
    }
    with tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False) as handle:
        json.dump(document, handle)  # JSON is YAML
        return handle.name


def create(
    *,
    name: str,
    image: str,
    command: list[str],
    environment: dict[str, str],
    memory_limit: str,
    cpu_limit: float,
    mounts: list[dict[str, Any]],
    labels: dict[str, str],
    policy_file: str | None = None,
    providers: list[str] | None = None,
) -> dict[str, Any]:
    args = [
        "sandbox", "create", "--name", name, "--from", image,
        "--cpu", f"{cpu_limit:g}", "--memory", memory_quantity(memory_limit),
        "--detach", "--restart-policy", "always", "--no-tty", "--no-auto-providers",
        "--no-credential-warnings", "-o", "json",
    ]
    for key, value in labels.items():
        args += ["--label", f"{key}={value}"]
    env: dict[str, str] = {}
    for index, (key, value) in enumerate(sorted(environment.items())):
        holder = f"NB_SECRET_{index}"
        env[holder] = value
        args += ["--env-from", f"{key}={holder}"]
    if mounts:
        args += ["--driver-config-json", json.dumps({"docker": {"mounts": mounts}})]
    if policy_file:
        args += ["--policy", policy_file]
    for provider in providers or []:
        args += ["--provider", provider]
    args += ["--", *command]
    created_policy = None
    if not policy_file:
        created_policy = policy_file = base_policy_file()
        args[args.index("--") : args.index("--")] = ["--policy", policy_file]
    try:
        out = run(args, timeout=300, env=env)
    finally:
        if created_policy:
            try:
                os.unlink(created_policy)
            except OSError:
                pass
    try:
        return json.loads(out)
    except ValueError:
        return {"name": name}


def lifecycle(name: str, action: str) -> None:
    if action == "restart":
        run(["sandbox", "stop", name], timeout=90)
        run(["sandbox", "start", name], timeout=120)
    elif action == "remove":
        run(["sandbox", "delete", name], timeout=120)
    else:
        run(["sandbox", action, name], timeout=120)


def wait_for_container(find, name: str, seconds: float = 30.0) -> Any | None:
    """Poll until the workload container exists (the gateway creates it async)."""
    deadline = time.time() + seconds
    while time.time() < deadline:
        found = find(name)
        if found is not None:
            return found
        time.sleep(1)
    return None


def exec_command(name: str, command: list[str], *, workdir: str | None, timeout: int) -> tuple[int, str]:
    args = ["sandbox", "exec", "-n", name, "--no-tty", "--timeout", str(timeout)]
    if workdir:
        args += ["--workdir", workdir]
    args += ["--", *command]
    try:
        done = subprocess.run([CLI, *args], capture_output=True, timeout=timeout + 10, check=False)
    except subprocess.TimeoutExpired as exc:
        raise OpenShellError(f"command did not finish within {timeout}s") from exc
    return done.returncode, (done.stdout + done.stderr).decode(errors="replace")


def set_policy(name: str, document: dict[str, Any]) -> None:
    """`openshell policy set`, with the document written to a private temp file."""
    import tempfile

    current = get(name)
    if current is None:
        raise OpenShellError("sandbox not found on the gateway")
    # Startup-only sections (filesystem, Landlock, process) cannot change on a live
    # sandbox, so keep the sandbox's own and replace just the network policies.
    merged = {key: value for key, value in (current.get("policy") or {}).items() if key != "network_policies"}
    merged["version"] = 1
    merged["network_policies"] = document.get("network_policies", {})
    with tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False) as handle:
        json.dump(merged, handle)  # JSON is YAML
        path = handle.name
    try:
        run(["policy", "set", name, "--policy", path, "--wait", "--timeout", "45"], timeout=75)
    finally:
        os.unlink(path)


_VERSION: dict[str, Any] = {"value": None, "at": 0.0}


def gateway_version() -> str:
    """The gateway's version, cached for a minute (it is read for every listing)."""
    if time.time() - _VERSION["at"] > 60 or _VERSION["value"] is None:
        _VERSION["value"] = status().get("version") or ""
        _VERSION["at"] = time.time()
    return _VERSION["value"]
