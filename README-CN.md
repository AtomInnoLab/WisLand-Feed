# WisLand-Feed

## 配置说明
主配置文件：`base.toml`
主要区块与字段：

- `[server]`：`host`, `port`, `api_prefix`
- `[database]`：`url`, `timeout_ms`, `idle_timeout_ms`, `acquire_timeout_ms`, `max_connections`, `min_connections`
- `[llm]`：`temperature`, `prompt_max_token`, `endpoint`, `model`, `api_key`
- `[agent]`：`max_history`, `plan_suffix`, `search_plan_suffix`, `verifier_user_prompt`
- `[search]`：`serp_api_key`
- 根级字段：`name`, `version`, `log`

## 入口（server / worker）
- `server`：HTTP 服务入口，提供路由、API 与文档挂载。
- `worker`：任务处理入口，负责与 Feed 相关的异步/定时任务处理；与 `server` 共用同一套配置加载逻辑（见下文 `APP_PROFILE`）。
- 重要：生产环境必须同时部署并运行 `server` 与 `worker`。若未运行 `worker`，将导致抓取/解析/入队/消费等后台任务无法执行，进而造成数据不更新、任务积压或接口返回过期数据。

覆盖优先级（实际生效顺序）：
1. `base.toml`
2. 由 `APP_PROFILE` 选中的覆盖文件
3. （如果 `config` crate 内实现了环境变量 provider，则环境变量可再覆盖）

## 环境选择（APP_PROFILE）
运行时根据环境变量 `APP_PROFILE` 决定加载哪个覆盖文件：

| APP_PROFILE 值 | 加载覆盖文件       | 说明 |
|----------------|--------------------|------|
| `dev`          | `base.dev.toml`    | 开发模式，通常日志更详细 |
| `prod`         | `base.prod.toml`   | 生产模式 |
| 未设置         | `base.prod.toml`   | 默认回退到生产覆盖 |
| 其他值         | `base.prod.toml`   | 当未知值处理为生产（若逻辑未扩展） |

示例：
```bash
APP_PROFILE=dev cargo run -p server
APP_PROFILE=prod ./target/release/server
./target/release/server   # 未设置 APP_PROFILE → 生产覆盖
APP_PROFILE=dev cargo run -p worker
APP_PROFILE=prod ./target/release/worker
./target/release/worker   # 未设置 APP_PROFILE → 生产覆盖
```

如需新增 profile（如 `staging`），请在 `config` crate 中扩展选择逻辑，并添加 `base.staging.toml`。

## 运行方式
开发模式（可结合 `cargo watch` 等）：
```bash
APP_PROFILE=dev cargo run -p server
APP_PROFILE=dev cargo run -p worker
```

生产模式：
```bash
cargo build -p server --release
cargo build -p worker --release
APP_PROFILE=prod ./target/release/server
APP_PROFILE=prod ./target/release/worker
```
生产环境建议将二者作为独立进程或容器分别运行，并使用 `systemd`/`supervisord`/Kubernetes 等进行守护与重启。

保证当前工作目录包含：
- `base.toml`
- `base.dev.toml`（若使用 dev）
- `base.prod.toml`

## Release 构建
```bash
cargo build -p server --release
cargo build -p worker --release
```
产物：
- `./target/release/server`
- `./target/release/worker`

部署建议打包项：
1. 二进制：`server` 与 `worker`
2. `base.toml`
3. `base.prod.toml`
4. （可选）`base.dev.toml`（如果允许在特定环境启用 dev）

## 环境覆盖文件（dev / prod）
- `base.dev.toml`：开发覆盖（当前仅覆盖 `name` 与 `log`）
- `base.prod.toml`：生产覆盖
- 选择逻辑：完全由 `APP_PROFILE` 控制
- 扩展：添加更多环境时新建对应覆盖文件并扩展匹配逻辑

## 日志
由配置项 `log` 控制（tracing env filter 语法）：
- Dev 示例：`debug,sqlx=off`
- Prod 示例：`info,sqlx=warn`

建议将修改放入覆盖文件（而非直接改基础配置）。

## API 文档 (OpenAPI / Swagger / Scalar)
使用 `utoipa` 系列生成与展示：
- Swagger UI：可能路径 `/swagger-ui`
- Scalar UI：可能路径 `/scalar`
- OpenAPI JSON：可能路径 `/api-doc/openapi.json`

（请查看 `server` crate 中的路由挂载源代码确认实际路径，如有差异可更新本节。）

## 数据库
默认开发 DSN：
```
postgresql://postgres:testpassword@127.0.0.1:5432/wisland_feed?sslmode=disable
```
检查清单：
1. PostgreSQL 已启动。
2. 数据库 `wisland_feed` 已创建：`createdb wisland_feed`
3. 账号、密码与网络访问正常。

迁移（泛化示例，需查看 `migration` crate 实际实现）：
```bash
cargo run -p migration
```
若存在自定义子命令（如 generate / up / down），请在阅读代码后补充至本文。


## 环境变量

