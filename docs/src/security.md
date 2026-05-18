# 安全模型

## Master Key

启动时从环境变量 `GATEWAY_MASTER_KEY` 读入,base64 编码的 32 字节。两处用途:

1. **BLAKE3 keyed-hash 派生 Gateway API Key 的 hash**(数据库里只存 hash)。
2. **HS256 签名 Admin JWT**。

只要 master key 不泄漏,数据库被拷走也无法伪造 token 或反推 Gateway Key 明文。

**风险面**:

- 丢失 master key → 所有 Admin JWT 失效、所有 Gateway Key 无法再被验证(需重建)。
- 泄漏 master key → 攻击者拿到数据库后能伪造 Admin JWT、构造可被验证通过的 Gateway Key。
- 上游供应商凭证(`providers.<x>.credential`)不进数据库,所以这里跟它没关系 —— 那部分依赖的是 YAML / 环境变量本身的保护。

强烈建议从 KMS/Vault 注入,本地不留副本,且不写进任何配置文件或镜像。

目前**不支持轮换**,见 [部署指南 > Master Key 轮换](./deployment.md#master-key-轮换)。

## Gateway API Key

形态:`sk-gw-{live|test}-{32 个随机 alphanumeric 字符}`。

- **生成**(`security/gateway_key.rs:47`):用系统 RNG 产生 32 个字符,拼前缀。
- **落库**:`hash = blake3::keyed_hash(master_key, plaintext_bytes)`,32 字节,只存 hash + 前缀 + last4。
- **验证**(`gateway_key.rs:90`):接收到的明文同样 hash 后,用 `subtle::ConstantTimeEq` 常时间比较。
- **状态**:`active` / `revoked`,撤销是软删,可在 Admin API 看到。
- **来源**(`origin` 列):`admin`(`POST /admin/keys` 创建,可撤销)或 `config`(配置文件 `gateway_keys[]` 预置,**不能从 Admin 撤销**,只能从 YAML 删条目再 reload)。

Key 创建后明文**只在 HTTP 响应里出现一次**,不会再次返回。预置 key 的明文写在 YAML 里就是磁盘留底,落库前同样会被 hash —— 不留原文,但配置文件本身要做好权限管控,或改用 `secret: env://VAR_NAME` 形态从环境变量注入。详见 [配置参考 > gateway_keys](./configuration.md#gateway_keys)。

## Admin 鉴权

两种 principal:

### Root Token

来源由 `admin.root_token` 决定:
- 默认 `env://GATEWAY_ROOT_TOKEN`,从该环境变量读明文;
- 写成 `env://<其他变量名>` 指向别的变量;
- 直接写一个非 `env://` 开头的字符串,会被当成 token 字面量(本地开发方便,生产建议走 env://);
- 留空则关闭 Admin API。

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

## 上游凭证

通过 YAML 中 `providers.<x>.credential` 注入。两种形态:

- `env://VAR_NAME` —— 启动时从该环境变量读取(推荐生产场景,凭证不进 YAML 文件,也不进 git)。
- 任意其他字符串 —— 作为字面量直接用,适合本地开发或一次性试运行。

不再支持 `secret://<id>` 那条经过 Admin API + 数据库加密落库的路径。轮换上游 key 时直接改 env 或 YAML 然后让网关热重载 / 重启即可。

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
