"""Acceptance for the image-builder catalogues, repositories, MCP assignments and templates.

Runs on the appliance against the live API with a short-lived bearer token in
`api_tokens`. Every artefact it creates (recipes, templates, sandboxes, images)
is removed again; the token row is deleted in `finally`.

    python3 deploy/scripts/acceptance-image-builder.py

Scenarios, each a real build on this box:

1. **catalogue** — the live package index of a baseline answers.
2. **template** — save a selection, read it as default, apply it to a recipe.
3. **alpine** — an Alpine recipe installs a package from the apk catalogue and
   the verification finds it.
4. **repository** — a third-party APT repository with its signing key installs a
   package that the base image does not ship.
5. **mcp** — a recipe's MCP assignment is attached when the sandbox starts.
"""
from __future__ import annotations

import hashlib
import json
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:8080/api/v1"
TOKEN = "acc-" + secrets.token_hex(12)
FAILURES: list[str] = []


def psql(sql: str) -> None:
    subprocess.run(
        [
            "docker", "exec", "-i", "nestbox-postgres-1", "psql",
            "-U", "nestbox", "-d", "nestbox", "-v", "ON_ERROR_STOP=1",
        ],
        input=sql.encode(),
        check=True,
        capture_output=True,
    )


def wait_for_api(seconds: int = 240) -> None:
    """Wait until the control plane answers health again.

    The box is shared: a parallel deploy stops and recreates the API while this
    script runs. Waiting is what keeps that from reading as a feature failure.
    """
    start = time.time()
    while time.time() - start < seconds:
        try:
            with urllib.request.urlopen(BASE + "/healthz", timeout=5) as answer:
                if answer.status == 200:
                    return
        except OSError:
            pass
        time.sleep(5)
    raise SystemExit("the control API did not come back within the wait window")


def call(path: str, method: str = "GET", body=None, expect=None):
    data = None if body is None else json.dumps(body).encode()

    def attempt():
        request = urllib.request.Request(
            BASE + path,
            data=data,
            method=method,
            headers={"authorization": f"Bearer {TOKEN}", "content-type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=900) as answer:
                return answer.status, answer.read()
        except urllib.error.HTTPError as error:
            return error.code, error.read()

    # A deploy of the control plane closes every connection and leaves nginx
    # answering 502 for a moment; one retry keeps a parallel session's restart
    # from looking like a feature failure. A 503 from the API itself is a real
    # answer and is not retried.
    try:
        status, payload = attempt()
    except (ConnectionResetError, urllib.error.URLError):
        wait_for_api()
        status, payload = attempt()
    if status in {502, 504}:
        wait_for_api()
        status, payload = attempt()
    if expect is not None and status not in (expect if isinstance(expect, (set, tuple, list)) else {expect}):
        raise AssertionError(f"{method} {path} -> {status}: {payload[:400]!r}")
    try:
        return status, json.loads(payload or b"null")
    except json.JSONDecodeError:
        return status, payload


def check(name: str, condition: bool, detail: str) -> None:
    print(f"  {'ok  ' if condition else 'FAIL'} {name}: {detail}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def create_recipe(name: str, baseline: str, **extra) -> dict:
    body = {
        "name": name,
        "baseline_key": baseline,
        "tools": [],
        "agents": [],
        "resources": {"memory_limit": "1Gi", "cpu_limit": 1, "gpu_count": 0, "pids_limit": 256},
        "project_mounts": [],
        "node": None,
        **extra,
    }
    return call("/recipes", "POST", body, expect=201)[1]


def build(recipe: dict) -> dict:
    status, _ = call(f"/recipes/{recipe['id']}/build", "POST", {})
    if status not in {200, 202}:
        raise AssertionError(f"build -> {status}")
    for _ in range(180):
        time.sleep(5)
        fresh = call(f"/recipes/{recipe['id']}", expect=200)[1]
        if fresh["state"] in {"built", "failed"}:
            return fresh
    return fresh


def image_runs(image: str, entrypoint: str, argument: str) -> tuple[bool, str]:
    result = subprocess.run(
        ["docker", "run", "--rm", "--network", "none", "--entrypoint", entrypoint, image, argument],
        capture_output=True,
        text=True,
    )
    return result.returncode == 0, (result.stdout or result.stderr).strip()[:120]


def scenario_catalogue() -> None:
    print("catalogue")
    for baseline, query in (("ubuntu-24.04", "ripgrep"), ("rocky-9", "vim"), ("alpine-3.21", "nginx")):
        try:
            answer = call(f"/packages?baseline={baseline}&q={query}", expect=200)[1]
        except AssertionError:
            # The runner is recreated by a parallel deploy now and then; a live
            # index is a read, so asking once more is honest and cheap.
            wait_for_api()
            time.sleep(15)
            answer = call(f"/packages?baseline={baseline}&q={query}", expect=200)[1]
        names = [hit["name"] for hit in answer["items"]]
        check(
            f"{baseline} live index",
            answer["available"] and bool(names),
            f"{len(names)} hits, first {names[:2]}",
        )


def scenario_template() -> tuple[dict, dict]:
    print("template")
    name = "acc-tpl-" + secrets.token_hex(3)
    template = call(
        "/image-templates",
        "POST",
        {
            "name": name,
            "description": "acceptance run",
            "baseline_key": "ubuntu-24.04",
            "extra_packages": ["jq"],
            "repositories": [],
            "mcp_server_keys": [],
            "is_default": True,
            "is_shared": True,
        },
        expect=201,
    )[1]
    default = call("/image-templates/default", expect=200)[1]["template"] or {}
    check("default template", default.get("name") == name, default.get("name"))

    recipe = create_recipe("acc-tpl-recipe-" + secrets.token_hex(3), "ubuntu-24.04")
    applied = call(
        f"/recipes/{recipe['id']}/apply-template",
        "POST",
        {"template_id": template["id"]},
        expect=200,
    )[1]
    check(
        "apply template",
        applied["extra_packages"] == ["jq"] and applied["revision"] == 2 and applied["image_ref"] is None,
        f"revision {applied['revision']}, packages {applied['extra_packages']}, image {applied['image_ref']}",
    )
    built = build(applied)
    check("ubuntu build", built["state"] == "built", built["build_error"] or built["image_ref"])
    if built["state"] == "built":
        ok, output = image_runs(built["image_ref"], "jq", "--version")
        check("jq from the template", ok, output)
    return template, {**built, "id": recipe["id"]}


def scenario_alpine() -> dict | None:
    print("alpine")
    recipe = create_recipe(
        "acc-alpine-" + secrets.token_hex(3),
        "alpine-3.21",
        extra_packages=["jq"],
    )
    built = build(recipe)
    check("alpine build", built["state"] == "built", built["build_error"] or built["image_ref"])
    if built["state"] == "built":
        ok, output = image_runs(built["image_ref"], "jq", "--version")
        check("jq from the apk index", ok, output)
        return {**built, "id": recipe["id"]}
    return None


def scenario_repository() -> dict | None:
    print("repository")
    key = ""
    for attempt in range(3):
        try:
            with urllib.request.urlopen(
                "https://download.docker.com/linux/ubuntu/gpg", timeout=60
            ) as answer:
                key = answer.read().decode()
            break
        except OSError:
            if attempt == 2:
                raise
            time.sleep(5)
    check("signing key fetched", key.startswith("-----BEGIN PGP PUBLIC KEY BLOCK-----"), f"{len(key)} bytes")
    recipe = create_recipe(
        "acc-repo-" + secrets.token_hex(3),
        "ubuntu-24.04",
        repositories=[
            {
                "name": "docker",
                "url": "https://download.docker.com/linux/ubuntu",
                "suite": "noble",
                "components": ["stable"],
                "signing_key": key,
            }
        ],
        extra_packages=["docker-ce-cli"],
    )
    built = build(recipe)
    check("repository build", built["state"] == "built", built["build_error"] or built["image_ref"])
    if built["state"] == "built":
        ok, output = image_runs(built["image_ref"], "docker", "--version")
        check("package from the private repository", ok, output)
        return {**built, "id": recipe["id"]}
    return None


def scenario_mcp() -> None:
    print("mcp")
    recipe = create_recipe(
        "acc-mcp-" + secrets.token_hex(3),
        "ubuntu-24.04",
        mcp_server_keys=["nestbox-hub"],
    )
    built = build(recipe)
    check("mcp recipe build", built["state"] == "built", built["build_error"] or built["image_ref"])
    if built["state"] != "built":
        return
    sandbox_name = "acc-mcp-" + secrets.token_hex(3)
    created = call(
        f"/recipes/{recipe['id']}/instantiate",
        "POST",
        {"name": sandbox_name},
        expect=201,
    )[1]
    try:
        attached = call(f"/sandboxes/{created['id']}/mcp/servers", expect=200)[1]
        keys = [entry.get("server_key") or entry.get("key") for entry in attached["items"]] if isinstance(attached, dict) else []
        check("mcp attached at start", "nestbox-hub" in keys, str(keys))
    finally:
        call(f"/sandboxes/{created['id']}", "DELETE")


def main() -> int:
    digest = hashlib.sha256(TOKEN.encode()).hexdigest()
    psql(
        "INSERT INTO api_tokens (user_id, name, token_hash, expires_at) "
        f"SELECT id, 'acceptance', '{digest}', now() + interval '45 minutes' "
        "FROM users WHERE role = 'admin' ORDER BY created_at LIMIT 1;\n"
    )
    wait_for_api()
    recipes: list[str] = []
    templates: list[str] = []
    images: list[str] = []
    try:
        scenario_catalogue()
        template, ubuntu = scenario_template()
        templates.append(template["id"])
        recipes.append(ubuntu["id"])
        if ubuntu.get("image_ref"):
            images.append(ubuntu["image_ref"])
        for scenario in (scenario_alpine, scenario_repository):
            try:
                result = scenario()
            except Exception as failure:  # a scenario's failure must not hide the others
                check(scenario.__name__, False, str(failure)[:200])
                continue
            if result:
                recipes.append(result["id"])
                if result.get("image_ref"):
                    images.append(result["image_ref"])
        try:
            scenario_mcp()
        except Exception as failure:
            check("mcp", False, str(failure)[:200])
        return 1 if FAILURES else 0
    finally:
        for image in images:
            subprocess.run(["docker", "image", "rm", "-f", image], capture_output=True)
        for recipe in recipes:
            call(f"/recipes/{recipe}", "DELETE")
        for template in templates:
            call(f"/image-templates/{template}", "DELETE")
        psql(f"DELETE FROM api_tokens WHERE name = 'acceptance' AND token_hash = '{digest}';\n")
        if FAILURES:
            print("\nfailures:")
            for failure in FAILURES:
                print(" -", failure)


if __name__ == "__main__":
    sys.exit(main())
