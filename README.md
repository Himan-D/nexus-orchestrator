# Nexus — Asset-Centric Data & AI Orchestrator

[![CI](https://github.com/Himan-D/nexus-orchestrator/actions/workflows/ci.yml/badge.svg)](https://github.com/Himan-D/nexus-orchestrator/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Nexus is an open orchestration engine for data and AI pipelines, built in the spirit of
[Dagster](https://dagster.io/): **pipelines are defined by the assets they produce**, runs are
the history of how each asset came to be, and everything is observable through a single API
and UI.

```
┌─────────────────────────────────────────────────────────────────┐
│  Next.js UI (dashboard · pipelines · run logs · asset catalog)  │
└──────────────────────────────┬──────────────────────────────────┘
                               │  /api/* → :3002 (same-origin cookie auth)
┌──────────────────────────────▼──────────────────────────────────┐
│                    Rust core (axum, /v1 REST API)               │
│  ┌───────────┐   ┌────────────┐   ┌──────────────────────────┐  │
│  │ Scheduler │──▶│ Run engine │──▶│ Executor (pluggable)     │  │
│  │ atomic    │   │ DAG waves  │   │ local shell → containers │  │
│  │ claim     │   │ retries    │   │                          │  │
│  └─────┬─────┘   └─────┬──────┘   └──────────────────────────┘  │
│        └───────┬───────┘                                        │
│        SQLite (jobs · runs · task_runs · assets · users)        │
└─────────────────────────────────────────────────────────────────┘
```

## Design (hexagonal architecture)

- **`domain/`** — pure business logic, zero infrastructure dependencies:
  `Job` + `Task` with DAG validation (unique ids, resolvable deps, cycle detection),
  `Run` / `TaskRun` with **enforced state machines**, `Asset` + `Materialization`.
- **`application/`** — use cases and ports (traits): `JobRepository`, `RunRepository`,
  `AssetRepository`, `UserRepository`, `TaskExecutor`. The **run engine** executes DAGs in
  topological waves (independent tasks run in parallel), applies per-task retries and
  timeouts, records logs, and materializes assets. The **scheduler** claims queued runs
  atomically (transactional `UPDATE … WHERE status='queued'`) so multiple schedulers can
  share a database without double-dispatch.
- **`infrastructure/`** — adapters: SQLite repositories (WAL, versioned migrations) and the
  local shell executor. Postgres and container executors are drop-in replacements.
- **`interfaces/`** — axum HTTP layer: versioned `/v1` API, JWT + httpOnly-cookie auth
  middleware, uniform error envelope, health/readiness probes.

Every error is mapped — no `unwrap()` in request paths — and corrupt rows surface as
errors instead of silently succeeding.

## Quickstart (local)

```bash
# 1. Backend
ORCH_DEV_MODE=1 cargo run            # dev mode: ephemeral JWT secret, warns loudly
# → Rust API on http://127.0.0.1:3002

# 2. Frontend (separate terminal)
cd frontend && npm install && npm run dev
# → UI on http://localhost:3000 (proxies /api/* to :3002)
```

Sign up in the UI, create a pipeline (a sample DAG is pre-filled), trigger it, and watch
task logs and asset freshness update live.

### Quickstart (Docker)

```bash
cp .env.example .env            # then: ORCH_JWT_SECRET=$(openssl rand -hex 32) > .env
docker compose up --build       # UI on :3000, API on :3002, data in a named volume
```

## Configuration

| Variable                | Default                          | Meaning                                        |
| ----------------------- | -------------------------------- | ---------------------------------------------- |
| `ORCH_JWT_SECRET`       | — (required unless dev mode)     | ≥32-char JWT signing secret                    |
| `ORCH_DB_URL`           | `sqlite://data/orchestrator.db?mode=rwc` | SQLite connection URL                  |
| `ORCH_BIND`             | `0.0.0.0:3002`                   | API bind address                               |
| `ORCH_DEV_MODE`         | off                              | Allows ephemeral JWT secret for local dev      |
| `ORCH_COOKIE_SECURE`    | `0`                              | Set `Secure` on session cookies (use behind TLS)|
| `ORCH_CORS_ORIGIN`      | —                                | Optional exact origin for credential CORS      |
| `ORCH_SCHED_TICK_SECS`  | `1`                              | Scheduler poll interval                        |
| `ORCH_SCHED_BATCH`      | `10`                             | Max runs claimed per tick                      |
| `ORCH_SHUTDOWN_GRACE_SECS` | `30`                          | Drain window on SIGINT/SIGTERM                 |

## API sketch

All routes under `/v1` require a session (Bearer JWT or the `nexus_session` cookie);
`/healthz`, `/readyz`, and the auth entry points are public.

```
POST /v1/auth/signup | signin | signout     GET /v1/auth/me
POST /v1/jobs            GET /v1/jobs            GET|PATCH|DELETE /v1/jobs/:id
POST /v1/jobs/:id/runs   GET  /v1/jobs/:id/runs
GET  /v1/runs[?status=]  GET /v1/runs/:id        POST /v1/runs/:id/cancel
GET  /v1/assets          GET /v1/assets/:key     GET /v1/stats
```

A pipeline task may declare `materializes: "warehouse.events"`; successful tasks are
recorded as materializations and update the asset's freshness pointer.

## Guarantees & behavior

- **Runs never hang**: startup recovery fails runs orphaned by a crash; per-task timeouts
  kill processes; cancellation (queued: atomic in DB, running: cooperative token) is
  first-class; SIGTERM drains in-flight runs.
- **DAGs are validated at submit time** — cycles, unknown dependencies, and duplicate task
  ids are rejected with actionable errors.
- **Failures cascade**: a failed task skips its dependents (marked `skipped`) and fails the
  run with the task's error and captured logs.
- **Deleting a pipeline never rewrites history** — runs keep a name snapshot.

## Tests

```bash
cargo test        # 32 tests: domain state machines, DAG validation, auth,
                  # repository round-trips, executor kill/timeout/cancel,
                  # end-to-end run execution + asset materialization
```

## Roadmap to Dagster parity

The engine foundation above is real and tested. What a production deployment at large-team
scale additionally needs, in rough order:

1. **Executor isolation** — container/nsjail-based `TaskExecutor` before accepting
   task definitions from untrusted users (the current shell executor is for trusted
   environments; the port makes the swap mechanical).
2. **Postgres adapter** — the ports are in place; add connection pooling + `SELECT … FOR
   UPDATE SKIP LOCKED` claiming for multi-node schedulers.
3. **Schedules & sensors** — cron schedules on jobs, file/API sensors that trigger runs.
4. **RBAC & teams** — per-project namespaces, roles, SSO (OIDC).
5. **Lineage & partitions** — asset dependency graphs derived from `materializes` +
   `depends_on`, partitioned runs/backfills.
6. **Observability** — OpenTelemetry export, Prometheus metrics, log streaming (WebSocket)
   instead of post-hoc logs.
7. **Queue decoupling** — external run queue (Redis/Postgres LISTEN/NOTIFY) to scale
   workers independently of the API.
