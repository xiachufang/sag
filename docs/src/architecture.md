# 架构概览

## 进程结构

simple-ai-gateway 是单二进制 (`gateway`) 的 Rust 程序,内部分为四个 crate:

| Crate | 职责 |
| --- | --- |
| `gateway-core` | 配置、代理引擎、供应商适配、缓存、定价、安全(master key / Argon2 / JWT)。 |
| `gateway-storage` | `MetadataStore` / `LogStore` / `KvStore` / `CounterStore` 抽象,以及 SQLite、Postgres、Redis、内存四套实现。 |
| `gateway-api` | axum 路由、Admin API、Gateway Key 认证、限流、预算、配置热重载。 |
| `gateway-bin` | 进程入口:加载配置、初始化 stores、装配 `ProxyEngine`、启动 HTTP 服务。 |
| `gateway-ui` | 内置的最简 Admin UI(`include_str!` 注入)。 |

## 请求生命周期

一次代理请求(`/v1/{namespace}/...`)经过的环节,按顺序:

1. **认证** (`auth.rs`) — 从 `Authorization` 头取 Gateway Key,BLAKE3 keyed-hash 后查表,常时间对比,核对 `status` 与 `expires_at`。
2. **读 body** (`routes/proxy.rs`) — 整体读入内存(流式响应除外),计算 blake3 摘要,提取 `model` 字段。
3. **预算检查** (`budget.rs`) — 命中 `action: block` 阈值则直接 402。
4. **限流** (`ratelimit.rs`) — 按 `limits[]` 配置依次检查 RPM / TPM / 并发。
5. **路由匹配** — 按数组顺序找第一条满足 `match.namespace == URL段`(未设置时默认 `primary.provider`)且 `match.model_prefix`(若设置)命中的路由。`namespace` 是对外暴露的 URL 段,`primary.provider` 才是 `providers` 表的 key,两者解耦。没有匹配则用 `ProviderChain::primary_only`,把 URL 段直接当作 provider 名(无缓存、无 fallback、默认重试)。
6. **缓存查找** — 若 `cache.enabled`,以 `(provider, path, body_blake3)` 为 key 查 L1(内存)→ L2(SQLite/Redis)。命中则直接返回,`X-Cache: HIT`。
7. **构造 forward request** — 改写凭证、Host,追加 `providers.headers`。
8. **执行链** (`proxy/executor.rs`) — 对 `primary` 尝试 `retry.max_attempts` 次(指数退避);失败若命中 `fallback.trigger` 则切换到下一个 target,直到耗尽。
9. **写缓存 + 落日志** — 非流式且可缓存的响应写回缓存;不论成败都通过 `LogStore` 异步落库。
10. **更新预算与指标** — 用上游返回的 token usage 算成本,累加到 `BudgetManager`,bump Prometheus counter/histogram。
11. **返回响应** — 透传 status / headers / body,附加 `X-Gateway-Request-Id`、`X-Cache`、`X-Provider`。

## 存储抽象

`StoreBundle` 把四种 store 组合在一起,供 API 层使用:

| Store | Lite (SQLite + Memory) | Standard (Postgres + Redis) | Memory (测试) |
| --- | --- | --- | --- |
| `MetadataStore` | SQLite | Postgres | 进程内 |
| `LogStore` | SQLite(异步批量写) | Postgres(异步批量写) | 环形缓冲 10k |
| `KvStore` (cache) | 内存 L1 + SQLite L2 | 内存 L1 + Redis L2 | 内存 |
| `CounterStore` | 内存 | Redis | 内存 |

Lite 模式下 `CounterStore` 是进程内 —— 这就是为什么 `GATEWAY_WORKERS>1` 与 `lite` 不兼容:多进程之间无法共享 RPM/TPM/并发计数。

## 配置热重载

`gateway-api/reload.rs` 用 `notify` 监听配置文件变化:

- 文件变更触发再次 `AppConfig::load_from_path` + `validate`。
- 验证通过则 `ArcSwap::store` 替换当前 config 快照。
- 失败则保留旧配置,记录 `gateway_config_reload_error_total`。

热重载范围:**全部字段**。`ProxyEngine` 持有的 reqwest client 不重建,但 routes / providers 的 base_url、credentials、limits、budgets、cache TTL 都会立即生效。

`server.bind` 改了不会生效(已经在监听)。

## 缓存设计

L1 (`moka`) 在所有 profile 下都是进程内 LRU,容量由 `cache.l1_memory_mb` 控制。

L2:

- **Lite**:与元数据同库,放在 SQLite 的 `kv` 表,按 `l2_max_size_mb` 做粗粒度淘汰。
- **Standard**:Redis,key 形如 `cache:<fingerprint>`,带 TTL。
- **Memory**:无 L2。

响应 body 超过 2 MB 不写缓存(`MAX_CACHEABLE_BODY_BYTES`),避免污染。

流式响应也会被缓存:转发途中 chunk 被同时累积进 `cache_chunks`,流正常结束后整段写入 KV。命中时 `build_cached_response` 用 `Body::from_stream` 按原 chunk 边界 replay,所以 SSE 客户端看到的事件流跟首次一致。但缓存写入需要请求体确定性(`temperature == 0` 且 `top_p >= 0.999`),否则一律 bypass。

## 重试 / 回退

`proxy/executor.rs` 把 `primary + fallbacks` 当成一条链,每个 entry 各自有 `retry.max_attempts`:

- 同一 entry 内的失败按指数退避重试。
- 切换到下一个 entry 前,先看上一个 entry 的 `AttemptOutcome` 是否在当前 entry 的 `trigger` 集合中;不匹配就直接返回错误。
- `trigger` 为空数组表示"无条件接管"。

可能的 `AttemptOutcome`:`Success` / `UpstreamError(status)` / `Timeout` / `RetryableNetwork`,以及 `RateLimited`(429)。

## 凭证解析

`provider.credential_ref` 在每次构造 forward request 时解析:

- `env://VAR_NAME` — `std::env::var`,缺失则启动时就会因校验失败拒绝。
- `secret://<credential-id>` — 从 `MetadataStore` 取出加密 blob,用 master key 走 AES-256-GCM 解密。

两种方式都在每次请求时执行,因此切换凭证不需要重启。

## 关键非目标

- **不做 prompt 翻译/改写** —— 网关只搬运字节流。
- **不做模型路由(按 prompt 内容选模型)** —— 只按路径 + model_prefix 静态路由。
- **不实现 SDK 抽象** —— 客户端继续按上游 SDK 写。
