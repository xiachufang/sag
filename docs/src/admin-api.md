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
    "expires_at": null
  }
]
```

`status` 取值:`active` / `revoked`。

### DELETE `/admin/keys/:id`

撤销 Key(软删,`status` 变为 `revoked`)。

权限:Any。

响应:

```json
{ "id": "key_...", "status": "revoked" }
```

---

## 上游凭证 `/admin/providers/credentials`

用于把上游 API Key 加密存到数据库,然后在 YAML 里用 `credential_ref: secret://<id>` 引用。

### POST `/admin/providers/credentials`

权限:Any。

请求:

```json
{
  "provider": "openai",
  "name": "team-shared",
  "api_key": "sk-...",
  "project_id": "default"
}
```

`api_key` 用 AES-256-GCM(随机 nonce)加密后落库,明文不再存在。

响应:

```json
{
  "id": "cred_...",
  "project_id": "default",
  "provider": "openai",
  "name": "team-shared",
  "status": "active",
  "created_at": 1715760000
}
```

### GET `/admin/providers/credentials`

列出已存凭证(不返回明文)。可选 `project_id` 过滤。

### DELETE `/admin/providers/credentials/:id`

删除凭证。响应 `{"id":"cred_...","status":"deleted"}`。

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
| `provider` | string | 按供应商过滤。 |
| `model` | string | 按模型名过滤(精确)。 |
| `status` | string | `ok` / `gateway_error` / `upstream_error`。 |
| `from` / `to` | int | unix 秒,时间范围。 |
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
| `from` / `to` | 时间范围(unix 秒)。 |
| `group_by` | `provider` / `model` / `gateway_key_id` / `day` 之一。 |

响应:

```json
{
  "rows": [
    {
      "key": "openai",
      "input_tokens": 12345,
      "output_tokens": 6789,
      "cost_usd": 0.12
    }
  ],
  "total_usd": 0.12
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

注入一个加密凭证然后用 `secret://` 引用:

```sh
CRED_ID=$(curl -s -X POST http://localhost:8080/admin/providers/credentials \
  -H "Authorization: Bearer $GATEWAY_ROOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"provider":"openai","name":"team","api_key":"sk-..."}' \
  | jq -r .id)

# 然后在 YAML 里:
# providers:
#   openai:
#     base_url: https://api.openai.com
#     credential_ref: secret://$CRED_ID
```
