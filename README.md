# Callifornia2

Real-time calling stack: **Rust** services for routing, sessions, and HTTP/WebSocket APIs, plus a **C++ SFU** (Selective Forwarding Unit) built on **gRPC** and **libdatachannel** for WebRTC media forwarding.

---

## What each component does

| Component | Role |
|-----------|------|
| **connector** | Public edge: WebSocket API for clients, JWT/session handoff, Redis-backed coordination. Routes users to a **signaling** instance. |
| **signaling** | Core call signaling: WebRTC SDP/ICE exchange with clients, talks to **room_manager** (gRPC) for rooms and SFU allocation, and to **SFU** (gRPC, `proto/signaling.proto`) for peer lifecycle and media plane control. |
| **room_manager** | gRPC control plane: room state, SFU pool / provisioning hints (`ROOM_MANAGER_SFU_CANDIDATES`), health checks against SFU instances. Uses **Redis** as shared storage. |
| **sfu** (`sfu/`) | C++ executable: **libdatachannel** WebRTC stack, exposes **SFUService** over gRPC (same protos as Rust). |
| **Redis** | Required external dependency: shared state and leases across connector, signaling, and room_manager. |

Protobuf definitions live under [`proto/`](proto/). The SFU CMake build generates C++ stubs from `proto/signaling.proto`; Rust crates use `tonic-prost-build` with a vendored `protoc` (no system `protoc` required for Rust).

---

## Why `git clone` is not enough (submodules)

This repo vendors heavy native dependencies as Git submodules:

- `sfu/vendor/grpc` — gRPC-C++ (pulls **many** nested submodules: Abseil, protobuf, etc.).
- `sfu/vendor/libdatachannel` — WebRTC data channel / peer connection stack (nested deps: libjuice, libsrtp, usrsctp, …).

**You must initialize submodules recursively.** A shallow clone or missing `--recursive` will break CMake with errors like “gRPC not found” or missing `deps/libjuice` under libdatachannel.

```bash
git clone --recursive <your-repo-url>
cd <cloned-directory>
```

If you already cloned without submodules:

```bash
git submodule update --init --recursive
```

The first recursive fetch can take a long time and use significant disk space—this is expected for gRPC-from-source builds.

---

## Prerequisites

**All platforms**

- **CMake** 3.21 or newer (4.x is supported; the SFU sets policies for bundled deps).
- **C++20** compiler: MSVC 2019+ (Windows), GCC 10+ or Clang 12+ (Linux/macOS).
- **Rust** (stable): for `connector`, `signaling`, `room_manager`.
- **Redis** server reachable by all Rust services (default in `.env` files: `redis://127.0.0.1:6379/`).

**SFU-only extras**

- **OpenSSL** development package or install tree visible to CMake:
  - **Linux**: e.g. `libssl-dev` / `openssl-devel` (distribution-specific).
  - **macOS**: e.g. OpenSSL from Homebrew; you may need `OPENSSL_ROOT_DIR` if CMake does not find it automatically.
  - **Windows**: libdatachannel’s dependency chain expects OpenSSL headers and import libs. Either install the **Win64 OpenSSL full** package (not “Light”) and point CMake at it, or extract/build OpenSSL under a path CMake recognizes—see comments in [`sfu/CMakeLists.txt`](sfu/CMakeLists.txt) (`OPENSSL_ROOT_DIR`, or local trees under `sfu/vendor/openssl` / `sfu/vendor/OpenSSL-Win64`).

Optional but recommended: **Ninja** build generator for faster CMake builds.

---

## Building the SFU (C++)

From the repository root:

```bash
cd sfu
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build -j
```

On Windows (example with Visual Studio generator):

```powershell
cd sfu
cmake -S . -B build -G "Visual Studio 17 2022" -A x64
cmake --build build --config Release
```

The output executable is `sfu` (or `sfu.exe` on Windows) inside `sfu/build/` (exact path depends on generator).

Environment at runtime: **`SFU_GRPC_ADDR`** — bind address for the SFU gRPC server (default in code: `0.0.0.0:50051`). This address must match what **room_manager** advertises to signaling via **`ROOM_MANAGER_SFU_CANDIDATES`** (see [`room_manager/.env`](room_manager/.env)).

---

## Building the Rust services

There is **no** workspace `Cargo.toml` at the repo root; build each crate separately from its directory:

```bash
cd connector && cargo build --release && cd ..
cd signaling && cargo build --release && cd ..
cd room_manager && cargo build --release && cd ..
```

`protoc` is supplied by **`protoc-bin-vendored`** in `signaling` and `room_manager` builds—no extra install step for codegen.

---

## Configuration and local run order

1. Start **Redis** on the URL used in the `.env` files (default `127.0.0.1:6379`).
2. Copy or symlink env files: each service reads **`.env`** from its own crate directory ([`connector/.env`](connector/.env), [`signaling/.env`](signaling/.env), [`room_manager/.env`](room_manager/.env)).
3. Set **`ROOM_MANAGER_SFU_CANDIDATES`** in `room_manager` so it lists your SFU gRPC endpoint(s), e.g. `local|http://127.0.0.1:50051|200` (format: `id|http(s)://host:port|max_rooms`; see comments in `room_manager/.env`).
4. Start **room_manager**, then the **sfu** binary, then **signaling**, then **connector** (connector must list signaling WebSocket URLs in **`SIGNALING_INSTANCES`** — see `connector/.env`).

Align **`CONNECTOR_TOKEN_SECRET`** between connector and signaling so issued tokens validate.

---

## Quick sanity checklist

- [ ] `git submodule update --init --recursive` completed without errors.
- [ ] `sfu/vendor/grpc/CMakeLists.txt` and `sfu/vendor/libdatachannel/deps/libjuice` exist.
- [ ] SFU CMake configures and links (OpenSSL resolved on Windows).
- [ ] Redis is running; `.env` URLs match.
- [ ] `ROOM_MANAGER_SFU_CANDIDATES` points at the same host/port as **`SFU_GRPC_ADDR`** for the SFU process.

---

## Repository layout (high level)

```
connector/       # Axum WebSocket edge → signaling handoff
signaling/       # WebSocket signaling + gRPC to room_manager & SFU
room_manager/    # tonic gRPC server + Redis
sfu/             # C++ SFU (CMake: gRPC + libdatachannel submodules)
proto/           # Shared .proto files
```
