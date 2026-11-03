# live/ — the agent situation

## Purpose

The promise: see what every agent is doing **right now**, and what it did in the
last five minutes, without refreshing anything.

One node per agent, one edge per recent target, and a dot on an edge for each
event that actually arrived. Motion is evidence, never decoration: an idle map
is still, because an idle environment is still.

## How it works

- The snapshot is read first (`live/agents/snapshot`); the stream
  (`live/agents`) is opened at the same time and creates nodes for agents the
  snapshot does not know yet. An event is newer evidence than a snapshot that
  never heard of the agent.
- The stream is one `EventSource` for the page, closed on unmount. A reconnect
  refreshes the snapshot, because the gap may have hidden events that no retry
  can replay.
- The event log is bounded per agent (20 rows), and the animation queue is
  bounded too. A live view whose memory grows with runtime fails during the
  incident it exists for.
- Counters are the snapshot value plus the events received since that snapshot.
  A refreshed snapshot resets the live delta so the same event is not counted
  twice. Tokens are only shown when the stream reports them; otherwise `n/a`,
  because the snapshot carries no token total.
- `prefers-reduced-motion` turns the moving dot into a static highlight of the
  edge, the target, and the agent. No animation frame runs when motion is
  reduced; only the highlight's removal timer remains.

## Layout

The map is drawn in a 720 px viewBox with a deterministic layout: agents sorted
by sandbox and name down the left, up to four recent targets per agent on the
right. Each agent gets its own vertical band, so nodes cannot collide as agents
or targets are added. Targets are re-used by `kind|target`; the four most recent
stay, then they are sorted, so a new event does not reshuffle the map.

## States

| State | What is shown |
|---|---|
| Connecting, no snapshot yet | "Connecting to the live agent stream…" |
| Empty snapshot, no events | "No agent activity yet — this is not an error" |
| Stream lost | Warning banner with a retry; the last known map stays visible |
| Snapshot failed, stream alive | Notice; nodes still appear from events |

## Files

| File | Responsibility |
|---|---|
| `useLiveAgents.js` | Snapshot, stream, bounded buffers, selection, retry |
| `LivePage.jsx` | Header strip, banner, map/panel layout |
| `AgentSituationMap.jsx` | SVG map and the event-driven dot animation |
| `AgentPanel.jsx` | Selected agent: status, pack, activity, counters, log |
| `layout.js` | Deterministic node/target placement and edge geometry |
| `metrics.js` | Counter arithmetic, shared by page and panel |
| `format.js` | Time, count, cost, and target shortening |
| `kinds.js` | The five event kinds and their labels |
| `live.css` | Page-specific layout and kind colours |

## Rules

- No `fetch` and no `EventSource` in a component; both live in `api/live.js`.
- Only real events animate. There is no idle pulse and no fake replay.
- A new kind of event gets a colour in `live.css` and a label in `kinds.js`;
  unknown kinds stay grey and remain readable instead of disappearing.
| `RequestStream.jsx` | Which models are working right now |
| `AllChats.jsx` | Every active session, side by side, live |

Restyle (F25): `LivePage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
