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
| `ControlStrip.jsx` | The one-click strip: allow everything, proxy mode, recording, Internet / DNS / MCP switches; every click is one `quick.set` call |
| `controlStrip.js`, `controlStrip.test.js` | Pure logic of the strip, the live log lines and the rules card: `stripState`, `nextAllowAllAction`, `logLine`, `laneAlerts`, `summarizeTraffic`, `notYetGranted` |
| `LiveLogCard.jsx` | 300 px live log of this sandbox with a red lane (`AlertPopup inline`), filter chips, Pause and pop-out |
| `RulesFromLogCard.jsx` | "N destinations were seen, M would have been refused" and the bulk-draft button |
| `NetworkCard.jsx` | now only the open evaluation windows and the recreate hint; the mode switch moved to the strip |
| `ModelsCard.jsx` | "Add model" offers the models the user holds and this sandbox lacks, with "Another endpoint" as the old free form |
| `MyBoxesPage.jsx` | box cards with status, node, allow-all badge and Open / Shell / Desktop |

Note: `SessionsPanel.jsx` now renders `components/ConsoleFrame.jsx` under the presence line.


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `agents.test.js` | Node tests for `agents`. |
| `move.test.js` | Node tests for `move`. |
