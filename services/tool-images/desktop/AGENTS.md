# desktop — the sandbox's own screen

## Purpose

The `desktop` agent pack: a VNC server, XFCE, Google Chrome, and the tools an
agent needs to see and operate that screen. A sandbox with this pack is the one
a person opens in the console's desktop tab and an agent drives at the same
time; both look at the same framebuffer.

## Files

| File | Responsibility |
|---|---|
| `install.sh` | apt packages, Chrome, Node 22, `chrome-devtools-mcp` |
| `nestbox-desktop` | starts Xtigervnc, re-applies the saved monitor layout, then XFCE and Chrome |
| `nestbox-display` | monitor layouts: `get`, `set`, `add`, `remove`, `resize`, `move`, `mode`, `span`, `place` |
| `test_nestbox_display.py` | the layout arithmetic and validation, against captured `xrandr` output |
| `nestbox-desktop-mcp` | the `desktop-control` MCP server: 19 tools over stdio |
| `test_nestbox_desktop_mcp.py` | the coordinate rule, the parsers, and a JSON-RPC round trip |
| `idfix.c` | the preload shim a gateway sandbox needs before the X server starts |

## The two MCP servers

They answer different questions, and an agent should pick by the question:

| Server | Scope | Use it for |
|---|---|---|
| `chrome-browser` (`chrome-devtools-mcp`) | one Chrome | pages, the DOM, console logs, clicking an element by selector |
| `desktop-control` (`nestbox-desktop-mcp`) | the whole screen | screenshots of any monitor, clicks and typing in any window, moving windows, changing the monitor layout |

`desktop-control` is registered by the control plane (migration
`0089_desktop_control_mcp.sql`, stdio, `packs = {desktop}`) and installed to
`/usr/local/bin/nestbox-desktop-mcp` by the generated Dockerfile. It speaks
line-delimited JSON-RPC 2.0 on stdin/stdout, protocol `2025-03-26`, and
implements `initialize`, `notifications/initialized`, `ping`, `tools/list` and
`tools/call`.

## The coordinate rule

A monitor is addressed by `monitor`:

- `m1` … `m8` — the id in the layout;
- `1` … `8` — the 1-based position in the layout;
- `Monitor 2` — the name, case-insensitively;
- `all` — the whole framebuffer (the default for `desktop_screenshot` and for
  `desktop_set_layout`'s counterpart, `desktop_layout`).

`x` and `y` are pixels **from that monitor's top-left corner**. The answer always
echoes `absolute`, the framebuffer point, so an agent can chain a screenshot of
`m2` with a click on `m2` without doing arithmetic on offsets. A point outside
the monitor is clamped to its last pixel — a click meant for an edge must never
land on the neighbour. `desktop_windows` reports the monitor each window starts
on (or `null` for the black corner of an L shape).

## Rules

1. **No shell.** Every command is an argument vector. A window class, a title or
   a typed string comes from a model, so a shell string would be a second command.
2. **The layout belongs to `nestbox-display`.** This server never calls `xrandr`
   itself; a change goes through `nestbox-display`, which validates it, applies
   it and writes the state file the console and the control API read.
3. **Only allowlisted programs start.** `desktop_launch` knows `chrome`,
   `terminal`, `files` and `text-editor`. Nothing else, so a model cannot start a
   shell of its own choosing.
4. **Attribution is honest.** `NESTBOX_DISPLAY_ACTOR` defaults to `agent`, so the
   console can say who moved a monitor. The control API passes the operator's
   name for a change made in the UI.
5. **Failures are tool results, not crashes.** A missing binary, a refused
   layout or a timeout becomes `isError: true` with one sentence; the server
   keeps serving.
6. **Standard library only.** No install step beyond copying the file, so a
   rebuild cannot depend on a package index.

## Limits

Same as U09's: 1–8 monitors, each side 320–7680 px, positions ≥ 0, no overlap, a
monitor must touch another at an edge, bounding box ≤ 16384 × 8640 and total area
≤ 36 000 000 px. Screenshot `scale` is 0.25–1 (default 0.5); scrot's crop is the
monitor rectangle in the framebuffer.

## Live proof

The commands run inside the running sandbox, not on the host:

```bash
BOX 'docker exec -i nestbox-sandbox-desktop-01 nestbox-desktop-mcp' < requests.jsonl
BOX 'docker exec nestbox-sandbox-desktop-01 nestbox-display get'
```
