FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl git nano jq less procps iputils-ping dnsutils ripgrep vim && rm -rf /var/lib/apt/lists/*
WORKDIR /workspace
CMD ["sleep", "infinity"]

## desktop/

`install.sh`, `nestbox-desktop` and `nestbox-display` are the scripts of the
`desktop` agent pack (install kind `desktop`). They are embedded in the generated
Dockerfile and hashed into the image tag, so editing one produces a new image.
The desktop is Xtigervnc + XFCE + Google Chrome (remote debugging on
127.0.0.1:9222) and the Chrome DevTools MCP server; nothing listens outside the
container. `nestbox-display` sets the monitor layout (1–8 monitors, any size,
separate or one stretched screen) through RandR 1.5 and remembers it in
`$HOME/.config/nestbox-display.json`, which `nestbox-desktop` re-applies before
XFCE starts.

## claude-code/

`Dockerfile` builds the image whose agents sign in themselves: Claude Code plus the
OpenAI Codex CLI, each pinned by an `ARG`, so one sandbox can switch between them.
