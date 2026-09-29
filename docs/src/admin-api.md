# Admin API

所有 Admin 端点挂在 `/admin` 前缀下,统一通过 `Authorization: Bearer <token>` 鉴权。Token 可以是两种之一:

- **Root token** — 启动时从 `GATEWAY_ROOT_TOKEN` 环境变量读取,长期有效,拥有所有权限。运维侧使用。
- **Admin JWT** — 通过 `/admin/auth/login` 用用户名密码换取,默认 TTL 12 小时。UI 登录场景使用。

错误响应统一为:

```json
{ "error": { "code": "unauthorized", "message": "..." } }
```

下文每张表中的"权限":`Any` 表示两种 token 都可用。目前没有更细粒度的角色划分。

---

## 鉴权 `/admin/auth`

### POST `/admin/auth/login`

用用户名密码换 JWT。仅在 `admin.password_login: true` 时启用。

请求:

```json
{ "username": "alice", "password": "..." }
```

响应:

```json
{
  "token": "<jwt>",
  "username": "alice",
  "expires_at": 1715760000
}
```

### GET `/admin/auth/me`

返回当前 token 对应的 principal。

权限:Any。

响应:

```json
{ "principal": "user", "username": "alice", "id": "adm_..." }
```

`principal` 取值为 `root` 或 `user`。

### POST `/admin/admins`

创建一个 admin 用户(密码用 Argon2 哈希落库)。

权限:Any。

请求:

```json
{ "username": "alice", "password": "strong-passphrase" }
```

字段约束:`username` 非空,`password` ≥ 6 位。不满足返回 `400 bad_request`。

响应:

```json
{ "id": "adm_...", "username": "alice" }
```

### GET `/admin/admins`

列出所有 admin。

权限:Any。

响应:

```json
[
  {
    "id": "adm_...",
    "username": "alice",
    "created_at": 1715000000,
    "last_login_at": 1715760000
  }
]
```

---

## API Keys `/admin/keys`

### POST `/admin/keys`

创建一个 Gateway API Key。**明文只在响应里出现一次**,落库的只是 BLAKE3 keyed hash。

权限:Any。

请求:

```json
{
  "name": "my-app-prod",
  "env": "live",
  "project_id": "default",
  "scopes": ["proxy"],
  "expires_at": 1735689600
}
```

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `name` | 是 | 显示用,任意字符串。 |
| `env` | 是 | `live` 或 `test`,**只影响 key 的前缀**(`sk-gw-live-` / `sk-gw-test-`),目前不参与认证、路由、限流、预算等任何逻辑。仅作为肉眼可见的标签,用于区分生产与测试 key(借鉴 Stripe 的命名约定)。 |
| `project_id` | 否 | 默认 `server.default_project_id`。 |
| `scopes` | 否 | 默认 `["proxy"]`。目前 scope 不参与执行,占位。 |
| `expires_at` | 否 | unix 秒。null 表示不过期。 |

响应:

```json
{
  "id": "key_...",
  "name": "my-app-prod",
  "prefix": "sk-gw-live-",
  "last4": "a1b2",
  "secret": "sk-gw-live-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
  "created_at": 1715760000
}
```

### GET `/admin/keys`

列出 Key(不返回明文)。

权限:Any。

查询参数:`project_id`(可选,按项目过滤)。

响应:

```json
[
  {
    "id": "key_...",
    "project_id": "default",
    "name": "my-app-prod",
    "prefix": "sk-gw-live-",
    "last4": "a1b2",
    "status": "active",
    "created_at": 1715760000,
    "last_used_at": 1715800000,
    "expires_at": null,
    "origin": "admin"
  }
]
```

`status` 取值:`active` / `revoked`。`origin` 取值:`admin`(由本 API 创建)或 `config`(由 `gateway_keys[]` 预置,见 [配置参考 > gateway_keys](./configuration.md#gateway_keys))。

### DELETE `/admin/keys/:id`

撤销 Key(软删,`status` 变为 `revoked`)。`origin` 为 `config` 的 key 不能从这里撤销 —— 接口会返回 `400 bad_request`,需要在 YAML 里把这条记录删掉再 reload。

权限:Any。

响应:

```json
{ "id": "key_...", "status": "revoked" }
```

---

## 请求日志 `/admin/logs`

### GET `/admin/logs`

分页检索请求日志。

权限:Any。

查询参数(都可选):

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `project_id` | string | 按项目过滤。 |
| `gateway_key_id` | string | 按 Key 过滤。 |
| `namespace` | string | 按 URL namespace 过滤(对应 `/v1/{namespace}/...` 段)。 |
| `model` | string | 按模型名过滤(精确)。 |
| `status` | string | `success` / `upstream_error` / `gateway_error` / `timeout` / `cancelled`。 |
| `from` / `to` | int | unix 毫秒,时间范围。 |
| `limit` | int | 默认 50,上限 200。 |
| `cursor` | string | 上一页响应里的 `next_cursor`。 |

响应:

```json
{
  "items": [ { "id": "log_...", "...": "..." } ],
  "next_cursor": "..."
}
```

### GET `/admin/logs/:id`

返回单条日志的完整 JSON(包含请求/响应 body 摘要,大于 64KB 部分会被截断)。

---

## 配置只读 `/admin/routes`

### GET `/admin/routes`

返回当前生效的 `AppConfig`(脱敏后),用于 UI 展示。配置改动会被自动重载,这里读到的总是最新版本。

权限:Any。

---

## 成本聚合 `/admin/cost`

### GET `/admin/cost`

按维度聚合 token 用量与美元成本。

权限:Any。

查询参数:

| 参数 | 说明 |
| --- | --- |
| `project_id` | 限定项目。 |
| `from` / `to` | 时间范围(unix 毫秒)。 |
| `group_by` | `namespace` / `model` / `gateway_key` / `day` / `hour` 之一,或逗号分隔的组合。`key` 是 `gateway_key` 的别名。 |

响应:

```json
{
  "groups": [
    {
      "key": { "namespace": "openai", "model": "gpt-4o-mini" },
      "requests": 42,
      "prompt_tokens": 12345,
      "completion_tokens": 6789,
      "cost_usd": 0.12,
      "cached_savings_usd": 0.03
    }
  ],
  "total_cost_usd": 0.12
}
```

---

## 模型价格 `/admin/pricing`

模型价格用于按请求日志中的 provider、model 和 token 用量计算成本。启动时先从 OpenRouter Models API 加载基础价格,按完整 `canonical_slug` 或其最后一段匹配模型名,再加载 `pricing-catalog.json` 覆盖文件。Admin 设置的 override 会覆盖同名 `(provider, model)` 条目并立即生效。`provider: "*"` 表示适用于所有上游的通用价格。

### GET `/admin/pricing`

返回当前生效的模型价格列表。

权限:Any。

响应:

```json
[
  {
    "provider": "openai",
    "model": "gpt-4o-mini",
    "input_per_1k": 0.00015,
    "output_per_1k": 0.0006,
    "cached_input_per_1k": 0.000075,
    "source": "catalog"
  }
]
```

`source` 取值:

- `openrouter` — 来自 OpenRouter API,以完整 canonical slug 展示,provider 为 `*`。
- `catalog` — 来自 `pricing-catalog.json`。
- `override` — 来自 Admin 存储,会覆盖 catalog 中相同 `(provider, model)` 的价格。

### PUT `/admin/pricing`

新增或更新一个模型价格 override。

权限:Any。

请求:

```json
{
  "provider": "openai",
  "model": "gpt-4o-mini",
  "input_per_1k": 0.00015,
  "output_per_1k": 0.0006,
  "cached_input_per_1k": 0.000075
}
```

字段约束:`provider` 和 `model` 非空;价格必须是大于等于 0 的有限数字。`cached_input_per_1k` 可省略,省略时缓存输入按 `input_per_1k` 计价。

响应:

```json
{ "ok": true }
```

### DELETE `/admin/pricing/:provider/:model`

删除一个模型价格 override。删除后如果 catalog 中存在同名条目,会回退到文件或 OpenRouter 基础价格;否则该模型没有可用价格,后续请求仍会记录 token,但无法计算成本。

权限:Any。

响应:

```json
{ "ok": true }
```

### POST `/admin/pricing/recompute`

用当前生效的模型价格重新计算历史请求日志里的 `cost_usd` 和 `would_have_cost_usd`。

权限:Any。

请求:

```json
{
  "from": 1715760000000,
  "to": 1715846400000
}
```

`from` / `to` 都是可选的 unix 毫秒时间戳;都省略时会重算全部历史。这个操作会覆盖已落库的历史成本字段,执行前建议先限定时间范围。

响应:

```json
{
  "updated_rows": 128,
  "models_matched": 3,
  "models_without_price": ["unknown-provider/unknown-model"]
}
```

---

## 预算 `/admin/budgets`

### GET `/admin/budgets`

返回当前所有预算的实时用量。

权限:Any。

响应:

```json
[
  {
    "budget_id": "monthly-team",
    "name": "monthly-team",
    "period": "monthly",
    "period_start": 1714521600000,
    "amount_usd": 500.0,
    "used_usd": 87.3,
    "pct": 0.1746,
    "blocked": false
  }
]
```

`blocked: true` 表示已跨过 `action: block` 阈值,后续请求会被拒(HTTP 402)。

---

## 调用示例

用脚本批量创建 Key:

```sh
for i in 1 2 3; do
  curl -s -X POST http://localhost:8080/admin/keys \
    -H "Authorization: Bearer $GATEWAY_ROOT_TOKEN" \
    -H "Content-Type: application/json" \
    -d "{\"name\":\"bot-$i\",\"env\":\"live\"}" \
    | jq '{name, secret}'
done
```
