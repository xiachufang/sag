# simple-ai-gateway

一个用 Rust 编写的轻量级 AI API 网关,在你的应用和上游模型供应商(OpenAI、Anthropic)之间提供统一的代理层,内置认证、限流、预算、缓存、重试和回退能力。

📖 **完整文档:[tech.xiachufang.xyz/sag](http://tech.xiachufang.xyz/sag/)** (源在 `docs/src/`,基于 mdBook)

## 特性

- **统一代理**:`/v1/{provider}/*` 透明转发到上游,支持流式响应。
- **多供应商**:目前内置 OpenAI、Anthropic;按路由配置主备和回退触发条件。
- **认证与密钥管理**:基于 Argon2 的 API Key,Admin 走 JWT;凭证用 AES-GCM + master key 加密落库。
- **限流与并发控制**:基于 `governor` 的 RPM / TPM / 并发上限,目标可按 Key 或 `*` 通配。
- **预算**:按 Key / 项目设置成本/Token 用量阈值,内嵌定价表自动核算。
- **缓存**:L1 内存 + L2(Redis 或 SQLite);按路由开启,可配 TTL。
- **重试与回退**:可配置最大次数、退避;遇 5xx 或超时自动切换到 fallback 供应商。
- **两种部署形态**:
  - **Lite**:单机 SQLite,内存计数器,适合单进程小规模部署。
  - **Standard**:Postgres + Redis,适合多副本横向扩展。
- **可观测性**:Prometheus `/metrics`,JSON 结构化日志,可选 OTLP tracing。
- **配置热重载**:监听 YAML 文件改动,无需重启即可生效。
- **内置 Admin UI**:`/ui/` 路径直接访问。

## 快速开始

### 准备环境变量

```sh
export GATEWAY_ROOT_TOKEN=$(openssl rand -hex 32)
export GATEWAY_MASTER_KEY=$(openssl rand -base64 32)
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...
```

`GATEWAY_MASTER_KEY` 用于加密落库的上游凭证,丢失即无法解密历史数据,务必妥善保存。

### Lite 模式(单机 SQLite)

```sh
docker compose -f docker-compose.lite.yml up --build
```

或本地编译:

```sh
cargo run --release --bin gateway -- --config config/example.lite.yaml
```

### Standard 模式(Postgres + Redis)

```sh
docker compose up --build
```

网关在 `http://localhost:8080` 暴露:

| 路径 | 用途 |
| --- | --- |
| `/ui/` | 管理 UI |
| `/v1/{provider}/*` | 上游代理入口 |
| `/admin/*` | 管理 API(需 Admin Token) |
| `/healthz`, `/readyz` | 健康检查 |
| `/metrics` | Prometheus 指标 |

## 使用示例

通过网关调用 OpenAI:

```sh
curl http://localhost:8080/v1/openai/v1/chat/completions \
  -H "Authorization: Bearer <gateway-api-key>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "hello"}]
  }'
```

API Key 在 Admin UI 或通过 `POST /admin/keys`(带 `GATEWAY_ROOT_TOKEN`)创建。

## 配置

完整字段说明见 [配置参考文档](./docs/src/configuration.md),示例见 `config/example.standard.yaml` 和 `config/example.lite.yaml`,主要字段:

- `server` — 监听地址、请求超时、默认项目 ID。
- `storage` — `profile: lite | standard | memory`,以及对应的 SQLite / Postgres / Redis / 缓存设置。
- `providers` — 上游 base URL 和凭证引用(`env://` 或加密落库的引用)。
- `routes` — 路径匹配、主备、缓存 TTL、重试、回退触发条件。
- `limits` — 按 Key 的 RPM / TPM / 并发上限。
- `budgets` — 按目标的成本/Token 预算。
- `observability` — Prometheus、tracing、OTLP 端点。

修改配置文件后会被自动重载,无需重启进程。

## 项目结构

```
crates/
├── gateway-core/      # 配置、代理引擎、供应商适配、缓存、定价、加密
├── gateway-storage/   # 抽象 trait + SQLite / Postgres / Redis / 内存实现
├── gateway-api/       # axum 路由、Admin、认证、限流、预算、热重载
├── gateway-bin/       # 二进制入口
└── gateway-ui/        # 内置管理 UI(打包后的静态文件)
migrations/            # SQLite 和 Postgres 数据库迁移
config/                # 示例配置
pricing-catalog.json   # 内嵌定价表(供成本核算)
scripts/mock-openai.py # 集成测试用的 mock 上游
```

## 开发

```sh
# 编译并跑单元测试
cargo test --workspace

# 构建 release 二进制
cargo build --release --bin gateway

# 用内存后端快速起一个 dev 实例
cargo run --bin gateway -- --config config/test.memory.yaml
```

要求 Rust 1.85+。

## 许可证

MIT
