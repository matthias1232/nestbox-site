# Claude Code in a sandbox, for SSH: `ssh -t ... nestbox@node claude`.
#
# The version is pinned by an argument rather than `latest`: the tag
# `nestbox-claude-code:local` is what the control plane records, and the build log
# holds the resolved version. The credential is NOT baked in: ANTHROPIC_API_KEY
# arrives in the sandbox environment, or the operator signs in on first run.
#
# Runs as `nestbox`, the user the SSH door and the agents use, so a login lands in
# the home the agent works in.

FROM node:22-bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive
ARG CLAUDE_CODE_VERSION=2.1.312
ARG CODEX_VERSION=0.166.0

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl git jq less procps htop ripgrep vim tmux openssh-client python3 \
    && rm -rf /var/lib/apt/lists/*

# Graphify: the local knowledge-graph MCP server an agent can be given.
#
# Installed here rather than in the home at run time, so a sandbox starts with the
# command already on PATH and an offline sandbox still has it. A venv keeps it off
# the system Python (PEP 668), and the pin is deliberate: the tool reads a graph
# format, and a silent upgrade would change what the agent sees.
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3-venv \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv /opt/graphify \
    && /opt/graphify/bin/pip install --no-cache-dir graphifyy==0.7.5 \
    && ln -sf /opt/graphify/bin/graphify /usr/local/bin/graphify \
    && graphify --help >/dev/null

RUN useradd --create-home --shell /bin/bash nestbox
USER nestbox
WORKDIR /home/nestbox

# A user-level prefix: no root needed at run time, and `docker exec` and the SSH
# door find the command without a login shell reading ~/.bashrc.
ENV NPM_CONFIG_PREFIX=/home/nestbox/.local
ENV PATH=/home/nestbox/.local/bin:${PATH}

RUN npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
    && claude --version

# Codex CLI: its sign-in (ChatGPT or gateway) is switched by the Nestbox launcher.
#
# Installed into the same user prefix as Claude Code, so both agents are found on
# PATH by `docker exec` and the SSH door without a login shell. Pinned like every
# other tool here: a silent upgrade would change what the sandbox's agent sees.
RUN npm install -g "@openai/codex@${CODEX_VERSION}" \
    && codex --version

CMD ["sleep", "infinity"]

# nestbox-sandbox-user: the sandbox does not run as root.
#
# A gateway (OpenShell) sandbox refuses to start when its working directory
# belongs to another user than the sandbox identity, and an agent that signs in
# on a root account has the whole container to damage. The user is added here,
# after every install step, so the image layers above stay cached.
# The steps below need root to create the account, whatever the base image left set.
USER root
RUN id -u nestbox >/dev/null 2>&1 || useradd -m -d /home/nestbox -u 10001 -s /bin/bash nestbox \
 && install -d -o nestbox -g nestbox /workspace /home/nestbox \
 && chown -R nestbox /workspace /home/nestbox
USER nestbox
WORKDIR /workspace
