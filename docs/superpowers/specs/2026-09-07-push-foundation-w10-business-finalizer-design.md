# 推送 Foundation W10 通用 Finalizer 与业务 CAS 设计

**状态：** 已冻结设计，待 TDD 实现、双轴评审与 fresh 门禁。W10 只实现 foundation business intent 的终态编排、CAS、append、冲突隔离和提交确认恢复；不接生产 authority、不迁移任何 Unit 的领域通知游标。

**决策日期：** 2026-09-07

## 1. 目标与证据边界

W10 落实 WBS 的验收句：

> CAS 冲突进入 ResolutionRequired；Accepted 不可撤销，重复 finalize 只推进一次。

权威输入为：

- RFC“业务意图转换”“权威处置与最终化资格”“跨库恢复顺序”“最终化事务与恢复边界”；
- W08 已 attested 的 business intent、同库 CAS + append 和稳定事件链；
- W09 首次强终态验证与 finalize 前二次重查 capability；
- W03 注册完成策略和 `CompletionDirective` 的 schedule/cursor/retry/manual 正交提案；
- 冻结 `push-system-foundation.v1.sql` 的八态业务状态机及 terminal transition 字段。

本切片不实现 W11 reconciler/lease takeover，不实现 W12 concrete durable adapter，不实现 W18 operator authentication/control plane，也不接 monitor 或任何生产数据库。

## 2. 模块与职责

新增私有模块 `src/push_foundation/business_finalizer.rs`，与 W08/W09 的关系为：

```text
BusinessIntentStore (W08, attested business DB)
        +
TerminalAuthorityPort (W09, exact decision requery)
        +
CompletionPolicy (W03, registered owner/authority/proposal)
        |
        v
BusinessFinalizer
  ├─ accepted qualification: AwaitingAuthority/authorized ResolutionRequired -> AwaitingFinalizer
  ├─ accepted finalization:  AwaitingFinalizer -> Completed
  ├─ manual not-delivered:   AwaitingAuthority/eligible ResolutionRequired -> NotDelivered
  └─ real CAS conflict:      reread winner -> ResolutionRequired
```

`BusinessIntentStore` 继续独占 rusqlite connection、事务、SQL 和完整链校验。finalizer 只能提交 crate-private、带 W09 capability 的受控命令；不能取得 raw connection 或构造任意 terminal columns。

W10 的“业务 CAS”专指 business DB 中 foundation intent 与其 transition 事件的原子提交。各 Unit 的 `last_notified_snapshot`、归因通知状态、board_notified 等领域 owner 事实必须在后续逐 Unit 切片中与同库 finalizer 绑定；W10 没有这些具体 schema，因此不得把 generic `Completed` 冒充某 Unit 已完成 cursor migration。

## 3. 两阶段接受完成

接受路径必须分成两个本地事务，中间状态用于崩溃恢复：

1. 从当前 attested snapshot 以 W09 `verify_terminal` 第一次查询 authority；
2. 只接受 `Accepted` 或 `ManualConfirmedAccepted`；
3. 用原 state/version/lease fence 执行 `AwaitingAuthority -> AwaitingFinalizer`；若起点是 `ResolutionRequired`，还必须消费 opaque `VerifiedResolutionClearance`，证明已认证处置明确清除了当前冲突；reason 均为 `intent.authority_verified`，同库 append 非终态事件；
4. 返回不可 Clone 的 `PendingAcceptedFinalization`，保存 prior 强引用、intent、qualified version 和 exact lease fence；
5. commit 阶段重读当前 snapshot，再以 W09 `reverify_for_finalization` 第二次查询并比较 prior/fresh 稳定绑定；
6. 重新用 W03 policy 计算接受完成提案；
7. 用 fresh capability 和 exact state/version/lease fence 执行 `AwaitingFinalizer -> Completed`，reason=`finalizer.completed`；
8. 同一 business transaction 释放 lease、append terminal event，全部成功才 commit。

如果调用从已有 `AwaitingFinalizer` 恢复，prepare 只进行第一次 fresh 查询并形成 pending capability，不额外追加同态事件；随后 commit 仍须第二次查询。若已是完整 `Completed`，读取并验证现有 terminal event 后返回 `AlreadyFinalized`，不查询 authority、不追加事件、不重复完成副作用。

## 4. 不投递终态

`ManualConfirmedNotDelivered` 不是接受完成，不能进入 `AwaitingFinalizer -> Completed`。通用 finalizer 只接受同时具备以下 capability 的命令：

- W09 首次验证和 finalize 前二次重查均为同一 `ManualConfirmedNotDelivered`；
- opaque `VerifiedOperatorAuditRef`，包含非空 audit ref 和独立 audit SHA；其 production 构造留给 W18；
- 当前 state 为 `AwaitingAuthority`，或其最近一次进入 `ResolutionRequired` 的来源是 `AwaitingAuthority + transport.uncertain`；
- 历史中从未进入 `AwaitingFinalizer` 或 `Completed`；
- intent decision 与 terminal decision 精确相同；
- exact state/version/lease fence CAS 成功。

成功只写 `NotDelivered` terminal event，reason=`operator.not_delivered`，保存 terminal ref/binding、原 decision 和 operator audit ref/SHA，释放 lease；不关闭 schedule、不推进 notification cursor、不授权重发、不计入 accepted 成功。

W10 定义 operator audit 的不透明消费边界，但不认证用户、不签发生产 audit；这些属于 W18。

同理，`ResolutionRequired -> AwaitingFinalizer` 不能因为新的 Accepted 查询结果就自动解封。W10 只定义并消费不可伪造的 `VerifiedResolutionClearance`；其 production 构造由 W18 的 inspect/resolve 权限、当前冲突引用和审计证据共同签发。W10 测试可使用 `cfg(test)` fixture，但普通调用方没有 constructor。

## 5. Lease fence

`FinalizerFence` 固定包含 `owner`、`generation`、`until`。prepare 和 commit 都必须核对：

- snapshot owner 与请求 owner 相同；
- snapshot generation 与请求 generation 相同；
- snapshot until 与请求 until 相同；
- `until > occurred_at`，不能使用过期租约；
- CAS 的 `WHERE` 继续绑定 intent/state/version/generation/owner/until。

资格转换保留 lease；成功终态释放 lease但不增加 generation。W10 不接管、续租或推断 lease 到期后的发送许可；这些属于 W11。

## 6. Terminal event

Completed 事件只允许：

- disposition=`Accepted | ManualConfirmedAccepted`；
- terminal ref 与 `FinalizationTerminalRef` 的 `ref_id` 相同；
- terminal binding SHA 与 fresh W09 binding 相同；
- operator/decision 扩展字段为空。

NotDelivered 事件只允许：

- disposition=`ManualConfirmedNotDelivered`；
- terminal ref/binding 来自 fresh W09 capability；
- terminal decision 等于 intent 原 durable decision；
- operator audit ref/SHA 来自 opaque audit capability。

事件 ID 继续使用 W08 的 `(intent_id, expected_version, result_version)`；canonical SHA 继续覆盖数据库中除自身外的全部字段。terminal evidence 原文和 receipt bytes 不复制进 business DB。

## 7. 重复、确认丢失与 Accepted 不可撤销

重复 finalize 的判定不只看当前状态：

- 同 stable event ID 已存在且 terminal ref、binding、disposition、actor、reason、time 全部相同，返回 `AlreadyCommitted`；
- 当前已是完整 Completed/NotDelivered 且 head terminal event 验真，恢复调用返回该既有事实；
- commit 确认丢失后先查 event + current head + 完整 hash chain，完全匹配才视为成功；
- 不匹配的同 event ID 不是幂等成功，而是真实冲突。

Accepted 不可撤销表示：

- `Completed` 没有到 NotDelivered、AwaitingAuthority、AwaitingFinalizer 或 PendingDispatch 的边；
- terminal Completed event append-only，后续即使隔离到 ResolutionRequired 也保留接受历史和 terminal binding；
- 冲突不得撤回已经推进的领域游标，也不得创建新 intent/decision 重新发送；
- manual not-delivered 的历史门禁拒绝任何已有 AwaitingFinalizer/Completed 的 intent。

## 8. CAS 冲突隔离

主 CAS affected rows 为零时必须先 rollback，随后重读 current intent 与完整 event chain：

1. 若目标 terminal event 已完整提交且与命令相同，返回 `AlreadyCommitted`；
2. 若 current 已是同一合法终态，返回 `AlreadyFinalized`；
3. 否则以重读的 current state/version 再做一次独立 CAS 到 `ResolutionRequired`，reason=`finalizer.cas_conflict`，append 非终态冲突事件；
4. 隔离 CAS 自身若再竞争，不循环盲写，返回带 current snapshot 的 `ConflictUnresolved`，交给 W11/W18；
5. 已经是 ResolutionRequired 时不重复追加相同冲突，只返回现状。

隔离允许 `Completed -> ResolutionRequired`，但不会删除 Completed terminal event、回退游标或形成 NotDelivered，因此保留“外部 Accepted 不可撤销”。

## 9. 失败语义

| 场景 | 结果 | 持久副作用 |
| --- | --- | --- |
| authority missing/pending/unavailable/binding drift | `TerminalInvalid` | 若当前仍为 AwaitingFinalizer，以 exact version 追加 `finalizer.terminal_ref_invalid` 同态事件；不完成 |
| 非接受 disposition 进入接受 finalizer | `DispositionNotCompletable` | 零写 |
| policy 不允许 authority/accepted completion | `PolicyRejected` | 零写 |
| ResolutionRequired 缺少当前冲突的认证 clearance | `ResolutionClearanceRequired` | 零写，不自动解封 |
| lease owner/generation/until 不一致或过期 | `FenceMismatch` | 零写；W11 后续恢复 |
| 主 CAS 真实冲突 | `ResolutionRequired` | 独立重读 CAS + append 成功时恰好一条冲突事件 |
| terminal append/check/commit 失败 | typed storage error | 原事务整体回滚，旧状态/version 不变 |
| commit ack lost | 读取 exact event/current/chain | 已提交则返回原 receipt，不再写 |
| 已完整 Completed 重复调用 | `AlreadyFinalized` | 零写、零 authority query |

## 10. 完成门禁

1. Accepted 从 AwaitingAuthority 经两次 exact authority query、两次业务转换到 Completed；terminal event 精确绑定 fresh ref。
2. ManualConfirmedAccepted 同样 Completed，但 completion directive/处置保持人工接受，不计 transport accepted。
3. Rejected、Uncertain、ManualConfirmedNotDelivered 不能进入接受完成分支。
4. ResolutionRequired 只有消费匹配当前 intent/version 的认证 clearance 才能恢复 AwaitingFinalizer。
5. ManualConfirmedNotDelivered 只在合格来源与独立 operator audit 下进入 NotDelivered，cursor directive 必为 Never。
6. stale state/version/lease 主 CAS 冲突会重读并进入 ResolutionRequired；无 terminal event。
7. 两个相同并发 finalize 只有一个 Completed 事件，loser 返回原事实；不同 terminal material 不冒充幂等。
8. CAS 后、append 后、commit ack lost 故障边界全部验证；前两者整体回滚，后一者精确恢复。
9. Completed 后 public/internal API 均无反向边；冲突隔离保留原 Completed terminal event。
10. W01--W10 foundation/push_job/rustdoc/check/lint/format/architecture docs 门禁 fresh 运行。
11. 相对 W09 的生产 wiring、DDL、Cargo 和配置零改动；原 release monitor 不重启、不替换。

W10 完成后仍不能宣称任何 Unit 已迁移。W11 负责恢复，W12 负责 authority adapter，W18 负责 operator capability，逐 Unit 切片负责具体领域完成 owner 的同库原子事实与生产接线。
