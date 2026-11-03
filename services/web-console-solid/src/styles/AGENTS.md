# styles/ — tokens and layout

## Purpose

Design tokens first, layout second, everything else last. A component that needs
a new colour asks for a token; it does not invent a hex value.

## Files

| File | Responsibility |
|---|---|
| `tokens.css` | Colour, spacing, radius, and type scale |
| `app.css` | Reset, shell layout, tables, forms, drawer, graph |
| `nodes.css` | Node budget bars and the node drawer; imported by `features/nodes/NodesPage.jsx` |
| `providers-status.css` | Provider status, install origin, and the in-use refusal |
| `sandbox-resources.css` | The create form's node budget and the Agents tab |
| `recipes-persist.css` | Recipe table, editor, and detail; persistent roots and their mounts |
| `builder.css` | The builder's sections: baseline, tools, agents, resources, node, mounts |
| `controls.css` | Workbox primitives: switch, segmented, countdown, dialog, banner, presence dots; imported by those components |
| `workbox-sessions.css` | The shared terminal tab strip with its per-tab menu and SSH line, the desktop panel, and the two pop-out layouts; imported by `TerminalTab`, `DesktopTab`, `SessionsPanel` and the pop-outs |
| `workbox.css` | The Workbox header, its card and panel grids, and the "my sandboxes" tiles |

A screen-specific file is the exception, not the pattern: it exists where a
feature says something app.css has no word for. Import it from the module that
uses it, not from `main.jsx`.

## Rules

- A colour is defined once. Two shades of "error red" is how a design drifts.
- Density is a token, not a per-table decision: the console is for scanning.
- The graph and the table must survive a narrow viewport: horizontal scroll
  rather than a squeezed layout.
| `login.css` | The login page's two-column layout, form and illustration frame; imported by `LoginPage` |
| `dlp.css` | The content-check colour language, the shared table and the chat bubbles |

## Look (Control Center)

Every stylesheet except `tokens.css` reads colours only through tokens: square corners
(`--radius`), 1 px olive lines, lime accent, tinted state backgrounds written as
`color-mix(in srgb, var(--bad) 14%, transparent)`. No hex or rgb colour outside `tokens.css`.
`app.css` also defines `.eyebrow`, `.figure` and `.num`.
- `dock.css` — dock tabs, alert cards and the alert stack (F13); imported by `features/dock/AlertStack.jsx`.
- `traffic.css` — Live traffic page: filter bar, red lane, side panel, sticky table header (F17); imported by `features/traffic/TrafficPage.jsx`.
