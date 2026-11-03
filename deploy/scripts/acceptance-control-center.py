#!/usr/bin/env python3
"""Acceptance of the Control Center routes and console bundle against the appliance.

    python3 deploy/scripts/acceptance-control-center.py \\
        --base http://192.168.55.50:8080 --token <ADMIN_BEARER_TOKEN>

Every check prints ``ok <name>: <message>``, ``WARN <name>: <message>`` or
``FAIL <name>: <message>``. The exit code is 1 when at least one check failed.
Standard library only; no secret is ever printed. The browser part is the
checklist in ``acceptance-control-center-browser.md``.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import time
import urllib.error
import urllib.request

DEFAULT_BASE = "http://192.168.55.50:8080"
NIL_UUID = "00000000-0000-0000-0000-000000000000"
BASE = DEFAULT_BASE
TOKEN = ""
FAILURES: list[str] = []
WARNINGS: list[str] = []
PASSED: list[str] = []
SANDBOXES: list[str] = []


def raw(path: str, method: str = "GET", body: object | None = None, auth: bool = True, timeout: int = 60) -> tuple[int, bytes]:
    """One HTTP call against the base URL; an HTTP error is an answer, not an exception."""
    headers = {"accept": "*/*"}
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        headers["content-type"] = "application/json"
    if auth:
        headers["authorization"] = f"Bearer {TOKEN}"
    request = urllib.request.Request(BASE + path, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as answer:
            return answer.status, answer.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def api(path: str, method: str = "GET", body: object | None = None, timeout: int = 60) -> tuple[int, object]:
    """One call under /api/v1; returns (status, parsed JSON or {"raw": ...})."""
    status, payload = raw("/api/v1" + path, method, body, timeout=timeout)
    try:
        return status, json.loads(payload or b"null")
    except json.JSONDecodeError:
        return status, {"raw": payload[:200].decode("utf-8", "replace")}


def short(value: object) -> str:
    """A short secret-free rendering for a message."""
    text = value if isinstance(value, str) else json.dumps(value, sort_keys=True, ensure_ascii=True)
    return text[:160]


def run(name: str, function) -> None:
    """Run one check; it returns a message, raises AssertionError for FAIL or Warning for WARN."""
    try:
        message = function()
    except Warning as warning:
        WARNINGS.append(name)
        print(f"WARN {name}: {warning}")
    except AssertionError as failure:
        FAILURES.append(name)
        print(f"FAIL {name}: {failure}")
    except Exception as failure:  # noqa: BLE001 - one broken check must not stop the run
        FAILURES.append(name)
        print(f"FAIL {name}: exception {failure}"[:240])
    else:
        PASSED.append(name)
        print(f"ok {name}: {message}")


def expect(status: int, good: set[int], payload: object) -> None:
    """Fail unless the status is one of `good`."""
    assert status in good, f"HTTP {status}, wanted {sorted(good)}: {short(payload)}"


def items_of(payload: object) -> list[dict]:
    """The `items` list of a list answer."""
    items = payload.get("items") if isinstance(payload, dict) else None
    assert isinstance(items, list), f"no items list: {short(payload)}"
    return [row for row in items if isinstance(row, dict)]


# ---------------------------------------------------------------------------
# Checks 1-8: routes that need no sandbox
# ---------------------------------------------------------------------------

OPENAPI_PATHS = [
    "/inventory/tree", "/search", "/sandboxes/bulk", "/models/{key}/install-bulk",
    "/model-clusters", "/model-clusters/{id}", "/model-clusters/{id}/members",
    "/model-clusters/{id}/members/{install_id}", "/nodes/{id}/summary", "/nodes/{id}/drain",
    "/nodes/quick", "/alerts", "/sandboxes/{id}/mounts", "/policies/preview",
    "/policies/{id}/compiled", "/sandboxes/{id}/quick", "/permissions",
]


def check_health() -> str:
    """Check 1: healthz answers and openapi.json lists every C3-C13 path."""
    status, payload = api("/healthz")
    expect(status, {200}, payload)
    status, spec = api("/openapi.json")
    expect(status, {200}, spec)
    paths = spec.get("paths") if isinstance(spec, dict) else None
    assert isinstance(paths, dict), "openapi.json has no paths"
    missing = [path for path in OPENAPI_PATHS if not any(key.endswith(path) for key in paths)]
    assert not missing, f"missing in openapi: {missing}"
    return f"healthz 200, {len(OPENAPI_PATHS)} paths listed"


def check_tree() -> str:
    """Check 2: the inventory tree answers in all three views."""
    kinds = {}
    for view in ("infrastructure", "workloads", "models"):
        status, payload = api(f"/inventory/tree?view={view}")
        expect(status, {200}, payload)
        root = payload.get("root") if isinstance(payload, dict) else None
        assert isinstance(root, dict), f"{view}: no root"
        kinds[view] = root.get("kind")
    assert kinds["infrastructure"] == "organization", f"infrastructure root is {kinds['infrastructure']!r}"
    return f"root kinds {kinds}"


def local_node() -> dict:
    """The first node of GET /nodes (the local one on the box)."""
    status, payload = api("/nodes")
    expect(status, {200}, payload)
    rows = items_of(payload) if isinstance(payload, dict) else [r for r in payload if isinstance(r, dict)]
    assert rows, "no node in /nodes"
    local = [row for row in rows if "local" in str(row.get("name", "")).lower()]
    return (local or rows)[0]


def check_search() -> str:
    """Check 3: search finds the local node with a /node/ path."""
    status, payload = api("/search?q=local&kind=node")
    expect(status, {200}, payload)
    hits = [row for row in items_of(payload) if row.get("kind") == "node" and str(row.get("path", "")).startswith("/node/")]
    assert hits, f"no node hit with a /node/ path: {short(payload)}"
    return f"{len(hits)} node hit(s), first {hits[0].get('name')}"


def check_bulk_sandboxes() -> str:
    """Check 4: bulk sandbox actions validate and a bogus id fails on the job."""
    status, payload = api("/sandboxes/bulk", "POST", {"ids": [], "action": "stop", "params": {}})
    expect(status, {422}, payload)
    status, payload = api("/sandboxes/bulk", "POST", {"ids": [NIL_UUID], "action": "stop", "params": {}})
    expect(status, {202}, payload)
    job_id = payload.get("job_id") if isinstance(payload, dict) else None
    assert job_id, f"no job_id: {short(payload)}"
    job: object = {}
    for _ in range(30):
        status, job = api(f"/jobs/{job_id}")
        if status == 200 and isinstance(job, dict) and (job.get("job") or job).get("status") in {"completed", "failed", "succeeded", "done"}:
            break
        time.sleep(2)
    if isinstance(job, dict) and isinstance(job.get("job"), dict):
        job = job["job"]
    failed = job.get("failed") if isinstance(job, dict) else None
    assert failed == 1, f"job failed counter is {failed!r}: {short(job)}"
    return "empty ids 422, bogus id gives a job with failed == 1"


def check_bulk_models() -> str:
    """Check 5: model installs list and bulk install validates its targets."""
    status, payload = api("/models")
    expect(status, {200}, payload)
    rows = payload if isinstance(payload, list) else items_of(payload)
    keys = [str(row.get("key")) for row in rows if isinstance(row, dict) and row.get("key")]
    assert keys, "no model in the catalogue"
    key = keys[0]
    status, payload = api("/models/installs")
    expect(status, {200}, payload)
    status, payload = api(f"/models/{key}/install-bulk", "POST", {"nodes": [], "site_id": None, "environment_wide": False, "force": False})
    expect(status, {422}, payload)
    return f"install list 200, empty targets 422 for {key}"


def check_clusters() -> str:
    """Check 6: the cluster list answers and a bad strategy is refused."""
    status, payload = api("/model-clusters")
    expect(status, {200}, payload)
    count = len(items_of(payload))
    status, payload = api("/model-clusters", "POST", {"name": "acc-bad", "model_key": "none", "strategy": "bogus"})
    expect(status, {422}, payload)
    return f"{count} cluster(s), bad strategy 422"


def check_node_summary() -> str:
    """Check 7: the node summary carries tunnel and sandboxes."""
    node = local_node()
    status, payload = api(f"/nodes/{node['id']}/summary")
    expect(status, {200}, payload)
    assert isinstance(payload, dict) and "tunnel" in payload and "sandboxes" in payload, f"missing keys: {short(sorted(payload) if isinstance(payload, dict) else payload)}"
    return f"summary of {node.get('name')} has tunnel and sandboxes"


def check_alerts() -> str:
    """Check 8: the alert list answers."""
    status, payload = api("/alerts?window=15m")
    expect(status, {200}, payload)
    return f"{len(items_of(payload))} alert(s) in 15m"


# ---------------------------------------------------------------------------
# Check 9: one throw-away sandbox
# ---------------------------------------------------------------------------


def make_sandbox() -> str:
    """Create the throw-away ubuntu-dev sandbox used by check 9."""
    name = f"cc{time.strftime('%H%M')}{int(time.time()) % 100}"
    status, payload = api("/sandboxes", "POST", {"name": name, "pack_key": "ubuntu-dev", "memory_limit": "1Gi", "cpu_limit": 1.0, "pids_limit": 256}, timeout=300)
    assert status == 201 and isinstance(payload, dict) and payload.get("id"), f"create: HTTP {status}: {short(payload)}"
    SANDBOXES.append(str(payload["id"]))
    return str(payload["id"])


def check_sandbox_routes() -> str:
    """Check 9: mounts, quick recording, policy preview and sandbox permissions."""
    sandbox_id = make_sandbox()
    status, payload = api(f"/sandboxes/{sandbox_id}/mounts")
    expect(status, {200}, payload)
    for value in ("off", "full"):
        status, payload = api(f"/sandboxes/{sandbox_id}/quick", "PUT", {"what": "recording", "value": value, "minutes": None})
        expect(status, {200}, payload)
    spec = {"network_policies": {"acc-rule": {"endpoints": [{"host": "example.org", "port": 443, "protocol": "rest"}]}}}
    status, payload = api("/policies/preview", "POST", {"spec": spec, "sandbox_id": sandbox_id})
    expect(status, {200}, payload)
    hot = payload.get("hot") if isinstance(payload, dict) else None
    assert hot == ["network_policies"], f"hot is {hot!r}: {short(payload)}"
    status, payload = api(f"/permissions?scope_type=sandbox&scope_id={sandbox_id}")
    expect(status, {200}, payload)
    return "mounts 200, recording off/full 200, preview hot == [network_policies], permissions 200"


# ---------------------------------------------------------------------------
# Checks 10-13: static files and the bundle
# ---------------------------------------------------------------------------


def check_help() -> str:
    """Check 10: the help index has 14 entries and one document is served."""
    status, payload = raw("/help/index.json", auth=False)
    expect(status, {200}, "")
    entries = json.loads(payload)
    assert isinstance(entries, list), "index.json is not a list"
    assert len(entries) == 14, f"{len(entries)} entries, wanted 14"
    status, _ = raw("/help/deploy-on-many-nodes.md", auth=False)
    expect(status, {200}, "")
    return "14 entries, deploy-on-many-nodes.md served"


def check_illustration() -> str:
    """Check 11: the login illustration is served (warning only)."""
    status, _ = raw("/illustrations/login.webp", auth=False)
    if status != 200:
        raise Warning(f"HTTP {status}; fine when D03 was skipped")
    return "login.webp served"


BUNDLE_STRINGS = ["inventory/tree", "`search${", "model-clusters", "alerts", "/quick", "permissions", "help/index.json", "viewBox", "Allow 15 min"]


def check_bundle() -> str:
    """Check 12: the built JavaScript carries the strings that prove the packages are in."""
    status, page = raw("/", auth=False)
    expect(status, {200}, "")
    scripts = re.findall(rb'src="(/assets/[^"]+\.js)"', page)
    assert scripts, "index.html names no /assets/*.js"
    text = ""
    queue = [s.decode() for s in scripts]
    seen: set[str] = set()
    while queue and len(seen) < 60:
        path = queue.pop(0)
        if path in seen:
            continue
        seen.add(path)
        status, body = raw(path, auth=False)
        if status != 200:
            continue
        chunk = body.decode("utf-8", "replace")
        text += chunk
        # Lazy chunks are named inside the entry file as "./Name-hash.js".
        queue.extend("/assets/" + name for name in re.findall(r'\./([A-Za-z0-9_.-]+\.js)', chunk))
    missing = [needle for needle in BUNDLE_STRINGS if needle not in text]
    assert not missing, f"not found in {len(seen)} script(s): {missing}"
    return f"all {len(BUNDLE_STRINGS)} strings found in {len(seen)} script(s)"


def check_spa_fallback() -> str:
    """Check 13: a console path serves the shell HTML."""
    status, body = raw(f"/dc/{NIL_UUID}", auth=False)
    expect(status, {200}, "")
    assert b"<html" in body.lower() or b"<!doctype" in body.lower(), "not HTML"
    return "SPA fallback serves HTML"


def teardown() -> None:
    """Delete the throw-away sandbox(es)."""
    for sandbox_id in list(SANDBOXES):
        api(f"/sandboxes/{sandbox_id}/actions", "POST", {"action": "stop"}, timeout=180)
        status = 0
        for _ in range(4):
            status, _ = api(f"/sandboxes/{sandbox_id}", "DELETE", timeout=180)
            if status not in {409, 423, 503}:
                break
            time.sleep(5)
        print(f"{'ok' if status in {200, 204, 404} else 'WARN'} teardown: sandbox {sandbox_id[:8]} delete HTTP {status}")


def main() -> int:
    """Parse arguments, run every check, print the verdict."""
    global BASE, TOKEN
    parser = argparse.ArgumentParser(description="Control Center acceptance against the appliance.")
    parser.add_argument("--base", default=DEFAULT_BASE)
    parser.add_argument("--token", required=True, help="Admin bearer token.")
    arguments = parser.parse_args()
    BASE = arguments.base.rstrip("/")
    TOKEN = arguments.token.strip()
    print(f"Control Center acceptance against {BASE}")
    try:
        run("1 health+openapi", check_health)
        run("2 inventory tree", check_tree)
        run("3 search", check_search)
        run("4 bulk sandboxes", check_bulk_sandboxes)
        run("5 bulk models", check_bulk_models)
        run("6 clusters", check_clusters)
        run("7 node summary", check_node_summary)
        run("8 alerts", check_alerts)
        run("9 sandbox routes", check_sandbox_routes)
        run("10 help", check_help)
        run("11 illustration", check_illustration)
        run("12 bundle", check_bundle)
        run("13 spa fallback", check_spa_fallback)
    finally:
        teardown()
    print(f"\npassed {len(PASSED)}, warnings {len(WARNINGS)}, failed {len(FAILURES)}")
    for name in FAILURES:
        print(f"  FAIL {name}")
    return 1 if FAILURES else 0


if __name__ == "__main__":
    sys.exit(main())
