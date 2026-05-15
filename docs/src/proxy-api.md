# 代理 API

应用方调网关的接口。形态上是 **完全透传** —— 网关只在中间做认证、限流、改写凭证、记录日志、按需缓存/回退,**不做任何上游 API 的封装或参数翻译**。

## 路径形态

```
/v1/{provider}/{...upstream_path}
```

- `{provider}` 必须在 `providers` 配置中存在(如 `openai`、`anthropic`)。
- `{...upstream_path}` 原样拼到 `providers.{name}.base_url` 后。

例如:

| 客户端调用 | 网关转发到 |
| --- | --- |
| `POST /v1/openai/v1/chat/completions` | `POST https://api.openai.com/v1/chat/completions` |
| `POST /v1/anthropic/v1/messages` | `POST https://api.anthropic.com/v1/messages` |
| `GET  /v1/openai/v1/models` | `GET https://api.openai.com/v1/models` |

## 认证

请求头携带 Gateway API Key:

```
Authorization: Bearer sk-gw-live-xxxxxxxxxxxx
```

网关会:

1. 用 BLAKE3-keyed-hash 算出 hash,常时间对比数据库里的 hash。
2. 检查 Key 状态(`active` / `revoked`)和 `expires_at`。
3. 异步更新 `last_used_at`。

不会把这个 header 透传给上游 —— 网关会把它换成 `providers.{name}.credential_ref` 解出的真实上游凭证。

## 请求体

直接按上游格式写,网关不做改写。但有两点会被网关读取:

- `model` 字段 — 用来做 `routes[].match.model_prefix` 匹配,以及计入成本日志。
- 整个 body 的 **blake3 摘要** — 用作响应缓存的 key(若该路由 `cache.enabled: true`)。

## 流式响应

如果请求带 `stream: true`(SSE/chunked),网关会以流式方式回给客户端,**不做整体缓冲**(chunk 边发边转)。

流式响应**会被缓存**(若该路由 `cache.enabled` 且其他条件满足):chunks 在转发的同时被记录,流正常结束后整段写入 KV;命中时用 `Body::from_stream` 按原 chunk 边界 replay,客户端看到的 SSE 序列与首次一致。

流式的 token 用量在最后一帧到达时统计写入日志。

## 头部处理

| 类型 | 行为 |
| --- | --- |
| `Authorization` | 替换为上游真实凭证。 |
| `Host` | 改写到上游主机名。 |
| `User-Agent` | 透传,并写入日志。 |
| `Content-Type` | 透传。 |
| 配置里的 `providers.{name}.headers` | 追加到上游请求(覆盖同名)。 |
| 上游响应 `Content-Length` / `Transfer-Encoding` | 由网关重新计算。 |
| 其他 | 透传。 |

## 路由选择

按数组顺序逐条评估,**第一条同时满足以下三个条件**的路由被选中:

- `primary.provider` 等于 URL 中的 `{provider}`
- 若 `match.path` 有值,完整请求路径(`/v1/{provider}/...`)按规则匹配(末尾 `*` = 前缀匹配,否则精确匹配)
- 若 `match.model_prefix` 有值,请求体 `model` 字段以该前缀开头(没有 `model` 字段视为不匹配)

详见 [配置参考 > routes](./configuration.md#routes)。命中后:

1. 检查 `cache.enabled`,且请求体是确定性的(`temperature == 0` 且 `top_p >= 0.999`,或带 `X-Gateway-Cache-Force` 头),则查缓存,命中则直接返回,响应头加 `X-Gateway-Cache-Status: hit`。
2. 否则按 `primary` 转发,失败重试至 `retry.max_attempts` 次。
3. 仍失败且 `trigger` 命中 → 切到下一个 `fallbacks[]`。
4. 成功响应若满足缓存条件(≤ 2 MB、状态 2xx),写回 KV;流式响应也会缓存,replay 时保留 chunk 边界。
5. 写日志,返回响应。

**没有匹配的路由时**(或 `routes` 为空),走 `primary_only` 链:仅主供应商、默认重试 3 次/500ms 退避、**缓存禁用**、无 fallback。

## 响应头

网关附加的特殊头(其他都透传):

| Header | 含义 |
| --- | --- |
| `X-Gateway-Request-Id` | 本次请求的内部 ID,与 `/admin/logs` 中的 id 对应。 |
| `X-Gateway-Cache-Status` | `hit` / `miss` / `refresh` / `bypass`。 |
| `X-Gateway-Cache-Key` | 命中/写入的缓存 key 前缀(截断到 48 字符,便于排障)。 |
| `X-Gateway-Cache-Age` | 仅 HIT 时出现,缓存条目年龄(秒)。 |
| `X-Provider` | 实际命中的供应商(可能是 fallback)。 |

## 错误码

| HTTP | 场景 |
| --- | --- |
| `401` | 缺少 `Authorization` 或 Key 无效 / 已撤销 / 已过期。 |
| `402` | 命中 `budgets[].thresholds[].action: block`。 |
| `404` | URL 中的 `{provider}` 未配置。 |
| `408` | 请求超时(`server.request_timeout_ms` 触发)。 |
| `429` | 命中 `limits[]` 中的 RPM / TPM / 并发上限。 |
| `502` | 上游所有重试 + fallback 都失败。 |
| `504` | 上游超时,且无可用 fallback。 |

错误响应体格式:

```json
{
  "error": {
    "code": "rate_limited",
    "message": "request exceeded RPM limit"
  }
}
```

## 健康检查 / 指标

| 路径 | 用途 |
| --- | --- |
| `GET /healthz` | 进程存活,总是 `200 OK`。 |
| `GET /readyz` | 数据库 + 上游可达性检查,失败返回 `503`。 |
| `GET /metrics` | Prometheus text 格式指标,详见 [可观测性](./observability.md)。 |

这些路径无需认证。
