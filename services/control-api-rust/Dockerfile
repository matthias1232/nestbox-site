# Build the API, then ship only the binary.
#
# The migrations are copied in because `sqlx::migrate!` embeds them at compile
# time: the running container needs no migrations directory and no database
# access at build.
#
# The build context is `services/`, not this folder, because the binary embeds
# `tool-images/supervisor/supervisor.py` with `include_str!`. A context of this
# folder alone would compile locally and fail only in the image, which is the
# expensive place to find out.

FROM rust:1.90-bookworm AS build
# The crate sits one level down so `include_str!` paths inside it resolve the
# same way they do in the repository (`control-api-rust/src/services/../../../tool-images`).
WORKDIR /app/control-api-rust

# Dependencies first, so a source-only change does not re-download the world.
COPY control-api-rust/Cargo.toml control-api-rust/Cargo.lock ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY control-api-rust/migrations ./migrations
COPY control-api-rust/src ./src
# Same relative path the compiler resolves from `src/services/`.
COPY tool-images/supervisor/supervisor.py /app/tool-images/supervisor/supervisor.py
COPY tool-images/desktop/install.sh /app/tool-images/desktop/install.sh
COPY tool-images/desktop/nestbox-desktop /app/tool-images/desktop/nestbox-desktop
COPY tool-images/desktop/nestbox-display /app/tool-images/desktop/nestbox-display
COPY tool-images/desktop/idfix.c /app/tool-images/desktop/idfix.c
# Touch main.rs so cargo rebuilds the crate after the dependency cache pass.
RUN touch src/main.rs && cargo build --release --locked

FROM debian:bookworm-slim

# ca-certificates for outbound TLS to gateways and providers; nothing else.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home nestbox

COPY --from=build /app/control-api-rust/target/release/nestbox-control-api /usr/local/bin/nestbox-control-api

# Unprivileged. The API writes nothing to disk; all state is in PostgreSQL.
USER 10001:10001
EXPOSE 8000
CMD ["nestbox-control-api"]
