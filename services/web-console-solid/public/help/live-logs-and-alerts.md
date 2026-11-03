---
title: Live logs and alerts
slug: live-logs-and-alerts
tags: logs, alerts, red lane, pop-out, dock, allow
anchors: [open-the-logs, panes, the-red-lane, answer-an-alert, the-dock-and-the-bell]
---

# Live logs and alerts

An *alert* is a destination a sandbox tried to reach and was refused, which nobody has answered yet. This page shows where alerts appear and what to do with them.

## Open the logs

Choose **Live logs** in the sidebar menu. A member sees their own sandboxes; an operator sees every sandbox of the data center. The same page opens for a single sandbox from its page. To open it in its own window, press **Pop out**; the window has no menus and keeps showing live data while you work elsewhere.

## Panes

The page shows up to four panes side by side. A pane can show one sandbox, all your sandboxes merged, a node (its sandboxes merged) or a platform component. In a pane header:

1. Choose the source.
2. Switch *Follow* on to keep the newest line in view.
3. Type in the filter box, or switch on *Problems only*.
4. Use the copy button, the pop-out button for this pane, or close the pane.

Press **Add pane** to add one, or **Pop out all**. Your layout is remembered.

## The red lane

Above the panes, refused connections for the shown sources collect in a red lane. Each entry names the destination (host and port), the sandbox and how many times it was refused. More than three collapse into "3 refused". An entry disappears once someone answers it. Colour is never the only signal: each entry also says "refused".

## Answer an alert

Four answers, on every alert wherever it appears:

1. **Allow now**: adds a permanent allow rule for that destination for that sandbox. Use it when you are sure.
2. **Allow for 15 min**: opens a timed window for that channel. Use it to try something. Choose other lengths from the same control.
3. **Draft rule**: saves a draft rule from the event without changing the sandbox. Review it later in **Policies** (see [Policies and the wizard](/docs/policies)).
4. Ignore: hides the alert in this browser until you reload.

A destination that has already been answered, by a rule or by an open window, does not alert again.

## The dock and the bell

- A new refused destination pops up as a card at the top right. Up to three show at once; older ones move to the dock.
- The dock's *Alerts* tab lists every open alert with the same four answers.
- The *Live events* tab streams events; refused lines flash red once.
- The bell in the topbar shows how many alerts are open. Click it to open the *Alerts* tab.

If you expect many refusals while setting up a sandbox, consider an [allow-everything window](/docs/egress-modes) and then create drafts from what it saw.

## Related

- [Egress modes and observation](/docs/egress-modes)
- [Keyboard shortcuts](/docs/shortcuts)
