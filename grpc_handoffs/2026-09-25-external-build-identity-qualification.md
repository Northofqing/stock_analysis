# ExternalV1 Health 构建身份资格：上线前需要当前部署对账（2026-09-25）

本地下游已把 `GetHealth.build_identity` 的四字段与仓内公开 `client-bundle/bundle-metadata.json` 的 `deployment_build_identity` 做严格比较：`service_version`、`source_revision`、`contract_sha256`、`binary_sha256`，并拒绝缺身份、`identity_error` 和字段不一致。在线查询、opening 探针及新版控制记录恢复共用此规则；缓存连接再次使用前重读 Health/Capabilities。旧版控制记录保持原审计解释，但不授权新的 ExternalV1 数据请求。

当前仓内 metadata 的 `generated_at_utc` 是 **2026-09-17T01:40:03Z**，`source_revision` 是 `098021444d7b3c0dea4732c1b5a03e8773047cfb`。这只说明本次客户端编译所钉住的期望身份；本轮没有拿到当前生产 ExternalV1 `GetHealth` 响应，因此**不能声称当前服务匹配，也不能据此部署放行**。9 月 24 日 TDX E2103 修复与该构建身份是否同一服务制品尚未对账。

请上游提供当前 ExternalV1 部署的公开 `deployment_build_identity`、对应 descriptor 的原始 `FileDescriptorSet` 摘要口径、二进制 SHA-256 及一次脱敏 `GetHealth` 原文和 `request_id`。若与 9 月 17 日 metadata 不同，请更新公开 bundle/manifest；本地随后重编译，并用同一 `request_id` 核对 Health、Capabilities 与一条只读业务请求。不能从首次 Health 回包自动学习“可信”期望值，也不能把 LocalBridgeV1 的构建身份替代 ExternalV1。

本地源码验证：构建身份边界单元测试 2 项、跨重启 Health→Capabilities 恢复、拒绝回执恢复、提交不确定恢复各 1 项通过；`cargo check --bin grpc_bundle_probe` 通过。以上不是生产服务验收。
