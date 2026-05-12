# Deploy Overview

This folder contains the single source of truth for local/prod-like deployment.

## Services and responsibilities

- `proxy` (nginx): single public entrypoint on port `80`.
  - serves frontend static files from `client/dist`
  - proxies API requests (`/auth`, `/create`, `/join`, `/logout`, `/session/renew`, `/health`) to `connector`
  - proxies WebSocket `/ws` to `signaling`
- `connector` (port `8090` internal): HTTP API for auth/create/join/session lifecycle.
- `signaling` (port `8080` internal): WebSocket signaling and admin gRPC `50071` internal.
- `sfu` (port `50051` internal): media SFU gRPC service.
- `supervisor`: health/probing/reconciliation over Redis.
- `redis`: shared state store.

## Environment files: which one matters

### Main deployment env (use this)

- `deploy/.env` (create from `deploy/.env.example`)
- this file is consumed by:
  - `docker compose --env-file deploy/.env ...`
  - all container env interpolation in `deploy/compose.yaml`

### Per-service `.env` files

Services call `dotenv::dotenv()` for bare local runs (without Docker), but in Compose flow
you should treat `deploy/.env` as the only required env file.

## Required variables in `deploy/.env`

- `PUBLIC_HOST`:
  - host/IP visible to clients (e.g. `192.168.1.45` in LAN)
  - used to build signaling node id and `ws://...` route for clients
- `CONNECTOR_TOKEN_SECRET`:
  - shared secret used by connector/signaling token flow
- `SUPERVISOR_SFU_INSTANCES`:
  - SFU inventory for supervisor, default: `sfu-1|http://sfu:50051|200`
- `CONNECTOR_CORS_ALLOWED_ORIGINS`:
  - needed only when frontend is served from a different origin (e.g. Trunk on `:8081`)
  - not needed when using nginx as single origin

## Run modes

### A) Recommended: nginx serves frontend (single origin)

1. Build frontend static files:

```powershell
cd client
trunk build --release
```

2. Start backend + proxy:

```powershell
cd ..
docker compose -f deploy/compose.yaml --env-file deploy/.env up -d --build
```

3. Open app:

- `http://<PUBLIC_HOST>/`

### B) Dev mode: Trunk serves frontend

1. Start Trunk:

```powershell
cd client
$env:CONNECTOR_BASE_URL="http://127.0.0.1:8090"
trunk serve --port 8081 --address 0.0.0.0
```

2. Keep `CONNECTOR_CORS_ALLOWED_ORIGINS` including Trunk origins, for example:

```env
CONNECTOR_CORS_ALLOWED_ORIGINS=http://127.0.0.1:8081,http://localhost:8081
```

3. Open app:

- `http://127.0.0.1:8081/`

## Quick checks

- Proxy up: `http://127.0.0.1/`
- Connector health through proxy: `http://127.0.0.1/health`
- Connector direct: `http://127.0.0.1:8090/health`
- Signaling direct: `http://127.0.0.1:8080/health`
