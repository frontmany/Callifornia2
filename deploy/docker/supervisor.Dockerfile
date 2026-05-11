# syntax=docker/dockerfile:1
FROM rust:bookworm AS builder
WORKDIR /src
COPY common/control_store ./common/control_store
COPY common/proto ./common/proto
COPY services/supervisor ./services/supervisor
WORKDIR /src/services/supervisor
RUN cargo build --locked --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/services/supervisor/target/release/supervisor /usr/local/bin/supervisor
USER nobody
ENTRYPOINT ["/usr/local/bin/supervisor"]
