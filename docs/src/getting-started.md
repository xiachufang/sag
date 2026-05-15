# 快速开始

5 分钟在本机起一个 Lite 模式网关,调通一次 OpenAI 请求。

## 前置条件

- Docker / Docker Compose, 或者 Rust 1.85+。
- 至少一个上游 API Key(OpenAI 或 Anthropic)。

## 1. 准备环境变量

```sh
cp .env.example .env
```

打开 `.env`,生成并填入两个必需 secret:

```sh
# 生成 admin root token
openssl rand -hex 32

# 生成加密用 master key
openssl rand -base64 32
```

填入 `.env`:

```
GATEWAY_ROOT_TOKEN=<上面 hex 输出>
GATEWAY_MASTER_KEY=<上面 base64 输出>
OPENAI_API_KEY=sk-...
```

> ⚠️ `GATEWAY_MASTER_KEY` 用于加密落库的供应商凭证。**丢了就无法解密历史数据**,务必备份。

## 2. 启动网关

### 方式 A:Docker Compose (Lite)

```sh
docker compose -f docker-compose.lite.yml up --build
```

数据持久化到本地 `./data/gateway.db`(SQLite)。

### 方式 B:Docker Compose (Standard)

会同时起 Postgres 和 Redis:

```sh
docker compose up --build
```

### 方式 C:本地 cargo

```sh
cargo run --release --bin gateway -- --config config/example.lite.yaml
```

启动后日志里能看到:

```
{"level":"INFO","fields":{"message":"gateway listening","addr":"0.0.0.0:8080","profile":"lite"}}
```

## 3. 创建一个 Gateway API Key

用 root token 调 Admin API:

```sh
curl -X POST http://localhost:8080/admin/keys \
  -H "Authorization: Bearer $GATEWAY_ROOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"my-app","env":"live","scopes":["proxy"]}'
```

响应里的 `secret` 字段(形如 `sk-gw-live-xxxxx`)只显示这一次,务必保存。

## 4. 调一次模型

把 Gateway Key 当成上游 API Key 用,路径前加 `/v1/{provider}`:

```sh
curl http://localhost:8080/v1/openai/v1/chat/completions \
  -H "Authorization: Bearer sk-gw-live-xxxxx" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role":"user","content":"hello"}]
  }'
```

网关会:

1. 用 Argon2/BLAKE3 哈希验证 Gateway Key。
2. 检查限流、预算。
3. 用 `OPENAI_API_KEY` 转发到 `https://api.openai.com/v1/chat/completions`。
4. 计算成本、落库一条请求日志,更新指标。
5. 把上游响应原样返回。

## 5. 创建一个 Admin 账号(可选)

如果想用账号密码而不是 root token 登录 UI,先创建一个 admin。`example.lite.yaml` 默认已经开启 `admin.password_login: true`。

```sh
curl -X POST http://localhost:8080/admin/admins \
  -H "Authorization: Bearer $GATEWAY_ROOT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"admin123"}'
```

> 约束:`username` 非空,`password` ≥ 6 位。

成功返回 `{"id":"...","username":"admin"}`。验证登录:

```sh
curl -X POST http://localhost:8080/admin/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"admin123"}'
```

响应里的 `token` 字段是 12 小时有效的 JWT,后续所有 `/admin/*` 请求都可以用它替代 root token。

## 6. 看一眼内置 UI

浏览器打开 [http://localhost:8080/ui/](http://localhost:8080/ui/),用刚才创建的账号密码登录(或粘贴 root token),可以看到 Key、日志、Cost 聚合。

## 接下来

- 想搞清楚每个 YAML 字段:[配置参考](./configuration.md)
- 想用脚本管理 Key、查日志:[Admin API](./admin-api.md)
- 想上生产:[部署指南](./deployment.md)
