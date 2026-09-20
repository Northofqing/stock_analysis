# client-bundle 2026-09-17.1 增量适配

状态：开发中，未部署。本文件只说明本次新版增量，不替代原双合同计划，也不宣称整体推送迁移完成。

当前更新（2026-09-17 15:54，以本段为准）：真实V2数据库9种篡改拒绝已通过；最新安全合批8/13，4项在v11→v12迁移前置阶段失败，原因是兼容reader将旧RawResult V2误当另一种v12-native格式，正在修正分流；1项损坏测试仍假设原数据V1，需以V2→未知V3更新。详[诊断与修复范围](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-v12-migration-compatibility-diagnosis.md)。此前39项回归通过不代表这个跨布局迁移已通过。合法失败writer/reopen与attempts完整合同接线尚待。

当前更新（2026-09-17 13:32）：三路原生查询、异常响应、V1严格读取、V2封闭失败材料以及17个相关旧消费者已合并实测 **39/39通过**，见[当前证据](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-route-codec-matrix-current-evidence.md)。调用绑定/证据材料及真实成功重开的独立复审已通过；新测试复审、真实数据库篡改和失败恢复仍待。下表“本轮14项/已准备”是较早里程碑，不能当当前尚未运行；attempts完整合同接线仍未实现。全部代码仍未部署。

## 已核实的更新

- 当前 [README](../client-bundle/README.md) 标明 `2026-09-17.1`，来源提交 `098021444d7b3c0dea4732c1b5a03e8773047cfb`。在该目录运行 `shasum -a 256 -c manifest.sha256`，7 个公开文件全部通过。此校验证明文件一致性，不证明包来源真实性或线上服务就是该构建。
- R 与开发工作树 W 的 `market.proto` 均为 SHA-256 `2f2037a00250b90bbd30525be2e7e9ad2e64c6dd30defc150642b9f993896a7d`；63 个 RPC 的 proto 没有因本次更新改变。因此不能只重生成代码就认定适配完成。
- [公开接口文档](../client-bundle/grpc-external-api.md) 1125–1130 行的版本说明区分：`FinancialStatements` v2 是 9 月 16 日增量；9 月 17 日主要闭合 `provider_attempts` 语义，并明确部署构建身份及摘要口径。`T0Evidence` v2 早于本次更新，不当作 9 月 17 日新增功能。

## 已定位的客户端缺口

| 项目 | 证据与影响 | 处理状态 |
| --- | --- | --- |
| attempts 整条 trace 校验不足 | 新合同要求 1–16 项、从 1 连续的 ordinal、outcome/reason/布尔组合合法；当前 `provider_attempts.rs:84–116` 只限制超过 16 项，`errors.rs:498–530` 逐字段投影，因此非法组合仍可能取得 `accepted()` | 独立 agent 已审计；准备真实失败回归，再实现闭合解释状态 |
| Provider 的证据范围不够 | 新合同要求与同端点 `GetCapabilities` 身份逐字匹配；当前 decoder 只查本地固定枚举，尚未消费已有历史能力证据 | 已核实完整 Macro readiness episode 保存的 Capabilities 原响应及 endpoint/authority/ready-result 关联足够；待传递到在线与恢复 decoder，无需新增 SQL 或 JSON 字段 |
| 接收响应被重编码 | 原实现保存解码对象的 `encode_to_vec()`，真实反例中793字节变791字节，丢失尾部`5a00` | 已改为有界捕获真实响应payload；零长field11与合法unknown-group真实mTLS回归通过，I-3独立复审关闭；后续三路与异常响应矩阵正在补齐，未部署 |
| 在线与持久恢复必须一起适配 | V1 保持严格canonical snapshot；显式V2绑定External profile、method、descriptor和真实响应材料，不静默改义历史记录 | 本轮14项回归通过，含真实retry→success→同库关闭重开且零新连接/RPC；调用绑定/证据材料独立复审进行中。V1严格与V2失败材料补充测试已准备，真实DB篡改等完整恢复矩阵仍待 |

上述 Rust 路径相对 `W/src/grpc_client/`；`RawResult` 位于 `W/src/push_foundation/intent_store/chain_post_close_macro_codec.rs`。完整逐项证据和最小回归建议见[新版 attempts 专题审计](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-bundle-20260917-attempts-audit.md)。

原始响应反例于02:16:46Z真实终态，编译成功、0 pass/1 fail、无signal，730项列明输入前后及核验时一致；详见[首RED验证证据](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-native-data-first-red-verification.md)。这不是环境或编译失败，不把它误记为修复完成。

最新运行更新（2026-09-17 13:01）：上述14项于04:52:50Z成功终态，范围和限制见[验证记录](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-binding-restore-runtime-verification.md)。历史RED保留，但不再把已通过的首修复描述为“尚未编译”。整个S2及2026-09-17.1适配尚未完成。

边界：当前 attempts 没有直接参与生产重试决策，重试仍读取顶层 `ErrorDetail.retryable`；因此这次发现是解释状态与新版合同不一致，不能夸大为已经发生错误重试。在线与持久恢复共用 decoder，修复必须保持两者一致。超过 16 项整体拒绝、不截断，以及未知原值的脱敏保留，已有实现继续保留。

历史证据的使用边界已在[同 episode capability 审计](../.worktrees/push-reliability-20260905/.superpowers/sdd/2026-09-16-grpc-dual-contract/s2-macro-provider-catalog-evidence-audit.md)逐项核实：必须使用该 data begin 引用的原 Capabilities 响应，不能用当前查询替代历史；正式合同要求同 endpoint，不要求同 TCP。缺证据时保持 unsupported，但已有完整证据的记录不能永久一律 unsupported。篡改或非 canonical 的持久事实仍应拒绝恢复，不能降级为普通 unsupported。审计完成不等于接线已实现。

## 分工与下一步

接口主 agent 负责三项既有业务查询的原生 External 客户端、原始响应与版本化恢复；第二 agent 独立准备新版 attempts 回归和最小修复方案。主线程继续 Macro 生产路径优化验证，统一编译和集成，源码写入窗口互斥。

先验证真实行为反例，再分别完成修复与回归；旧版 S0/S1a/S1b 的验收只按其原范围保留，不自动代表新版全部适配。全程不读取或复制部署凭据，不调用生产 RPC，不改生产数据库，不启停 monitor。整体进度见[实施计划](2026-09-16-client-bundle-implementation-plan.md)。
