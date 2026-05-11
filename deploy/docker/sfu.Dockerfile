# syntax=docker/dockerfile:1
#
# Linux-only SFU image. Host `sfu/vendor` is .dockerignore'd; gRPC + libdatachannel are cloned here.
#
# First build is slow: cloning gRPC submodules can take 15–40+ minutes (network); compile is heavy.
# If the build "hangs" then dies: often OOM — lower BUILD_JOBS or give Docker more RAM (Desktop → Settings → Resources).
#
# Optional pins:
#   docker compose build sfu --build-arg GRPC_GIT_REF=v1.62.0 --build-arg LIBDATACHANNEL_GIT_REF=v0.21.2
#
# Build context: repository root.

FROM ubuntu:22.04 AS builder
ENV DEBIAN_FRONTEND=noninteractive
ENV GIT_TERMINAL_PROMPT=0

ARG GRPC_GIT_REF=
ARG LIBDATACHANNEL_GIT_REF=
# Cap compile parallelism (ninja -j). Default 4 avoids Docker Desktop OOM; raise on a big Linux builder.
ARG BUILD_JOBS=4

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        git ca-certificates cmake ninja-build build-essential pkg-config \
        libssl-dev zlib1g-dev perl python3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY common/proto common/proto
COPY sfu/CMakeLists.txt sfu/
COPY sfu/src sfu/src

WORKDIR /src/sfu

# --- Vendoring (split RUN = visible progress; long steps are normal, not deadlock) ---
RUN set -eux; mkdir -p vendor; rm -rf vendor/grpc vendor/libdatachannel; \
    echo "[sfu-docker] cloning gRPC …"; \
    if [ -n "${GRPC_GIT_REF}" ]; then \
        git clone --branch "${GRPC_GIT_REF}" --single-branch --depth 1 https://github.com/grpc/grpc.git vendor/grpc; \
    else \
        git clone --depth 1 https://github.com/grpc/grpc.git vendor/grpc; \
    fi

RUN set -eux; \
    echo "[sfu-docker] gRPC submodules (large download; be patient) …"; \
    cd vendor/grpc && git submodule update --init --recursive

RUN set -eux; \
    echo "[sfu-docker] cloning libdatachannel …"; \
    if [ -n "${LIBDATACHANNEL_GIT_REF}" ]; then \
        git clone --branch "${LIBDATACHANNEL_GIT_REF}" --single-branch --depth 1 https://github.com/paullouisageneau/libdatachannel.git vendor/libdatachannel; \
    else \
        git clone --depth 1 https://github.com/paullouisageneau/libdatachannel.git vendor/libdatachannel; \
    fi

RUN set -eux; \
    echo "[sfu-docker] libdatachannel deps …"; \
    cd vendor/libdatachannel && git submodule update --init --recursive

RUN set -eux; \
    echo "[sfu-docker] CMake configure …"; \
    rm -rf build \
    && cmake -S . -B build -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX=/usr/local

RUN set -eux; \
    echo "[sfu-docker] compiling with BUILD_JOBS=${BUILD_JOBS} …"; \
    cmake --build build --parallel "${BUILD_JOBS}"

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates netcat-openbsd \
        libssl3 libstdc++6 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/sfu/build/sfu /usr/local/bin/sfu
ENV SFU_GRPC_ADDR=0.0.0.0:50051
EXPOSE 50051
USER nobody
ENTRYPOINT ["/usr/local/bin/sfu"]
