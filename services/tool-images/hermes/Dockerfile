# Hermes Agent on Ubuntu 24.04.
#
# The installer is pinned by the version it reports at build time rather than by
# a floating tag: the image tag `nestbox-hermes:local` is the pin the control
# plane records, and the build log holds the resolved version.
#
# The model default is baked in here so a freshly created sandbox answers a plain
# `hermes -z "..."` without an operator running `hermes setup` first. The
# endpoint and the credential are NOT baked in: both arrive as environment
# variables from the provider profile at container start, so a rotated key does
# not require an image rebuild (ADR 0005).

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl git python3 nodejs npm jq less procps htop ripgrep vim tmux \
        libatomic1 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash nestbox
USER nestbox
WORKDIR /home/nestbox

RUN curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash

# The installer puts the command in `~/.local/bin` and adds it to `~/.bashrc`,
# which a non-login shell never reads. Baked into the image PATH so that
# `docker exec <sandbox> hermes ...` works without a shell in between — which is
# exactly how the control plane and an operator call it.
ENV PATH=/home/nestbox/.local/bin:${PATH}

# Pin the inference provider and model to what the appliance serves. A bare
# `hermes -z` otherwise falls back to the vendor's default model, which the local
# key is not allowed to use, and the failure looks like a broken agent rather
# than a wrong default.
RUN /home/nestbox/.local/bin/hermes config set model.default opencode-go-combo \
    && /home/nestbox/.local/bin/hermes config set model.provider openai \
    && /home/nestbox/.local/bin/hermes config check || true

# Hermes' MCP support lives in the `mcp` extra of its own pyproject
# (`mcp==2.3.0`), and the standalone installer does not pull it in. Without the
# package every `from mcp.client import streamable_http` fails at connect time,
# so a remote HTTP server is registered and then silently unusable. Installed
# into the interpreter Hermes itself runs on, at the version its pyproject pins.
RUN uv_bin="$(ls -d /home/nestbox/.hermes/tools/python-*/bin/python3 | head -1)" \
    && "$uv_bin" -m pip install --no-cache-dir --disable-pip-version-check "mcp==2.3.0" \
    && "$uv_bin" -c "from mcp.client import streamable_http; print('mcp http transport available')"

# The sandbox's own MCP hub, over HTTP on the sandbox network.
#
# The URL and the token are `${...}` references, resolved by Hermes at load time
# from the container's environment. That keeps the credential out of the image
# (ADR 0005) and out of a config file on disk, and it fails closed: a server
# whose variable is unset is skipped with a warning rather than called without a
# token. `NESTBOX_MCP_URL` has a default so only the token has to be provided.
#
# `nestbox-hub` is an ordinary MCP server to Hermes; the difference is that it
# lives on the sandbox network and is therefore reached through the egress proxy,
# which is what makes its calls visible (ADR 0026). The stdio servers a pack adds
# later run as child processes and stay invisible — that is documented, not
# hidden.
ENV NESTBOX_MCP_URL=http://mcp-hub:8080/mcp

RUN printf '%s\n' \
    '' \
    '# --- Nestbox: MCP servers -------------------------------------------------' \
    '# Added by the image. The token is a ${VAR} reference, never a literal.' \
    'mcp_servers:' \
    '  nestbox-hub:' \
    '    url: "${NESTBOX_MCP_URL}"' \
    '    headers:' \
    '      Authorization: "Bearer ${NESTBOX_MCP_TOKEN}"' \
    '    connect_timeout: 10' \
    '    timeout: 60' \
    >> /home/nestbox/.hermes/config.yaml \
    # Validate with Hermes' own parser rather than a second YAML library: a
    # malformed block makes `config get` exit non-zero and fails the build.
    && hermes config get mcp_servers.nestbox-hub.url >/dev/null \
    && hermes mcp list >/dev/null \
    && echo 'nestbox-hub is present and parses in ~/.hermes/config.yaml'

# Runs as the sandbox user, so it can only touch the user's own trust bundle and
# profile. The egress proxy's CA is generated at deploy time (it cannot be baked
# into an image that must also work without one), so it is installed here when
# the file is present and skipped when it is not.
COPY --chown=nestbox:nestbox entrypoint.sh /home/nestbox/entrypoint.sh
RUN chmod 0755 /home/nestbox/entrypoint.sh

# A sandbox is a long-lived container, not a one-shot command.
ENTRYPOINT ["/home/nestbox/entrypoint.sh"]
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
