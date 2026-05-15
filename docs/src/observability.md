# 可观测性

网关暴露三类信号:Prometheus 指标、结构化日志、按需 OTLP tracing。

## 健康检查

| 路径 | 含义 | 失败行为 |
| --- | --- | --- |
| `GET /healthz` | 进程在跑就返回 200,不查依赖。用于 K8s liveness。 | - |
| `GET /readyz` | 检查 DB / Redis 可达性。用于 K8s readiness。 | 任意依赖不可达返回 503。 |

两个端点无需认证。

## Prometheus 指标

`GET /metrics` 返回 Prometheus text 格式,需要在 YAML 里 `observability.metrics: true`(默认开)。

### 代理与缓存

| 指标 | 类型 | Labels | 含义 |
| --- | --- | --- | --- |
| `gateway_cache_hit_total` | counter | `tier={l1\|l2}` | KV 缓存命中次数(底层 KvStore)。 |
| `gateway_cache_miss_total` | counter | - | KV 缓存未命中。 |
| `gateway_cache_write_total` | counter | - | KV 缓存写入。 |
| `gateway_cache_response_hit_total` | counter | - | 路由级响应缓存命中(用户视角)。 |
| `gateway_cache_response_write_total` | counter | - | 路由级响应缓存写入。 |

### 限流与预算

| 指标 | 类型 | Labels | 含义 |
| --- | --- | --- | --- |
| `gateway_ratelimit_hit_total` | counter | `kind={rpm\|tpm\|concurrency}` | 触发限流的次数。 |
| `gateway_budget_pct` | histogram | `budget=<name>` | 每次请求后预算使用率(0–1)。 |
| `gateway_budget_threshold_total` | counter | `budget=<name>, action={alert\|block}` | 阈值跨越次数。 |

### 日志写入

| 指标 | 类型 | Labels | 含义 |
| --- | --- | --- | --- |
| `gateway_log_write_total` | counter | - | 日志条目成功落库。 |
| `gateway_log_write_error_total` | counter | - | 日志落库失败。 |
| `gateway_log_drop_total` | counter | `reason=full` | 异步队列满,日志被丢弃。 |

### 配置热重载

| 指标 | 类型 | Labels | 含义 |
| --- | --- | --- | --- |
| `gateway_config_reload_total` | counter | - | 配置热重载成功次数。 |
| `gateway_config_reload_error_total` | counter | - | 配置热重载失败次数。**持续 > 0 必报警**。 |

### 推荐告警

```
# 持续限流(可能是上游限速或恶意流量)
rate(gateway_ratelimit_hit_total[5m]) > 1

# 日志写入失败累积
increase(gateway_log_write_error_total[10m]) > 0

# 配置加载失败
increase(gateway_config_reload_error_total[5m]) > 0

# 任何预算跨过 80% 警告线
increase(gateway_budget_threshold_total{action="alert"}[1h]) > 0

# 任何预算跨过 100% 拦截线
increase(gateway_budget_threshold_total{action="block"}[1h]) > 0
```

## 结构化日志

进程 stdout 输出 tracing JSON。字段示意:

```json
{
  "timestamp": "2026-05-15T03:21:11.234Z",
  "level": "INFO",
  "fields": {
    "message": "proxy request completed",
    "request_id": "req_...",
    "project_id": "default",
    "gateway_key_id": "key_...",
    "provider": "openai",
    "model": "gpt-4o-mini",
    "status": 200,
    "input_tokens": 12,
    "output_tokens": 64,
    "cost_usd": 0.00031,
    "elapsed_ms": 842,
    "upstream_ms": 800,
    "queue_ms": 12,
    "cache": "MISS",
    "outcome": "primary"
  }
}
```

- 等级由 `RUST_LOG` 控制(EnvFilter 语法)。默认 `info,sqlx::query=warn`。
- `observability.tracing.format: text` 可改为人类可读格式。
- 不要在生产开 `debug`:Postgres / SQLite 的查询会以 INFO 级吐出。

## 请求日志(数据库)

每条 HTTP 代理请求都会异步写入 `request_logs` 表,字段比 stdout 日志更全(包含 request/response body 摘要)。通过 [Admin API](./admin-api.md#请求日志-adminlogs) 检索。

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `id` | string | `log_xxx`,响应头 `X-Gateway-Request-Id` 返回的就是它。 |
| `project_id` / `gateway_key_id` | string | 归属。 |
| `provider` / `model` / `path` | string | 上游信息。 |
| `status` | string | `ok` / `gateway_error` / `upstream_error`。 |
| `http_status` | int | 实际返回客户端的状态码。 |
| `error` | string? | 错误码字符串(如 `budget_exceeded`、`rate_limited`)。 |
| `input_tokens` / `output_tokens` | int | 上游或估算值。 |
| `cost_usd` | float | 由 `pricing-catalog.json` 计算。 |
| `request_body` / `response_body` | string | 各限 64KB,超出截断。 |
| `cache` | string | `hit` / `miss` / `refresh` / `bypass`。 |
| `outcome` | string | `primary` / `fallback:N` / `error`。 |
| `elapsed_ms` / `upstream_ms` / `queue_ms` | int | 计时拆分。 |

保留期:Lite 模式由 `storage.sqlite.log_retention_days` 控制(默认 30 天),后台定期清理。Standard 模式目前不自动清理,需要在 Postgres 侧用 partition 或定时任务管理。

## OTLP Tracing

```yaml
observability:
  tracing:
    enabled: true
    format: json
    otlp_endpoint: http://otel-collector:4318
```

启用后会同时把 span 推到 OTLP HTTP 端点。span 名:

- `proxy.request` — 顶层,涵盖整个请求。
  - `proxy.attempt` — 每次上游尝试,带 `attempt.index`、`target.provider`、`outcome` 等。
  - `cache.lookup` / `cache.write`。
  - `ratelimit.check` / `budget.check`。

Span 上的 `request_id` attribute 与 `X-Gateway-Request-Id`、Admin API 日志条目的 `id` 相同,便于跨系统对齐。
