---
title: Egress modes and observation
slug: egress-modes
tags: egress, proxy, ca, windows, allow all, recording
anchors: [the-four-modes, switch-the-mode, the-ca-certificate, allow-everything-for-a-while, recording]
---

# Egress modes and observation

*Egress* is traffic leaving a sandbox. The egress mode decides how Nestbox sees and controls it.

## The four modes

| Mode | What it does |
|---|---|
| Forward proxy | The sandbox is told to use the node's proxy. Rules are enforced; HTTPS content can be inspected. |
| Transparent | All web traffic is redirected to the proxy without the program knowing. |
| Hybrid | Both: programs that honour the proxy use it, the rest are redirected. |
| Off, allow all | Rules are not enforced. The proxy tunnels everything and records host, port and bytes. |

## Switch the mode

1. Open the sandbox. On the workbox, use *Proxy mode* in the control strip. In the Admin Center, use the egress card.
2. Choose Forward, Transparent, Hybrid or Off.
3. The next request is decided by the new mode. Open connections are cut within about 10 seconds.

A mode can be switched back automatically after a number of minutes. Only the `direct` mode needs a recreate, because it changes the container's network (ADR 0053).

## The CA certificate

To inspect HTTPS, the proxy signs connections with Nestbox's own certificate authority. Every sandbox that can use the proxy has it installed from the start, at `/run/secrets/egress-ca.crt`, and a bundle in `$HOME/.nestbox/ca-bundle.crt` that `SSL_CERT_FILE` points to. That is why switching modes does not require a rebuild. A program that ignores these variables may show a certificate error; point it at the bundle.

## Allow everything for a while

Sometimes you want to install something or let an agent work, and watch what happens.

1. Choose **Allow for 15 min** (or 1 h, or until you stop) in the control strip or the Admin Center.
2. A warning banner shows for as long as it is open.
3. Everything is allowed, but rules are still evaluated and `would_deny` is recorded: you can later turn what was observed into draft rules.

This opens a *window*. A window has a channel (proxy, DNS, MCP, gateway), a kind (*bypass* allows quietly; *audit* allows and records it as a block) and an end time or none. Closing the last window ends the state. Unbounded windows are allowed on purpose but are always badged in lists (ADR 0054).

## Recording

Recording decides what is stored for a sandbox.

- **Full**: flows, bodies and chats.
- **Flows**: flows only; no bodies or chats.
- **Off**: nothing is stored. The live view still works.

A mute ends after the minutes you set (24 hours by default) and the sandbox returns to full.

## Related

- [Live logs and alerts](/docs/live-logs-and-alerts)
- [Policies and the wizard](/docs/policies)
