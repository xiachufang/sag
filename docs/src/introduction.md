# simple-ai-gateway

一个用 Rust 编写的轻量级 AI API 网关,在你的应用和上游模型供应商(OpenAI、Anthropic)之间提供统一的代理层,内置认证、限流、预算、缓存、重试和回退能力。

## 它能做什么

- **统一入口**:把对 OpenAI、Anthropic 等供应商的调用,集中到一个网关后,统一管控密钥、配额、可观测性。
- **多租户与配额**:按 Gateway API Key / 项目维度做 RPM、TPM、并发、成本预算限制。
- **韧性**:可配重试、退避,以及在主供应商 5xx / 超时时自动切换到 fallback 供应商。
- **缓存**:对相同请求体的响应做内存 + 磁盘/Redis 二级缓存。
- **审计**:每个请求都落库,可通过 Admin API 检索。
- **可观测**:Prometheus 指标 + 结构化 JSON 日志 + 可选 OTLP tracing。

## 适合谁用

- 团队/小组在多个项目之间共享 LLM 上游密钥,需要可见性与成本控制。
- 想给应用加一层 **韧性 + 限流 + 审计**,而不引入 Kong/Envoy 这种重型 API Gateway。
- 想要 OSS、可自托管、单二进制 / Docker 部署的方案。

## 不适合谁

- 需要对话级别的 prompt 改写、agent 编排、RAG。这是 LLM 网关,不是 LLM 框架。
- 需要内置每家供应商的 SDK 抽象。本网关做的是 **透传 + 策略**,客户端仍然按上游 API 形态发请求。

## 阅读建议

第一次接触建议按顺序读:

1. [快速开始](./getting-started.md) — 5 分钟跑起来。
2. [代理 API](./proxy-api.md) — 客户端怎么调网关。
3. [配置参考](./configuration.md) — 所有 YAML 字段。
4. [部署指南](./deployment.md) — Lite vs Standard,生产注意事项。

需要排查问题或对接监控时,看 [可观测性](./observability.md) 和 [Admin API](./admin-api.md)。
