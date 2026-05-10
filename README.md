# Callifornia2

Real-time calling stack: **Rust** services for routing, sessions, and HTTP/WebSocket APIs, plus a **C++ SFU** (Selective Forwarding Unit) built on **gRPC** and **libdatachannel** for WebRTC media forwarding.

---

## What each component does

| Component | Role |
|-----------|------|
| **connector** (`services/connector`) | Public edge: REST API for clients, JWT/session handoff, Redis-backed coordination. Routes users to a **signaling** instance and checks supervisor liveness before `/create`. |
| **signaling** (`services/signaling`) | Core call signaling: WebRTC SDP/ICE exchange with clients, talks to **room_manager** (gRPC) for rooms and SFU allocation, and to **SFU** (gRPC, `common/proto/signaling.proto`) for peer lifecycle and media plane control. |
| **room_manager** (`services/room_manager`) | gRPC control plane: room state, SFU pool / provisioning hints (`ROOM_MANAGER_SFU_CANDIDATES`). Uses **Redis** as shared storage. |
| **supervisor** (`services/supervisor`) | Central health watchdog and janitor for Redis/signaling/room_manager/SFU reconciliation. |
| **control_store** (`common/control_store`) | Shared Rust library with Redis keys, models, and storage helpers. |
| **sfu** (`sfu/`) | C++ executable: **libdatachannel** WebRTC stack, exposes **SFUService** over gRPC (same protos as Rust). |
| **Redis** | Required external dependency: shared state and leases across services. |

Protobuf definitions live under [`common/proto/`](common/proto/). The SFU CMake build generates C++ stubs from `common/proto/signaling.proto`; Rust crates use `tonic-prost-build` with a vendored `protoc` (no system `protoc` required for Rust).

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
cd common/control_store      && cargo build --release && cd ../..
cd services/connector        && cargo build --release && cd ../..
cd services/signaling        && cargo build --release && cd ../..
cd services/room_manager     && cargo build --release && cd ../..
cd services/supervisor       && cargo build --release && cd ../..
```

`protoc` is supplied by **`protoc-bin-vendored`** — no extra install step for codegen.

`control_store` is a pure-library crate (no binary) and must be built before the crates that depend on it (`connector`, `supervisor`).  `signaling` and `room_manager` do not depend on it at compile time.

---

## Configuration and local run order

1. Start **Redis** on the URL used in the `.env` files (default `127.0.0.1:6379`).
2. Copy or symlink env files: each service reads **`.env`** from its own crate directory.
3. Set **`ROOM_MANAGER_SFU_CANDIDATES`** in `room_manager` so it lists your SFU gRPC endpoint(s), e.g. `local|http://127.0.0.1:50051|200` (format: `id|http(s)://host:port|max_rooms`).
4. Start **room_manager**, then the **sfu** binary, then **signaling** instances.
5. Start **supervisor** — it must be running before new rooms can be created.  Set `SIGNALING_ADMIN_INSTANCES` to a comma-separated list of `node_id|http://host:admin_port` pairs (one per signaling instance), and `ROOM_MANAGER_GRPC_ADDR` to the room_manager address.
6. Start **connector** last (it reads `SIGNALING_INSTANCES` for routing and checks supervisor liveness via Redis).

Align **`CONNECTOR_TOKEN_SECRET`** between connector and signaling so issued tokens validate.

### New supervisor env vars

| Variable | Default | Purpose |
|----------|---------|---------|
| `SIGNALING_ADMIN_INSTANCES` | _(empty)_ | Comma-separated `node_id\|http://host:port` per signaling node |
| `SUPERVISOR_PROBE_INTERVAL_MS` | `2000` | How often probes tick |
| `SUPERVISOR_PROBE_TIMEOUT_MS` | `1000` | Timeout for a single probe RPC |
| `JANITOR_INTERVAL_SEC` | `5` | Janitor reconciliation interval |
| `SIGNALING_STALE_SEC` | `30` | Seconds after last heartbeat before a node is reclaimed |
| `SUPERVISOR_STALE_SEC` | `30` | Max age of `supervisor:heartbeat` accepted by connector |
| `SUPERVISOR_INSTANCE_ID` | _(random UUID)_ | Written into `supervisor:heartbeat` |

### New signaling env var

| Variable | Default | Purpose |
|----------|---------|---------|
| `SIGNALING_ADMIN_GRPC_ADDR` | `0.0.0.0:50071` | Bind address for the internal admin gRPC server |

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
common/
  proto/          # Shared .proto files (incl. signaling_admin.proto)
  control_store/  # Shared Rust library: Redis keys, models, storage helpers
services/
  connector/      # Axum REST edge -> signaling handoff
  signaling/      # WebSocket signaling + gRPC to room_manager & SFU + admin gRPC
  room_manager/   # tonic gRPC server + Redis
  supervisor/     # Health watchdog + janitor (replaces per-service loops)
client/           # Rust client app
sfu/              # C++ SFU (CMake: gRPC + libdatachannel submodules)
```
