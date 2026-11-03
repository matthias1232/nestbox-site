"""The certificate files a sandbox is given, and which modes get them.

A tool inside a sandbox trusts the deployment CA only if it can find it, and two
shapes are needed: the CA alone, and one bundle of the public roots plus the CA
for the tools that read a single `SSL_CERT_FILE`. Both files are built here,
outside the sandbox, from the public half of the certificate; the key is never
copied, so nothing inside a sandbox can sign for the proxy.

The decision is the egress mode, not the sandbox: a `direct` sandbox has no proxy
to trust, every mode that uses the proxy does.
"""
from __future__ import annotations

import os
from typing import Any

# Where a sandbox sees the files. Both are read-only binds of files this process
# wrote, so nothing inside a sandbox can change what its tools trust.
EGRESS_CA_TARGET = "/run/secrets/egress-ca.crt"
BUNDLE_TARGET = "/run/secrets/nestbox-ca-bundle.crt"
# The image's own trust store. Every supported base image ships one here; a host
# without it falls back to `certifi`, and a host with neither gives no bundle
# rather than a bundle that trusts only Nestbox.
PUBLIC_ROOTS = "/etc/ssl/certs/ca-certificates.crt"


def publish_ca_bundle(public_ca: str | None, roots: str | None = None) -> str | None:
    """Public roots plus the egress CA, for tools that read one bundle file.

    The result sits next to the published CA, world-readable, so the sandbox user
    can read it. It is written to a temporary file and renamed, because a create
    that reads a half-written bundle would trust nothing at all. `roots` exists
    so a test does not depend on the host's trust store; production passes None.
    """
    if not public_ca:
        return None
    roots_path = roots or PUBLIC_ROOTS
    if not os.path.exists(roots_path):
        try:
            import certifi  # noqa: PLC0415

            candidate = certifi.where()
        except Exception:  # noqa: BLE001
            return None
        if not os.path.exists(candidate):
            return None
        roots_path = candidate
    target = os.path.join(os.path.dirname(public_ca), "bundle.crt")
    tmp = target + ".tmp"
    with open(roots_path, "rb") as src_roots, open(public_ca, "rb") as src_ca, open(tmp, "wb") as out:
        out.write(src_roots.read())
        out.write(b"\n")
        out.write(src_ca.read())
    os.chmod(tmp, 0o644)
    os.replace(tmp, target)
    return target


def ca_mounts(egress_mode: str, public_ca: str | None, bundle: str | None) -> list[dict[str, Any]]:
    """The read-only CA mounts a sandbox gets: every mode that uses the proxy.

    `direct` is the one mode with no proxy in the path, and a sandbox that is
    given no CA keeps failing closed: its tools reject the interception instead
    of trusting it. A missing bundle (no public roots on this host) only drops
    the bundle mount, not the CA.
    """
    if egress_mode == "direct" or not public_ca:
        return []
    mounts: list[dict[str, Any]] = [
        {"type": "bind", "source": public_ca, "target": EGRESS_CA_TARGET, "read_only": True}
    ]
    if bundle:
        mounts.append({"type": "bind", "source": bundle, "target": BUNDLE_TARGET, "read_only": True})
    return mounts
