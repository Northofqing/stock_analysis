# 推送 Foundation W12 通用 Transport Authority Adapter 实施计划

> 按 W12 设计以小提交执行 RED -> GREEN；保留历史 durable envelope 字节和十四态，不接生产 caller。

## Task 1：冻结设计与基线

- 提交 W12 设计与本计划。
- 记录 W11 HEAD、Foundation 68/68、push_job 52/52、durable 定向基线和 production-wiring 边界。

提交：`docs: design W12 generic transport adapter`

## Task 2：envelope binding RED/GREEN

- 先写 legacy canonical/identity 不变 golden。
- 写 Foundation binding 全字段和逐字段漂移测试。
- 在 `DeliveryEnvelope` 增加 skip-None 私有 binding；实现受控 foundation constructor/validation。

提交：

- `test: specify W12 foundation envelope binding`
- `feat: bind W12 decisions to durable envelopes`

## Task 3：terminal read model RED/GREEN

- 先写 Missing/Pending/Accepted/Rejected/Uncertain/manual 和 corrupt join 测试。
- 在 coordinator 内实现只读 `inspect_foundation_terminal`；复用并补齐 exact validation，返回脱敏 typed projection。
- 不暴露 connection/raw SQL/evidence Debug。

提交：

- `test: specify W12 generic terminal read model`
- `feat: expose validated W12 terminal authority`

## Task 4：Foundation adapter RED/GREEN

- 先写 W09 port mapping、两次 query、exact binding/evidence、state projection 测试。
- 实现 `GenericTerminalAuthorityAdapter` 和 `GenericTransportAuthorityAdapter`。
- 增加 required-channel sink wrapper：descriptor 前置校验，receipt channel 后置校验；错 channel 记为 Uncertain。

提交：

- `test: specify W12 generic transport adapter`
- `feat: adapt generic transport authority to W09`

## Task 5：required-channel 归集 RED/GREEN

- 覆盖 ordered unique、缺失/重复/额外、all accepted、partial、rejected、uncertain、COMPAT 禁入。
- 实现纯 `RequiredChannelResults`，Partial/Uncertain completion eligibility 固定 Never；不写 cursor/finalizer。

提交：

- `test: specify W12 required channel outcomes`
- `feat: classify W12 required channel outcomes`

## Task 6：双轴 review 与修复

- Standards：深模块接口、重复类型、bytes/Debug 泄露、public authority constructor、panic、旧 envelope 兼容。
- Spec：WBS 验收句、W02/W05/W09 绑定、十四态完整、强/弱 authority、no blind resend、零生产接线。
- 先用反例测试暴露问题，再修复。

提交：`fix: align W12 transport adapter with review`

## Task 7：fresh 门禁与中文结果

- W12 定向测试、Foundation、push_job、durable adjacent、rustdoc、cargo check、定向 rustfmt。
- strict/nonfatal Clippy 如实区分目标与既有基线。
- architecture docs 五组验证器、diff check、legacy golden、production-wiring relative diff。
- 写入 `docs/push-system/implementation-w12-results-2026-09-07.md`，更新 `.planning` 后提交。

提交：`docs: record W12 implementation evidence`
