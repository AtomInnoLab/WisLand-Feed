# WisLand-Feed

## Configuration
Main configuration file: `base.toml`

Key sections and fields:

- `server`: `host`, `port`, `api_prefix`
- `database`: `url`, `timeout_ms`, `idle_timeout_ms`, `acquire_timeout_ms`, `max_connections`, `min_connections`
- `llm`: `temperature`, `prompt_max_token`, `endpoint`, `model`, `api_key`
- `agent`: `max_history`, `plan_suffix`, `search_plan_suffix`, `verifier_user_prompt`
- `search`: `serp_api_key`
- root-level fields: `name`, `version`, `log`

## Entrypoints (server / worker)
- `server`: HTTP entrypoint that exposes routes, APIs, and docs.
- `worker`: Task entrypoint handling feed-related background jobs (async/scheduled). Shares the same configuration loading logic as `server` (see `APP_PROFILE`).
- Important: In production you must run both `server` and `worker`. Not running `worker` will break background processing (fetch/parse/enqueue/consume), leading to stale data or backlog.

Override precedence (from lowest to highest):
1. `base.toml`
2. Profile override selected by `APP_PROFILE`
3. (Optional) Environment variables, if a provider is implemented in the `config` crate

## Environment selection (APP_PROFILE)
At runtime, the environment variable `APP_PROFILE` decides which override file to load:

| APP_PROFILE | Override file     | Notes                                  |
|-------------|-------------------|----------------------------------------|
| `dev`       | `base.dev.toml`   | Development mode, usually more verbose |
| `prod`      | `base.prod.toml`  | Production mode                        |
| unset       | `base.prod.toml`  | Defaults to production override         |
| other       | `base.prod.toml`  | Treated as production if not extended  |

Examples:
```bash
APP_PROFILE=dev cargo run -p server
APP_PROFILE=prod ./target/release/server
./target/release/server   # APP_PROFILE unset → production override
APP_PROFILE=dev cargo run -p worker
APP_PROFILE=prod ./target/release/worker
./target/release/worker   # APP_PROFILE unset → production override
```

If you need a new profile (e.g., `staging`), extend the selection logic in the `config` crate and add `base.staging.toml`.

## How to run
Development (you can combine with tools like `cargo watch`):
```bash
APP_PROFILE=dev cargo run -p server
APP_PROFILE=dev cargo run -p worker
```

Production:
```bash
cargo build -p server --release
cargo build -p worker --release
APP_PROFILE=prod ./target/release/server
APP_PROFILE=prod ./target/release/worker
```
In production, run both processes separately (or containers) and manage them with `systemd`/`supervisord`/Kubernetes for supervision and restarts.

Ensure the current working directory contains:
- `base.toml`
- `base.dev.toml` (if using dev)
- `base.prod.toml`

## Release build
```bash
cargo build -p server --release
cargo build -p worker --release
```
Artifacts:
- `./target/release/server`
- `./target/release/worker`

Recommended deployment bundle:
1. Binaries: `server` and `worker`
2. `base.toml`
3. `base.prod.toml`
4. (Optional) `base.dev.toml` (if dev can be enabled in specific environments)

## Environment override files (dev / prod)
- `base.dev.toml`: development overrides (currently only `name` and `log`)
- `base.prod.toml`: production overrides
- Selection is controlled entirely by `APP_PROFILE`
- Extend by adding more override files and selection logic

## Logging
Controlled by the `log` configuration (tracing env filter syntax):
- Dev example: `debug,sqlx=off`
- Prod example: `info,sqlx=warn`

Prefer putting changes in override files rather than editing the base config directly.

## API Docs (OpenAPI / Swagger / Scalar)
Generated and served via the `utoipa` stack:
- Swagger UI: possibly at `/swagger-ui`
- Scalar UI: possibly at `/scalar`
- OpenAPI JSON: possibly at `/api-doc/openapi.json`

(Check the route mounting code in the `server` crate to confirm actual paths and update this section if they differ.)

## Database
Default development DSN:
```
postgresql://postgres:testpassword@127.0.0.1:5432/wisland_feed?sslmode=disable
```
Checklist:
1. PostgreSQL is running.
2. Database `wisland_feed` exists: `createdb wisland_feed`.
3. Account, password, and network access are correct.

Migrations (generic example, check the `migration` crate for the actual implementation):
```bash
cargo run -p migration
```
If there are custom subcommands (e.g., generate / up / down), document them here after reviewing the code.

## Environment variables
```bash
# Aliyun OSS
APP_OSS.ACCESS_KEY_ID="Aliyun OSS access key id"
APP_OSS.ACCESS_KEY_SECRET="Aliyun OSS access key secret"
APP_OSS.BUCKET="Aliyun OSS bucket name"   # e.g., yyzjupload-dev
APP_OSS.ENDPOINT="oss-cn-shanghai.aliyuncs.com"
APP_OSS.PREFIX="Prefix path inside the bucket"  # e.g., wisland-feed

# RSS and workers
APP_RSS.WORKERS.PULL_SOURCES.CRON="* 1 * * * *"
APP_RSS.WORKERS.UPDATE_USER_INTEREST_METADATA.CONCURRENCY=1
```


