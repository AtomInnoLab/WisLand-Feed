# WisLand-Search

一个由多个 Rust crate 组成的模块化搜索与智能代理（Agent）服务（Workspace 结构）。本文件帮助你快速建立全局认知，并给出构建、配置、运行与扩展的明确步骤。

## 目录
- [架构与 Crate 列表](#架构与-crate-列表)
- [功能特性](#功能特性)
- [技术栈](#技术栈)
- [5 分钟快速开始](#5-分钟快速开始)
- [配置说明](#配置说明)
- [环境选择（APP_PROFILE）](#环境选择app_profile)
- [运行方式](#运行方式)
- [Release 构建](#release-构建)
- [环境覆盖文件（dev / prod）](#环境覆盖文件dev--prod)
- [日志](#日志)
- [API 文档 (OpenAPI / Swagger / Scalar)](#api-文档-openapi--swagger--scalar)
- [数据库](#数据库)
- [LLM / Agent 相关参数](#llm--agent-相关参数)
- [搜索集成](#搜索集成)
- [开发流程建议](#开发流程建议)
- [安全与密钥管理](#安全与密钥管理)
- [故障排查](#故障排查)
- [规划 / 可能的增强](#规划--可能的增强)

## 架构与 Crate 列表
Workspace 下位于 `crates/` 目录的成员：

| Crate | 主要用途（概述） |
|-------|------------------|
| `server` | 程序入口：HTTP 服务、路由、API 暴露、OpenAPI 文档挂载。 |
| `config` | 分层配置加载（基础配置 + 环境覆盖），使用 Figment / TOML。 |
| `database` | 数据库连接与 SeaORM 设置（连接池管理、模型、辅助函数）。 |
| `migration` | 数据库迁移（SeaORM Migration）。 |
| `mxagent` | 智能 Agent 逻辑与 LLM 调用编排（规划、校验等）。 |
| `search` | 搜索相关工具 / 外部搜索 API 集成。 |
| `utils` | 共享工具与通用辅助函数。 |

（说明基于命名与依赖推断，进一步细节可阅读各 crate 源码。）

## 功能特性
- 分层配置（基础 + 环境覆盖）。
- 基于 `APP_PROFILE` 的环境选择（dev / prod）。
- Axum 0.8 异步 Web 服务。
- SeaORM + PostgreSQL 数据访问。
- `tracing` 结构化日志。
- 使用 `utoipa` 自动生成 OpenAPI，并提供 Swagger UI / Scalar 交互界面。
- 可配置的 LLM 终端与 Agent 参数。
- 外部搜索 API Key 集成。
- Workspace 统一依赖管理，构建可复现。

## 技术栈
- 语言：Rust (Edition 2024)
- 异步运行时：Tokio
- Web：Axum
- ORM：SeaORM（含迁移）
- 配置：Figment + TOML
- 日志 / 可观测：tracing + env-filter
- API 文档：utoipa / utoipa-swagger-ui / utoipa-scalar
- 错误处理：snafu

## 5 分钟快速开始
1. 安装 Rust 工具链（参见根目录 `rust-toolchain.toml` 若存在）。
2. 安装并启动 PostgreSQL，创建数据库：
   默认 DSN：`postgresql://postgres:testpassword@127.0.0.1:5432/wisland_search?sslmode=disable`
3. 克隆与构建：
   ```bash
   git clone <repo-url>
   cd WisLand-Search
   cargo build -p server
   ```
4. 设置环境：
   ```bash
   export APP_PROFILE=dev   # 开发模式
   # 或省略 / 设置为 prod 则使用生产覆盖
   ```
5. 确保运行目录存在：`base.toml`、`base.dev.toml`、`base.prod.toml`
6. 运行：
   ```bash
   APP_PROFILE=dev cargo run -p server
   # 或
   cargo build -p server --release && APP_PROFILE=prod ./target/release/server
   ```
7. 浏览器访问 API 文档（参见后文 [API 文档](#api-文档-openapi--swagger--scalar)）。

## 配置说明
主配置文件：`base.toml`
主要区块与字段：

- `[server]`：`host`, `port`, `api_prefix`
- `[database]`：`url`, `timeout_ms`, `idle_timeout_ms`, `acquire_timeout_ms`, `max_connections`, `min_connections`
- `[llm]`：`temperature`, `prompt_max_token`, `endpoint`, `model`, `api_key`
- `[agent]`：`max_history`, `plan_suffix`, `search_plan_suffix`, `verifier_user_prompt`
- `[search]`：`serp_api_key`
- 根级字段：`name`, `version`, `log`

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
```

如需新增 profile（如 `staging`），请在 `config` crate 中扩展选择逻辑，并添加 `base.staging.toml`。

## 运行方式
开发模式（可结合 `cargo watch` 等）：
```bash
APP_PROFILE=dev cargo run -p server
```

生产模式：
```bash
cargo build -p server --release
APP_PROFILE=prod ./target/release/server
```

保证当前工作目录包含：
- `base.toml`
- `base.dev.toml`（若使用 dev）
- `base.prod.toml`

## Release 构建
```bash
cargo build -p server --release
```
产物：`./target/release/server`

部署建议打包项：
1. 二进制：`server`
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
postgresql://postgres:testpassword@127.0.0.1:5432/wisland_search?sslmode=disable
```
检查清单：
1. PostgreSQL 已启动。
2. 数据库 `wisland_search` 已创建：`createdb wisland_search`
3. 账号、密码与网络访问正常。

迁移（泛化示例，需查看 `migration` crate 实际实现）：
```bash
cargo run -p migration
```
若存在自定义子命令（如 generate / up / down），请在阅读代码后补充至本文。

## LLM / Agent 相关参数
- `llm.endpoint` & `llm.model`：远端推理服务定义。
- `llm.prompt_max_token`：控制生成/上下文 token 上限。
- `agent.plan_suffix` / `agent.search_plan_suffix`：对生成计划或搜索规划段落做后缀标记，方便解析。
- `agent.verifier_user_prompt`：校验阶段使用的模板（包含 `{question}` 与 `{search_result}` 占位）。
- `llm.api_key`：请通过安全方式注入（环境变量 / Secret 管理），不要提交到仓库。

## 搜索集成
- `search.serp_api_key`：外部搜索 API Key。
- 仓库中示例值视为非安全示范，应在生产环境替换。
- 可改为运行时读取环境变量（视 `config` crate 是否支持）。

## 开发流程建议
常用命令：
```bash
cargo check
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

新增 crate：
1. `mkdir crates/<yourcrate>`
2. `cargo init --lib`
3. 根据需要在其他 crate 的 `Cargo.toml` 中添加 path 依赖（Workspace 模式 `crates/*` 会自动包含）。

依赖版本统一放在 workspace 根的 `[workspace.dependencies]`，避免版本漂移。

## 安全与密钥管理
- 不要提交真实生产数据库 DSN、API Key、LLM 密钥。
- 采用：
  - 环境变量注入
  - Vault / AWS Secrets Manager / GCP Secret Manager 等
- 定期轮换：
  - 搜索 API Key
  - LLM API Key
- 依赖安全扫描：
  ```bash
  cargo audit
  # 可选：cargo deny
  ```

## 故障排查
| 现象 | 可能原因 | 解决方案 |
|------|----------|----------|
| 服务无法启动 | 端口被占用 | 修改 `server.port` 或释放端口 |
| 无法连接数据库 | DSN 错误 / DB 未启动 | 核对 `[database]` 配置与数据库状态 |
| 接口响应慢 | 连接池耗尽 / 查询慢 | 增大 `max_connections` 或优化 SQL |
| API 文档 404 | 路由路径不匹配 | 检查 `server` crate 中文档挂载代码 |
| LLM 返回 401/403 | 缺少或错误的 `llm.api_key` | 正确注入密钥 |
| dev 配置未生效 | 未设置 `APP_PROFILE=dev` | 运行前导出环境变量 |
| 始终使用生产配置 | 没有设置或拼写错误 | 确认环境变量值为精确的 `dev` |

## 规划 / 可能的增强
- 提供 Dockerfile 与镜像构建说明
- CI 流水线（lint / test / audit / build）
- 统一的错误响应结构
- 指标导出（Prometheus）
- 搜索结果缓存（如 Redis）
- 优雅停机 & 健康检查（liveness / readiness）
- 可插拔向量存储
- 新增更多 Profile（如 staging）+ 配置选择扩展

---

保持本 README 与实际实现同步；在扩展配置字段、增加新环境或路由时请及时更新。祝你开发顺利！
