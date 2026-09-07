# 推送 Foundation W12 通用 Transport Authority Adapter 设计

**状态：** 边界冻结，待 tracer-bullet TDD；保持零生产接线。

**决策日期：** 2026-09-07

## 1. 目标与验收句

W12 关闭 WBS 的通用 authority seam：把 W05 `Ready(PreparedPush)` / W07 已持久化 Ready intent 安全投影到现有 `DurableDeliveryCoordinator`，并把 coordinator 的精确终态重新查询为 W09 `TerminalAuthorityPort`。验收句为：

> 逐 required channel 记录 typed 结果；Partial 拒绝游标，强 receipt 保留 exact bytes。

本切片依赖 W02、W05、W09，并复用已经落地的 W07--W11 安全边界。它不做 P01/N02 专用 conformance（W13），不接 monitor composition root，不改任何 Unit physical owner，不启动 scheduler，不迁移生产数据库。

## 2. 现有代码事实与不可绕过的缺口

现有 generic coordinator 已具备：

- 十四态 durable state machine；
- `DeliveryEnvelope` 的首次 exact rendered bytes；
- exactly-one `AuthoritativeSinkPort`、attempt lease/fence；
- Accepted/Rejected/Uncertain typed sink result；
- result/disposition/delivery audit exact canonical bytes、SHA 和 immutable append；
- restart reconciliation 与强终态。

但不能直接声称它已经实现 W12：

1. W05 `DecisionId` 固定为 `PreparedPushDecision/v1(IntentId)`；历史 `DeliveryEnvelope.decision_identity` 则由 kind、policy、scope、occurrence、source、subject 和 rendered hash 派生。两个 64 字符十六进制字符串语义不同，碰巧同形不等于同一 authority ID。
2. 历史 envelope 没有保存 Foundation 的 namespace、intent、Unit、audience 和 template version；W09 无法从它构造 17 项 `TerminalBinding/v1`。
3. `decision_state()` 只返回枚举。仅凭 `Delivered` 不能证明 current disposition、attempt、typed receipt、fence、delivery audit 与 immutable append 已精确联结。
4. coordinator 每个 decision 明确要求一个 authoritative sink。不能把多个 channel 的部分成功压缩成一个 Accepted，也不能把 COMPAT bool 升级为 receipt。

因此 W12 必须增加一个兼容的精确绑定和只读终态投影，而不是写一个只看 state 的薄包装器。

## 3. 兼容 envelope 绑定

在 `durable_delivery::DeliveryEnvelope` 增加私有、可选、`serde(default, skip_serializing_if = "Option::is_none")` 的 `FoundationDeliveryBinding`：

- schema version；
- Foundation namespace；
- application decision ID；
- intent ID；
- Unit ID；
- occurrence ID；
- business date；
- subject；
- delivery subject hash；
- audience；
- template ID / version；
- rendered SHA；
- source evidence fingerprint；
- required channel。

旧 constructor 永远写 `None`。因为 `None` 不序列化，既有 envelope canonical bytes、decision identity、数据库行和全部生产 caller 的行为逐字节不变。

只有 W12 adapter 能创建新绑定。`with_foundation_binding` 在写库前检查：

- application decision ID 等于 envelope `decision_identity`；
- business date、occurrence、delivery subject hash、rendered SHA 和 source fingerprint 与 envelope 相等；
- 新 envelope 下发 foundation binding 的注册 template ID，旧 envelope 仍下发 `PushKind::stable_template_id()`；
- required channel 是单个受校验非空 ID；
- 绑定 canonical bytes/SHA 可重建且字段集精确。

绑定存在时，envelope 使用 application decision ID；绑定不存在时继续执行历史 identity 算法。不能从调用方自由字符串覆盖旧 envelope 的 decision identity。

## 4. 一个 decision 与 required channel

现有 coordinator 的安全合同是一个 decision 恰好一个 authoritative sink。W12 保持这个不变量：一个 Foundation authoritative child intent 必须绑定一个 required channel，sink descriptor 与该 channel 在发送前匹配，Accepted receipt 中的 channel 在落库前再次匹配。

多个 required channel 不能共享一个 transport decision：上层必须为每个 channel 建立独立、可审计的 child intent/decision，并用 `RequiredChannelResults` 按有序且唯一的 required-channel 集合归集结果。归集规则：

- 每个 required channel 恰好一个 typed result；缺失、重复或额外 channel 均为合同错误；
- 全部是经 W09 校验的 Accepted/AlreadyTerminal-Accepted，才产生 `AllRequiredAccepted` 提案；
- 至少一项接受、至少一项非接受为 `PartialRequiredChannels`，completion eligibility 固定 `Never`；
- 任一 Uncertain 优先进入 `UncertainRequiredChannels`，不能被其他 Accepted 抵消；
- Rejected 仍只表示观察到拒绝，重试需独立显式授权；
- COMPAT `BestEffortAccepted`/`PartiallyAccepted` 永不进入强归集。

W12 只提供归集合同，不直接推进 parent cursor。具体 Unit 的 parent/child owner 和 finalizer 绑定在后续垂直迁移中实现。

## 5. 通用 adapter 的发送路径

`GenericTransportAuthorityAdapter` 是深模块，外部只看到一个受控调用：

```text
claimed AwaitingAuthority business intent
    -> validate exact lease/state/template/route/channel
    -> build foundation-bound DeliveryEnvelope from stored first bytes
    -> coordinator.prepare(exactly one sink)
    -> coordinator.resume_deliverable(channel-bound sink)
    -> coordinator.reconcile_all_pending(immutable append)
    -> exact terminal requery through W09
    -> DeliveryResult
```

关键规则：

- 只消费业务库已经持久化的首次 rendered bytes 和 PreparedPush snapshot；不接受 renderer，不调用 provider/LLM；
- `PendingDispatch` 未经 claim、外部 owner lease、过期 fence、template/route/channel mismatch 在 sink 前拒绝；
- coordinator 返回已有终态时 sink call 为零，随后重新查询原 exact bytes；
- `Reserved` 只有显式 dispatch 调用才可开始首次 attempt；W11 startup reconciler 没有该接口；
- `AttemptInFlight` 不会因进程重启盲重发；先由 durable reconciliation 变成可解释终态；
- sink 返回的 Accepted channel 若与 required channel 不同，包装器把它记录为 typed Uncertain，并把原 receipt canonical bytes 作为证据；不能把错渠道当成功；
- adapter 不直接写 business Completed，也不推进任何游标；W10 finalizer 是唯一业务完成入口。

## 6. 只读 generic terminal authority

`DurableDeliveryCoordinator::inspect_foundation_terminal(decision_id)` 返回私有只读投影：

- `Missing`：不存在该 decision；
- `PendingSeal`：十四态中任一非强终态，或 disposition/audit 仍未 sealed；
- `Terminal`：只有 `Delivered`、`RejectedDurable`、`UncertainManualReview`、`ManualResolvedRejected` 且完整 join 校验成功。

Terminal 投影包含 foundation binding、current disposition identity、validated attempt binding、terminal disposition、exact evidence bytes、evidence SHA 和 durable schema version。读取时必须重跑：

- envelope canonical SHA 和 foundation binding；
- current disposition canonical SHA、exact field set、immutable append ref；
- attempt/fence/current result 唯一联结；
- Accepted receipt typed columns、canonical result 和 delivery audit；
- Rejected/Uncertain typed evidence canonical 和 result columns；
- pre-attempt rejection 的 denial/disposition 精确绑定；
- manual disposition 的认证 evidence/audit 精确绑定。

`GenericTerminalAuthorityAdapter` 再把投影转换为 W09 私有 `AuthorityTerminalRecord`，固定 `AuthorityClass::GenericCounted`，并在返回前计算 `TerminalBinding/v1`。W09 仍会对业务 intent、policy、template、evidence hash 和 binding SHA 做第二层验证；W10 commit 前仍会再次查询。

## 7. 错误与隐私

- durable 缺失和 pending 是正常查询结果；SQLite、canonical、join、binding 失败是 typed adapter failure；
- 错误只暴露稳定 check/category，不携带 rendered bytes、receipt、投资组合内容、SQL、数据库路径或完整 business snapshot；
- terminal query/result 的 Debug 不输出 exact evidence bytes，只输出长度和 SHA；
- production 实现禁止 `unwrap`、`expect` 和 `panic`；测试 fault seam 仍限定 `cfg(test)`；
- adapter 不暴露 rusqlite connection、raw SQL、sink callback 或任意 authority record constructor。

## 8. TDD 验收矩阵

1. legacy envelope `None` 绑定时 canonical bytes 与 decision identity golden 完全不变。
2. Foundation binding 的全部字段、canonical SHA 和 application decision ID 精确；任一跨字段漂移在 prepare 前失败。
3. claimed intent -> one sink -> Accepted -> reconcile -> W09 TransportAccepted；exact result bytes 的 SHA 等于 terminal evidence SHA。
4. 同 decision 重放 sink call 为零并返回同一 terminal ref/binding/evidence SHA。
5. Pending audit/task transition 只返回 PendingSeal，不能从中构造强结果。
6. Rejected attempt 和 validated pre-attempt rejection 分别保留 Attempt/无 Attempt 语义，均不推进游标。
7. Uncertain 形成强 Uncertain 引用并要求人工隔离，显式证明无 blind resend。
8. manual accepted/not-delivered 保留独立 disposition，不伪装成 TransportAccepted。
9. receipt channel 与 required channel 不同被 durable 记录为 Uncertain，原 receipt exact bytes 可核验且无 Accepted。
10. required-channel 归集拒绝缺失、重复、额外 channel；Partial/Uncertain completion eligibility 为 Never。
11. durable envelope、result、disposition、audit、attempt/fence 任一篡改均查询失败，不降级为 Missing 或 Accepted。
12. adapter 代码无 provider/renderer/LLM/business finalizer 写入口；production wiring relative diff 为零。

## 9. 非目标与后续

- W13：P01/N02 专用 authority 通过同一应用合同的 conformance adapter；
- W14--W17：scheduler/readiness/activation/shadow/cutover；
- W18：operator 身份、审计 capability 与人工处置控制面；
- 各 Migration Unit：route/channel、parent/child owner、真实 sink 与业务 cursor 的实际接线和晋级；
- 真实生产 DB migration、灰盒/自然 occurrence 和 physical-owner 切换仍需独立门禁。

W12 完成只表示“通用 coordinator 可以被 Foundation 精确调用和重查”，不表示任何现有推送已经切到新路径。
