# 配置说明

simple-ai-gateway 的运行时配置全部来自一个 YAML 文件,启动时通过 `--config` 或环境变量 `GATEWAY_CONFIG` 指定。本文档列出全部字段、默认值与可选取值,示例见 `config/example.*.yaml`。

文件根结构:

```yaml
server: { ... }
storage: { ... }
admin: { ... }
providers: { ... }
routes: [ ... ]
limits: [ ... ]
budgets: [ ... ]
observability: { ... }
```

修改该文件会被自动监听并热重载,无需重启进程(由 `gateway-api/reload` 负责)。

---

## 环境变量

下列环境变量在启动时被读取,不在 YAML 中:

| 变量 | 必填 | 用途 |
| --- | --- | --- |
| `GATEWAY_MASTER_KEY` | 是 | 32 字节 base64,落库凭证的 AES-GCM 加密密钥。丢失即无法解密历史数据。生成: `openssl rand -base64 32`。 |
| `GATEWAY_ROOT_TOKEN` | 推荐 | Admin API 的初始 root token。变量名由 `admin.root_token_env` 决定,默认即此名;为空则关闭 Admin API。 |
| `GATEWAY_CONFIG` | 否 | 配置文件路径,等价于 `--config`,默认 `config/example.lite.yaml`。 |
| `GATEWAY_WORKERS` | 否 | 仅在校验时使用;`>1` 且 storage 为 `lite` 时会拒绝启动。 |
| `RUST_LOG` | 否 | `tracing` EnvFilter 表达式,默认 `info,sqlx::query=warn`。 |
| `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` / ... | 视配置 | 任何被 `credential_ref: env://VAR` 引用的变量。 |

---

## `server`

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `bind` | string | `0.0.0.0:8080` | 监听地址,标准 `host:port`。 |
| `request_timeout_ms` | u64 | `600000` | 单次请求总超时(毫秒),含上游往返。 |
| `default_project_id` | string | `default` | 启动时自动 seed 的默认项目 ID;新 API Key 默认归属此项目。 |

---

## `storage`

通过 `profile` 字段区分三种形态(serde tagged enum,`profile` 之外的字段按形态填):

### `profile: lite`

单进程 SQLite + 进程内缓存。适合小规模 / 开发。

```yaml
storage:
  profile: lite
  sqlite:
    path: ./data/gateway.db
    max_size_mb: 10240
    log_retention_days: 30
  cache:
    l1_memory_mb: 256
    l2_max_size_mb: 1024
```

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `sqlite.path` | `./data/gateway.db` | 数据库文件路径。若在网络盘 (NFS/SMB) 上会发出警告,SQLite 锁不可靠。 |
| `sqlite.max_size_mb` | `10240` | 软上限,用于日志清理触发参考。 |
| `sqlite.log_retention_days` | `30` | 请求日志保留天数。 |
| `cache.l1_memory_mb` | `256` | L1 内存缓存容量。 |
| `cache.l2_max_size_mb` | `1024` | L2(SQLite)缓存容量。 |

**注意**:`GATEWAY_WORKERS>1` 与 `lite` 不兼容,网关会拒绝启动。

### `profile: standard`

Postgres + Redis,可横向扩展。

```yaml
storage:
  profile: standard
  postgres:
    url: postgres://gateway:gatewaypass@postgres:5432/gateway
    max_connections: 32
  redis:
    url: redis://redis:6379/0
  cache:
    l1_memory_mb: 256
    l2_max_size_mb: 1024
```

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `postgres.url` | (必填) | sqlx 兼容连接串。 |
| `postgres.max_connections` | `32` | 连接池大小。 |
| `redis.url` | (必填) | Redis 连接串。 |
| `cache.l1_memory_mb` | `256` | L1 内存缓存。 |
| `cache.l2_max_size_mb` | `1024` | L2(Redis)缓存。 |

### `profile: memory`

全内存,无持久化。重启即失,只用于测试。

```yaml
storage:
  profile: memory
  cache:
    l1_memory_mb: 64
    l2_max_size_mb: 256
```

---

## `admin`

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `root_token_env` | string | `GATEWAY_ROOT_TOKEN` | 持有 root 权限的环境变量名。空字符串表示禁用 Admin API。 |
| `password_login` | bool | `false` | 是否启用用户名/密码登录(配合 `/admin/auth/login`)。 |

---

## `providers`

`map<string, ProviderConfig>`,key 是供应商名(被 `routes` 和路径 `/v1/{provider}/*` 引用)。

```yaml
providers:
  openai:
    base_url: https://api.openai.com
    credential_ref: env://OPENAI_API_KEY
    headers:
      X-Custom: value
  anthropic:
    base_url: https://api.anthropic.com
    credential_ref: env://ANTHROPIC_API_KEY
```

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `base_url` | 是 | 上游基础 URL,客户端路径会拼在后面。 |
| `credential_ref` | 是 | `env://VAR_NAME` 从环境变量读取,或 `secret://<credential-id>` 从加密落库的凭证库读取(凭证由 Admin API 注入)。 |
| `headers` | 否 | 透传给上游的额外固定 header。 |

---

## `routes`

按顺序匹配,数组元素结构:

```yaml
routes:
  - match:
      path: /v1/openai/*
      model_prefix: gpt-
    primary:
      provider: openai
      model: gpt-4o-mini      # 可选,改写模型
    cache:
      enabled: true
      ttl: 3600
    retry:
      max_attempts: 3
      initial_backoff_ms: 500
    fallbacks:
      - provider: anthropic
        model: claude-3-5-sonnet
        trigger: [upstream_5xx, timeout]
```

### `match`

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `path` | `None` | 路径前缀或通配 `*`。 |
| `model_prefix` | `None` | 请求体 `model` 字段的前缀匹配。 |

两者都为空表示匹配所有进入此路由表的请求。

### `primary` / `fallbacks` (RouteTarget)

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `provider` | 是 | 必须存在于 `providers` 表,否则启动失败。 |
| `model` | 否 | 改写请求体的 `model`,可用于将外部模型名映射到供应商内部名。 |
| `trigger` | 否 | 仅 fallback 使用;空数组等于"总是触发"。可选值见下。 |

**`trigger` 取值**(参考 `crates/gateway-core/src/proxy/retry.rs`):

| 值 | 触发场景 |
| --- | --- |
| `upstream_5xx` / `upstream_error` | 上游返回 5xx,或不可重试的服务端错误。 |
| `timeout` | 请求超时,或可重试的网络错误。 |
| `rate_limited` | 上游 429。 |
| `network` | 一般性可重试网络错误。 |

### `cache`

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `enabled` | `false` | 是否对该路由启用响应缓存。 |
| `ttl` | `3600` | 缓存秒数。 |

缓存 key 来自请求体 + 路径的 blake3 摘要;流式响应不缓存。

### `retry`

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `max_attempts` | `3` | 同一目标上的最大尝试次数(含首次)。 |
| `initial_backoff_ms` | `500` | 初始退避,按指数增长。 |

---

## `limits`

数组,每条规则独立检测;命中任意一条即拒绝请求(HTTP 429)。

```yaml
limits:
  - target: { type: key, id: "*" }
    rpm: 1000
    tpm: 200000
    concurrency: 50
  - target: { type: project, id: default }
    rpm: 5000
  - target: { type: global }
    concurrency: 200
```

### `target`

| `type` | `id` 语义 |
| --- | --- |
| `key` | 匹配 Gateway API Key 的 ID;`"*"` 表示所有 Key。 |
| `project` | 匹配项目 ID;`"*"` 表示所有项目。 |
| `global` | 全局,无视 `id`。 |
| `metadata` | 预留,目前未启用。 |

### 度量字段(都可省略)

| 字段 | 含义 |
| --- | --- |
| `rpm` | Requests per minute 上限。 |
| `tpm` | Tokens per minute 上限(基于请求中估算或上游返回的 token 数)。 |
| `concurrency` | 同时在飞的请求数上限。 |

---

## `budgets`

数组,每条独立累计。命中 `block` 阈值会拒绝后续请求(HTTP 402),`notify` 阈值会触发 webhook。

```yaml
budgets:
  - name: monthly-team-budget
    target:
      project_id: default
      # 或 gateway_key_id: xxx
    period: monthly
    amount_usd: 500.0
    thresholds:
      - { at: 0.8, action: notify, webhook: https://hooks.example.com/budget }
      - { at: 1.0, action: block }
```

| 字段 | 说明 |
| --- | --- |
| `name` | 预算唯一名,用于计数 key 与日志。 |
| `target.project_id` | 与 `gateway_key_id` 二选一;不填则不会累计任何流量。 |
| `target.gateway_key_id` | 限定到具体 Key。 |
| `period` | `daily` / `weekly` / `monthly`,UTC 边界。未知值按 `monthly` 处理。 |
| `amount_usd` | 周期总预算,单位美元。 |
| `thresholds[].at` | 0–1 的百分比阈值。 |
| `thresholds[].action` | `notify`(发 webhook,默认幂等) / `block`(拒绝请求)。 |
| `thresholds[].webhook` | 仅 `notify` 时使用,HTTP POST 一个 JSON 负载。 |

成本由内嵌的 `pricing-catalog.json` 计算,未知模型按 0 计。

---

## `observability`

```yaml
observability:
  metrics: true
  tracing:
    enabled: true
    format: json
    otlp_endpoint: null
```

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `metrics` | `true` | 是否暴露 Prometheus `/metrics` 端点。 |
| `tracing.enabled` | `true` | 是否开启 tracing。 |
| `tracing.format` | `json` | `json` 或 `text`,影响 stdout 日志格式。 |
| `tracing.otlp_endpoint` | `null` | 可选 OTLP HTTP/gRPC 端点。 |

---

## 校验规则

启动时 `AppConfig::validate()` 会做以下检查,失败立即退出:

- 所有 `routes[].primary.provider` 必须在 `providers` 中存在。
- 所有 `routes[].fallbacks[].provider` 必须在 `providers` 中存在。

其他不一致(如未知 `limit.target.type`)在运行时被忽略而非拒绝,便于在多版本间平滑过渡。
