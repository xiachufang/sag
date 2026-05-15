# 开发指南

## 环境要求

- Rust 1.85+(`rust-toolchain.toml` 未固定,跟 `workspace.package.rust-version` 对齐即可)。
- 测试用 Postgres + Redis 时,Docker 即可。
- 跑 mock 上游需要 Python 3.10+(`scripts/mock-openai.py`)。

## 项目结构

```
crates/
├── gateway-core/      # 配置、proxy engine、provider 适配、cache、pricing、安全
├── gateway-storage/   # store trait + sqlite / postgres / redis / memory 实现
├── gateway-api/       # axum 路由、admin、auth、ratelimit、budget、reload
├── gateway-bin/       # 二进制入口
└── gateway-ui/        # 内嵌管理 UI(打包后的静态 index.html)
migrations/
├── sqlite/            # sqlx migrate 文件
└── postgres/
config/                # 示例 + 测试用 YAML
docs/                  # mdBook 源(本目录)
scripts/mock-openai.py # 集成测试用的 mock 上游
pricing-catalog.json   # 内嵌定价表(模型 → 单价)
```

## 编译与运行

```sh
# Debug 编译
cargo build

# Release 编译
cargo build --release --bin gateway

# 用纯内存后端起一个 dev 实例(不需要 DB)
cargo run --bin gateway -- --config config/test.memory.yaml

# Lite + 自带 mock 上游
python3 scripts/mock-openai.py &  # 监听 :18181
cargo run --bin gateway -- --config config/test.mock.yaml
```

## 测试

```sh
# 单元 + 集成测试
cargo test --workspace

# 只跑某个 crate
cargo test -p gateway-core

# 关掉日志输出 noise
RUST_LOG=warn cargo test --workspace
```

集成测试中:

- `config/test.standard.yaml` 假定 httpbin.org 可达。
- `config/test.cost.yaml` / `test.mock.yaml` 用本地 `mock-openai.py`(端口 18181)。
- `config/test.limits.yaml` 用 httpbin.org 的 `/anything` 跑通限流。

## 数据库迁移

```sh
# SQLite (Lite)
sqlx migrate run --source migrations/sqlite --database-url sqlite://./data/gateway.db

# Postgres (Standard)
sqlx migrate run --source migrations/postgres --database-url postgres://gateway:gatewaypass@127.0.0.1:54329/gateway
```

启动时 `SqliteBackend::open` / `PostgresBackend::open` 会自动跑一次 migrate,所以平时不用手动跑;手动跑的场景是新增 migration 后调试。

## 代码风格

- `cargo fmt --all` 提交前过一遍。
- `cargo clippy --workspace --all-targets -- -D warnings` 当 CI 门槛。
- 错误传播用 `thiserror::Error` 自定义 + `anyhow::Result` 在 binary 边界。
- HTTP handler 返回 `Result<Response, ApiError>`,`ApiError` 知道如何映射到 HTTP code。
- 时间戳一律 unix ms / unix sec(看上下文),不用 chrono 类型穿过 trait。

## 加新的供应商

绝大多数情况下你**不需要写代码** —— 如果新供应商的 API 兼容 OpenAI 协议(豆包、DeepSeek、Groq、Together、Mistral、Azure OpenAI、vLLM、Ollama、LM Studio 等都属于这类),直接在 YAML 里加一条 `providers.<name>: { kind: openai-compatible, base_url: ..., credential_ref: ... }` 即可。具体写法见 [配置参考 > providers](./configuration.md#providers)。

**只有当上游使用完全不同的认证协议时**(既不是 OpenAI 也不是 Anthropic),才需要新建一个 auth adapter:

1. 在 `crates/gateway-core/src/providers/` 加一个 `<name>.rs`,实现 `AuthInjector` trait(看 `openai.rs` 和 `anthropic.rs` 作模板)。adapter 主要做两件事:
   - 改写 outgoing request 的 auth header(替换客户端送来的 Authorization)。
   - 注入该 provider 要求的固定 header(如 anthropic 的 `anthropic-version`)。
2. 在 `providers/mod.rs`:
   - `pub mod <name>;`
   - 在 `build_auth_injector` 的 match 里加一个 `kind` 分支
   - 在 `is_known_provider_kind` 里加同一个 `kind` —— 让启动期 `validate` 认识它
3. 在 `pricing-catalog.json` 加该供应商的模型 → 单价(如果你想跟踪成本)。
4. 写测试(参考 `crates/gateway-core/src/providers/openai.rs` 末尾的单元测试)。
5. 配置文件里用 `providers.<name>: { kind: <kind>, ... }` 引用。

至于 token usage 抽取(用于成本核算和 TPM 限流):目前在 `crates/gateway-api/src/tokens.rs::extract_token_usage` 集中处理,假设上游响应体里有标准的 `usage` 字段(OpenAI / Anthropic 都满足)。完全不同形态的响应需要在那里加一条解析分支。

## 加新的 Admin 端点

1. 在 `crates/gateway-api/src/routes/admin/` 加一个 `<resource>.rs`,handler 签名:
   ```rust
   pub async fn handler(
       State(state): State<AppState>,
       principal: AdminPrincipal,
       /* extractors */
   ) -> Result<Json<Response>, ApiError>
   ```
2. 在 `routes/admin/mod.rs` 暴露,然后在 `server.rs::build_router` 的 `admin` Router 里 `.route(...)`。
3. 给 store trait 加方法(若需要新数据);分别在 SQLite / Postgres / Memory 实现。
4. 写集成测试(参考 `crates/gateway-api/tests/` 模板)。
5. 更新 [Admin API 文档](./admin-api.md)。

## 配置字段加 / 改

- `crates/gateway-core/src/config.rs` 加字段,记得加 `#[serde(default)]` 和默认值函数,保持向后兼容。
- 在 `AppConfig::validate` 里加校验(若需要)。
- 跑 `cargo test -p gateway-core config` 验证序列化/反序列化。
- 更新 [配置参考](./configuration.md)。
- 改动若涉及行为,加到 [架构概览](./architecture.md) 中对应小节。

## 文档

文档源在 `docs/src/`,mdBook 项目根在 `docs/`。

```sh
cargo install mdbook
cd docs
mdbook serve --open      # 本地预览,改文件自动刷新
mdbook build             # 输出到 docs/book/
```

每次 PR 改了代码行为,记得同步改文档。GitHub Actions 会在 push 到 `main` 时自动构建并发布到 GitHub Pages,见 `.github/workflows/docs.yml`。
