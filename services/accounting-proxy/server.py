"""Nestbox accounting proxy — agent model traffic, accounted and privacy-checked.

The agent talks to this service as if it were the model provider; this service
talks to the provider. Every request is inspected before it leaves, and every
answer is turned into one metadata row. Two facts make the design what it is:

* **Credentials are forwarded, never stored.** The client's own `Authorization`
  (or `x-api-key`) header is passed on untouched. The proxy never logs in, never
  mints a token, and holds no provider key of its own.
* **Content is never kept.** Only a SHA-256 digest of the request body, the
  usage counters, the status, and the latency leave this process. Prompts and
  completions are not written to a row, not logged, and not sent anywhere except
  the upstream the agent chose.

The three pieces, in the order they happen:

1. **Routing** picks an upstream from the path (`/v1/messages` is Anthropic,
   `/v1/chat/completions` is OpenAI, `/v1beta/models/...` is Gemini-style) and
   lets an `X-Nestbox-Upstream` header override the choice outright.
2. **Inspection** asks the control API (`POST /api/v1/internal/privacy/inspect`)
   about the request text. A refusal answers `403` and the request is not
   forwarded; a rewrite is written back into the JSON body before forwarding.
3. **Accounting** posts one row to `POST /api/v1/internal/accounting/record`
   after the client has its answer.

A body that is not inspectable JSON (binary uploads, pre-encoded streams) is
forwarded unchanged and marked `not_inspectable` — the honest label, because
"we did not look" and "we looked and it was clean" are different facts.

One path carries its own routing: `/s/<sandbox>/r/…` is the sandbox's **model
router**. The control plane grants models per sandbox, several of them may live
on different upstreams (a gateway, a local llama.cpp on a node, a provider), and
the agent names one model. The router therefore picks the upstream and the
credential from the grant named in the request body, instead of from the path
family or the caller's headers. `GET /s/<sandbox>/r/v1/models` answers the
granted list in the OpenAI shape, so an agent's model picker can ask for it.

Standard library only, for the same reason as `services/tool-images/mcp-hub`:
this image has no package manager step, so a registry outage or a hijacked
dependency cannot change what ships.
"""
from __future__ import annotations

import dataclasses
import hashlib
import json
import os
import re
import threading
import time
from dataclasses import dataclass
from http.client import HTTPConnection, HTTPSConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import quote, urlsplit

# Path families. A path that is in none of them still goes to the default
# upstream, because an agent may use a provider we have not heard of yet.
ANTHROPIC_PATHS = frozenset({"/v1/messages", "/v1/complete"})
OPENAI_PATHS = frozenset(
    {"/v1/chat/completions", "/v1/responses", "/v1/embeddings"}
)
GEMINI_PREFIX = "/v1beta/models/"

INSPECT_PATH = "/api/v1/internal/privacy/inspect"
ACCOUNT_PATH = "/api/v1/internal/accounting/record"
# The per-sandbox model grants the router routes by. The sandbox name is
# appended, already checked against `_SANDBOX_SEGMENT`.
MODELS_PATH = "/api/v1/internal/accounting/models/"

# The model list an agent asks for, and the path segment that marks the router.
ROUTER_MODELS = "/v1/models"
ROUTER_SEGMENT = "r"

UPSTREAM_HEADER = "x-nestbox-upstream"

# Headers that belong to one hop and must not be copied across either way.
HOP_BY_HOP = frozenset(
    {
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "trailers",
        "transfer-encoding",
        "upgrade",
    }
)

# Our own headers: the routing hint is ours, and `host`/`content-length` are
# recomputed for the upstream connection rather than copied.
REQUEST_DROP = HOP_BY_HOP | {"host", "content-length", UPSTREAM_HEADER}
RESPONSE_DROP = HOP_BY_HOP | {"content-length"}

CHUNK = 65536
MAX_REQUEST_BYTES = 32 * 1024 * 1024


@dataclass(frozen=True)
class Config:
    """Everything this service knows, read from the environment once.

    Frozen so that a request handler cannot talk itself into a different
    upstream halfway through a request.
    """

    anthropic_upstream: str
    openai_upstream: str
    default_upstream: str
    control_api: str
    accounting_token: str
    sandbox: str
    agent_instance: str
    privacy_fail_closed: bool
    port: int
    upstream_timeout: float
    control_timeout: float

    @classmethod
    def from_env(cls, env: dict[str, str] | None = None) -> "Config":
        """Read the configuration. Every knob is an env var, none is a file."""
        source = os.environ if env is None else env
        return cls(
            anthropic_upstream=source.get(
                "NESTBOX_ANTHROPIC_UPSTREAM", "https://api.anthropic.com"
            ).rstrip("/"),
            openai_upstream=source.get(
                "NESTBOX_OPENAI_UPSTREAM", "https://api.openai.com"
            ).rstrip("/"),
            default_upstream=source.get("NESTBOX_DEFAULT_UPSTREAM", "").rstrip("/"),
            control_api=source.get(
                "NESTBOX_CONTROL_API", "http://127.0.0.1:8080"
            ).rstrip("/"),
            accounting_token=source.get("NESTBOX_ACCOUNTING_TOKEN", ""),
            sandbox=source.get("NESTBOX_SANDBOX", ""),
            agent_instance=source.get("NESTBOX_AGENT_INSTANCE", ""),
            privacy_fail_closed=source.get("NESTBOX_PRIVACY_FAIL_CLOSED", "0")
            in {"1", "true", "yes", "on"},
            port=int(source.get("NESTBOX_ACCOUNTING_PROXY_PORT", "8099")),
            upstream_timeout=float(
                source.get("NESTBOX_UPSTREAM_TIMEOUT", "120")
            ),
            control_timeout=float(source.get("NESTBOX_CONTROL_TIMEOUT", "5")),
        )


CONFIG = Config.from_env()


class ControlError(Exception):
    """The control API could not be reached or answered with something unusable.

    The message carries a path and a status, never a body: a body here could
    contain the very content we are inspecting.
    """


class PrivacyBlocked(Exception):
    """The control API refused the request. The detector names the reason."""

    def __init__(self, detector: str | None) -> None:
        super().__init__(detector or "privacy")
        self.detector = detector


class UpstreamUnavailable(Exception):
    """No usable upstream for this path, with the error code to answer with."""

    def __init__(self, message: str, code: str) -> None:
        super().__init__(message)
        self.code = code


@dataclass
class Outcome:
    """What inspection decided about one request body."""

    action: str
    detector: str | None
    body: bytes


# -- routing ---------------------------------------------------------------


_SANDBOX_SEGMENT = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
_UPSTREAM_HOST = re.compile(r"^[A-Za-z0-9.-]+(:[0-9]{1,5})?$")


def split_identity(raw_path: str) -> tuple[str, str | None, str, bool] | None:
    """Read `/s/<sandbox>[/u/<scheme>/<host>][/r]/rest` off a request path.

    Returns `(sandbox, upstream_or_None, remaining_path_with_query, router)`,
    or `None` when the path carries no identity prefix. A malformed prefix is
    treated as no prefix at all, so it can never smuggle a value into a row or
    an upstream.

    `router` is True for `/s/<sandbox>/r/…`: that path asks this service to pick
    the upstream from the sandbox's model grants instead of from the path family
    or the caller's headers. The `/u/` form keeps its meaning and is still
    parsed in front of `/r/`, but a router request ignores it: the grant, not the
    caller, names the upstream there.
    """
    parts = urlsplit(raw_path)
    segments = parts.path.split("/")
    # ["", "s", "<sandbox>", ...]
    if len(segments) < 3 or segments[1] != "s" or not _SANDBOX_SEGMENT.match(segments[2]):
        return None
    sandbox = segments[2]
    rest = segments[3:]
    upstream: str | None = None
    if (
        len(rest) >= 3
        and rest[0] == "u"
        and rest[1] in {"http", "https"}
        and _UPSTREAM_HOST.match(rest[2])
    ):
        upstream = f"{rest[1]}://{rest[2]}"
        rest = rest[3:]
    router = False
    if rest and rest[0] == ROUTER_SEGMENT:
        router = True
        rest = rest[1:]
    remaining = "/" + "/".join(rest)
    if parts.query:
        remaining += "?" + parts.query
    return sandbox, upstream, remaining, router


def dialect_of(path: str) -> str:
    """The API family a path belongs to: `anthropic`, `openai`, `gemini`, `other`.

    Only the path decides. The `X-Nestbox-Upstream` header moves the request,
    it does not change what dialect the body is written in.
    """
    clean = urlsplit(path).path
    if clean in ANTHROPIC_PATHS:
        return "anthropic"
    if clean in OPENAI_PATHS:
        return "openai"
    if clean.startswith(GEMINI_PREFIX) or clean.startswith("/v1beta/"):
        return "gemini"
    return "other"


def upstream_for(config: Config, path: str, override: str | None) -> str:
    """The base URL a request goes to.

    An explicit `X-Nestbox-Upstream` wins over the path, because an operator
    running a gateway or a local model server needs to say so per request. The
    override is validated, not trusted: only `http` and `https` are forwarded,
    so the header cannot turn the proxy into a `file://` reader.
    """
    if override:
        parts = urlsplit(override.strip())
        if parts.scheme not in {"http", "https"} or not parts.netloc:
            raise UpstreamUnavailable(
                "X-Nestbox-Upstream must be an http(s) URL", "upstream.invalid"
            )
        return override.strip().rstrip("/")
    dialect = dialect_of(path)
    if dialect == "anthropic":
        return config.anthropic_upstream
    if dialect == "openai":
        return config.openai_upstream
    if config.default_upstream:
        return config.default_upstream
    raise UpstreamUnavailable(
        "no default upstream is configured", "upstream.unconfigured"
    )


# -- the model router ------------------------------------------------------


def route_base(entry: dict[str, Any]) -> str:
    """The upstream base URL of one granted model, validated.

    A grant is written by an operator through the control API, but it is still
    not trusted here: only `http` and `https` are forwarded, so a grant can
    never turn this proxy into a `file://` reader.
    """
    base = str(entry.get("upstream_base") or "").strip().rstrip("/")
    parts = urlsplit(base)
    if parts.scheme not in {"http", "https"} or not parts.netloc:
        raise UpstreamUnavailable(
            "the granted model has no usable upstream_base", "upstream.invalid"
        )
    return base


def route_authorization(entry: dict[str, Any]) -> str | None:
    """The grant's own `Authorization` value, or None to keep the caller's.

    A value here belongs to the gateway that serves the model (an OmniRoute key,
    a shared local server): the sandbox gets the model without holding the key.
    """
    value = entry.get("authorization")
    return value if isinstance(value, str) and value else None


def requested_model(routes: dict[str, Any], body: Any) -> Any:
    """The model name a request asks for, else the sandbox's default."""
    if isinstance(body, dict):
        value = body.get("model")
        if isinstance(value, str) and value:
            return value
    return routes.get("default")


def choose_route(
    routes: dict[str, Any], body: Any
) -> tuple[dict[str, Any] | None, Any]:
    """Pick the granted model a request names, and the body to forward.

    Returns `(entry, new_body)`. `entry` is None when the sandbox has no grant
    for that name — the caller answers 404 rather than forwarding, because a
    model that is not granted must not be reachable by guessing its name. When
    the grant renames the model (`fast` is `qwen3:1.7b` on the node), `new_body`
    carries the upstream's own name; otherwise it is the body unchanged.
    """
    unchanged = body if isinstance(body, dict) else {}
    name = requested_model(routes, body)
    if not isinstance(name, str) or not name:
        return None, unchanged
    for entry in routes.get("models") or []:
        if not isinstance(entry, dict) or entry.get("model_name") != name:
            continue
        upstream_model = entry.get("upstream_model")
        if (
            isinstance(upstream_model, str)
            and upstream_model
            and upstream_model != name
            and isinstance(body, dict)
        ):
            renamed = dict(body)
            renamed["model"] = upstream_model
            return entry, renamed
        return entry, unchanged
    return None, unchanged


def model_list(routes: dict[str, Any]) -> dict[str, Any]:
    """The sandbox's granted models in the OpenAI list format.

    An agent asks for this list when it starts and builds its model picker from
    it, so the ids are the *granted* names — the ones an agent may ask for — not
    the upstream's internal names.
    """
    data: list[dict[str, Any]] = []
    for entry in routes.get("models") or []:
        if not isinstance(entry, dict):
            continue
        name = entry.get("model_name")
        if isinstance(name, str) and name:
            data.append({"id": name, "object": "model", "owned_by": "nestbox"})
    return {"object": "list", "data": data}


def not_granted_error(name: Any) -> dict[str, Any]:
    """The 404 body for a model this sandbox has no grant for."""
    label = name if isinstance(name, str) and name else "unknown"
    return {
        "error": {
            "type": "model_not_granted",
            "message": f"Model {label} is not granted to this sandbox.",
        }
    }


# -- reading the text out of a request -------------------------------------


# A slot is (holder, key) addressing one string in the parsed body, so a rewrite
# can be written back exactly where it came from. Nothing is rebuilt from a
# template: an unknown body shape simply yields no slots.
Slot = tuple[Any, Any]


def _content_slots(value: Any, holder: Any, key: Any, slots: list[Slot]) -> None:
    """Collect the text of a `content`/`parts` value: a string, or a block list."""
    if isinstance(value, str):
        slots.append((holder, key))
        return
    if not isinstance(value, list):
        return
    for part in value:
        if isinstance(part, dict) and isinstance(part.get("text"), str):
            slots.append((part, "text"))


def _response_input_slots(value: Any, holder: Any, key: Any, slots: list[Slot]) -> None:
    """Collect `/v1/responses` input: a string, or items that carry content."""
    if isinstance(value, str):
        slots.append((holder, key))
        return
    if not isinstance(value, list):
        return
    for index, item in enumerate(value):
        if isinstance(item, str):
            slots.append((value, index))
            continue
        if not isinstance(item, dict):
            continue
        if isinstance(item.get("text"), str):
            slots.append((item, "text"))
            continue
        if "content" in item:
            _content_slots(item["content"], item, "content", slots)


def text_slots(body: dict[str, Any]) -> list[Slot]:
    """Every string in a request body that a model would read as a prompt.

    Shapes covered: Anthropic `messages`/`system`, OpenAI chat `messages`, OpenAI
    responses `input`/`instructions`, legacy `prompt`, embeddings `input`, and
    Gemini `contents`. An unrecognised body has no slots.
    """
    slots: list[Slot] = []

    messages = body.get("messages")
    if isinstance(messages, list):
        for message in messages:
            if isinstance(message, dict) and "content" in message:
                _content_slots(message["content"], message, "content", slots)

    system = body.get("system")
    if isinstance(system, str):
        slots.append((body, "system"))
    elif isinstance(system, list):
        for block in system:
            if isinstance(block, dict) and isinstance(block.get("text"), str):
                slots.append((block, "text"))

    if "input" in body:
        _response_input_slots(body["input"], body, "input", slots)

    instructions = body.get("instructions")
    if isinstance(instructions, str):
        slots.append((body, "instructions"))

    prompt = body.get("prompt")
    if isinstance(prompt, str):
        slots.append((body, "prompt"))
    elif isinstance(prompt, list):
        for index, item in enumerate(prompt):
            if isinstance(item, str):
                slots.append((prompt, index))

    contents = body.get("contents")
    if isinstance(contents, list):
        for content in contents:
            if isinstance(content, dict) and "parts" in content:
                _content_slots(content["parts"], content, "parts", slots)

    return slots


def _dump(body: dict[str, Any]) -> bytes:
    return json.dumps(body, ensure_ascii=False, separators=(",", ":")).encode()


def _detector_of(answer: dict[str, Any]) -> str | None:
    """The first detector a finding names, or None when there is no finding."""
    findings = answer.get("findings")
    if not isinstance(findings, list):
        return None
    for finding in findings:
        if isinstance(finding, dict) and isinstance(finding.get("detector"), str):
            return finding["detector"]
    return None


# -- the control API -------------------------------------------------------


def _connection(base: str, timeout: float) -> HTTPConnection:
    parts = urlsplit(base)
    host = parts.hostname or ""
    port = parts.port
    if parts.scheme == "https":
        return HTTPSConnection(host, port, timeout=timeout)
    if parts.scheme != "http":
        raise ControlError(f"unsupported control API scheme: {parts.scheme}")
    return HTTPConnection(host, port, timeout=timeout)


def _control_call(
    config: Config, method: str, path: str, payload: dict[str, Any] | None = None
) -> dict[str, Any]:
    """One JSON call to the control API, or `ControlError`.

    The token is `NESTBOX_ACCOUNTING_TOKEN`, a node token for the *internal*
    endpoints — it is not a provider credential and never leaves this container
    except to the control API.
    """
    body = b"" if payload is None else json.dumps(payload).encode()
    headers: dict[str, str] = {}
    if body:
        headers["Content-Type"] = "application/json"
        headers["Content-Length"] = str(len(body))
    if config.accounting_token:
        headers["Authorization"] = f"Bearer {config.accounting_token}"
    base = urlsplit(config.control_api)
    target = (base.path.rstrip("/") or "") + path
    connection = None
    try:
        connection = _connection(config.control_api, config.control_timeout)
        connection.request(method, target, body=body or None, headers=headers)
        response = connection.getresponse()
        data = response.read()
        status = response.status
    except (OSError, ValueError) as exc:
        raise ControlError(f"{path} unreachable: {type(exc).__name__}") from None
    finally:
        if connection is not None:
            connection.close()
    if status // 100 != 2:
        raise ControlError(f"{path} answered {status}")
    if not data:
        # `204 No Content` is the honest answer for a row that was stored.
        return {}
    try:
        answer = json.loads(data)
    except ValueError:
        raise ControlError(f"{path} answered non-JSON") from None
    if not isinstance(answer, dict):
        raise ControlError(f"{path} answered a non-object")
    return answer


def control_post(
    config: Config, path: str, payload: dict[str, Any]
) -> dict[str, Any]:
    """One JSON POST to the control API, or `ControlError`."""
    return _control_call(config, "POST", path, payload)


def control_get(config: Config, path: str) -> dict[str, Any]:
    """One JSON GET from the control API, or `ControlError`."""
    return _control_call(config, "GET", path)


# -- the model grants of one sandbox ---------------------------------------

# Fetched per sandbox and kept for a few seconds: the router asks on every
# request, but a grant changes only when an operator edits one. Each entry is
# `(monotonic_fetch_time, routes_document)`.
_ROUTES: dict[str, tuple[float, dict[str, Any]]] = {}
_ROUTES_LOCK = threading.Lock()
ROUTES_TTL = 10.0


def routes_for(sandbox: str) -> dict[str, Any]:
    """The sandbox's granted models from the control API, cached for 10 s.

    A failure does not lose a working route: a cached answer is used even when it
    is old, because a short control API outage must not take every running agent
    down. With nothing cached the answer is an empty grant list, and every model
    request then gets the honest 404 from `not_granted_error`.

    The document carries upstream `Authorization` values, so neither it nor any
    field of it is ever logged.
    """
    now = time.monotonic()
    cached = _ROUTES.get(sandbox)
    if cached is not None and now - cached[0] < ROUTES_TTL:
        return cached[1]
    try:
        answer = control_get(CONFIG, MODELS_PATH + quote(sandbox, safe=""))
    except ControlError:
        if cached is not None:
            print(f"routes for {sandbox} unavailable, using cached", flush=True)
            return cached[1]
        return {"default": None, "models": []}
    models = answer.get("models")
    routes = {
        "default": answer.get("default"),
        "models": models if isinstance(models, list) else [],
    }
    with _ROUTES_LOCK:
        _ROUTES[sandbox] = (now, routes)
    return routes


def inspect_payload(config: Config, content: str, destination: str) -> dict[str, Any]:
    """The inspect request body: the text, never a whole request body."""
    return {
        "sandbox": config.sandbox,
        "direction": "outbound",
        "content": content,
        "destination": destination,
    }


def inspect_request(
    config: Config, body: bytes, destination: str
) -> Outcome:
    """Inspect one request body and say what may be forwarded.

    Raises `PrivacyBlocked` when the request must not go on, and
    `ControlError` only when `NESTBOX_PRIVACY_FAIL_CLOSED` is on — otherwise an
    unreachable control API is recorded as `not_inspected` and the request is
    forwarded, because a proxy that is down for inspection should not take the
    agent down with it unless an operator said that it should.
    """
    try:
        parsed = json.loads(body)
    except (ValueError, UnicodeDecodeError):
        return Outcome("not_inspectable", None, body)
    if not isinstance(parsed, dict):
        return Outcome("not_inspectable", None, body)

    slots = text_slots(parsed)
    if not slots:
        return Outcome("not_inspectable", None, body)

    texts = [holder[key] for holder, key in slots]
    joined = "\n".join(texts)

    try:
        answer = control_post(
            config, INSPECT_PATH, inspect_payload(config, joined, destination)
        )
    except ControlError:
        if config.privacy_fail_closed:
            raise
        return Outcome("not_inspected", None, body)

    if not answer.get("permitted", False):
        raise PrivacyBlocked(_detector_of(answer))

    detector = _detector_of(answer)
    action = str(answer.get("action") or "allowed")
    if action != "replaced":
        return Outcome("allowed" if action == "allowed" else action, detector, body)

    transformed = answer.get("transformed_content")
    if not isinstance(transformed, str):
        # `replaced` without a replacement is not a rewrite we can perform. Fail
        # closed on the *decision*: the control API asked for a change we cannot
        # carry out, and forwarding the original would ignore it.
        raise PrivacyBlocked(detector)

    if len(slots) == 1:
        holder, key = slots[0]
        holder[key] = transformed
        return Outcome("replaced", detector, _dump(parsed))

    # More than one text field: the combined answer cannot be put back into the
    # individual messages, so each one is inspected on its own with the same
    # call. A refusal on any single message blocks the whole request.
    for (holder, key), text in zip(slots, texts):
        try:
            one = control_post(
                config, INSPECT_PATH, inspect_payload(config, text, destination)
            )
        except ControlError:
            if config.privacy_fail_closed:
                raise
            return Outcome("not_inspected", None, body)
        if not one.get("permitted", False):
            raise PrivacyBlocked(_detector_of(one) or detector)
        detector = detector or _detector_of(one)
        replacement = one.get("transformed_content")
        if one.get("action") == "replaced" and isinstance(replacement, str):
            holder[key] = replacement
    return Outcome("replaced", detector, _dump(parsed))


# -- usage -----------------------------------------------------------------


def _as_int(value: Any) -> int | None:
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def extract_usage(answer: Any) -> dict[str, int | None] | None:
    """Normalise a provider's counters into one shape.

    OpenAI `prompt_tokens`/`completion_tokens`/`prompt_tokens_details.cached_tokens`,
    Anthropic `input_tokens`/`output_tokens`/`cache_read_input_tokens`, and
    Gemini `promptTokenCount`/`candidatesTokenCount` all become
    `prompt_tokens`/`completion_tokens`/`cached_tokens`/`total_tokens`, so the
    accounting table does not need a column per provider.
    """
    if not isinstance(answer, dict):
        return None
    usage = answer.get("usage")
    if not isinstance(usage, dict):
        usage = answer.get("usageMetadata")
    if not isinstance(usage, dict):
        return None

    prompt = _as_int(usage.get("prompt_tokens"))
    completion = _as_int(usage.get("completion_tokens"))
    cached = None
    total = _as_int(usage.get("total_tokens"))

    if prompt is None and completion is None:
        prompt = _as_int(usage.get("input_tokens"))
        completion = _as_int(usage.get("output_tokens"))
        cached = _as_int(usage.get("cache_read_input_tokens"))
    else:
        details = usage.get("prompt_tokens_details")
        if isinstance(details, dict):
            cached = _as_int(details.get("cached_tokens"))

    if prompt is None and completion is None:
        prompt = _as_int(usage.get("promptTokenCount"))
        completion = _as_int(usage.get("candidatesTokenCount"))
        cached = _as_int(usage.get("cachedContentTokenCount"))
        total = _as_int(usage.get("totalTokenCount"))

    if total is None and prompt is not None and completion is not None:
        total = prompt + completion
    if prompt is None and completion is None:
        return None
    return {
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "cached_tokens": cached,
        "total_tokens": total,
    }


def model_of(answer: Any, request_body: Any) -> str | None:
    """The model the response names, else the one the request asked for."""
    if isinstance(answer, dict):
        for key in ("model", "modelVersion"):
            value = answer.get(key)
            if isinstance(value, str) and value:
                return value
    if isinstance(request_body, dict):
        value = request_body.get("model")
        if isinstance(value, str) and value:
            return value
    return None


def wants_stream(request_body: Any, path: str) -> bool:
    """Whether the client asked for an incremental answer."""
    if isinstance(request_body, dict) and request_body.get("stream") is True:
        return True
    # Gemini spells it in the path rather than in the body.
    return urlsplit(path).path.endswith(":streamGenerateContent")


def record_row(
    config: Config,
    *,
    provider: str,
    model: str | None,
    endpoint: str,
    status: int,
    latency_ms: int,
    usage: dict[str, int | None] | None,
    content_digest: str,
    privacy_action: str,
    privacy_detector: str | None,
) -> None:
    """Post one metadata row. A failure changes the log, never the answer."""
    row = {
        "sandbox": config.sandbox,
        "agent_instance": config.agent_instance,
        "provider": provider,
        "model": model,
        "direction": "outbound",
        "endpoint": endpoint,
        "status": status,
        "latency_ms": latency_ms,
        "usage": usage,
        "content_digest": content_digest,
        "privacy_action": privacy_action,
        "privacy_detector": privacy_detector,
    }
    try:
        control_post(config, ACCOUNT_PATH, row)
    except ControlError as exc:
        # Metadata can be lost; the agent's answer cannot be. No content here.
        print(f"accounting row dropped: {exc}", flush=True)


def digest_of(body: bytes) -> str:
    """The lowercase hex SHA-256 of the bytes that were forwarded."""
    return hashlib.sha256(body).hexdigest()


# -- model clusters -------------------------------------------------------

# The proxy's view of one model cluster (contract C7). The cluster id is
# appended, already checked against `_CLUSTER_SEGMENT`.
CLUSTERS_PATH = "/api/v1/internal/accounting/clusters/"
CLUSTER_TTL = 10.0
# How long a member that refused a connection is skipped.
UNHEALTHY_FOR = 30.0
CLUSTER_MEMBER_HEADER = "x-nestbox-cluster-member"

_CLUSTER_SEGMENT = re.compile(r"^[A-Za-z0-9-]{1,64}$")

# `cluster_id -> (monotonic_fetch_time, view)`.
_CLUSTERS: dict[str, tuple[float, dict[str, Any]]] = {}
# `(cluster_id, install_id) -> monotonic time until which the member is skipped`.
_UNHEALTHY: dict[tuple[str, str], float] = {}
# In-flight requests per install, and the round-robin counter per cluster.
_INFLIGHT: dict[str, int] = {}
_COUNTERS: dict[str, int] = {}
_CLUSTER_LOCK = threading.Lock()


def cluster_view(cluster_id: str) -> dict[str, Any] | None:
    """The cluster definition from the control API, cached for 10 s.

    A failed refresh returns the stale cached view if there is one, so a short
    control API outage does not stop a working agent; with nothing cached the
    answer is `None` and the caller answers `503 cluster.unavailable`.
    """
    now = time.monotonic()
    cached = _CLUSTERS.get(cluster_id)
    if cached is not None and now - cached[0] < CLUSTER_TTL:
        return cached[1]
    try:
        answer = control_get(CONFIG, CLUSTERS_PATH + quote(cluster_id, safe=""))
    except ControlError:
        if cached is not None:
            print(f"cluster {cluster_id} unavailable, using cached", flush=True)
            return cached[1]
        return None
    members = answer.get("members")
    view = {
        "id": cluster_id,
        "strategy": answer.get("strategy"),
        "model_key": answer.get("model_key"),
        "members": members if isinstance(members, list) else [],
    }
    with _CLUSTER_LOCK:
        _CLUSTERS[cluster_id] = (now, view)
    return view


def choose_member(
    view: dict[str, Any], inflight: dict[str, int], counters: dict[str, int]
) -> dict[str, Any] | None:
    """Pick one healthy member by the cluster's strategy, or `None`.

    `round_robin` walks the members expanded by weight with a counter per
    cluster; `least_loaded` takes the fewest in-flight requests (ties go to the
    first); `failover` takes the first healthy member in definition order.
    """
    healthy = [
        m
        for m in view.get("members") or []
        if isinstance(m, dict) and m.get("healthy") is True and m.get("endpoint")
    ]
    if not healthy:
        return None
    strategy = view.get("strategy")
    if strategy == "least_loaded":
        return min(healthy, key=lambda m: inflight.get(str(m.get("install_id")), 0))
    if strategy == "round_robin":
        expanded: list[dict[str, Any]] = []
        for member in healthy:
            weight = member.get("weight")
            count = weight if isinstance(weight, int) and weight > 0 else 1
            expanded.extend([member] * count)
        key = str(view.get("id"))
        index = counters.get(key, 0)
        counters[key] = index + 1
        return expanded[index % len(expanded)]
    return healthy[0]


def _usable_view(view: dict[str, Any], skip: set[str]) -> dict[str, Any]:
    """The view with recently failed members and `skip` marked unhealthy."""
    now = time.monotonic()
    cluster_id = str(view.get("id"))
    members = []
    for member in view.get("members") or []:
        if not isinstance(member, dict):
            continue
        install = str(member.get("install_id"))
        down = _UNHEALTHY.get((cluster_id, install), 0.0) > now or install in skip
        members.append({**member, "healthy": member.get("healthy") is True and not down})
    return {**view, "members": members}


class _MarkedResponse:
    """An upstream response that also carries the answering member's header."""

    def __init__(self, response: Any, install_id: str) -> None:
        self._response = response
        self._install_id = install_id
        self.status = response.status

    def getheader(self, name: str, default: Any = None) -> Any:
        """Read one header of the wrapped response."""
        return self._response.getheader(name, default)

    def getheaders(self) -> list[tuple[str, str]]:
        """All headers of the wrapped response plus the member header."""
        headers = [
            (k, v)
            for k, v in self._response.getheaders()
            if k.lower() != CLUSTER_MEMBER_HEADER
        ]
        headers.append((CLUSTER_MEMBER_HEADER, self._install_id))
        return headers

    def read(self, *args: Any) -> bytes:
        """Read from the wrapped response."""
        return self._response.read(*args)


def handle_cluster(handler: "Handler", started: float) -> None:
    """Serve `/c/<cluster_id>/<rest>`: one healthy member answers.

    The request is inspected like any other, then sent to the member the
    strategy picks. A connection error or timeout before any byte arrives marks
    that member down for 30 s and the next one is tried once. The answer carries
    `x-nestbox-cluster-member`, and the accounting row names the install.
    """
    parts = urlsplit(handler.path)
    segments = parts.path.split("/")
    # ["", "c", "<id>", ...]
    if len(segments) < 3 or not _CLUSTER_SEGMENT.match(segments[2]):
        handler._send_json(
            404, {"error": {"code": "cluster.not_found", "message": "no such cluster path"}}
        )
        return
    cluster_id = segments[2]
    rest = "/" + "/".join(segments[3:])
    if parts.query:
        rest += "?" + parts.query
    config = CONFIG
    sandbox_header = handler.headers.get("x-nestbox-sandbox") or ""
    if _SANDBOX_SEGMENT.match(sandbox_header):
        config = dataclasses.replace(config, sandbox=sandbox_header)

    try:
        body = handler._read_body()
    except ValueError as exc:
        handler._send_json(
            413, {"error": {"code": "request.too_large", "message": str(exc)}}
        )
        return
    try:
        parsed_body: Any = json.loads(body) if body else None
    except (ValueError, UnicodeDecodeError):
        parsed_body = None
    handler.path = rest
    dialect = dialect_of(rest)
    clean = urlsplit(rest).path

    view = cluster_view(cluster_id)
    if view is None:
        handler._send_json(
            503,
            {"error": {"code": "cluster.unavailable", "message": "the cluster definition is unavailable"}},
        )
        return

    def row(status: int, usage: Any, model: Any, outcome_body: bytes, action: str,
            detector: str | None, install_id: str | None) -> None:
        payload = {
            "sandbox": config.sandbox,
            "agent_instance": config.agent_instance,
            "provider": dialect,
            "model": model,
            "direction": "outbound",
            "endpoint": clean,
            "status": status,
            "latency_ms": int((time.monotonic() - started) * 1000),
            "usage": usage,
            "content_digest": digest_of(outcome_body),
            "privacy_action": action,
            "privacy_detector": detector,
            "detail": {"cluster_id": cluster_id, "install_id": install_id},
        }
        try:
            control_post(config, ACCOUNT_PATH, payload)
        except ControlError as exc:
            print(f"accounting row dropped: {exc}", flush=True)

    try:
        outcome = inspect_request(config, body, f"cluster-{cluster_id}")
    except PrivacyBlocked as exc:
        handler._send_json(
            403,
            {"error": {"code": "privacy.blocked",
                       "message": "the content was refused by the privacy policy",
                       "detector": exc.detector}},
        )
        row(403, None, None, body, "blocked", exc.detector, None)
        return
    except ControlError:
        handler._send_json(
            503,
            {"error": {"code": "privacy.unavailable",
                       "message": "content inspection is unavailable and fail-closed is on"}},
        )
        row(503, None, None, body, "not_inspected", None, None)
        return

    streaming = wants_stream(parsed_body, rest)
    tried: set[str] = set()
    for _attempt in range(2):
        with _CLUSTER_LOCK:
            member = choose_member(_usable_view(view, tried), _INFLIGHT, _COUNTERS)
            if member is not None:
                install_id = str(member.get("install_id"))
                _INFLIGHT[install_id] = _INFLIGHT.get(install_id, 0) + 1
        if member is None:
            break
        tried.add(install_id)
        connection = None
        try:
            try:
                base = urlsplit(str(member["endpoint"]))
                if base.scheme not in {"http", "https"} or not base.hostname:
                    raise ValueError("bad member endpoint")
                cls = HTTPSConnection if base.scheme == "https" else HTTPConnection
                connection = cls(base.hostname, base.port, timeout=config.upstream_timeout)
                target = (base.path.rstrip("/") or "") + rest
                connection.request(
                    handler.command,
                    target,
                    body=outcome.body if outcome.body else None,
                    headers=handler._upstream_headers(outcome.body),
                )
                response = connection.getresponse()
            except (OSError, ValueError):
                if connection is not None:
                    connection.close()
                with _CLUSTER_LOCK:
                    _UNHEALTHY[(cluster_id, install_id)] = time.monotonic() + UNHEALTHY_FOR
                continue
            try:
                status, usage, model = handler._relay(
                    _MarkedResponse(response, install_id), streaming
                )
            finally:
                connection.close()
            row(status, usage, model or model_of(None, parsed_body), outcome.body,
                outcome.action, outcome.detector, install_id)
            return
        finally:
            with _CLUSTER_LOCK:
                _INFLIGHT[install_id] = max(0, _INFLIGHT.get(install_id, 1) - 1)

    handler._send_json(
        503,
        {"error": {"code": "cluster.no_healthy_member",
                   "message": "no healthy member answered"}},
    )
    row(503, None, None, outcome.body, outcome.action, outcome.detector, None)


# -- the HTTP surface ------------------------------------------------------


class Handler(BaseHTTPRequestHandler):
    """One request in, one request out, one row written."""

    protocol_version = "HTTP/1.1"
    server_version = "nestbox-accounting-proxy/0.1"
    # Set as soon as a status line is on the wire, so the last line of defence
    # knows whether a `500` can still be sent without corrupting the framing.
    _responded = False

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A002 - stdlib
        # Path, status, duration. Never headers, never bodies.
        print(f"{self.address_string()} {format % args}", flush=True)

    # -- small helpers --------------------------------------------------

    def _send_json(self, status: int, payload: dict[str, Any]) -> None:
        body = json.dumps(payload).encode()
        self.send_response(status)
        self._responded = True
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _read_body(self) -> bytes:
        try:
            length = int(self.headers.get("content-length") or 0)
        except ValueError:
            return b""
        if length <= 0:
            return b""
        if length > MAX_REQUEST_BYTES:
            raise ValueError(f"body exceeds {MAX_REQUEST_BYTES} bytes")
        return self.rfile.read(length)

    # -- the pipeline ---------------------------------------------------

    def do_GET(self) -> None:  # noqa: N802 - stdlib naming
        self._handle()

    def do_POST(self) -> None:  # noqa: N802 - stdlib naming
        self._handle()

    def do_PUT(self) -> None:  # noqa: N802 - stdlib naming
        self._handle()

    def do_PATCH(self) -> None:  # noqa: N802 - stdlib naming
        self._handle()

    def do_DELETE(self) -> None:  # noqa: N802 - stdlib naming
        self._handle()

    def do_HEAD(self) -> None:  # noqa: N802 - stdlib naming
        self._handle()

    def do_OPTIONS(self) -> None:  # noqa: N802 - stdlib naming
        self._handle()

    def _handle(self) -> None:
        """Run the pipeline. A bug is a `500`, never a dropped connection."""
        self._responded = False
        try:
            self._pipeline()
        except Exception as exc:  # noqa: BLE001 - last line of defence
            # Type and message only: an exception message here can name a JSON
            # field, never a prompt, and the body is not in this line.
            print(f"{self.path} failed: {type(exc).__name__}: {exc}", flush=True)
            if self._responded:
                self.close_connection = True
                return
            self._send_json(
                500,
                {"error": {"code": "proxy.failed", "message": "internal error"}},
            )

    def _pipeline(self) -> None:
        started = time.monotonic()
        config = CONFIG
        clean = urlsplit(self.path).path

        if clean == "/healthz":
            self._send_json(200, {"status": "ok", "service": "accounting-proxy"})
            return

        # Identity travels in the path, not in a header: the egress proxy that
        # sits between a sandbox and this service drops `X-Nestbox-*` headers, so
        # a header-borne identity would arrive empty. `/s/<sandbox>/…` names the
        # sandbox; an optional `/u/<scheme>/<host[:port]>/…` right after it names
        # the upstream (the gateway an agent is configured for). Both are
        # stripped before the request is forwarded.
        path_upstream: str | None = None
        router = False
        identity = split_identity(self.path)
        if identity is not None:
            sandbox_name, path_upstream, rest, router = identity
            config = dataclasses.replace(config, sandbox=sandbox_name)
            self.path = rest
            clean = urlsplit(self.path).path

        # `/c/<cluster_id>/…`: a model cluster; a healthy member answers.
        if identity is None and clean.startswith("/c/"):
            handle_cluster(self, started)
            return

        try:
            body = self._read_body()
        except ValueError as exc:
            self._send_json(
                413, {"error": {"code": "request.too_large", "message": str(exc)}}
            )
            return

        try:
            parsed_body: Any = json.loads(body) if body else None
        except (ValueError, UnicodeDecodeError):
            parsed_body = None

        dialect = dialect_of(self.path)
        authorization: str | None = None
        if router:
            # `/s/<sandbox>/r/…`: the grant named in the body decides the
            # upstream and the credential, not the path family or the headers.
            routes = routes_for(config.sandbox)
            if self.command == "GET" and urlsplit(self.path).path == ROUTER_MODELS:
                self._send_json(200, model_list(routes))
                return
            entry, routed_body = choose_route(routes, parsed_body)
            if entry is None:
                name = requested_model(routes, parsed_body)
                self._send_json(404, not_granted_error(name))
                return
            try:
                base = route_base(entry)
            except UpstreamUnavailable as exc:
                self._send_json(
                    400, {"error": {"code": exc.code, "message": str(exc)}}
                )
                return
            authorization = route_authorization(entry)
            if routed_body is not parsed_body and routed_body:
                # A grant may rename the model (`fast` is `qwen3:1.7b` on the
                # node). The rewrite happens before inspection so the bytes that
                # are inspected are the bytes that are forwarded.
                body = _dump(routed_body)
            parsed_body = routed_body
        else:
            try:
                base = upstream_for(
                    config, self.path, self.headers.get(UPSTREAM_HEADER) or path_upstream
                )
            except UpstreamUnavailable as exc:
                self._send_json(
                    502 if exc.code == "upstream.unconfigured" else 400,
                    {"error": {"code": exc.code, "message": str(exc)}},
                )
                return

        destination = urlsplit(base).hostname or base

        # 1. Inspection, before anything leaves this process.
        try:
            outcome = inspect_request(config, body, destination)
        except PrivacyBlocked as exc:
            answer = {
                "error": {
                    "code": "privacy.blocked",
                    "message": "the content was refused by the privacy policy",
                    "detector": exc.detector,
                }
            }
            self._send_json(403, answer)
            record_row(
                config,
                provider=dialect,
                model=None,  # nothing was forwarded, so no model was used
                endpoint=clean,
                status=403,
                latency_ms=int((time.monotonic() - started) * 1000),
                usage=None,
                content_digest=digest_of(body),
                privacy_action="blocked",
                privacy_detector=exc.detector,
            )
            return
        except ControlError:
            self._send_json(
                503,
                {
                    "error": {
                        "code": "privacy.unavailable",
                        "message": "content inspection is unavailable and fail-closed is on",
                    }
                },
            )
            record_row(
                config,
                provider=dialect,
                model=None,
                endpoint=clean,
                status=503,
                latency_ms=int((time.monotonic() - started) * 1000),
                usage=None,
                content_digest=digest_of(body),
                privacy_action="not_inspected",
                privacy_detector=None,
            )
            return

        # 2. Forwarding, with the inspect verdict applied to the body.
        streaming = wants_stream(parsed_body, self.path)
        status, usage, model = self._forward(
            base, outcome.body, streaming, authorization
        )
        latency_ms = int((time.monotonic() - started) * 1000)

        # 3. Accounting, after the client has its answer.
        record_row(
            config,
            provider=dialect,
            model=model or model_of(None, parsed_body),
            endpoint=clean,
            status=status,
            latency_ms=latency_ms,
            usage=usage,
            content_digest=digest_of(outcome.body),
            privacy_action=outcome.action,
            privacy_detector=outcome.detector,
        )

    def _upstream_headers(
        self, body: bytes, authorization: str | None = None
    ) -> dict[str, str]:
        """The client's headers, minus what belongs to this hop.

        `Authorization` and `x-api-key` are copied as they arrived, because this
        service has no credential of its own to substitute. The one exception is
        a granted model that carries its own `Authorization` (`authorization`):
        then that value replaces the caller's, since the grant is what the
        upstream accepts and the sandbox is not supposed to hold it.
        """
        headers: dict[str, str] = {}
        for name, value in self.headers.items():
            lower = name.lower()
            if lower in REQUEST_DROP:
                continue
            if authorization is not None and lower == "authorization":
                continue
            headers[name] = value
        if authorization is not None:
            headers["Authorization"] = authorization
        headers["Content-Length"] = str(len(body))
        return headers

    def _forward(
        self,
        base: str,
        body: bytes,
        streaming: bool,
        authorization: str | None = None,
    ) -> tuple[int, dict[str, int | None] | None, str | None]:
        """Send the request on, relay the answer, return (status, usage, model)."""
        config = CONFIG
        parts = urlsplit(base)
        path = urlsplit(self.path)
        target = (parts.path.rstrip("/") or "") + path.path
        if path.query:
            target += "?" + path.query

        connection = None
        try:
            if parts.scheme == "https":
                connection = HTTPSConnection(
                    parts.hostname, parts.port, timeout=config.upstream_timeout
                )
            else:
                connection = HTTPConnection(
                    parts.hostname, parts.port, timeout=config.upstream_timeout
                )
            connection.request(
                self.command,
                target,
                body=body if body else None,
                headers=self._upstream_headers(body, authorization),
            )
            response = connection.getresponse()
        except (OSError, ValueError):
            if connection is not None:
                connection.close()
            self._send_json(
                502,
                {
                    "error": {
                        "code": "upstream.unreachable",
                        "message": "the upstream did not answer",
                    }
                },
            )
            return 502, None, None

        try:
            return self._relay(response, streaming)
        finally:
            connection.close()

    def _relay(
        self, response: Any, streaming: bool
    ) -> tuple[int, dict[str, int | None] | None, str | None]:
        content_type = (response.getheader("content-type") or "").lower()
        is_stream = streaming or "text/event-stream" in content_type
        length_header = response.getheader("content-length")

        self.send_response(response.status)
        self._responded = True
        for name, value in response.getheaders():
            if name.lower() in RESPONSE_DROP:
                continue
            self.send_header(name, value)

        if is_stream:
            # No length is known: the connection itself delimits the body, and
            # it must close so the client sees the end.
            self.send_header("Connection", "close")
            self.close_connection = True
            self.end_headers()
            while True:
                chunk = response.read(CHUNK)
                if not chunk:
                    break
                self.wfile.write(chunk)
                self.wfile.flush()
            return response.status, None, None

        if length_header is not None:
            data = response.read()
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(data)
        else:
            # Upstream sent a close-delimited body; relay it the same way.
            self.send_header("Connection", "close")
            self.close_connection = True
            self.end_headers()
            while True:
                chunk = response.read(CHUNK)
                if not chunk:
                    break
                self.wfile.write(chunk)
                self.wfile.flush()
            return response.status, None, None

        usage: dict[str, int | None] | None = None
        model: str | None = None
        try:
            answer = json.loads(data) if data else None
        except (ValueError, UnicodeDecodeError):
            answer = None
        if isinstance(answer, dict):
            usage = extract_usage(answer)
            model = model_of(answer, None)
        return response.status, usage, model


def main() -> None:
    if not CONFIG.accounting_token:
        print(
            "NESTBOX_ACCOUNTING_TOKEN is unset: no inspection and no accounting "
            "will succeed",
            flush=True,
        )
    if CONFIG.privacy_fail_closed:
        print("privacy fail-closed is ON: an unreachable control API answers 503", flush=True)
    ThreadingHTTPServer(("0.0.0.0", CONFIG.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
