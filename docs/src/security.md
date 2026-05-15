# 安全模型

## Master Key

启动时从环境变量 `GATEWAY_MASTER_KEY` 读入,base64 编码的 32 字节。三处用途:

1. **AES-256-GCM 加密落库的供应商凭证**(`secret://` 引用的目标)。
2. **BLAKE3 keyed-hash 派生 Gateway API Key 的 hash**(数据库里只存 hash)。
3. **HS256 签名 Admin JWT**。

只要 master key 不泄漏,数据库被拷走也无法解出凭证,也无法伪造 token 或 Key。

**风险面**:

- 丢失 master key → 所有 `secret://` 凭证、所有 Admin JWT、所有 Gateway Key 全部作废。
- 泄漏 master key → 攻击者只要拿到数据库,就能解出明文凭证、伪造 token、构造可被验证通过的 Gateway Key。

强烈建议从 KMS/Vault 注入,本地不留副本,且不写进任何配置文件或镜像。

目前**不支持轮换**,见 [部署指南 > Master Key 轮换](./deployment.md#master-key-轮换)。

## Gateway API Key

形态:`sk-gw-{live|test}-{32 个随机 alphanumeric 字符}`。

- **生成**(`security/gateway_key.rs:47`):用系统 RNG 产生 32 个字符,拼前缀。
- **落库**:`hash = blake3::keyed_hash(master_key, plaintext_bytes)`,32 字节,只存 hash + 前缀 + last4。
- **验证**(`gateway_key.rs:90`):接收到的明文同样 hash 后,用 `subtle::ConstantTimeEq` 常时间比较。
- **状态**:`active` / `revoked`,撤销是软删,可在 Admin API 看到。

Key 创建后明文**只在 HTTP 响应里出现一次**,不会再次返回。

## Admin 鉴权

两种 principal:

### Root Token

从 `GATEWAY_ROOT_TOKEN` 环境变量读取的明文字符串(默认变量名,可通过 `admin.root_token_env` 改)。

- 直接放在 `Authorization: Bearer <token>` 中。
- 常时间比较,无过期。
- 任何 Admin 端点都可用。
- 适合运维 / 引导 / 脚本场景。**不要写进客户端代码或 UI。**

清空该环境变量 → Admin API 全部 401。

### Admin JWT

通过 `/admin/auth/login` 用用户名 + 密码换取。

- HS256 签名,签名密钥为 master key。
- Claims:`{ sub: user_id, username, iat, exp }`。
- TTL 默认 **12 小时**(`SESSION_TTL_SECS`)。
- 仅在 `admin.password_login: true` 时启用 `/login` 端点。

密码用 Argon2 默认参数哈希后落库(`security/admin_auth.rs`)。校验失败统一返回 `unauthorized`,不暴露差异(防用户名枚举)。

## 凭证落库加密

`POST /admin/providers/credentials` 提交的 `api_key` 走 AES-256-GCM:

```
blob = nonce(12B) || ciphertext || tag(16B)
```

nonce 每次写入随机生成。读取时(`security/credentials.rs:10-41`)检查 blob 长度 ≥ 12,小于即认为损坏,拒绝解密。

YAML 中的 `credential_ref: secret://<id>` 在**每次请求**时都会解密一次(无缓存),便于通过 Admin API 轮换上游 key 而不需要重启。

## 传输安全

网关进程**不监听 TLS**。生产环境必须放在 TLS 终端后面:

- Caddy / nginx / Envoy
- 云负载均衡(ALB / NLB / GCP LB)
- K8s Ingress(`cert-manager`)

如果直接暴露 8080 端口,Gateway Key 和 Admin token 都会以明文走线。

## 输入校验

- 请求 body 大小:axum 默认上限(`tower-http::limit::RequestBodyLimitLayer` 未额外设置,框架默认 2MB / chunked 无限),建议在反代层加 `client_max_body_size`。
- `model_prefix` 匹配前会先尝试解析 JSON 取 `model` 字段;不是 JSON 或字段不存在则当作"无 model"处理。
- 配置文件 YAML 在加载时跑 `AppConfig::validate`,引用未知 provider 会拒绝启动 / 拒绝热重载。

## 日志中的敏感数据

- 请求 / 响应 body **会写入日志表**(限 64KB,超出截断)。如果 prompt 中包含 PII,请按合规要求决定是否关掉 body 落库 —— 目前没有开关字段,需要修改 `MAX_LOG_BODY_BYTES` 或在反代层做脱敏。
- 上游 API Key、Gateway Key 不会落库或出现在 stdout 日志。
- Master key 不会出现在任何日志。

## 已知边界

| 类别 | 现状 |
| --- | --- |
| 角色 / 权限 | 仅 root vs admin,无更细粒度。 |
| 审计日志 | Admin API 调用本身不落库,只代理请求落库。 |
| Key 轮换 | 没有自动轮换,需要手动撤旧建新。 |
| Master Key 轮换 | 无内置流程。 |
| CSRF | UI 是 SPA,登录后 JWT 用 `Authorization`(非 cookie),无 CSRF 表面。 |
| Rate-limit on /admin | Admin API 不参与 `limits[]`,假定运维不会 DDoS 自己。 |
