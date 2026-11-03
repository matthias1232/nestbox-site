# runtime/ — execution

## Purpose

The boundary between deciding and doing. A service says "start this sandbox";
this layer knows how that is actually performed, and nothing above it knows.

## Why a trait

The product runs on Docker today and on OpenShell gateways next. Keeping the
adapter behind `RuntimeAdapter` means that change is one file, and it means the
services can be tested against `FakeAdapter` without a container runtime.

## Security rules

1. A command is an argument vector. No shell string is ever built from a name, a
   label, or anything else that came from a request.
2. Only images from the approved catalogue may be started; the sandbox service
   rejects an unpinned image before this layer is reached, and this layer refuses
   one too rather than trusting the caller.
3. A new container runs without privileges: no privileged mode, no host network,
   dropped capabilities, resource limits set.
4. The adapter holds the only path to the container socket, and that socket is
   mounted into the runner, not into the API.

## Files

| File | Responsibility |
|---|---|
| `mod.rs` | The `RuntimeAdapter` trait (create, inspect, list, lifecycle, logs, exec, hardware, processes, inventory, build, model download/serve/stop, terminal), the `TerminalSocket` port, and its errors |
| `http_runner.rs` | Adapter that speaks to the sandbox runner's internal API |
| `fake.rs` | In-memory adapter for tests, including a terminal whose other end the test drives |

## Extending

Implement `RuntimeAdapter`, add it to the construction in `main.rs`, and say in
the module docs which guarantees it does *not* provide. A fake that quietly
satisfies more than the real thing hides bugs.

## Hardware and builds

`hardware()` returns the runner host's raw `/proc/cpuinfo`, `/proc/meminfo`,
and `nvidia-smi` text plus whether the Docker daemon has an NVIDIA runtime.
Raw rather than parsed, because the parsers are tested Rust in
`services/model_service.rs`. `build()` sends a generated Dockerfile and gets an
image tag back; the runner only accepts tags under `nestbox-recipe/`.

## Live reads

`processes()` and `inventory()` are questions about a container *now*, so they
are calls rather than stored state: the runner answers with `docker top` and with
the output of a probe script that ships in its own image.

Both are parameterless on the container side. The `ps` column set and the probe
script are the runner's, never the request's, so neither an agent inside the
container nor a caller outside it can steer what is read. The process table comes
back typed and capped (the runner says whether it cut it); the inventory comes
back as the probe's document, because the runner may gain a section and the table
it lands in has fixed columns — `services/inventory_service.rs` is what rebuilds
it by allow-list, and that rebuild is where a value would be rejected.

## The terminal

`open_terminal` is the one method that returns a live channel instead of a
finished answer, because a shell is a conversation. Two things follow:

1. **It is a port, not a socket.** `TerminalSocket` is a trait, so the session
   rules in `services/terminal_service.rs` are tested against a fake, and the day
   the runtime is an OpenShell gateway exactly one adapter changes.
2. **The command is fixed.** The runner starts `bash` with a PTY; nothing from a
   request reaches the command line. This layer moves bytes and a resize frame,
   and nothing else.

The runner demands a node token on the upgrade. `HttpRunner` reads it from
`NESTBOX_NODE_TOKEN_FILE` (preferred, ADR 0005) or `NESTBOX_NODE_TOKEN`, and
sends it as the second entry of `Sec-WebSocket-Protocol` — a header, not a query
string, because a URL ends up in access logs and in error messages. With no token
configured the runner refuses every terminal, which is the honest failure: the
runner also sits on the sandbox network, so an unauthenticated endpoint there
would let one container open a shell in another.

It is read **when a terminal opens**, not cached at start (ADR 0016): rotating
the file applies to the next session without restarting the API. One consequence
is worth knowing: the value must be a single header token — hex, not base64,
whose `+`, `/` and `=` the handshake parser rejects with a bare 400.

A text frame from the runner is output, except a JSON object with
`"type":"close"`, which is the reason the session ended. `close_control_frame`
is that rule, and it is unit-tested here rather than trusted.

## The dispatcher

`router.rs` implements `RuntimeAdapter` in front of one runner per node (ADR 0047). A call about a
sandbox goes to the runner of the node it is placed on (looked up once, remembered); `create` carries
its node in the spec; `list` merges every runner that answers. It does not decide placement and does not
fail a sandbox over to another node. `for_node` gives the adapter of one node for the calls that name a
node instead of a sandbox (hardware, images, snapshots).

`download_model` and `serve_model` belong to that second group: the request names the install's node,
so both follow `request.node_id` — a joined node fetches and serves on its own disk. `stop_model`
stays local because a model is named by its key alone and nothing in the call says which machine.

`RuntimeRouter::host_load_for(node)` is a Wave 0 stub for package B07: the local node answers, a remote runner is reported
unreachable until B07 routes the call to it.
