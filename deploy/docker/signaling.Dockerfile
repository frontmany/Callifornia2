# syntax=docker/dockerfile:1
FROM rust:bookworm AS builder
WORKDIR /src
COPY common/proto ./common/proto
COPY services/signaling ./services/signaling
WORKDIR /src/services/signaling
RUN cargo build --locked --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/services/signaling/target/release/signaling /usr/local/bin/signaling
EXPOSE 8080 50071
USER nobody
ENTRYPOINT ["/usr/local/bin/signaling"]
