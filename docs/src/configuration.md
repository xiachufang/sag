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
| `GATEWAY_MASTER_KEY` | 是 | 32 字节 base64,用于 Gateway Key 的 BLAKE3 keyed-hash 与 Admin JWT 签名。丢失即所有 Gateway Key 失效、所有已签发 JWT 失效。生成: `openssl rand -base64 32`。 |
| `GATEWAY_ROOT_TOKEN` | 推荐 | Admin API 的初始 root token。变量名由 `admin.root_token`(如 `env://GATEWAY_ROOT_TOKEN`)指定,该字段为空则关闭 Admin API。 |
| `GATEWAY_CONFIG` | 否 | 配置文件路径,等价于 `--config`,默认 `config/example.lite.yaml`。 |
| `GATEWAY_WORKERS` | 否 | 仅在校验时使用;`>1` 且 storage 为 `lite` 时会拒绝启动。 |
| `RUST_LOG` | 否 | `tracing` EnvFilter 表达式,默认 `info,sqlx::query=warn`。 |
| `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` / ... | 视配置 | 任何被 `credential: env://VAR` 引用的变量。 |

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
| `root_token` | string | `env://GATEWAY_ROOT_TOKEN` | root 权限的 Admin token。三种取值:`""`(空)→ 禁用 Admin API;`env://VAR_NAME` → 从环境变量读(未设或为空也视作禁用);其他字符串 → 直接当明文 token 用(与 `gateway_keys[].secret` 写明文同样的安全权衡,适合本地开发)。 |
| `password_login` | bool | `false` | 是否启用用户名/密码登录(配合 `/admin/auth/login`)。 |

---

## `gateway_keys`

可选数组,声明式预置一批 gateway API key。每次进程启动 + 配置热重载都会与库里 `origin='config'` 的行做一次同步:新增的写入、删除的丢弃、name/scopes 等元数据有变更则更新。

```yaml
gateway_keys:
  - id: prod-svc                          # 稳定 id,日志/限流/预算靠它关联
    name: prod-svc
    project_id: default                   # 可省略,默认取 server.default_project_id
    secret: sk-gw-live-xxxxxxxxxxxxxxxx   # 直接写明文,或 env://PROD_SVC_KEY 从环境变量读
    scopes: []                            # 可省
  - id: ci-readonly
    name: ci
    secret: env://CI_GATEWAY_KEY
```

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `id` | string | 是 | 行 id,跨重载稳定。`gateway_keys[]` 内必须唯一。 |
| `name` | string | 是 | 展示名,Admin 端列表里看到的就是它。 |
| `project_id` | string | 否 | 落到哪个 project。省略则用 `server.default_project_id`。 |
| `secret` | string | 是 | 明文 `sk-gw-{live,test}-...`,或 `env://VAR_NAME`。明文形态会用 `derive_hash` 哈希后落库,不会保留原文;env 形态延迟到 sync 时解析(变量缺失或为空会被打印到日志并跳过这一轮)。 |
| `scopes` | string[] | 否 | 与 Admin API 创建的 key 含义相同,默认 `[]`。 |

约束与行为:
- 启动期校验:`id` 唯一、`name`/`secret` 非空,明文 secret 必须带 `sk-gw-live-` 或 `sk-gw-test-` 前缀。
- Admin API **拒绝** revoke `origin='config'` 的 key(返回 400,提示从 YAML 移除)。UI 列表里这些 key 带 `config` 徽标且没有 Revoke 按钮。
- 配置文件里**移除某条 key** 后下一次热重载会从库里删除它(此后该明文 key 无法再认证)。要"暂时禁用",直接从 YAML 删掉即可。
- 明文 secret 会被 redacted 处理,不会出现在 `Debug` 日志里 —— 但 YAML 在磁盘上是明文,操作时注意文件权限;若想做最小权限注入,把 secret 写成 `env://...`。

---

## `providers`

`map<string, ProviderConfig>`,key 是上游身份名(被 `routes[].primary.provider` 和 `fallbacks[].provider` 引用)。URL `/v1/{namespace}/*` 中的 `namespace` 是另一个独立概念,默认与该 key 同名,可通过 `match.namespace` 解耦。

```yaml
providers:
  openai:                          # key = openai → 隐式 kind = openai
    base_url: https://api.openai.com
    credential: env://OPENAI_API_KEY
    headers:
      X-Custom: value
  anthropic:
    base_url: https://api.anthropic.com
    credential: env://ANTHROPIC_API_KEY
  doubao:                          # 任意名字
    kind: openai                   # ← 显式声明走 OpenAI 协议
    base_url: https://ark.cn-beijing.volces.com/api/v3
    credential: env://DOUBAO_API_KEY
```

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `kind` | 否 | Auth adapter 类型。未设置时,落到 map 里的 key 名(只有那些 key 名等于内置 adapter 名的配法才能省略)。已支持的取值见下。 |
| `base_url` | 是 | 上游基础 URL,客户端路径会拼在后面。 |
| `credential` | 是 | 上游 API key 来源。`env://VAR_NAME` 在启动期读环境变量;任何其他字符串作为字面量直接当 token(YAML 里写明文,适合本地开发,生产建议走 env://)。 |
| `headers` | 否 | 透传给上游的额外固定 header。 |

### 支持的 `kind`

| 值 | 适用上游 | Auth 行为 |
| --- | --- | --- |
| `openai` | OpenAI 官方,以及任何沿用 OpenAI 协议的第三方(豆包、DeepSeek、Together、Groq、Mistral、Azure OpenAI、vLLM、Ollama、LM Studio…) | `Authorization: Bearer <key>` + `api-key: <key>` 双 header |
| `anthropic` | Anthropic | `x-api-key: <key>`,并在 client 未设置时注入 `anthropic-version: 2023-06-01` |

未设置 `kind` 时,系统直接拿 providers map 的 key 名去匹配上表 —— 所以历史配法 `providers.openai: { ... }`、`providers.anthropic: { ... }` 仍然有效。但只要 key 名不是 `openai` / `anthropic` 之一,就**必须**写 `kind`,否则启动期 `validate` 拒绝加载。

### 接入一个新的 OpenAI 兼容供应商

只需要一条配置,**不需要改任何代码**:

```yaml
providers:
  deepseek:
    kind: openai
    base_url: https://api.deepseek.com
    credential: env://DEEPSEEK_API_KEY

  groq:
    kind: openai
    base_url: https://api.groq.com/openai
    credential: env://GROQ_API_KEY

  local-vllm:
    kind: openai
    base_url: http://10.0.0.5:8000
    credential: env://VLLM_TOKEN
```

如果上游需要额外固定 header(比如 Azure OpenAI 的 `api-version` 查询参数走 header,或自定义路由 key),写在 `headers:` 里即可。如果上游使用**完全不同的 auth 协议**(非 OpenAI、非 Anthropic),才需要在 `crates/gateway-core/src/providers/` 加一个新 adapter,详见 [开发指南](./development.md)。

---

## `routes`

每条路由把"哪些请求"绑定到"上游身份和代理策略(缓存、重试、回退)"。按数组顺序匹配,**第一条同时满足以下条件**的命中:

1. URL 中的 `/v1/{namespace}/...` 段等于 `match.namespace`(**必填**,启动时校验);
2. 若 `match.model_prefix` 有值,请求体里的 `model` 字段以该前缀开头。

注意:**`namespace`(URL 段)与 `primary.provider`(`providers` 表里的 key)是两个独立概念**。命名相同是约定,不是要求。让它们不同,就能实现"客户端继续叫 `/v1/openai/...`,但后端偷偷转发给 azure"这类场景。

```yaml
providers:
  openai:                          # 上游身份:实际网络目的地 + 凭证
    base_url: https://api.openai.com
    credential: env://OPENAI_API_KEY
  azure-prod:
    base_url: https://prod.openai.azure.com
    credential: env://AZURE_KEY

routes:
  # 透明迁移:客户端用的还是 /v1/openai/...,网关后端走 azure
  - match:
      namespace: openai
      model_prefix: gpt-
    primary:
      provider: azure-prod         # ← 与 namespace 不同
    cache: { enabled: true, ttl: 3600 }

  # 兜底:其他 openai 请求(o1- 等)继续走真正的 OpenAI,默认无缓存
  - match:
      namespace: openai
    primary:
      provider: openai
```

**没有匹配的路由(或 `routes: []`)时**:网关直接返回 `404`(错误码 `not_found`)。namespace 必须显式登记在 `routes[]` 里才能被代理 —— 最简写法:
```yaml
- match: { namespace: openai }
  primary: { provider: openai }
```
这样 `/v1/openai/...` 就会按默认重试 3 次/500ms 退避、无缓存、无 fallback 的方式转发到 `providers.openai`。启动时如果发现某条 route 缺 `match.namespace`,网关会直接拒绝加载配置。

### `match`

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `namespace` | **必填**(无默认) | URL `/v1/{namespace}/...` 段。**不是 `providers` 表的 key**,只是对外暴露的别名。即使想用"namespace = provider"的一对一配法,也必须显式写出 `match.namespace`,启动时校验不通过会拒绝加载。 |
| `model_prefix` | `None` | 请求体 `model` 字段的字符串前缀。例:`gpt-` 匹配 `gpt-4o-mini`、`gpt-3.5-turbo` 等。若设置了 `model_prefix` 但请求没有 `model` 字段(GET /v1/models 等),视为不匹配。 |

如果同一 namespace 配多套策略,把更具体的放在数组前面,兜底的放后面。

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

缓存 key 来自请求体 + 路径的 blake3 摘要。流式响应**也会被缓存**(replay 时按原 chunk 边界发回),前提是请求体确定性(`temperature == 0` 且 `top_p >= 0.999`)且大小 ≤ 2 MB。

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

成本由启动时通过 `--pricing-catalog`(或环境变量 `GATEWAY_PRICING_CATALOG`)指定的 `pricing-catalog.json` 计算,未知模型按 0 计。默认读取工作目录下的 `pricing-catalog.json`。

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
