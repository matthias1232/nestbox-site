# features/workbox

The end-user view of one sandbox (`/box/<id>`) and the list of the user's own
sandboxes (`/my`): the switches for the agents' sign-in, the proxy mode and the
evaluation windows, plus live chats, the topology and the shared terminals.

## Files

| File | What it shows |
|---|---|
| `WorkboxPage.jsx` | the whole view for one sandbox: header, cards, panels |
| `MyBoxesPage.jsx` | the sandboxes of the signed-in user |
| `useWorkbox.js` | `useWorkbox` (sandbox row + control summary, polling and events) and `useMyBoxes` (the list plus the cheap overview); `splitByOwner` for "Mine" |
| `AgentsCard.jsx` | one row per agent: subscription ↔ gateway, sessions, restart |
| `agents.js` | its pure logic: agent labels, the busy sentence of the switch dialog, the login line |
| `NetworkCard.jsx` | the four proxy modes and the evaluation windows |
| `AllowAllBanner.jsx` | the loud banner while everything is allowed |
| `network.js` | the proxy modes, durations and banner sentence; pure, tested with `node --test network.test.js` |
| `PrivacyCard.jsx` | The DLP findings of this sandbox for its owner: the dashboard and the masked log, collapsed by default |
| `SessionsPanel.jsx` | all shared terminal tabs and the desktop |
| `TerminalPopout.jsx`, `DesktopPopout.jsx` | the pop-out windows without a sidebar |
| `SessionsPanel.jsx` | all shared terminal tabs ("Connected now" from `control.presence`, then terminals and desktop behind two tabs) |
| `sessions.js`, `sessions.test.js` | pure logic of the shared tabs: `mergeTabs` (my list + everybody's, each tab once), `sshCommand`, `puttyHint`; tested with `node --test sessions.test.js` |
| `TerminalPopout.jsx`, `DesktopPopout.jsx` | the pop-out windows without a sidebar: one tab with its viewers, or the desktop, each filling the window |
| `LiveChatsPanel.jsx` | the agents' conversation, live |
| `livechats.js` | turns a recorded call into a turn, groups turns by session, shortens text (pure, tested by `livechats.test.js`) |
| `TopologyPanel.jsx` | the live topology of the sandbox |
| `ModelsCard.jsx` | the models the sandbox may use, with warm-up |
| `models.js`, `models.test.js` | the card's logic: readiness badges, scope labels, who may manage |
| `MoveDialog.jsx` | preflight and progress of a move to another node |
| `move.js` | the nine move steps, their state map from events and polls, the node order |
| `useLive.js` | one subscription to the state stream, filtered per sandbox |

## Rules

This feature may import components from `features/sandboxes/` (`TerminalTab`,
`DesktopTab`, `StackTopology`, `chats.js`). That is an explicit exception to the
one rule in `../AGENTS.md` ("a feature may not import from another feature"),
because a second terminal and a second desktop would be a second truth.

Every panel receives its data as props or loads it through `api/`, never by
reaching into another feature's state.

The vocabulary travels the other way once: `features/sandboxes/PoliciesTab.jsx`
imports the mode labels from `network.js`, so an administrator and a user read
the same words for the same mode. That module is data and pure functions only —
no state, no API call.
`sessions.js` is the reverse of the exception above: `features/sandboxes/` takes
that one module from here, because the tab strip in the Admin Center and in the
Workbox is the same list. It is pure logic — no JSX, no state, no API call — so
it cannot become a second truth. The same holds for `useLive.js`, which a
terminal imports for its presence events so a page keeps one event stream.
| `RequestStreamCard.jsx` | Which models this sandbox uses right now |
| `WorkboxNotice.jsx` | What the end user is told about who may read the chats |
| `LiveEventsPanel.jsx` | Proxy, OpenShell and MCP events of this sandbox |
| `ControlStrip.jsx` | The one-click strip in two rows of labelled groups: "Allow everything", "Proxy mode", "Recording", then "Internet" / "DNS" / "MCP" with their switch and a "for" select (15 min / 1 h / 24 h). Every click is one `quick.set` call |
| `controlStrip.js`, `controlStrip.test.js` | Pure logic of the strip, the live log lines and the rules card: `stripState`, `nextAllowAllAction`, `logLine`, `laneAlerts`, `summarizeTraffic`, `notYetGranted`. `stripState` reads only the keys `/sandboxes/{id}/control` really answers (`egress`, `windows`, `allow_all`, `allow_all_until`, `recording`); the recording level comes from `recording.mode` (or a bare level string an older answer carries), and without it the strip says full. `logLine` reads a live stream frame (flat, tagged `type`/`kind`) and a history row (`detail`) |
| `LiveLogCard.jsx` | 300 px live log of this sandbox with a red lane (`AlertPopup inline`), filter chips, Pause and pop-out; each row shows time, decision, kind, the destination (or hostname / model and status), protocol and binary |
| `RulesFromLogCard.jsx` | "N destinations were seen, M would have been refused" and the bulk-draft button |
| `NetworkCard.jsx` | now only the open evaluation windows and the recreate hint; the mode switch moved to the strip |
| `ModelsCard.jsx` | "Add model" (in the card head, right of the title) offers the models the user holds and this sandbox lacks, with "Another endpoint" as the old free form |
| `MyBoxesPage.jsx` | box cards with status, node, allow-all badge and Open / Shell / Desktop |

Note: `SessionsPanel.jsx` now renders `components/ConsoleFrame.jsx` under the presence line.

## Layout and language

Every card of this page is a `section.wb-card` (or a `div.wb-card`), and its one
title sits in a `div.wb-card-head` together with the one action of that card, so
a button ("Add model", "Show") stays inside the card's frame instead of floating
next to a bare heading. `.wb-cards` stretches its cells: cards that start in the
same row share their top and bottom edge and one column width, whatever each
holds. `.wb-main` keeps its panels at their own height.

The control strip is two `div.wb-strip-row` lines of `div.wb-strip-group`. A group
is one `span.wb-strip-label` with the exact words ("Allow everything", "Proxy
mode", "Recording", "Internet", "DNS", "MCP") followed by its control and, for a
channel, the "for" select. The strip sits inside the page padding: no negative
margin and no `overflow: hidden` may be added, because that clips the first label.

Language: every string in this folder is English. "Findings", "Show"/"Hide" and
"Events appear here" are the words for the privacy card, its button and the
events stub — no German label may come back here.


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `agents.test.js` | Node tests for `agents`. |
| `move.test.js` | Node tests for `move`. |
