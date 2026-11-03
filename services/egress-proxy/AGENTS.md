# egress-proxy/ — the only way out of a sandbox

## Purpose

Envoy configured as a forward proxy, plus the one process Envoy cannot be: a
certificate broker that mints a leaf certificate per destination. Every outbound
connection from a sandbox passes through here, and for every request it asks the
control API what to do.

It owns **no rules**. The rule set lives in `network_rules` and is compiled by the
control API (ADR 0001, 0013). If this folder ever grows an allowlist, there are
two policies and one of them is wrong.

## The two promises

1. **Default deny, fail closed.** No matching rule is a deny. A control API that
   does not answer is a deny (`failure_mode_allow: false`). A sandbox with no
   rules reaches nothing. If the certificate broker is not running, CONNECT is a
   503, not an uninspected tunnel.
2. **What it could not see, it says it could not see.** A request that was
   decrypted says so (`listener: terminated`) and carries the real path, body
   and SNI. A CONNECT whose tunnel never produced a readable request says that
   instead (`listener: proxy`, `connect: true`, `path: null`, no decision).
   The console labels both; neither looks like an empty field that might be a bug.

A consequence worth stating in the same place: **a client that does not trust this
node's CA cannot use HTTPS through the proxy.** It gets a TLS error, not a blind
tunnel. There is no fallback, because a proxy cannot terminate a handshake and
then un-terminate it. This is why the CA certificate ships in every sandbox image
(`deploy/scripts/make-egress-ca.sh` writes it, the image build installs it as a
trusted root). An image without it is a broken image, and the log shows the
CONNECT with no readable request rather than pretending the request was empty.

## Content check on the decrypted hop (ADR 0059)

`mitm/dlp.go` asks the control API about every JSON body the broker forwards, and
does what the answer says: refuse with `403`, forward the rewritten text, or
forward unchanged. It is here and not in the Lua filter because a body rewrite
needs a reader, not a pattern match. `observe` is the only mode that forwards a
body it could not check; everywhere else an unanswered check is a refusal. Only
requests are inspected — a streamed answer cannot be rewritten mid-stream. The
sandbox id and the egress mode reach this process as upstream headers that Envoy
copied out of the policy answer; a client cannot set them.

### What the console learns about the far end (`NESTBOX_ROUTER_HEADERS`)

`mitm/meta.go` adds the address a request actually reached to the answer
(`x-nestbox-upstream-ip`), taken from the connection the transport opened and not
from the name in the `Host` header: a routing proxy makes the two differ, and the
console must show the address that was used. A refused request has no connection,
so `x-nestbox-upstream-ip-would` carries the first address the name resolves to,
looked up with a 1 s deadline and cached for 5 minutes. That field says "would
have reached" and never "reached".

When the sandbox sends its traffic through an LLM router, the router names the
provider and the model in headers of its own. `NESTBOX_ROUTER_HEADERS` names
them, comma separated, provider first and model second — for example
`x-omniroute-provider,x-omniroute-model`. Their values are copied to
`x-nestbox-router-provider` and `x-nestbox-router-model`, and only when the
router sent one. Unset or empty writes nothing, which is the truth for a direct
connection: an empty header would read as "the router was asked and did not
answer", a different fact.

A refusal this hop writes itself (`unavailable`, `too_large`) is reported to the
control API in the background, because the control API never saw that request and
the console would otherwise show a gap where a refusal was. A `blocked` refusal
is not reported twice: the control API wrote it, together with its findings.

## Update (ADR 0034)

The default HTTPS path is a *decided tunnel*, not the broker described below: the
Lua filter asks the control API about a CONNECT (`check_tunnel` in `extensions/mcp.lua`)
and refuses on anything but 200. Sandboxes are identified by `x-nestbox-source-ip`,
which the filter sets from the socket after removing every `x-nestbox-*` header the
client sent. The node id and token are written into `/tmp/envoy.yaml` and
`/tmp/mcp.lua` by `start-egress-proxy.sh`. The broker and the loopback listener are
kept for a deliberate TLS-reading mode.

## How a request flows

```text
sandbox ── CONNECT example.com:443 ──▶ listener `egress` :3128
                                          │  Lua: target host/port, connect=true, sni=CONNECT target
                                          │  buffer: disabled for CONNECT (a tunnel has no end of body)
                                          │  ext_authz: disabled for CONNECT (see below)
                                          ▼  connect_config: terminate, forward the TLS bytes
                                    egress-mitm :10001 (loopback)
                                          │  reads the ClientHello, mints a leaf for the SNI from the
                                          │  node CA, decrypts, re-encrypts with the same SNI
                                          ▼
                                    listener `egress_terminated` :10000 (loopback, TLS)
                                          │  buffer → lua → ext_authz → dynamic_forward_proxy → router
                                          ▼
                                    the destination, verified against system roots
```

Plain HTTP skips the broker: `listener egress` routes it straight out through
`dynamic_forward_proxy`, with the same filter chain.

Two decisions were made here that are easy to get wrong:

* **The CONNECT itself is not sent to the control API.** Envoy forwards a CONNECT
  to ext_authz as a CONNECT whose request target is `host:port`, which the
  control API's `/internal/policy/check` route cannot receive (and which would
  double every decision). The decision is made once, on the request that was
  decrypted, where the control API sees the path, the body and the MCP facts.
  A tunnel that opens sends nothing to the destination until that decision.
* **The buffer filter is disabled on the CONNECT route.** It waits for the end of
  the request body, and a CONNECT's body is the tunnel — so every CONNECT hung
  until the client gave up, with no Lua metadata and no decision. `BufferPerRoute`
  on the CONNECT route is what fixed it.

## Files

| File | Responsibility |
|---|---|
| `Dockerfile` | Two stages: the Go certificate broker, then Envoy pinned by digest |
| `envoy.yaml` | Listener 3128, terminating listener 10000, ext_authz, JSON access log, 64 KiB buffer |
| `extensions/mcp.lua` | Recognise JSON-RPC MCP on request and response; never rewrite a body |
| `mitm/main.go` | Sign a leaf per SNI, terminate TLS, forward to the inner listener |
| `mitm/restore.go` | Turn pseudonymised placeholders back into their values in the answer, for JSON and for server-sent events |
| `mitm/meta_test.go` | The upstream address from the connection trace, the `x-nestbox-upstream-ip-would` lookup, and the router headers |
| `mitm/livetap_test.go` | The live reader: the sandbox gets the same bytes, the control API gets the text so far |
| `mitm/main_test.go` | The certificate tests: host SAN, CA signature, cache |
| `start-egress-proxy.sh` | Start the broker, then `exec envoy` so Envoy stays PID 1 |

## The certificate broker

Envoy can terminate TLS, but it cannot **mint** a certificate: a TLS server must
present a certificate during the handshake, before anything could ask an API which
certificate belongs to the requested name. Envoy's `custom_tls_certificate_selector`
is an extension point without an implementation; SDS serves fixed names. So the
one capability Envoy lacks lives in `mitm/main.go`:

* reads the node CA certificate and key, exits if either is unreadable;
* one shared ECDSA P-256 leaf key, one certificate per SNI, cached for 24 h and
  renewed an hour before expiry;
* `DNSNames` for a host, `IPAddresses` for a literal address;
* no policy, no HTTP parsing, no log of its own. It never decides anything.

The broker's CA key is the only secret in the image's path. It arrives as a
**Docker secret**, never as a bind mount and never in the image.

### How the CA key gets into the container

`deploy/scripts/make-egress-ca.sh` writes `deploy/secrets/egress/ca.key` (0600)
and `ca.crt`. The compose service must mount them as secrets; the integrator adds
this, this folder cannot (compose is owned by the integrator):

```yaml
secrets:
  egress_ca_cert:
    file: ./secrets/egress/ca.crt
  egress_ca_key:
    file: ./secrets/egress/ca.key

services:
  egress-proxy:
    secrets: [egress_ca_cert, egress_ca_key]
```

Docker mounts those at `/run/secrets/egress_ca.crt` and
`/run/secrets/egress_ca.key` — inside that one container, mode 0444, not on the
host. A bind mount is the wrong tool: the file is 0600 on the host, so the
container user (uid 101, `envoy`) cannot read it, and making it world-readable to
compensate hands the CA key to every local user. A Docker secret is per-container,
a bind mount is per-host.

The broker can also be pointed elsewhere for tests with `NESTBOX_EGRESS_CA_CERT`
and `NESTBOX_EGRESS_CA_KEY`.

**Without the secret** the broker exits with a message, Envoy keeps running, and
every CONNECT fails with 503. That is deliberate: a proxy that cannot inspect
must not silently tunnel. Rotation means a new CA, a rebuild of every sandbox
image, and a restart of this container.

## The access log

One JSON line per stream, from two listeners with different `sni` sources:

| Field | Source | Meaning |
|---|---|---|
| `listener` | literal | `proxy` for what the sandbox sent, `terminated` for what was decrypted |
| `host` | `:authority` | `example.com:443` for a CONNECT, `example.com` for a request |
| `target_host` / `target_port` | Lua, split authority | the same value, split; port defaults from the scheme when absent |
| `path` | `:path` | null for a CONNECT — a tunnel has no path this listener can see |
| `connect` | Lua | `true` only for CONNECT streams |
| `sni` | `proxy`: Lua from the CONNECT target; `terminated`: the real ClientHello SNI | what the client asked for, and what it actually sent |
| `app_protocol` | Lua | `mcp` when the body is JSON-RPC MCP |
| `mcp_session_id` | request or response `Mcp-Session-Id` | the session, including the one the server assigns on `initialize` |
| `mcp_method` / `mcp_tool` | Lua from the request body | `initialize`, `tools/call`, … and `params.name` |
| `mcp_is_error` / `mcp_result_kind` | Lua from the response body | `true`/`false`, `result` / `error` / `tool_error` when the body was readable |
| `decision` / `reason` / `rule_id` | ext_authz response headers via `dynamic_metadata_from_headers` | what the decider said. Empty until a decider answers — that is the truth, not a bug |
| `would_deny` | `x-nestbox-would-deny` via `dynamic_metadata_from_headers` | `1` when only a window or an allow-all mode let the request through although a rule would have denied it |

A CONNECT tunnel has `idle_timeout: 10s` on both routes, so a changed egress mode
reaches the next connection within seconds instead of five minutes (ADR 0052); the
Lua filter calls `clearRouteCache()` after setting `x-nestbox-inspect`, because the
route is chosen from that header.

The decision headers are copied into filter metadata **by Envoy**, so neither the
sandbox nor the destination host sees them. The previous configuration read them
from `%RESP(...)`, where they never appear: ext_authz adds response headers to the
upstream *request* or the client *response*, not to the response this stream
proxies.

## MCP recognition

`extensions/mcp.lua` looks at `Content-Type: application/json`, requires
`jsonrpc: "2.0"`, and matches the MCP method family. For `tools/call` it reads
`params.name` — searching after the `params` key so a `name` field in a preceding
object is not mistaken for the tool. Nothing is rewritten; the body goes upstream
byte for byte.

The response side reads `Mcp-Session-Id` (servers assign it on `initialize`) and,
when Envoy has buffered the response body, `result` / `error` / `isError`. The
buffer filter buffers requests, not responses, so a large response is classified
by its headers only. Lua has no JSON library; batch requests are classified by
their first method. Both limits are stated in the file itself.

## Why Envoy (and why one Go process)

mitmproxy terminates TLS out of the box but puts a whole Python stack and event
loop in the path of every request. `agentgateway` is a reverse proxy with explicit
routes, which would duplicate `network_rules` with a config file. Envoy is one
static binary, has CONNECT termination and the decision stays where the rules are.

The Go broker is the smallest thing that closes the certificate gap: standard
library only, no HTTP parsing, no policy, no dependencies, and its tests run
during the image build. The alternative — an Envoy C++ certificate-selector
extension — is a larger, unmaintainable piece of code for the same result.

## The one component with extra rights

`NET_ADMIN` and `NET_RAW`, and only for the redirect rules that catch a tool which
ignores `HTTP_PROXY`. The sandbox network is `internal: true`, so a raw socket
still has no route out; the capabilities are for the proxy container's own
iptables, not for the sandbox. This is documented in the deploy README because it
is the only exception to "nothing is privileged".

## Not published on the host

An unauthenticated forward proxy on a LAN interface is an open relay with a
friendly name. Sandboxes reach it over the `runtime` network as
`egress-proxy:3128`; an operator who wants to test it goes through
`docker exec`. The broker and the terminating listener are bound to 127.0.0.1
inside the container and are unreachable from any network.

## Open work

- **The compose secret mount** (two lines, above) — without it termination is
  inert and HTTPS is a 503. This is the one integration step this folder cannot do.
- **The control API must accept the suffixed check path.** Envoy appends the
  request path to `path_prefix`: `POST /api/v1/internal/policy/check/` (a request
  for `/`) currently answers 404, so a rule never sees it. `…/check/foo` works.
  Route and handler live in `api/mod.rs` / `api/policies.rs`, which are not this
  package's files.
- The status endpoint that reports `policy_hash_reported`, which is what makes
  the console's "deployed" badge true.
- Body capture into the access log (base64). The bodies are already visible to the
  filters up to 64 KiB; putting them in the log line is a size decision, not a
  capability one.

## The hop that leaves for the internet

`mitm/upstream.go` listens on `127.0.0.1:10002`. Envoy's inner listener routes
every decrypted request to the `egress_reencrypt` cluster, which is this port; the
broker encrypts the request again towards the host in its `Host` header and checks
that server's certificate against the system roots plus `/etc/envoy/mozilla-ca.pem`
(both root lists). Envoy's own upstream TLS was dropped because it checked a second
name on a shared address against the first one's and refused wildcard certificates
(both a bare 503 in the sandbox). Policy is still decided by Envoy's ext_authz
before the request reaches this port; the port is loopback only.
| `mitm/livetap.go` | Report a streamed answer live, without changing a byte of it |
| `mitm/meta.go` | Record the upstream address and the router's own headers |
