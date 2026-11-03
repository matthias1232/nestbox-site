---
title: Your sandbox (for end users)
slug: workbox
tags: workbox, member, control strip, models, console, move
anchors: [the-control-strip, models, the-live-log, console, move-your-sandbox]
---

# Your sandbox (for end users)

If you are a member, **My sandboxes** is your home. Each sandbox you own has a page, the *workbox*, where you work and decide what the sandbox may do.

## The control strip

The strip at the top gives one-click switches. Every switch shows its state as a word.

- **Allow everything**: Off, 15 min, 1 h or until I stop. While on, a warning banner shows and a countdown runs.
- **Proxy mode**: Forward, Transparent, Hybrid or Off. See [Egress modes and observation](/docs/egress-modes).
- **Recording**: Full ("store flows, bodies and chats"), Flows ("store flows only") or Off ("store nothing, live view still works").
- **Internet, DNS, MCP**: open each one for a while without allowing everything.

## Models

The models card lists the models this sandbox may use. To add one:

1. Press **Add model**.
2. Choose from the models you may use that are not yet on this sandbox.
3. Confirm. The sandbox's agents learn about it within moments.

Press **Remove** on a row to take a model away from this sandbox only. Models granted to your whole data center are removed by an administrator. See [Deploy one model](/docs/deploy-a-model).

## The live log

Under the strip, the live log shows what the sandbox does on the network. A connection that was refused turns red and stays at the top. Each red line offers:

- **Allow now**: a permanent rule for this destination.
- **Allow for 15 min**: a timed window.
- **Draft rule**: saves a draft for an administrator or yourself to review.
- Ignore: hides it for this session.

Press **Create rule drafts from these** to turn everything refused so far into drafts in one go; the card then says "N drafts created, review them". See [Live logs and alerts](/docs/live-logs-and-alerts).

## Console

Press **Console** to open a terminal. The button is split: its menu offers Shell, Desktop and SSH. The last choice is remembered per sandbox. Choose **Pop out** to open the console in its own window. Shell tabs are shared tmux sessions, so a second person can watch the same shell.

## Move your sandbox

If the machine runs short of memory or you want your work somewhere else, press **Move…**, choose a node and review the preflight. Your files and the agent's login travel with it. See [Moving a sandbox](/docs/moves).

## Related

- [Getting started](/docs/getting-started)
- [Keyboard shortcuts](/docs/shortcuts)
