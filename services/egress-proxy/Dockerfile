# Nestbox egress proxy (Envoy + the certificate broker it cannot be).
#
# Two stages. The first builds `egress-mitm`, the ~200-line TLS terminator that
# mints a leaf certificate per requested host from this node's CA — the one
# thing Envoy cannot do (see `mitm/main.go`). Its tests run during the build, so
# an image whose certificate minting is broken does not exist.
#
# The envoy stage is pinned by digest. The readable tag stays next to it so a
# reviewer can tell what is pinned. Re-pinning is deliberate: pull, read
# `docker image inspect --format '{{index .RepoDigests 0}}'`, update this file,
# and note it in the changelog.
FROM golang:1.23-bookworm@sha256:167053a2bb901972bf2c1611f8f52c44d5fe7e762e5cab213708d82c421614db AS mitm

WORKDIR /src
# Standard library only: no module downloads, so the build needs no proxy and
# cannot pull a dependency that was not reviewed.
ENV CGO_ENABLED=0 \
    GOFLAGS=-mod=mod \
    GOPROXY=off

COPY mitm/go.mod mitm/main.go mitm/main_test.go mitm/upstream.go mitm/upstream_test.go \
    mitm/dlp.go mitm/dlp_test.go mitm/restore.go mitm/restore_test.go \
    mitm/livetap.go mitm/meta.go ./
RUN go vet ./... \
 && go test ./... \
 && go build -trimpath -ldflags='-s -w' -o /out/egress-mitm .

# The roots Envoy checks real servers against. The envoy image carries a bundle
# from its build date; a server that moved to a newer intermediate (Let's
# Encrypt's 2025 "YE" chain, for one) then fails with CERTIFICATE_VERIFY_FAILED
# and the sandbox sees a bare 503. The current Mozilla list is fetched here and
# checked against the checksum published beside it.
FROM golang:1.23-bookworm@sha256:167053a2bb901972bf2c1611f8f52c44d5fe7e762e5cab213708d82c421614db AS roots
RUN cd /tmp \
 && curl -fsSL --proto '=https' --tlsv1.2 -o cacert.pem https://curl.se/ca/cacert.pem \
 && curl -fsSL --proto '=https' --tlsv1.2 -o cacert.pem.sha256 https://curl.se/ca/cacert.pem.sha256 \
 && sha256sum -c cacert.pem.sha256

FROM envoyproxy/envoy:v1.31-latest@sha256:caa5b411be1633b90023592a34a7e010c933d6e60206c758f631485e53006865

# The config and the MCP extension are part of the image, not a bind mount: a
# proxy whose rules come from the host filesystem is a proxy that can be
# changed without a deploy.
COPY envoy.yaml /etc/envoy/envoy.yaml
COPY extensions/ /etc/envoy/extensions/

COPY --from=mitm /out/egress-mitm /usr/local/bin/egress-mitm
COPY --from=roots /tmp/cacert.pem /tmp/mozilla-ca.pem
# Both lists, not one: BoringSSL follows the chain a server sends. api.anthropic.com
# sends GTS Root R4 cross-signed by GlobalSign, which only the older list still
# has; a server on a 2025 Let's Encrypt chain needs the newer one. OpenSSL finds
# either path, Envoy does not, so the trust file is the union.
RUN cat /etc/ssl/certs/ca-certificates.crt /tmp/mozilla-ca.pem > /etc/envoy/mozilla-ca.pem \
 && rm /tmp/mozilla-ca.pem
COPY start-egress-proxy.sh /usr/local/bin/start-egress-proxy.sh
RUN chmod 0755 /usr/local/bin/start-egress-proxy.sh /usr/local/bin/egress-mitm

# The loopback hop (`egress_terminated`) needs a certificate at startup, and
# that certificate is not a secret: only 127.0.0.1 inside this container can
# reach it, and the broker does not verify it. Generating it here keeps a
# private key out of the repository and out of the runtime secret set, while
# still letting `envoy --mode validate` load the configuration in a fresh
# container.
RUN mkdir -p /etc/envoy/tls \
 && openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
      -keyout /etc/envoy/tls/loopback.key \
      -out /etc/envoy/tls/loopback.crt \
      -days 3650 -subj "/CN=nestbox-egress-loopback" \
 && chown -R envoy:envoy /etc/envoy/tls \
 && chmod 0600 /etc/envoy/tls/loopback.key

USER envoy
EXPOSE 3128
# The script starts the broker (which reads the CA from Docker secrets) and
# execs Envoy, so Envoy stays PID 1 and `docker compose run --entrypoint envoy`
# still validates the image without the broker.
ENTRYPOINT ["/usr/local/bin/start-egress-proxy.sh"]
