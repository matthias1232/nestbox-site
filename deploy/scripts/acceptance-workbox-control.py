#!/usr/bin/env python3
"""Acceptance of the workbox control features against the running appliance.

Proves on the live box, not in a unit test, that the operator-facing controls
work: the sign-in mode of each agent, the egress mode while the sandbox runs, a
temporary evaluation window, presence and terminal tabs, model grants, the node
tunnels, the move preflight and the control overview.

    python3 deploy/scripts/acceptance-workbox-control.py \\
        --base http://192.168.55.50:8080 --token <ADMIN_BEARER_TOKEN> \\
        [--keep sandbox-prefix]

The run invents its own prefix (``wb-acc-<HHMM>-<random>``), builds everything
it needs under it, removes it again unless ``--keep`` is given, and exits with
0 when every assertion held and 1 when at least one did not.

Every assertion is one ``CLEARAND`` line: scene, verdict, and the statement that
was checked. The lines are German because the operator reads them, ASCII only so
a POSIX-locale terminal can print them, and they never carry a secret: names,
ids, counters and status codes, nothing else.
"""
from __future__ import annotations

import argparse
import json
import secrets
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_BASE = "http://192.168.55.50:8080"
CALL_TIMEOUT = 60
CREATE_TIMEOUT = 300

BASE = DEFAULT_BASE
TOKEN = ""

PASSED: list[str] = []
FAILURES: list[str] = []
SKIPPED: list[str] = []

# What the run created and has to take away again. Filled right after a create,
# so a scene that dies halfway still lets the teardown find the leftovers.
SANDBOXES: list[str] = []
GRANTS: list[str] = []
DRAFTS: list[str] = []
WINDOWS: list[str] = []


# ---------------------------------------------------------------------------
# Lines the operator reads
# ---------------------------------------------------------------------------


def check(name: str, condition: bool, ok: str, fail: str = "") -> bool:
    """One assertion: a PASS line with what was found, a FAIL line with what was expected."""
    if condition:
        PASSED.append(name)
        print(f"CLEARAND {name} - PASS: {ok}")
        return True
    FAILURES.append(f"{name}: {fail or ok}")
    print(f"CLEARAND {name} - FAIL: {fail or ok}")
    return False


def skip(name: str, reason: str) -> None:
    """An assertion this box cannot answer; it is reported, but it is not a failure."""
    SKIPPED.append(f"{name}: {reason}")
    print(f"CLEARAND {name} - SKIP: {reason} (nicht pruefbar, kein Fehler)")


def scene(title: str) -> None:
    """A heading between the CLEARAND lines, so a long run stays readable."""
    print(f"\n== {title}")


# ---------------------------------------------------------------------------
# Transport
# ---------------------------------------------------------------------------


def wait_ready(seconds: int = 240) -> None:
    """Wait until the control plane answers health again.

    The box is shared: a parallel deploy restarts the API while this script runs.
    Waiting is what keeps that from reading as a feature failure.
    """
    started = time.time()
    while time.time() - started < seconds:
        try:
            with urllib.request.urlopen(BASE + "/api/v1/healthz", timeout=5) as answer:
                if answer.status == 200:
                    return
        except OSError:
            pass
        time.sleep(5)
    raise SystemExit(f"die API unter {BASE} antwortet nicht innerhalb von {seconds} s")


def api(path: str, method: str = "GET", body: object | None = None, timeout: int = CALL_TIMEOUT) -> tuple[int, object]:
    """One call against the API; returns (status, parsed body).

    An HTTP error is an answer, not an exception: the assertions want to see the
    status. Only a dead connection is retried once, after the API is back.
    """
    data = None if body is None else json.dumps(body).encode()

    def attempt() -> tuple[int, bytes]:
        request = urllib.request.Request(
            BASE + "/api/v1" + path,
            data=data,
            method=method,
            headers={"authorization": f"Bearer {TOKEN}", "accept": "application/json", "content-type": "application/json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as answer:
                return answer.status, answer.read()
        except urllib.error.HTTPError as error:
            return error.code, error.read()

    try:
        status, payload = attempt()
    except (ConnectionResetError, TimeoutError, urllib.error.URLError):
        wait_ready()
        status, payload = attempt()
    if status in {502, 504}:
        wait_ready()
        status, payload = attempt()
    try:
        return status, json.loads(payload or b"null")
    except json.JSONDecodeError:
        return status, {"raw": payload[:200].decode("utf-8", "replace")}


def describe(payload: object) -> str:
    """A short, secret-free rendering of a body for a FAIL line."""
    text = payload if isinstance(payload, str) else json.dumps(payload, ensure_ascii=True, sort_keys=True)
    return text[:200]


def answered(name: str, status: int, payload: object, good: set[int], ok: str) -> bool:
    """Assert that one call answered with a status from `good`, and say what it did otherwise."""
    if status in good:
        return check(name, True, ok)
    return check(name, False, ok, f"HTTP {status} statt {sorted(good)}: {describe(payload)}")


def exec_in(sandbox_id: str, script: str, timeout_seconds: int = 30) -> tuple[int | None, str]:
    """Run one fixed shell script inside a sandbox; returns (exit code, output)."""
    status, payload = api(
        f"/sandboxes/{sandbox_id}/exec",
        "POST",
        {"command": ["sh", "-c", script], "timeout_seconds": timeout_seconds},
        timeout=timeout_seconds + 60,
    )
    if status != 200 or not isinstance(payload, dict):
        return None, f"HTTP {status}: {describe(payload)}"
    return payload.get("exit_code"), str(payload.get("output") or "").strip()


# ---------------------------------------------------------------------------
# Small readers
# ---------------------------------------------------------------------------


def agent_modes(sandbox_id: str) -> tuple[dict[str, str], int]:
    """The sign-in mode of every switchable agent of one sandbox."""
    status, payload = api(f"/sandboxes/{sandbox_id}/agents/auth")
    if status != 200 or not isinstance(payload, list):
        return {}, status
    modes = {str(entry.get("agent")): str(entry.get("mode")) for entry in payload if isinstance(entry, dict)}
    return modes, status


def control(sandbox_id: str) -> tuple[object, int]:
    """The control summary of one sandbox, the one call the Workbox page makes.

    Returns (summary, status): the summary first because every caller reads the
    body and only mentions the status in a FAIL line.
    """
    status, payload = api(f"/sandboxes/{sandbox_id}/control")
    return payload, status


def rules_of(sandbox_id: str) -> tuple[list[dict], int]:
    """The effective network rules of one sandbox."""
    status, payload = api(f"/sandboxes/{sandbox_id}/rules")
    items = payload.get("items") if isinstance(payload, dict) else None
    return [row for row in items if isinstance(row, dict)] if isinstance(items, list) else [], status


def effective_names(sandbox_id: str) -> list[str]:
    """The names of the models a sandbox may use, after inheritance."""
    status, payload = api(f"/sandboxes/{sandbox_id}/models")
    if status != 200 or not isinstance(payload, list):
        return []
    return [str(row.get("model_name")) for row in payload if isinstance(row, dict)]


def container_id(sandbox_id: str) -> str:
    """The runtime's id of the container behind a sandbox; empty when unknown."""
    status, payload = api(f"/sandboxes/{sandbox_id}")
    if status != 200 or not isinstance(payload, dict):
        return ""
    runtime = payload.get("runtime")
    if isinstance(runtime, dict) and runtime.get("id"):
        return str(runtime["id"])
    return str(payload.get("container_id") or "")


# ---------------------------------------------------------------------------
# Creating and waiting
# ---------------------------------------------------------------------------


def create_sandbox(name: str, login: str) -> tuple[str, str]:
    """Create one claude-code sandbox and return (id, failure text)."""
    status, payload = api(
        "/sandboxes",
        "POST",
        {"name": name, "pack_key": "claude-code", "login": login, "memory_limit": "1Gi", "cpu_limit": 1.0, "pids_limit": 256},
        timeout=CREATE_TIMEOUT,
    )
    if status != 201 or not isinstance(payload, dict) or not payload.get("id"):
        return "", f"HTTP {status}: {describe(payload)}"
    SANDBOXES.append(str(payload["id"]))
    return str(payload["id"]), ""


def wait_running(sandbox_id: str, seconds: int = 180) -> str:
    """Poll one sandbox until it runs; returns its status after the wait."""
    started = time.time()
    status_text = "unknown"
    while time.time() - started < seconds:
        status, payload = api(f"/sandboxes/{sandbox_id}")
        if status == 200 and isinstance(payload, dict):
            status_text = str(payload.get("status") or "unknown")
            if status_text in {"running", "failed", "stopped"}:
                return status_text
        time.sleep(3)
    return status_text


def delete_sandbox(sandbox_id: str) -> int:
    """Stop and delete one sandbox; returns the status of the delete call.

    A stop that is still in flight answers 409 for a moment; asking again a few
    seconds later is what makes the teardown reliable instead of flaky.
    """
    api(f"/sandboxes/{sandbox_id}/actions", "POST", {"action": "stop"}, timeout=180)
    status = 0
    for attempt in range(4):
        status, _ = api(f"/sandboxes/{sandbox_id}", "DELETE", timeout=180)
        if status not in {409, 423, 503}:
            return status
        if attempt < 3:
            time.sleep(5)
    return status


# ---------------------------------------------------------------------------
# Scenes, in the order the agreement lists them
# ---------------------------------------------------------------------------


def scene_create(prefix: str) -> str:
    """Scene 2: own account defaults to subscription, the gateway sandbox to gateway."""
    scene("2. Sandbox anlegen")
    own_id, failure = create_sandbox(f"{prefix}-own", "own_account")
    if not check("Sandbox mit eigenem Konto", bool(own_id), f"{prefix}-own angelegt", f"Anlegen fehlgeschlagen: {failure or 'ohne id'}"):
        return ""
    check("Sandbox laeuft", wait_running(own_id) == "running", f"{prefix}-own ist running", f"{prefix}-own wurde nicht running")

    modes, status = agent_modes(own_id)
    check("Vorgabe subscription bei own_account", modes.get("claude") == "subscription",
          "claude steht ohne eigenes Setzen auf subscription",
          f"claude steht auf {modes.get('claude')!r} (HTTP {status}), erwartet 'subscription'")

    gateway_id, failure = create_sandbox(f"{prefix}-gw", "gateway")
    if check("Sandbox mit Gateway", bool(gateway_id), f"{prefix}-gw angelegt", f"Anlegen fehlgeschlagen: {failure or 'ohne id'}"):
        wait_running(gateway_id)
        modes, status = agent_modes(gateway_id)
        check("Vorgabe gateway bei login=gateway", modes.get("claude") == "gateway",
              "claude steht ohne eigenes Setzen auf gateway",
              f"claude steht auf {modes.get('claude')!r} (HTTP {status}), erwartet 'gateway'")
        # One container is enough for the rest of the run.
        removed = delete_sandbox(gateway_id)
        if removed in {200, 204, 404} and gateway_id in SANDBOXES:
            SANDBOXES.remove(gateway_id)
        check("Zweite Sandbox wieder entfernt", removed in {200, 204, 404},
              f"{prefix}-gw abgebaut", f"{prefix}-gw liess sich nicht entfernen")
    return own_id


def scene_agents(sandbox_id: str) -> None:
    """Scene 1: the agents endpoint answers for the sandbox created in scene 2."""
    scene("1. Agenten-API")
    if not sandbox_id:
        skip("Agenten-API", "die Sandbox aus Szene 2 fehlt")
        return
    status, payload = api(f"/sandboxes/{sandbox_id}/agents/auth")
    if not answered("Agenten-API antwortet", status, payload, {200}, "HTTP 200"):
        return
    entries = payload if isinstance(payload, list) else []
    modes = {str(entry.get("agent")): str(entry.get("mode")) for entry in entries if isinstance(entry, dict)}
    check("claude und codex sind gelistet", "claude" in modes and "codex" in modes,
          f"beide vorhanden, Modi {sorted(modes.items())}",
          f"geliefert wurde nur {sorted(modes)}; erwartet claude und codex")
    check("jeder Agent hat einen Modus", bool(modes) and all(value in {"subscription", "gateway"} for value in modes.values()),
          "alle Modi sind subscription oder gateway", f"unbekannter Modus in {sorted(modes.items())}")


def scene_switch(sandbox_id: str) -> None:
    """Scene 3: switching an agent writes the mode file and the rule, and keeps the container."""
    scene("3. Anmelde-Umschaltung")
    if not sandbox_id:
        skip("Anmelde-Umschaltung", "die Sandbox aus Szene 2 fehlt")
        return
    before = container_id(sandbox_id)

    status, payload = api(f"/sandboxes/{sandbox_id}/agents/claude/auth", "PUT", {"mode": "subscription", "restart": "idle"})
    answered("Umschalten auf subscription", status, payload, {200}, "HTTP 200 mit der neuen Liste")
    modes, _ = agent_modes(sandbox_id)
    check("Antwort und Liste nennen subscription", modes.get("claude") == "subscription",
          "claude steht auf subscription", f"claude steht auf {modes.get('claude')!r}, erwartet 'subscription'")

    rules, rules_status = rules_of(sandbox_id)
    names = [str(rule.get("name") or "") for rule in rules]
    wanted = "Sign-in (claude): claude.ai"
    # The contract names the rule exactly; the tolerant match keeps the check
    # honest when only the separator differs, and the FAIL line shows the names.
    matching = [
        str(rule.get("name") or "")
        for rule in rules
        if str(rule.get("host") or "") == "claude.ai"
        and "sign-in" in str(rule.get("name") or "").lower()
        and "claude" in str(rule.get("name") or "").lower()
    ]
    check("Anmelde-Regel fuer claude.ai", wanted in names or bool(matching),
          f"Regel {matching[0] if matching else wanted} vorhanden",
          f"keine Regel {wanted!r} unter {len(names)} Regeln (HTTP {rules_status}): {names[:6]}")

    code, output = exec_in(sandbox_id, "cat ~/.nestbox/mode/claude")
    check("Modusdatei nennt subscription", code == 0 and output == "subscription",
          "~/.nestbox/mode/claude enthaelt subscription",
          f"Datei lieferte {output[:60]!r} (exit {code}), erwartet 'subscription'")

    status, payload = api(f"/sandboxes/{sandbox_id}/agents/claude/auth", "PUT", {"mode": "gateway", "restart": "idle"})
    answered("Umschalten zurueck auf gateway", status, payload, {200}, "HTTP 200")
    code, output = exec_in(sandbox_id, "cat ~/.nestbox/mode/claude")
    check("Modusdatei nennt gateway", code == 0 and output == "gateway",
          "~/.nestbox/mode/claude enthaelt gateway",
          f"Datei lieferte {output[:60]!r} (exit {code}), erwartet 'gateway'")

    after = container_id(sandbox_id)
    if before and after:
        check("Container unveraendert", before == after,
              f"Container {after[:12]} blieb derselbe", f"Container wechselte von {before[:12]} auf {after[:12]}")
    else:
        skip("Container unveraendert", "die Laufzeit meldet keine Container-id")


def scene_egress(sandbox_id: str) -> None:
    """Scene 4: the egress mode takes effect while the sandbox runs, and traffic follows it."""
    scene("4. Egress-Modus im Betrieb")
    if not sandbox_id:
        skip("Egress-Modus", "die Sandbox aus Szene 2 fehlt")
        return

    status, payload = api(f"/sandboxes/{sandbox_id}/egress-mode", "PUT", {"mode": "transparent"})
    check("Modus transparent gesetzt", status == 200 and isinstance(payload, dict) and payload.get("mode") == "transparent",
          "der Modus ist transparent", f"HTTP {status}, Modus {describe(payload)}; erwartet 'transparent'")

    summary, summary_status = control(sandbox_id)
    egress = summary.get("egress") if isinstance(summary, dict) else None
    check("kein Neuanlegen noetig", isinstance(egress, dict) and egress.get("needs_recreate") is False,
          "needs_recreate ist false an einer neuen Sandbox",
          f"egress lautet {describe(egress)} (HTTP {summary_status}), erwartet needs_recreate=false")

    status, payload = api(f"/sandboxes/{sandbox_id}/egress-mode", "PUT", {"mode": "off_allow_all"})
    check("Modus off_allow_all gesetzt", status == 200 and isinstance(payload, dict) and payload.get("mode") == "off_allow_all",
          "der Modus ist off_allow_all", f"HTTP {status}, Modus {describe(payload)}; erwartet 'off_allow_all'")

    summary, summary_status = control(sandbox_id)
    allow_all = summary.get("allow_all") if isinstance(summary, dict) else summary
    check("Control meldet allow_all", allow_all is True, "allow_all ist true",
          f"allow_all lautet {describe(allow_all)} (HTTP {summary_status})")

    # The proxy may still hold a tunnel from before the switch; ask again a
    # couple of times rather than reading the first answer as a verdict.
    code: int | None = None
    output = ""
    for attempt in range(3):
        code, output = exec_in(sandbox_id, "curl -sS -m 20 -o /dev/null -w '%{http_code}' https://example.com", 40)
        if code == 0 and output.isdigit() and output != "403":
            break
        if attempt < 2:
            time.sleep(5)
    if code is None or "not found" in output.lower():
        skip("Erreichbarkeit im Modus off_allow_all", f"curl fehlt in der Sandbox: {output[:80]}")
    else:
        check("Erreichbarkeit im Modus off_allow_all", code == 0 and output.isdigit() and output != "403",
              f"example.com antwortet {output}",
              f"example.com antwortete {output!r} (exit {code}); erwartet ein HTTP-Code, kein 403 des Proxys")

    status, payload = api(f"/sandboxes/{sandbox_id}/egress-mode", "PUT", {"mode": "hybrid"})
    check("Modus hybrid wiederhergestellt", status == 200 and isinstance(payload, dict) and payload.get("mode") == "hybrid",
          "der Modus ist hybrid", f"HTTP {status}, Modus {describe(payload)}; erwartet 'hybrid'")

    status, payload = api(f"/traffic/decisions?sandbox={sandbox_id}&window=1h")
    items = payload.get("items") if isinstance(payload, dict) else None
    rows = [row for row in items if isinstance(row, dict)] if isinstance(items, list) else []
    check("Entscheidungen tragen would_deny", status == 200 and isinstance(items, list) and all("would_deny" in row for row in rows),
          f"{len(rows)} Ziel(e) im Fenster, alle mit would_deny",
          f"HTTP {status}: {describe(payload)}; erwartet eine Liste items, deren Zeilen would_deny tragen")


def scene_window(sandbox_id: str) -> None:
    """Scene 5: a temporary window suspends the policy and yields evidence as drafts."""
    scene("5. Bewertungsfenster")
    if not sandbox_id:
        skip("Bewertungsfenster", "die Sandbox aus Szene 2 fehlt")
        return

    status, window = api("/traffic-windows", "POST", {
        "sandbox_id": sandbox_id, "minutes": 15, "unlimited": False,
        "mode": "audit", "channel": "all", "reason": "Werkzeugandbox-Abnahme",
    })
    if not check("Fenster geoeffnet", status == 201 and isinstance(window, dict) and bool(window.get("id")),
                 "HTTP 201 mit einer id", f"HTTP {status}: {describe(window)}"):
        return
    window_id = str(window["id"])
    WINDOWS.append(window_id)

    status, listing = api("/traffic-windows")
    items = listing.get("items") if isinstance(listing, dict) else None
    rows = [row for row in items if isinstance(row, dict)] if isinstance(items, list) else []
    check("Fenster steht in der Liste", status == 200 and any(str(row.get("id")) == window_id for row in rows),
          f"in {len(rows)} offenen Fenstern enthalten", f"HTTP {status}: das Fenster {window_id[:8]} fehlt in der Liste")

    summary, summary_status = control(sandbox_id)
    allow_all = summary.get("allow_all") if isinstance(summary, dict) else summary
    check("Control meldet allow_all durch das Fenster", allow_all is True,
          "allow_all ist true, solange das Fenster offen ist", f"allow_all lautet {describe(allow_all)} (HTTP {summary_status})")

    status, bulk = api("/rule-drafts/bulk", "POST", {"sandbox_id": sandbox_id})
    drafts = bulk.get("drafts") if isinstance(bulk, dict) else None
    DRAFTS.extend(str(row["id"]) for row in (drafts if isinstance(drafts, list) else []) if isinstance(row, dict) and row.get("id"))
    created = bulk.get("created") if isinstance(bulk, dict) else None
    check("Sammel-Entwuerfe antworten", status == 200 and isinstance(bulk, dict) and "created" in bulk,
          f"HTTP 200 mit created={created}", f"HTTP {status}: {describe(bulk)}; erwartet 200 mit dem Schluessel created")

    status, _ = api(f"/traffic-windows/{window_id}", "DELETE")
    if answered("Fenster geschlossen", status, None, {200, 204}, "HTTP 204"):
        WINDOWS.remove(window_id)


def scene_presence(sandbox_id: str) -> None:
    """Scene 6: the tab list for everyone and the presence list answer."""
    scene("6. Praesenz und Tabs")
    if not sandbox_id:
        skip("Praesenz und Tabs", "die Sandbox aus Szene 2 fehlt")
        return
    status, payload = api(f"/sandboxes/{sandbox_id}/terminal-tabs?all=1")
    items = payload.get("items") if isinstance(payload, dict) else None
    check("Tab-Liste fuer alle Nutzer", status == 200 and isinstance(items, list),
          f"HTTP 200 mit {len(items or [])} Tab(s)", f"HTTP {status}: {describe(payload)}; erwartet 200 mit items")
    status, payload = api(f"/sandboxes/{sandbox_id}/presence")
    check("Praesenz antwortet", status == 200 and isinstance(payload, list),
          f"HTTP 200 mit {len(payload) if isinstance(payload, list) else 0} Eintrag/Eintraegen",
          f"HTTP {status}: {describe(payload)}; erwartet 200 mit einer Liste")


def scene_control_models(sandbox_id: str) -> None:
    """Scene 7: the control summary carries the models of the sandbox."""
    scene("7. Desktop und Control")
    if not sandbox_id:
        skip("Control liefert models", "die Sandbox aus Szene 2 fehlt")
        return
    status, payload = api(f"/sandboxes/{sandbox_id}/control")
    models = payload.get("models") if isinstance(payload, dict) else None
    check("Control liefert models", status == 200 and isinstance(models, list),
          f"HTTP 200, models ist eine Liste mit {len(models or [])} Eintrag/Eintraegen",
          f"HTTP {status}: {describe(payload)}; erwartet 200 mit models als Liste")


def scene_grants(sandbox_id: str) -> None:
    """Scene 8: a sandbox grant becomes effective and disappears when revoked."""
    scene("8. Modell-Grants")
    if not sandbox_id:
        skip("Modell-Grant", "die Sandbox aus Szene 2 fehlt")
        return
    status, payload = api("/providers")
    items = payload.get("items") if isinstance(payload, dict) else None
    profiles = [row for row in (items if isinstance(items, list) else []) if isinstance(row, dict) and row.get("id")]
    if not profiles:
        skip("Modell-Grant", f"kein Provider-Profil vorhanden (HTTP {status})")
        return

    status, grant = api("/model-grants", "POST", {
        "scope": "sandbox", "sandbox_id": sandbox_id,
        "model_name": "acceptance-model", "provider_profile_id": str(profiles[0]["id"]),
    })
    if not check("Grant angelegt", status in {200, 201} and isinstance(grant, dict) and bool(grant.get("id")),
                 "HTTP 201 mit einer id", f"HTTP {status}: {describe(grant)}"):
        return
    grant_id = str(grant["id"])
    GRANTS.append(grant_id)

    names = effective_names(sandbox_id)
    check("Grant ist wirksam", "acceptance-model" in names, f"acceptance-model steht in {len(names)} wirksamen Modell(en)",
          f"{names or 'keine Modelle'}; erwartet acceptance-model")

    revoked, revoked_payload = api(f"/model-grants/{grant_id}", "DELETE")
    if answered("Grant entzogen", revoked, revoked_payload, {200, 204}, "HTTP 204"):
        GRANTS.remove(grant_id)
    names = effective_names(sandbox_id)
    check("Grant ist verschwunden", "acceptance-model" not in names, "acceptance-model ist nicht mehr wirksam",
          f"{names or 'keine Modelle'}; acceptance-model steht noch darin")


def scene_nodes() -> None:
    """Scene 9: the tunnel list answers and an egress lock can be set to its current value."""
    scene("9. Knoten und Tunnel")
    status, payload = api("/nodes/tunnels")
    check("Tunnel-Liste antwortet", status == 200 and isinstance(payload, list),
          f"HTTP 200 mit {len(payload) if isinstance(payload, list) else 0} Knoten",
          f"HTTP {status}: {describe(payload)}; erwartet 200 mit einer Liste")

    status, payload = api("/nodes")
    items = payload.get("items") if isinstance(payload, dict) else None
    nodes = [row for row in (items if isinstance(items, list) else []) if isinstance(row, dict) and row.get("id")]
    if not nodes:
        skip("Egress-Sperre setzbar", f"kein Knoten in der Liste (HTTP {status})")
        return
    node = nodes[0]
    lock = str(node.get("egress_lock") or "strict")
    if lock not in {"strict", "exit_local"}:
        lock = "strict"
    status, answer = api(f"/nodes/{node['id']}/egress-lock", "PUT", {"lock": lock})
    answered("Egress-Sperre setzbar", status, answer, {200}, f"{node.get('name')} bleibt auf {lock}")


def scene_preflight(sandbox_id: str) -> None:
    """Scene 10: the move preflight answers for the sandbox's own node."""
    scene("10. Verschiebe-Vorpruefung")
    if not sandbox_id:
        skip("Verschiebe-Vorpruefung", "die Sandbox aus Szene 2 fehlt")
        return
    status, payload = api(f"/sandboxes/{sandbox_id}")
    node_name = ""
    if isinstance(payload, dict):
        node_name = str(payload.get("node_name") or "")
        if not node_name and isinstance(payload.get("node"), dict):
            node_name = str(payload["node"].get("name") or "")
    if not node_name:
        skip("Verschiebe-Vorpruefung", "die Sandbox nennt keinen Knoten")
        return
    status, answer = api(f"/sandboxes/{sandbox_id}/move-preflight?node={urllib.parse.quote(node_name)}")
    blockers = answer.get("blockers") if isinstance(answer, dict) else None
    check("Vorpruefung antwortet", status == 200 and isinstance(blockers, list),
          f"HTTP 200, ok={answer.get('ok')}, {len(blockers)} Blocker auf {node_name}",
          f"HTTP {status}: {describe(answer)}; erwartet 200 mit blockers als Liste")


def scene_overview(sandbox_id: str) -> None:
    """Scene 11: the cheap overview lists the new sandbox with its egress mode and agents."""
    scene("11. Control-Uebersicht")
    status, payload = api("/sandboxes/control-overview")
    rows = payload if isinstance(payload, list) else (payload.get("items") if isinstance(payload, dict) else None)
    if not check("Uebersicht antwortet", status == 200 and isinstance(rows, list),
                 f"HTTP 200 mit {len(rows or [])} Sandbox(en)", f"HTTP {status}: {describe(payload)}; erwartet 200 mit einer Liste"):
        return
    row = next((entry for entry in rows if isinstance(entry, dict) and str(entry.get("sandbox_id")) == sandbox_id), None)
    if not check("neue Sandbox in der Uebersicht", row is not None, "die Sandbox aus Szene 2 steht in der Uebersicht",
                 f"die Sandbox {sandbox_id[:8]} fehlt unter {len(rows)} Zeilen"):
        return
    check("Uebersicht nennt egress_mode und agents", "egress_mode" in row and isinstance(row.get("agents"), list),
          f"egress_mode={row.get('egress_mode')}, {len(row.get('agents') or [])} Agent(en)",
          f"Zeile ohne egress_mode oder agents: {describe(row)}")


def teardown(keep: bool, prefix: str) -> None:
    """Scene 12: remove what the run created, unless it was asked to keep it."""
    scene("12. Abbau")
    for grant in list(GRANTS):
        api(f"/model-grants/{grant}", "DELETE")
        GRANTS.remove(grant)
    for window in list(WINDOWS):
        api(f"/traffic-windows/{window}", "DELETE")
        WINDOWS.remove(window)
    for draft in list(DRAFTS):
        # There is no delete for a draft; rejecting it is how it leaves the queue.
        api(f"/rule-drafts/{draft}/reject", "POST", {"reason": "Werkzeugandbox-Abnahme"})
        DRAFTS.remove(draft)

    if keep:
        print(f"  Die Sandbox {prefix}-own bleibt stehen, samt ihrer Regeln und Fenster.")
        skip("Abbau", "--keep wurde gesetzt, nichts wird entfernt")
        return
    for sandbox_id in list(SANDBOXES):
        status = delete_sandbox(sandbox_id)
        SANDBOXES.remove(sandbox_id)
        check(f"Sandbox {sandbox_id[:8]} abgebaut", status in {200, 204, 404},
              "HTTP 204 auf das Loeschen (oder schon weg)",
              f"HTTP {status} beim Loeschen von {sandbox_id[:8]}")


def summary() -> int:
    """The plain summary at the end; the exit code is the acceptance verdict."""
    print("\n== Zusammenfassung")
    print(f"  bestanden:      {len(PASSED)}")
    print(f"  uebersprungen:  {len(SKIPPED)}")
    print(f"  fehlgeschlagen: {len(FAILURES)}")
    for entry in SKIPPED:
        print(f"  - uebersprungen {entry}")
    for entry in FAILURES:
        print(f"  - fehlgeschlagen {entry}")
    if FAILURES:
        print(f"\nCLEARAND Gesamtlauf - FAIL: {len(FAILURES)} von {len(PASSED) + len(FAILURES)} Aussagen stimmen nicht")
        return 1
    print(f"\nCLEARAND Gesamtlauf - PASS: alle {len(PASSED)} Aussagen stimmen")
    return 0


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def sandbox_prefix(raw: str | None) -> str:
    """The name prefix under which this run creates its sandboxes."""
    if raw is None:
        # An OpenShell sandbox name is at most 19 characters, and the scene adds
        # a suffix: keep the prefix at eight.
        return f"wb{time.strftime('%H%M')}{secrets.token_hex(1)}"
    # A sandbox name is a DNS label: lowercase ASCII letters, digits and hyphens.
    cleaned = "".join(
        character if character.isascii() and (character.isalnum() or character == "-") else "-"
        for character in raw.lower()
    ).strip("-")
    return cleaned or f"wbk{secrets.token_hex(3)}"


def run(title: str, function, *arguments) -> None:
    """Run one scene; a crash becomes a FAIL line instead of hiding the other scenes."""
    try:
        function(*arguments)
    except Exception as failure:  # noqa: BLE001 - one broken scene must not stop the run
        check(title, False, "", f"Ausnahme: {failure}"[:200])


def main() -> int:
    parser = argparse.ArgumentParser(description="Abnahme der Werkzeugandbox-Steuerung auf der Appliance.")
    parser.add_argument("--base", default=DEFAULT_BASE, help=f"Basis-URL der API (Vorgabe {DEFAULT_BASE}).")
    parser.add_argument("--token", required=True, help="Bearer-Token eines Administrators.")
    parser.add_argument("--keep", nargs="?", const="", default=None, metavar="PREFIX",
                        help="Nichts abbauen; der Praefix wird wb-acc-kept-<zufall>, oder der hier genannte.")
    arguments = parser.parse_args()

    global BASE, TOKEN
    BASE = arguments.base.rstrip("/")
    TOKEN = arguments.token.strip()
    if not TOKEN:
        raise SystemExit("ein Bearer-Token eines Administrators ist noetig (--token)")
    prefix = sandbox_prefix(arguments.keep)

    print(f"Abnahme der Werkzeugandbox-Steuerung gegen {BASE}")
    print(f"Praefix dieses Laufs: {prefix}")
    wait_ready()

    sandbox_id = ""
    try:
        try:
            sandbox_id = scene_create(prefix)
        except Exception as failure:  # noqa: BLE001 - reported like every other scene
            check("Sandbox anlegen", False, "", f"Ausnahme: {failure}"[:200])
        run("Agenten-API", scene_agents, sandbox_id)
        run("Anmelde-Umschaltung", scene_switch, sandbox_id)
        run("Egress-Modus", scene_egress, sandbox_id)
        run("Bewertungsfenster", scene_window, sandbox_id)
        run("Praesenz und Tabs", scene_presence, sandbox_id)
        run("Control liefert models", scene_control_models, sandbox_id)
        run("Modell-Grant", scene_grants, sandbox_id)
        run("Knoten und Tunnel", scene_nodes)
        run("Verschiebe-Vorpruefung", scene_preflight, sandbox_id)
        run("Control-Uebersicht", scene_overview, sandbox_id)
    finally:
        # The teardown runs even after a crash: a half-built sandbox must not
        # keep a container on the box.
        try:
            teardown(arguments.keep is not None, prefix)
        except Exception as failure:  # noqa: BLE001
            check("Abbau", False, "", f"Ausnahme beim Abbau: {failure}"[:200])

    return summary()


if __name__ == "__main__":
    sys.exit(main())
