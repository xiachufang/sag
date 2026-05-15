# 部署指南

## 形态选择

| 维度 | Lite | Standard |
| --- | --- | --- |
| 元数据 / 日志 | SQLite | Postgres |
| 计数器 (RPM/TPM/并发) | 进程内内存 | Redis |
| L2 缓存 | SQLite | Redis |
| 副本数 | **1**(进程内计数无法跨副本) | 多副本 |
| 部署难度 | 单容器,挂卷即可 | 三组件 + 健康检查 |
| 数据迁移 | 卷迁移 | `pg_dump` |
| 适用 | 个人 / 小团队 / 单机 | 团队 / 多副本 / 生产 |

切换 profile 需要重启进程,**数据不会自动迁移**;先备份再换。

## Docker

仓库自带两份 compose:

```sh
# Lite,本地 SQLite,数据落 ./data
docker compose -f docker-compose.lite.yml up -d

# Standard,起 Postgres + Redis
docker compose up -d
```

`Dockerfile` 是两阶段构建,基镜像 `debian:bookworm-slim`,运行时只装 `ca-certificates libssl3 sqlite3`,镜像约 50 MB。

容器内默认 `EXPOSE 8080`,`VOLUME /app/data`(Lite 模式下挂这个)。

## 必需环境变量

任何形态都需要:

| 变量 | 说明 |
| --- | --- |
| `GATEWAY_MASTER_KEY` | base64 编码 32 字节。**所有加密凭证依赖它**。生成: `openssl rand -base64 32`。 |
| `GATEWAY_ROOT_TOKEN` | Admin API 初始凭证。生成: `openssl rand -hex 32`。可改名,通过 `admin.root_token_env` 指定。 |

上游凭证按 YAML 中 `credential_ref` 决定:`env://VAR` 引用的变量需要导出;`secret://<id>` 引用的不需要(已加密落库)。

## Kubernetes 提示

没有自带 chart,自行写 Deployment 时注意:

- **副本数**:Lite 模式 `replicas: 1`,且配 `strategy.type: Recreate`(避免两个 pod 同时写一个 SQLite 文件)。Standard 可任意横向扩展。
- **Secret 注入**:`GATEWAY_MASTER_KEY` 和 `GATEWAY_ROOT_TOKEN` 都用 K8s Secret,通过 `envFrom: secretRef`。
- **存储**:Lite 必须挂 PVC 到 `/app/data`;Standard 不需要 PVC。
- **探针**:liveness 用 `/healthz`,readiness 用 `/readyz`。
- **资源**:基线 50m CPU / 128Mi memory,缓存命中场景下大致够用;预留 L1 缓存的 `cache.l1_memory_mb` 用量(默认 256MB)。

## 配置文件管理

配置改动会热重载(`gateway-api/reload.rs` 用 `notify` 监听文件 inode 变化)。

- **Docker**: `-v $(pwd)/config:/app/config:ro`,然后改本地文件即可。
- **K8s**: 用 ConfigMap + `subPath` 注入,改 ConfigMap 后 pod 内文件会异步刷新。建议把 ConfigMap 改动配合 `kubectl rollout restart` 以确保安全。

热重载失效的字段:`server.bind`(已经在监听)、`storage.profile`(stores 已经初始化)。

## Master Key 轮换

目前**没有内置轮换机制**。要换 master key 必须:

1. 用旧 key 启动,通过 Admin API 把 `secret://` 引用导出为明文(或临时改成 `env://`)。
2. 停服,导出 DB。
3. 用新 key 启动空库,重新创建 admin、key、credentials。
4. 把日志库恢复(日志不加密)。

因此生产场景**强烈建议在密钥管理系统(AWS KMS / GCP Secret Manager / Vault)里存 master key**,并把它视为同等级别的根密钥。

## 备份

| 数据 | 形态 | 备份方式 |
| --- | --- | --- |
| Lite SQLite | `./data/gateway.db` | `sqlite3 .backup` 或冷拷贝(停服) |
| Standard Postgres | volume | `pg_dump` |
| Standard Redis | volume(带 AOF) | RDB / AOF 持久化(本仓库 compose 已开 `--appendonly yes`) |

Master key **必须单独备份**,否则上面的备份都还原不出加密凭证。

## 生产 checklist

- [ ] `GATEWAY_MASTER_KEY` 在 KMS 中,本地无明文副本。
- [ ] `GATEWAY_ROOT_TOKEN` 已轮换为强随机值,只给运维使用;日常通过 Admin JWT 操作。
- [ ] 配置文件里的 `admin.password_login` 仅在确实有 UI 用户时打开。
- [ ] `limits[]` 至少有一条 `target.type: key, id: "*"` 兜底,防止单 Key 拖死整个网关。
- [ ] `routes[].cache` 对幂等的只读端点(`/v1/models` 等)开启,对 chat completions **谨慎开启**(prompt 一字不差才命中,但缓存"对话历史"是非预期行为)。
- [ ] Postgres + Redis 都设了密码,且不暴露到公网。
- [ ] Prometheus 抓取 `/metrics`,告警至少覆盖 `gateway_ratelimit_hit_total` 突增和 `gateway_config_reload_error_total > 0`。
- [ ] 日志聚合(Loki / ELK / CloudWatch)接好;`RUST_LOG` 不要长期开 `debug`,JSON 日志量会很大。
- [ ] 在前面挂一层 TLS 终端(Caddy / nginx / cloud LB);网关本身不监听 TLS。
