FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl git nano jq less procps iputils-ping dnsutils ripgrep vim && rm -rf /var/lib/apt/lists/*
WORKDIR /workspace
CMD ["sleep", "infinity"]

## desktop/

`install.sh`, `nestbox-desktop`, `nestbox-display` and `nestbox-desktop-mcp` are
the scripts of the `desktop` agent pack (install kind `desktop`). They are
embedded in the generated Dockerfile and hashed into the image tag, so editing
one produces a new image. The desktop is Xtigervnc + XFCE + Google Chrome (remote
debugging on 127.0.0.1:9222); nothing listens outside the container.

Two MCP servers come with that desktop, and they answer different questions:

- **`chrome-browser`** (`chrome-devtools-mcp`, installed by `install.sh`) works
  on pages: navigate, click a DOM element, read the console. It knows nothing
  outside Chrome.
- **`desktop-control`** (`nestbox-desktop-mcp`, this folder) works on the screen:
  a screenshot of any monitor, a click or a typed string anywhere, windows, and
  the monitor layout itself. Its registry row is `desktop-control` (migration
  `0089_desktop_control_mcp.sql`), stdio, `packs = {desktop}`.

Monitors are named `m1`…`m8`, a 1-based index, a name, or `all`. A coordinate in
a `desktop-control` call is **relative to the monitor the call names**, and the
answer echoes the absolute framebuffer point, so a click matches what a
screenshot of that monitor showed. A point past a monitor's edge is clamped to
it, never moved onto a neighbour. Layout limits (1–8 monitors, 320–7680 px a
side, bounding box ≤ 16384×8640, ≤ 36 000 000 px) are the ones
`nestbox-display` enforces; this server only passes requests through, so the
console, the API and an agent all get the same sentence for the same mistake.
`desktop_launch` starts only `chrome`, `terminal`, `files` and `text-editor`.

`nestbox-display` sets the monitor layout (1–8 monitors, any size, separate or
one stretched screen) through RandR 1.5 and remembers it in
`$HOME/.config/nestbox-display.json`, which `nestbox-desktop` re-applies before
XFCE starts.


## claude-code/

`Dockerfile` builds the image whose agents sign in themselves: Claude Code plus the
OpenAI Codex CLI, each pinned by an `ARG`, so one sandbox can switch between them.
