#!/bin/sh
# Install the desktop pack: a VNC server, a light desktop, Google Chrome, and the
# Chrome DevTools MCP server that lets an agent drive that Chrome.
#
# Run once at image build time, as root. Ubuntu/Debian only (apt): the pack's
# `base_os` says so, and the builder refuses it on another baseline.
#
# XFCE rather than GNOME: GNOME needs systemd and a session bus the container
# does not have, and costs about twice the memory for the same use.
set -eu
export DEBIAN_FRONTEND=noninteractive

# tmux carries the shared terminals: every console tab attaches to its own
# session, and a tab outlives the browser that opened it.
apt-get update
apt-get install -y --no-install-recommends \
  ca-certificates curl gnupg python3 \
  tigervnc-standalone-server tigervnc-common \
  xfce4 xfce4-terminal dbus-x11 x11-utils xauth xdg-utils \
  xdotool wmctrl scrot \
  libnss3-tools tmux \
  fonts-dejavu-core fonts-liberation fonts-noto-color-emoji xz-utils

# Google Chrome comes from Google's own repository: Ubuntu's chromium is a snap,
# and a snap does not run in a container.
curl -fsSL --proto '=https' --tlsv1.2 https://dl.google.com/linux/linux_signing_key.pub \
  | gpg --dearmor -o /usr/share/keyrings/google-chrome.gpg
echo "deb [arch=amd64 signed-by=/usr/share/keyrings/google-chrome.gpg] https://dl.google.com/linux/chrome/deb/ stable main" \
  > /etc/apt/sources.list.d/google-chrome.list
apt-get update
apt-get install -y --no-install-recommends google-chrome-stable

# Node 22 LTS from nodejs.org: chrome-devtools-mcp refuses Node 18, which is what
# Ubuntu 24.04 ships. The archive is checked against the checksum list served
# next to it, so a truncated or corrupted download fails the build.
case "$(uname -m)" in
  x86_64) NODE_ARCH=x64 ;;
  aarch64) NODE_ARCH=arm64 ;;
  *) echo "unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
NODE_BASE=https://nodejs.org/dist/latest-v22.x
curl -fsSL --proto '=https' --tlsv1.2 -o /tmp/SHASUMS256.txt "$NODE_BASE/SHASUMS256.txt"
NODE_FILE=$(grep "linux-$NODE_ARCH.tar.xz" /tmp/SHASUMS256.txt | awk '{print $2}')
curl -fsSL --proto '=https' --tlsv1.2 -o "/tmp/$NODE_FILE" "$NODE_BASE/$NODE_FILE"
(cd /tmp && grep " $NODE_FILE\$" SHASUMS256.txt | sha256sum -c -)
tar -xJf "/tmp/$NODE_FILE" -C /usr/local --strip-components=1 --exclude='*.md' --exclude=LICENSE
rm -f "/tmp/$NODE_FILE" /tmp/SHASUMS256.txt

# The MCP server agents use to open pages, click, type and take screenshots. It
# attaches to the Chrome that `nestbox-desktop` starts (remote debugging on
# 127.0.0.1:9222), so what the agent does is visible on the desktop.
npm install --global chrome-devtools-mcp@latest
npm cache clean --force

rm -rf /var/lib/apt/lists/*
