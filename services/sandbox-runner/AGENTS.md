# sandbox-runner — the Docker adapter

## Purpose

The only process that holds the Docker socket. The control API never talks to
Docker; it talks to this service over an internal HTTP contract, and this
service turns the request into a container call.

That separation is the security boundary: a compromise of the API does not
yield the host, because the API has no socket. It is also the migration path:
replacing Docker with NVIDIA OpenShell means replacing this adapter, not the
control plane.

## Contract (internal, not published)

| Method | Path | Does |
|---|---|---|
| `GET` | `/healthz` | Docker reachable |
| `GET` | `/v1/sandboxes` | Every managed container with live facts |
| `POST` | `/v1/sandboxes` | Create from an approved image; idempotent on name |
| `GET` | `/v1/sandboxes/{name}` | Live facts for one |
| `POST` | `/v1/sandboxes/{name}/{start,stop,restart,remove}` | Lifecycle |
| `GET` | `/v1/sandboxes/{name}/logs?tail=` | Last lines of stdout/stderr |
| `POST` | `/v1/sandboxes/{name}/exec` | Run one argument vector, return output |
| `GET` | `/v1/hardware` | Raw `/proc` and `nvidia-smi` text for the control plane to parse |
| `POST` | `/v1/builds` | Build a recipe image from a generated Dockerfile (the body may carry the desktop pack's embedded scripts; `MAX_DOCKERFILE_BYTES` is 1 MiB) |
| `POST` | `/v1/catalogue` | Search a baseline image's native package index in a throwaway container |
| `POST` | `/v1/images/verify` | Check a built image's package database against the recipe's packages |
| `POST` | `/v1/models/download` | Fetch one GGUF into `NESTBOX_MODEL_BASE` |
| `POST` | `/v1/models/serve` | Start one model server container; on a joined node publish it on the node's overlay address and answer `published: "<ip>:<port>"` |
| `GET`/`DELETE` | `/v1/models/serve/{key}` | Inspect or stop it |
| `GET` | `/v1/egress` | The sandbox network, the proxy, and the redirect state, as facts |
| `GET` | `/v1/sandboxes/{name}/stats` | One `docker stats` sample |
| `GET` | `/v1/sandboxes/{name}/processes` | `docker top`, capped and typed |
| `GET` | `/v1/sandboxes/{name}/inventory` | A fresh snapshot from the fixed probe |
| `GET` | `/v1/sandboxes/{name}/terminal` | The terminal WebSocket |
| `PUT`/`GET`/`DELETE` | `/v1/sandboxes/{name}/forwards` | Hold a sandbox's published ports open (complete set + public keys); report `listening` and `target_up`; close them |
| `GET` | `/v1/host/load` | The host's CPU, memory, disk, load average, and what running sandboxes were promised |
| `PATCH` | `/v1/sandboxes/{name}/resources` | `docker update` memory, CPU and PID limits of a running sandbox |
| `GET` | `/v1/forwards` | Every forward and the published range |
| `GET` | `/v1/system/components` | State and health of the appliance's own containers (fixed list, own project only) |
| `GET` | `/v1/system/components/{service}/logs?tail=` | Last lines of one of them |

## Security rules

1. Only images in `NESTBOX_ALLOWED_IMAGES` can be started. A request for any
   other image is a 422, whatever the API said.
2. Containers run with every capability dropped, `no-new-privileges`, a memory
   limit, a CPU limit, and a PID limit. No privileged mode, no host network, no
   host mounts; the request model has no field for them.
3. A command for `exec` is an argument vector. There is no field that takes a
   shell string, and the adapter never builds one.
4. Exec output is capped (default 64 KiB) and the call has a timeout, so one
   command cannot exhaust the API's memory or hold a worker forever.
5. The runner's API (port 9000) listens only on the internal `control` network and
   is not published on the host. What is published is the forward range
   (`NESTBOX_FORWARD_RANGE`, default `22200-22299`): one listener per sandbox
   forward, an SSH door (`ssh_gateway.py`, key auth, no SFTP, no tunnels) or a
   byte relay to the sandbox (ADR 0033).
6. A model server starts only from an image in `NESTBOX_MODEL_IMAGES`, compared
   as an exact `repo@sha256:…` reference. Every absolute path in its command
   must be the mounted model file, the model directory is mounted read-only at
   `/models`, and the container joins the internal runtime network only — so it
   can answer the API and the sandboxes, and reach nothing else.
7. A model download is written to `<file>.part` and renamed after the last
   byte; the key, repository, and file name are validated and the resolved
   directory must stay inside `NESTBOX_MODEL_BASE`.
8. The sandbox network is created `internal: true`, and a network that exists
   with `internal=false` is refused with 503 instead of reused. See below.
9. The egress proxy is joined to that network by this adapter, under the alias
   the sandbox environment names.

## Publishing a model server on a joined node

A sandbox and the control plane reach a local model server by container name, on
the runtime network. A joined node has no such name: the control plane is on
another machine and arrives through the overlay, so the server has to be
published there. When `NESTBOX_FORWARD_BIND` and `NESTBOX_MODEL_PUBLISH_RANGE`
are both set (the node package sets them), `serve_model` maps container `8080` to
the first free port of the range on that address and answers
`published: "<ip>:<port>"`; the control plane writes that into the provider
profile. `pick_port` is the decision, `port_is_free` binds the candidate and
closes it again, and `published_address` reads the mapping back from Docker, so a
re-serve after a runner restart reports the port this process did not choose.
The appliance's runner sets neither variable and keeps today's behaviour.

## The sandbox network has no route out

Docker has no per-container "no default route", so the network carries the
property: `internal: true` means the bridge has no gateway and no NAT. Measured
on the appliance, a container on it has a route to its own `/16` and nothing
else — `/proc/net/route` holds one entry with gateway `0.0.0.0`, `wget` to a
public address fails with `Network unreachable`, and an external name answers
`SERVFAIL`. That last point is why the environment the control API writes names
the proxy by hostname: container names resolve, public names do not.

`ensure_network` creates the network and refuses a drift. A network that already
exists with `internal=false` is the state this replaced — every sandbox on it had
a default route and full egress, and nothing was policed while the compose file
said `internal: true` — so a create against it fails instead of quietly
succeeding. `internal` cannot be flipped on a live bridge, so the fix is to
remove the network; the next create recreates it. `/healthz` and `/v1/egress`
report which state you are in.

The proxy is on that network because this adapter puts it there
(`attach_egress_proxy`), with `aliases=["egress-proxy"]`: a plain
`network connect` gives a container only its container name, so without the
alias the sandbox environment would name a host that does not resolve. It is the
only container besides this adapter that belongs on the network; the control API
stays on `control`. The join happens on `create` and again on `start`/`restart`,
because a `docker compose up` that recreates the proxy container drops its
endpoint and an existing sandbox would otherwise come back with a proxy it cannot
reach. The compose file passes this adapter the network name Compose actually
creates (`${COMPOSE_PROJECT_NAME:-nestbox}_runtime`), so the sandbox, model, API,
and proxy containers share one internal bridge; the runtime connect is then a
no-op that only repairs a proxy that Compose recreated.

## Redirect rules, and what they are not

`prepare_redirects` installs `80/443/8080 → 3128` in the proxy container's own
NAT table, idempotently (`-C` before `-A`). The proxy container is the only
component with `NET_ADMIN` (deploy README); a sandbox has every capability
dropped and cannot install anything, which is the point.

The rules are convenience, never the boundary. A sandbox has no default route, so
a raw socket is already unroutable; the redirect only helps a client that
addresses the proxy as a gateway-like host on 80/443/8080 instead of 3128. A
failure to install them is therefore logged and reported, not fatal: a redirect
can only narrow what is reachable, so its absence cannot open a hole. The envoy
image as pinned has no `iptables` binary, so `/v1/egress` currently answers
`unavailable: the proxy image has no iptables` — the honest value, and the
package that owns `services/egress-proxy/Dockerfile` can add the package.

## The PTY

`terminal.py` is the second transport: `exec_create(tty=True, stdin=True)` plus
`exec_start(socket=True)`, and the socket is pumped in both directions. Exec
stays request/response because that is what a probe and the console's "run" box
need; a shell needs a terminal, and `vim` and `htop` need a window size, which is
what `{"type":"resize","cols":80,"rows":24}` is for.

Three rules hold it together:

1. **Exactly one reader for the PTY, for the life of the session.** A reader per
   loop iteration looks simpler and silently drops bytes: cancelling an
   `asyncio.to_thread` does not stop the thread, so the abandoned reader steals
   the next chunk.
2. **The PTY is closed on every path**, including the idle timeout and a browser
   that vanishes. Closing it is also what releases the blocked read.
   `PtySocket` also clears the 60 s timeout docker-py leaves on the hijacked
   exec socket: left in place, `recv` raises after a minute of silence, the pump
   reads that as the shell exiting, and every idle terminal dies after sixty
   seconds regardless of the idle policy. A read timeout is never EOF.
3. **Nothing is logged or recorded.** The control API keeps who/when/how long
   (ADR 0023); a keystroke in a log line here would undo that.

The endpoint requires the node token in the handshake (`Sec-WebSocket-Protocol`
or the `token` query parameter), read from `NESTBOX_NODE_TOKEN_FILE` when one is
configured and from `NESTBOX_NODE_TOKEN` otherwise, and refuses every connection
when neither is set: the runner is also on the sandbox network, so an
open terminal endpoint there would be a shell into every neighbour.
`NESTBOX_TERMINAL_IDLE_SECONDS` exists so a test can watch the idle close in
seconds instead of half an hour.

## Files

| File | Responsibility |
|---|---|
| `app/main.py` | The FastAPI adapter, the sandbox network, the egress wiring, and model serving |
| `app/ca_bundle.py` | The CA and CA-bundle files a sandbox is given: built here, mounted read-only in every mode that uses the proxy, never in `direct` |
| `app/images.py` | The one image allowlist shared by sandbox creation and the package catalogue |
| `app/catalogue.py` | Live package search and post-build verification: fixed scripts, validated query, throwaway container |
| `app/stats.py` | One-shot `docker stats` samples and the poster loop |
| `app/processes.py` | `docker top` for one sandbox |
| `app/commands.py` | What starts in a sandbox and by which way in (terminal, ssh, inside), from the parent chain; a ring buffer filled while somebody polls |
| `app/gateway_exec.py` | Running things inside a sandbox whichever runtime holds it: `exec_run` (probes), `GatewayPty` (terminal through `openshell sandbox exec --tty`; `open_command` for any argv, as the SSH door needs), `GatewayStream` (desktop bridge). `docker exec` runs outside a gateway sandbox's boundary, so gateway sandboxes never use it |
| `app/ssh_gateway.py` | The SSH door: key auth, no SFTP, no tunnels. A login lands in the tmux tab `nestbox-ssh` (`attach <tab>` joins a browser tab) and a dropped client only detaches; gateway sandboxes go through `GatewayPty.open_command` |
| `tests/test_ssh_gateway.py` | Frame demux, the tab-versus-command choice, and the end of a session (Ctrl-D without tmux, detach with it), without Docker |
| `app/connections.py` | The sockets of one sandbox and the process that owns each, by a fixed `sh` probe run as the sandbox user; a sample, not a trace |
| `app/vnc.py` | The desktop transport: node token, then a fixed `python3` bridge in the container copies bytes between `127.0.0.1:5900` and the WebSocket; nothing is recorded. With `?view=1`, `RfbClientFilter` lets the RFB handshake and framebuffer requests through and drops every key, pointer, clipboard and resize message |
| `app/inventory.py` | Runs the probe and parses its sections |
| `app/inventory.sh` | The fixed probe script, executed inside the container |
| `app/terminal.py` | The browser terminal: node token, PTY pump, resize, idle close |
| `app/remote_ssh.py` | `POST /v1/remote/ssh-run`: one script over SSH on a machine that is not a node yet, credentials used once |
| `tests/test_terminal.py` | The pump and the frame rules, without FastAPI and without Docker (`python3 -m unittest discover -s tests -t .`) |
| `tests/test_ca_mounts.py` | Which egress modes get the CA and the bundle, and what the bundle contains |
| `tests/test_vnc.py` | The view-only RFB filter: what passes, what is dropped, what makes the stream close |
| `tests/test_remote_ssh.py` | `env_prefix` quoting and which environment keys are refused, without a machine to reach |
| `tests/test_model_publish.py` | The model publish port: which range a joined node publishes in, the first free port, and the address read back from a running container |
| `app/telemetry_shipper.py` | Reads the egress proxy's JSON access log from its container output and posts it as batches; the field map is pure, including the `would_deny` verdict |
| `tests/test_terminal.py` | The pump and the frame rules, without FastAPI and without Docker (`python3 -m unittest discover -s tests -t .`) |
| `tests/test_telemetry_shipper.py` | The access-log field map, the batch size cut, and the ship/drop/retry rule, without Docker |
| `Dockerfile` | Image build |

## Samples, processes, inventory

Three reads of the same container, each with its own cost:

- **A sample is one `docker stats` one-shot**, not a stream. A stream is a socket
  per sandbox that has to survive a runner restart, and the console only needs a
  point every five seconds. The poster thread starts only when
  `NESTBOX_METRICS_POST_URL` is set and posts one batch per tick with
  `NESTBOX_NODE_TOKEN`; a failed batch is dropped rather than queued, because a
  queue of stale points draws a lie.
- **The process table is `docker top`**, which is the list `ps` inside the
  container would show without running a command through the agent's shell. The
  `ps` column set is a constant in `app/processes.py`: `ps` accepts a format
  string, and a caller-supplied one is enough to read a file the caller should
  not see. A stopped container answers 409, not an empty table.
- **The inventory is a script that ships with this image**, piped to `sh` inside
  the container. It takes no argument, so nothing an agent inside the container
  or a caller outside it can do changes what it prints. It prints environment
  variables **by name only** (`cut -d= -f1`), and lists files under the agent's
  home rather than reading them. A partial answer carries a `note` instead of
  looking like a complete one.

## Catalogues and verification

Two questions, both answered by a container rather than by a scraped page:

- **What does this distribution ship?** `POST /v1/catalogue` starts the baseline
  image, runs `apt-get update && apt-cache search`, `dnf list --available` or
  `apk search`, and returns parsed hits. The query is a package-name fragment
  validated against a regex and passed as `NESTBOX_QUERY`; the script per manager
  is a constant. The container runs under the same allowlist, capability drop and
  limits as a build, and is removed on every path, including a timeout.
- **Does the built image contain what the recipe asked for?** `POST
  /v1/images/verify` starts the built tag and reads its package database
  (`dpkg-query`, `rpm -qa`, `apk info -v`). The control API fails the build when
  a package is missing, because "the Dockerfile ran" is not "the tool is there".

The ten-minute cache in `app/catalogue.py` is keyed by image, manager and query.
It is a courtesy to the package index, never control state: emptying it changes
nothing an operator can observe except how long the next search takes.

`app/remote_ssh.py` runs a script over SSH on a machine that is not a node yet (ADR 0057): the
credentials arrive with the request and are used once. The host key is not checked, on purpose —
a first contact has no trusted key, and what is trusted afterwards is the WireGuard key the new
node answers on its overlay address. `env_prefix` single-quotes every value and refuses a key
that is not a shell name, and no password, key or environment ever reaches a log line.
