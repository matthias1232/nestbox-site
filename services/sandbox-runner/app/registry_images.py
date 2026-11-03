"""Images on this node, and keeping them in step with the control plane's registry (ADR 0048).

A joined node pulls what the control plane assigns it from the registry the control plane runs
(`NESTBOX_IMAGE_REGISTRY`, reached over the overlay). Images keep the name the rest of the product
knows them by (`nestbox-claude-code:local`): the registry holds the same name, and a pulled image
is tagged back to it. So nothing above this module learns that an image came from elsewhere.

The runner pulls only from that registry and only names that pass the same allow rule as a
sandbox start: an assignment cannot make a node fetch an arbitrary image from the internet.
"""
from __future__ import annotations

import logging
import os
from typing import Any

import docker
from docker.errors import APIError, DockerException, ImageNotFound
from fastapi import APIRouter, HTTPException
from pydantic import BaseModel, Field

from app.images import image_allowed

logger = logging.getLogger("nestbox.runner.images")
router = APIRouter()

REGISTRY = os.getenv("NESTBOX_IMAGE_REGISTRY", "").strip().rstrip("/")


def _client() -> Any:
    return docker.from_env()


def split_reference(image: str) -> tuple[str, str]:
    """`repo:tag` -> (repo, tag). A digest reference keeps its digest as the tag part."""
    if "@" in image:
        repo, digest = image.split("@", 1)
        return repo, digest
    last = image.rsplit("/", 1)[-1]
    if ":" in last:
        repo, tag = image.rsplit(":", 1)
        return repo, tag
    return image, "latest"


def registry_reference(image: str) -> str | None:
    """Where the registry keeps `image`, or None when this node has no registry to pull from."""
    if not REGISTRY:
        return None
    if image.startswith(f"{REGISTRY}/"):
        return image
    return f"{REGISTRY}/{image}"


def present(client: Any, image: str) -> bool:
    try:
        client.images.get(image)
        return True
    except ImageNotFound:
        return False
    except DockerException:
        return False


def pull_from_registry(client: Any, image: str) -> dict[str, Any]:
    """Pull `image` from the registry and tag it with the name the product uses."""
    source = registry_reference(image)
    if source is None:
        raise HTTPException(409, "this node has no image registry configured")
    repo, tag = split_reference(source)
    try:
        pulled = client.images.pull(repo, tag=tag)
    except (APIError, DockerException) as exc:
        raise HTTPException(502, f"the registry could not deliver {image}: {exc}") from exc
    if not image.startswith(f"{REGISTRY}/"):
        local_repo, local_tag = split_reference(image)
        pulled.tag(local_repo, tag=local_tag)
    return {"image": image, "id": pulled.id, "size": pulled.attrs.get("Size", 0)}


def ensure_image(client: Any, image: str) -> None:
    """Make sure a sandbox start finds its image: pull it from the registry if it is missing.

    A failure is logged and not raised: Docker's own error on start names the real problem, and a
    public image (`ubuntu:24.04`) may legitimately come from Docker Hub.
    """
    if present(client, image) or not REGISTRY:
        return
    try:
        pull_from_registry(client, image)
        logger.info("pulled %s from the registry", image)
    except HTTPException as exc:
        logger.info("%s is not in the registry (%s); leaving it to Docker", image, exc.detail)


class PullRequest(BaseModel):
    image: str = Field(min_length=1, max_length=300)


@router.get("/v1/images")
def list_images() -> dict[str, Any]:
    """The tagged images on this node, with sizes, for the control plane's matrix."""
    items = []
    try:
        for image in _client().images.list():
            for tag in image.tags:
                items.append({"image": tag, "id": image.id, "size": image.attrs.get("Size", 0)})
    except DockerException as exc:
        raise HTTPException(503, f"docker is not answering: {exc}") from exc
    return {"items": items, "registry": REGISTRY or None}


@router.post("/v1/images/pull")
def pull_image(body: PullRequest) -> dict[str, Any]:
    """Pull one assigned image from the registry. Names the allow rule refuses are refused."""
    if not image_allowed(body.image):
        raise HTTPException(422, "image is not in the approved catalog for this node")
    return pull_from_registry(_client(), body.image)
