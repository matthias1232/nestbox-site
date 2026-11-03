"""Snapshots of a persistent root, with restic, for moving work between nodes (ADR 0049).

One restic repository per data center lives on the control plane's rest-server and is encrypted
with a password that only the control plane and the nodes of that data center hold. A snapshot
is taken of a directory under this node's persistent base; a restore puts it under the same
kind of path on another node. The runner decides nothing about *which* root or *where*: the
control plane says so, and the runner refuses any path outside the persistent base.

Why restic and not rsync: the way back sends only what changed, the repository holds history
and survives the node (a rented machine can be deleted once its last snapshot is in), and the
same repository is the backup.
"""
from __future__ import annotations

import json
import logging
import os
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any

from fastapi import APIRouter, HTTPException
from pydantic import BaseModel, Field

logger = logging.getLogger("nestbox.runner.snapshots")
router = APIRouter()

BASE = Path(os.getenv("NESTBOX_PERSISTENT_BASE", "/var/lib/nestbox/persistent")).resolve()
TIMEOUT = int(os.getenv("NESTBOX_SNAPSHOT_TIMEOUT", "3600"))


def inside_base(path: str) -> Path:
    """The resolved path, or a refusal: nothing outside the persistent base is ever touched."""
    resolved = Path(path).resolve()
    if resolved == BASE or BASE not in resolved.parents:
        raise HTTPException(422, "that path is not inside the persistent base of this node")
    return resolved


def restic(repo: str, password: str, *args: str, check: bool = True) -> subprocess.CompletedProcess:
    """Run restic with the repository and password in its environment, never on its command line."""
    environment = {**os.environ, "RESTIC_REPOSITORY": repo, "RESTIC_PASSWORD": password, "RESTIC_CACHE_DIR": "/tmp/restic-cache"}
    try:
        return subprocess.run(
            ["restic", *args], env=environment, capture_output=True, text=True, timeout=TIMEOUT, check=check
        )
    except FileNotFoundError as exc:
        raise HTTPException(501, "restic is not installed on this node") from exc
    except subprocess.TimeoutExpired as exc:
        raise HTTPException(504, "restic took too long") from exc
    except subprocess.CalledProcessError as exc:
        # restic's stderr names the repository URL, which can carry a credential: say what failed,
        # not what the command line held.
        message = (exc.stderr or "").strip().splitlines()[-1:] or ["restic failed"]
        raise HTTPException(502, f"restic failed: {message[0][:300]}") from exc


def ensure_repository(repo: str, password: str) -> None:
    probe = restic(repo, password, "cat", "config", check=False)
    if probe.returncode == 0:
        return
    restic(repo, password, "init")


def parse_summary(stdout: str) -> dict[str, Any]:
    """The summary line of `restic backup --json`: the snapshot id and what was added."""
    summary: dict[str, Any] = {}
    for line in stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if item.get("message_type") == "summary":
            summary = item
    return summary


class BackupRequest(BaseModel):
    repo: str = Field(min_length=1, max_length=400)
    password: str = Field(min_length=8, max_length=200)
    path: str = Field(min_length=1, max_length=400)
    host: str = Field(default="nestbox", max_length=80)
    tags: list[str] = Field(default_factory=list, max_length=8)


class RestoreRequest(BaseModel):
    repo: str = Field(min_length=1, max_length=400)
    password: str = Field(min_length=8, max_length=200)
    snapshot: str = Field(min_length=6, max_length=80, pattern=r"^[A-Za-z0-9]+$")
    path: str = Field(min_length=1, max_length=400)
    # The original path the snapshot was taken at, so a restore can pick that folder out.
    source_path: str = Field(min_length=1, max_length=400)


@router.post("/v1/snapshots/backup")
def backup(body: BackupRequest) -> dict[str, Any]:
    """Snapshot one directory. The repository is created on first use."""
    target = inside_base(body.path)
    if not target.is_dir():
        raise HTTPException(404, "that directory does not exist on this node")
    ensure_repository(body.repo, body.password)
    arguments = ["backup", "--json", "--host", body.host, str(target)]
    for tag in body.tags:
        arguments[1:1] = ["--tag", tag]
    started = time.monotonic()
    result = restic(body.repo, body.password, *arguments)
    summary = parse_summary(result.stdout)
    if not summary.get("snapshot_id"):
        raise HTTPException(502, "restic did not report a snapshot")
    return {
        "snapshot": summary["snapshot_id"][:16],
        "bytes_added": int(summary.get("data_added", 0)),
        "bytes_total": int(summary.get("total_bytes_processed", 0)),
        "files": int(summary.get("total_files_processed", 0)),
        "seconds": round(time.monotonic() - started, 2),
    }


@router.post("/v1/snapshots/restore")
def restore(body: RestoreRequest) -> dict[str, Any]:
    """Put a snapshot's directory at `path`.

    What was there is not deleted: it is set aside as `<path>.replaced-<time>` so a restore onto
    a node that holds an older copy can be undone by hand, the same rule the tar restore follows.
    """
    target = inside_base(body.path)
    source = inside_base(body.source_path)
    aside = None
    if target.exists() and any(target.iterdir()):
        aside = target.with_name(f"{target.name}.replaced-{int(time.time())}")
        shutil.move(str(target), str(aside))
    target.mkdir(parents=True, exist_ok=True)
    restic(body.repo, body.password, "restore", f"{body.snapshot}:{source}", "--target", str(target))
    files = sum(1 for _ in target.rglob("*"))
    return {"restored_to": str(target), "set_aside": str(aside) if aside else None, "entries": files}


REPO_BASE = Path(os.getenv("NESTBOX_RESTIC_BASE", "/var/lib/nestbox/restic"))


class PruneRequest(BaseModel):
    # A directory name under the repository base, never a path: the runner only prunes repositories
    # the control plane's own snapshot server holds, and only through the filesystem.
    name: str = Field(min_length=1, max_length=80, pattern=r"^[A-Za-z0-9-]+$")
    password: str = Field(min_length=8, max_length=200)
    keep_last: int = Field(default=20, ge=1, le=1000)
    keep_daily: int = Field(default=14, ge=0, le=366)


@router.post("/v1/snapshots/prune")
def prune(body: PruneRequest) -> dict[str, Any]:
    """Forget old snapshots of one data center's repository and reclaim the space.

    The snapshot server is append-only, so no node can do this; it runs where the files are: on the
    control plane's own runner, against the directory the server writes.
    """
    repo = REPO_BASE / body.name
    if not repo.is_dir():
        raise HTTPException(404, "that repository does not exist on this node")
    arguments = ["forget", "--prune", "--group-by", "host,tags", "--keep-last", str(body.keep_last)]
    if body.keep_daily:
        arguments += ["--keep-daily", str(body.keep_daily)]
    result = restic(str(repo), body.password, *arguments)
    tail = [line for line in result.stdout.splitlines() if line.strip()][-3:]
    return {"repository": body.name, "summary": tail}
