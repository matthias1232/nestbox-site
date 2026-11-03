"""Run a script over SSH on a machine that is not a node yet (ADR 0057).

The credentials arrive with the request, are used for this call only, and are
never written anywhere: no log line, no file, no exception text. The endpoint
exists so that "enter the public IP of a new machine" works without opening a
port or copying a script by hand — the control plane sends `prepare.sh`, reads
the public key back out of the output, and then sends `apply.sh`.

Why the runner and not the control plane: this process already holds the
credentials of the node it runs on, and it is the one place that is allowed to
reach a machine outside the overlay. The control plane never speaks SSH.

The host key is not checked (`known_hosts=None`). That is deliberate and it is
the whole point of a first contact: the user typed the address, there is no
trusted key on this side yet, and refusing an unknown host would make the flow
impossible. What is trusted afterwards is the WireGuard key the new node proves
by answering on its overlay address.
"""
from __future__ import annotations

import asyncio
import logging
import re

import asyncssh
from fastapi import APIRouter, HTTPException
from pydantic import BaseModel, Field

logger = logging.getLogger("nestbox.runner.remote_ssh")
router = APIRouter()

# A name `sh` accepts on the left of `export`. Anything else is refused rather than quoted: an
# environment key with a `;` or a newline in it would be a way to run a second command.
SHELL_NAME = re.compile(r"^[A-Z_][A-Z0-9_]*$")

# Exec output is capped like the sandbox exec: one script cannot exhaust the API's memory, and the
# caller sees that it was cut instead of getting half a line that looks complete.
MAX_OUTPUT = 65_536


class SshRun(BaseModel):
    """One script run on a machine that is not a node yet."""

    host: str = Field(min_length=1, max_length=253)
    port: int = Field(default=22, ge=1, le=65535)
    user: str = Field(default="root", min_length=1, max_length=64)
    password: str | None = None
    private_key: str | None = None
    script: str = Field(min_length=1, max_length=200_000)
    env: dict[str, str] = Field(default_factory=dict)
    timeout_seconds: int = Field(default=900, ge=10, le=3600)


def quote(value: str) -> str:
    """One value as a POSIX shell single-quoted word.

    Single quotes have exactly one escape in a POSIX shell (`'` becomes `'\\''`), so a value with
    a quote, a newline or a `$` reaches the script as written instead of being expanded by the
    shell it is pasted into.
    """
    return "'" + value.replace("'", "'\\''") + "'"


def env_prefix(env: dict[str, str]) -> str:
    """`export K='v'` lines for a POSIX shell; every key must be a shell name.

    A key that is not a name is refused rather than quoted: `A; rm -rf /` on the left of `export`
    would be a second command, and no quoting of the value would prevent it.
    """
    lines = []
    for key, value in env.items():
        if not SHELL_NAME.match(key):
            raise ValueError(f"{key!r} is not a shell variable name")
        lines.append(f"export {key}={quote(value)}\n")
    return "".join(lines)


@router.post("/v1/remote/ssh-run")
async def ssh_run(body: SshRun) -> dict:
    """Run one script over SSH and answer with its exit code and its output."""
    if not body.password and not body.private_key:
        raise HTTPException(status_code=422, detail="a password or a private key is required")
    try:
        prefix = env_prefix(body.env)
    except ValueError as bad:
        raise HTTPException(status_code=422, detail=str(bad)) from None
    keys = None
    if body.private_key:
        try:
            keys = [asyncssh.import_private_key(body.private_key)]
        except (asyncssh.Error, ValueError):
            raise HTTPException(status_code=422, detail="the private key could not be read") from None
    try:
        async with asyncio.timeout(body.timeout_seconds):
            async with asyncssh.connect(body.host, port=body.port, username=body.user,
                                        password=body.password, client_keys=keys,
                                        known_hosts=None) as conn:
                result = await conn.run("sh -s", input=prefix + body.script, check=False)
    except (OSError, asyncssh.Error, TimeoutError) as failure:
        # The type only: a message can carry the host, the user name or a fingerprint, and this
        # response travels back through the control plane and into the console.
        logger.warning("ssh run on %s:%s failed: %s", body.host, body.port, type(failure).__name__)
        raise HTTPException(status_code=502, detail=f"ssh failed: {type(failure).__name__}") from None
    output = (result.stdout or "") + (result.stderr or "")
    # Address and outcome only: the script, its environment and the credentials never reach a log.
    logger.info("ssh run on %s:%s exited %s", body.host, body.port, result.exit_status)
    return {
        "exit_code": result.exit_status,
        "output": output[-MAX_OUTPUT:],
        "truncated": len(output) > MAX_OUTPUT,
    }
