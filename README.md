# WisLand-Search

A modular Rust search & agent service composed of multiple crates (workspace). This README gives you a clear mental model plus the exact steps to build, configure, run, and extend the system.

## Contents
- [Architecture & Crates](#architecture--crates)
- [Features](#features)
- [Tech Stack](#tech-stack)
- [Quick Start (5 min)](#quick-start-5-min)
- [Configuration](#configuration)
- [Environment Selection (`APP_PROFILE`)](#environment-selection-app_profile)
- [Running](#running)
- [Building for Release](#building-for-release)
- [Environment Overlays (dev / prod)](#environment-overlays-dev--prod)
- [Logging](#logging)
- [API Documentation (OpenAPI / Swagger / Scalar)](#api-documentation-openapi--swagger--scalar)
- [Database](#database)
- [LLM / Agent Settings](#llm--agent-settings)
- [Search Integration](#search-integration)
- [Development Workflow](#development-workflow)
- [Security & Secrets](#security--secrets)
- [Troubleshooting](#troubleshooting)
- [Planned / Possible Enhancements](#planned--possible-enhancements)

## Architecture & Crates
Workspace members (located under `crates/`):

| Crate | Purpose (high-level) |
|-------|----------------------|
| `server` | Entrypoint: HTTP server, routing, API exposure, OpenAPI docs mounting. |
| `config` | Layered configuration loading (base + environment overlays) via Figment / TOML. |
| `database` | Database connection & SeaORM setup (pool management, models, helpers). |
| `migration` | Schema migrations (SeaORM migration crate). |
| `mxagent` | Agent logic & orchestration with LLM (planning, verification helpers). |
| `search` | Search related utilities / external search API integration. |
| `utils` | Shared helpers/utilities (common types, error helpers, etc.). |

(Descriptions are inferred from crate names and dependencies; inspect each crate for implementation details.)

## Features
- Layered configuration (base + environment-specific overlay).
- `APP_PROFILE` driven environment selection (dev/prod).
- Axum 0.8 async web server.
- SeaORM for database access (PostgreSQL).
- Structured logging with `tracing`.
- OpenAPI generation with `utoipa` + interactive UIs (Swagger UI & Scalar).
- Pluggable LLM endpoint & agent parameters.
- External search API key integration.
- Workspace dependency sharing & reproducible builds.

## Tech Stack
- Language: Rust (Edition 2024)
- Async Runtime: Tokio
- Web: Axum
- ORM: SeaORM (+ migration crate)
- Configuration: Figment + TOML
- Logging / Observability: tracing, env-filter
- OpenAPI / Docs: utoipa, utoipa-swagger-ui, utoipa-scalar
- Error Handling: snafu

## Quick Start (5 min)

1. Install Rust toolchain (see `rust-toolchain.toml` if present).
2. Install PostgreSQL and create DB:
   Default DSN: `postgresql://postgres:testpassword@127.0.0.1:5432/wisland_search?sslmode=disable`
3. Clone & build:
   `git clone <repo-url>`
   `cd WisLand-Search`
   `cargo build -p server`
4. Choose environment profile via `APP_PROFILE`:
   - `export APP_PROFILE=dev` (development)
   - Omit or set `APP_PROFILE=prod` for production defaults
5. Ensure `base.toml` exists in run directory (binary working dir). `base.dev.toml` / `base.prod.toml` should also be present for overlays.
6. Run:
   - Dev: `APP_PROFILE=dev cargo run -p server`
   - Release: `cargo build -p server --release && APP_PROFILE=prod ./target/release/server`
7. Open API docs (see [API Documentation](#api-documentation-openapi--swagger--scalar)).

## Configuration
Primary configuration file: `base.toml`

Sections:

- `[server]`
  - `host`, `port`
  - `api_prefix`
- `[database]`
  - `url`
  - `timeout_ms`, `idle_timeout_ms`, `acquire_timeout_ms`
  - `max_connections`, `min_connections`
- `[llm]`
  - `temperature`, `prompt_max_token`
  - `endpoint`, `model`, `api_key`
- `[agent]`
  - `max_history`
  - `plan_suffix`, `search_plan_suffix`
  - `verifier_user_prompt` (template using `{question}` / `{search_result}`)
- `[search]`
  - `serp_api_key`
- Root
  - `name`, `version`, `log`

Override precedence (effective):
1. `base.toml`
2. Overlay file selected by `APP_PROFILE`
3. (Potential) environment variable providers (check `config` crate implementation)

## Environment Selection (`APP_PROFILE`)
The runtime selects which overlay file to merge after `base.toml` based on the `APP_PROFILE` environment variable:

| APP_PROFILE value | Overlay file loaded | Notes |
|-------------------|---------------------|-------|
| `dev`             | `base.dev.toml`     | Development settings (e.g., verbose logging) |
| `prod`            | `base.prod.toml`    | Production defaults |
| unset             | `base.prod.toml`    | Falls back to production overlay |
| any other value   | `base.prod.toml` (assumed) | Treat unexpected values as production unless logic changes |

Example invocations:
```bash
APP_PROFILE=dev cargo run -p server
APP_PROFILE=prod ./target/release/server
./target/release/server   # APP_PROFILE unset → prod overlay
```

(If custom behavior is required for additional profiles, extend the selection logic inside the `config` crate.)

## Running

Development (auto recompilation on edits when using tools like `cargo watch`):
```bash
APP_PROFILE=dev cargo run -p server
```

Production-style run:
```bash
cargo build -p server --release
APP_PROFILE=prod ./target/release/server
```

Ensure the working directory contains:
- `base.toml`
- `base.dev.toml` (if using APP_PROFILE=dev)
- `base.prod.toml`

## Building for Release
```bash
cargo build -p server --release
```
Artifact: `./target/release/server`

Ship together:
1. `server` (binary)
2. `base.toml`
3. `base.prod.toml`
4. (Optionally) `base.dev.toml` if you allow dev mode in deployed environment

## Environment Overlays (dev / prod)
- `base.dev.toml`: development overrides (currently name + log level)
- `base.prod.toml`: production overrides
- Selection: strictly via `APP_PROFILE` (see above)
- If you later add staging or test, introduce `base.staging.toml` and extend logic.

## Logging
Configured via `log` key using tracing env filter syntax:
- Example dev: `debug,sqlx=off`
- Example prod: `info,sqlx=warn`
Adjust in overlay files to avoid editing `base.toml`.

## API Documentation (OpenAPI / Swagger / Scalar)
The server uses `utoipa` + `utoipa-swagger-ui` + `utoipa-scalar`.
Common mount points (verify in `server` crate source):
- Swagger UI: e.g., `/swagger-ui`
- Scalar UI: e.g., `/scalar`
- Raw OpenAPI JSON: e.g., `/api-doc/openapi.json`
(Confirm exact routes; update this section if they differ.)

## Database
Default DSN (development):
```
postgresql://postgres:testpassword@127.0.0.1:5432/wisland_search?sslmode=disable
```
Checklist:
1. PostgreSQL running.
2. Database `wisland_search` exists: `createdb wisland_search`
3. Network / auth correct.

SeaORM migration usage (generic pattern—adjust to actual CLI implemented):
```bash
cargo run -p migration
```
If custom subcommands exist, document them here after inspecting the `migration` crate.

## LLM / Agent Settings
- `llm.endpoint` & `llm.model` define remote inference target.
- `llm.prompt_max_token` constrains token budget.
- `agent.plan_suffix`, `agent.search_plan_suffix` identify planning segments.
- `agent.verifier_user_prompt` supplies a templated verification stage.
- Provide `llm.api_key` via secret injection (env var or secret mount), not committed file.

## Search Integration
- `search.serp_api_key` consumed by external search logic.
- Treat committed key as non-secure; replace for real deployments.
- Consider removing from VCS and sourcing from env:
  - e.g., `SERP_API_KEY=xxxx ./target/release/server` (if code supports env fallbacks).

## Development Workflow
Core commands:
```bash
cargo check
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```
Add new crate:
1. `mkdir crates/newcrate && cd crates/newcrate`
2. `cargo init --lib`
3. Add path dependencies where needed; pattern `crates/*` auto-includes.

## Security & Secrets
- Never commit real production DSNs, API keys, or LLM credentials.
- Externalize secrets:
  - Environment variables
  - Vault / AWS Secrets Manager / GCP Secret Manager
- Rotate keys regularly (search, LLM).
- Run periodic security scans:
  ```bash
  cargo audit
  ```
  (Add `cargo-deny` if policy enforcement needed.)

## Troubleshooting
| Symptom | Cause | Resolution |
|---------|-------|------------|
| Server fails to start | Port busy | Change `server.port` |
| DB connection errors | DSN invalid / DB down | Verify `[database]` section & DB status |
| Slow responses | Connection pool saturation | Increase `max_connections` / optimize queries |
| Missing API docs | Route mismatch | Inspect `server` routing configuration |
| LLM 401 / 403 | Missing/invalid `llm.api_key` | Provide correct secret |
| Incorrect configuration in dev | Forgot `APP_PROFILE=dev` | Export `APP_PROFILE` before run |
| Always using prod overlay | `APP_PROFILE` unset / typo | Ensure exact value `dev` |

## Planned / Possible Enhancements
- Dockerfile & container deployment guide
- CI pipeline (lint, test, audit, build)
- Structured error response schema
- Metrics (Prometheus exporter)
- Search result caching layer (Redis)
- Graceful shutdown hooks & readiness/liveness endpoints
- Pluggable vector store integration
- Additional profiles (staging) via extended `APP_PROFILE` logic

---

Happy building. For deeper internals, inspect each crate’s `Cargo.toml` and `src` directory. Keep this README updated when adding new profiles or configuration keys—especially if you extend `APP_PROFILE` logic.
