# syntax=docker/dockerfile:1
FROM rust:bookworm AS builder
WORKDIR /src
COPY common/control_store ./common/control_store
COPY services/connector ./services/connector
WORKDIR /src/services/connector
RUN cargo build --locked --release \
    && test -x target/release/connector

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/services/connector/target/release/connector /usr/local/bin/connector
EXPOSE 8090
USER nobody
ENTRYPOINT ["/usr/local/bin/connector"]
