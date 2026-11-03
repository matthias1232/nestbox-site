# 0052 — Gateway sandboxes are driven through the gateway, and the proxy chain does not reach them

## Context

OpenShell is the default runtime. A gateway sandbox is a Docker container with a
boundary inside it: the supervisor gives the workload its own network namespace,
the environment and the policy, and decides every connection (OPA, and an L7
parser for HTTP and MCP).

Three things turned out to be different from a Docker sandbox, and each one broke
something that had worked there.

## Findings (OpenShell 0.1.6, verified on the appliance)

* **`docker exec` runs outside the boundary.** No proxy, no CA, no route: `curl`
  answers 000 and the CA file is present but unused. A terminal, a probe or a
  desktop bridge started that way sees a different machine than the agent.
* **The filesystem policy is startup-only and tight.** Only `/tmp` and `/dev/null`
  are writable. `tmux` needs `/dev/ptmx` and `/dev/pts`, Chrome and the VNC server
  need `/dev/shm`, and the X server compiles its keymap into `/var/lib/xkb`.
* **The syscall filter refuses every `setuid`-family call, even to the same id.**
  The X server starts `xkbcomp` through a helper that calls `setuid(getuid())` and
  treats the refusal as fatal ("Failed to compile keymap").
* **A connection to our egress proxy is parsed as HTTP and `CONNECT` is rejected**
  ("request-target is not a valid origin-form path"), with or without
  `protocol: tcp` on the endpoint. A forward proxy chained behind the gateway's own
  proxy therefore does not work. Reverse-proxy style routes (origin-form requests,
  such as the accounting proxy's `/s/<sandbox>/…`) do.
* The gateway runs the command it is given *instead of* the image's entrypoint.

## Decision

* **Everything that looks into or acts on a gateway sandbox goes through the
  gateway** (`gateway_exec.py`): the terminal (`sandbox exec --tty` on a local PTY,
  `tmux` inside, so a session outlives the browser), the desktop bridge (`sandbox
  exec` pipes to the sandbox's loopback VNC port), and the probes for processes,
  connections, commands and inventory.
* **The runner gives every gateway sandbox a filesystem policy** with the devices a
  terminal and a desktop need, at creation. The desktop image ships a ten-line
  preload library (`idfix.c`) that answers `setuid(getuid())` with success; it
  cannot change an identity, and every other call still reaches libc.
* **Recipe images run as `nestbox`**, not root: the gateway refuses a working
  directory its user cannot write. The user is added after all install steps, so
  cached layers stay valid. A recipe sandbox starts `nestbox-entrypoint`
  explicitly.
* **Crashed gateway sandboxes come back.** OpenShell ends a workload whose
  supervisor lost the gateway (phase `Error`). The reconcile loop restarts (stop,
  start) any sandbox that is `running` in the database but not on the runtime, with
  doubling back-off and an audit line per attempt. A stopped sandbox is left alone.
* **Gateway sandboxes are observed and governed by the gateway.** Live traffic comes
  from its OCSF events (binary, destination, decision, DNS); rules are pushed as
  gateway policy. Reading TLS bodies for chats and token use needs the egress proxy
  and so needs the Docker runtime; `direct` also stays on Docker.

## Consequences

The egress modes `transparent` and `hybrid` ("agents only") have an effect only on
the Docker runtime. On a gateway sandbox, usage can still be measured for agents
that are pointed at the accounting proxy by path, but not from a signed-in account's
direct calls. This is a product decision still to be made: either reading modes put
a sandbox on Docker, or the gateway's L7 inspection is extended.
