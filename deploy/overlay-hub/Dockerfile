# The WireGuard hub every joined node dials (ADR 0047). One small container on the host network:
# it owns the `nb-hub` interface, applies the peer list the control plane writes, keeps nodes of
# different data centers from reaching each other, and forwards the registry to the overlay.
#
# Debian rather than Alpine on purpose: the host's Docker uses the nftables iptables front end,
# and a hub that writes rules through the legacy one would put them where nothing reads them.
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends wireguard-tools iproute2 iptables socat procps \
 && rm -rf /var/lib/apt/lists/*
COPY entrypoint.sh /entrypoint.sh
RUN chmod 0755 /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]
