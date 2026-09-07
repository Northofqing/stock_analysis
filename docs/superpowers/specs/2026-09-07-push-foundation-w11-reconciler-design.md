# 推送 Foundation W11 Reconciler、Lease/Fence 与启动恢复设计

**状态：** 已实现并通过 tracer-bullet TDD、双轴评审与 fresh 门禁；保持零生产接线。

**决策日期：** 2026-09-07

## 1. 目标与范围

W11 落实 WBS 验收句：

> 恢复所有原业务日既存 intent；过期 lease 重取 fence，Uncertain 不得盲重发。

本切片只实现 business intent 侧的启动恢复深模块：

- 对所有业务日的既存非终态 intent 做完整、确定性、可分页扫描；
- 验证每条 snapshot 和 append-only transition chain 后才交给恢复器；
- 对无 lease 或已过期 lease 做 expected state/version/旧 fence 约束下的 generation CAS；
- 对 `AwaitingAuthority`/`AwaitingFinalizer` 只读查询 W09 authority，调用 W10 finalizer 恢复强终态；
- 将强绑定的 Uncertain 隔离为 `ResolutionRequired`；
- 把 Rejected、authority 未封存/不可用、live foreign lease、PendingDispatch 和已有 ResolutionRequired 作为 typed boundary；
- 迭代到本地 fixed point，或在有界迭代后失败关闭。

W11 不实现 concrete durable adapter（W12/W13）、物理发送、Rejected retry 授权（W18）、scheduler/catch-up（W14）、readiness 判级（W15）、activation（W16）或任一 Unit cursor。模块没有 sink/resume/provider 方法，因此“不盲重发”不是调用约定，而是本切片接口能力集合中根本不存在发送权限。

## 2. 模块边界

新增私有 `src/push_foundation/reconciler.rs`：

```text
BusinessIntentStore (W08)
  ├─ scan_recovery_page: 全业务日 keyset scan + snapshot/chain attestation
  ├─ claim_recovery_lease: 无/过期 lease 的同态 generation CAS + append
  └─ apply_recovery_observation: rejected/uncertain/invalid 的 exact-fence CAS + append
                 |
                 v
StartupReconciler (W11)
  ├─ PendingDispatch     -> lease/fence + DispatchPending boundary
  ├─ AwaitingAuthority   -> W09 query -> W10 finalizer / rejected / uncertain / blocked
  ├─ AwaitingFinalizer   -> W09 query -> W10 finalizer / conflict / blocked
  └─ ResolutionRequired  -> ManualResolutionRequired，绝不自动解封
                 |
                 v
RecoveryBindingsPort
  └─ 仅按 attested Unit/owner 提供 template、CompletionPolicy、TerminalAuthorityPort
```

`RecoveryBindingsPort` 是只读依赖解析 seam。它返回的 authority 仍是 W09 私有 `TerminalAuthorityPort`，不能返回弱 audit 或发送函数。W12/W13 后续提供 production binding；W11 测试只使用内存 fake。

## 3. 全日期确定性扫描

`BusinessIntentStore::scan_recovery_page` 只扫描：

- `PendingDispatch`；
- `AwaitingAuthority`；
- `AwaitingFinalizer`；
- `ResolutionRequired`。

明确排除完整终态 `Completed/NotDelivered/NoData/Disabled`。SQL 不接受“当前业务日”参数，也不使用 lookback；按 `(business_date ASC, intent_id ASC)` keyset 分页，所以最早历史日、上一交易日和今日一视同仁。cursor 只能从实际返回的 attested snapshot 派生，普通调用方不能用任意字符串跳过前缀。

每个候选在返回前必须：

1. 经 `query_intent` 重算 intent identity、不可变 payload/render/hash 和状态约束；
2. 经 `query_transition_chain` 校验版本连续、event ID、previous SHA、canonical SHA、当前 head；
3. 确认仍处于 recovery state；
4. 再构造只包含 business date、intent ID 和 snapshot 的恢复候选。

任一条损坏使整页失败；不得跳过坏行后宣称 startup fixed point。

## 4. Lease 与 Fence

需要自动工作的三态为 `PendingDispatch/AwaitingAuthority/AwaitingFinalizer`。处理前：

- `lease_owner=NULL`：用新 owner/until 获取 lease，generation + 1；
- `lease_until <= now`：不论旧 owner 是否相同，按旧 state/version/owner/until/generation CAS 接管，generation + 1；
- 同 owner 且 `lease_until > now`：复用当前 exact fence，不续租、不增加 generation；
- 其他 owner 且未过期：返回 `LiveForeignLease`，不写业务库、不查 authority；
- `ResolutionRequired`：不自动获取 lease，保留给 W18 inspect/resolve。

claim 仍是同库一个事务内的 same-state CAS + append，reason=`intent.dispatch_claimed`，lease action=`Acquire`。SQL 的 `WHERE` 除 intent/state/version/generation 外继续匹配旧 owner/until；竞争者获胜时返回 current 并重新分类，不能拿旧 snapshot 生成 fence。

实现没有再造第二套 lease 类型：W11 从已验真的当前 snapshot 读取 intent ID、owner、generation、until 和 version，按值构造 W10 的 crate-private `FinalizerFence`，或把同一组精确材料交给受控 store 方法。所有写入仍在事务临界区复核完整 fence；模块不公开 raw SQL，也不把 lease 到期解释为发送许可。

## 5. 各状态恢复规则

### 5.1 PendingDispatch

取得/复用 exact fence 后返回 `DispatchPending`。W11 不查询 provider、不重渲染、不调用 sink，也不把它推进为 AwaitingAuthority。后续 W12/W16 接线必须用原 intent、原 durable decision 与原 immutable bytes；在 durable authority 证明无 attempt 之前不得发送。

### 5.2 AwaitingAuthority

使用 W10 `prepare_accepted_finalization` 作为首次 authority 查询和接受资格门：

- Accepted / ManualConfirmedAccepted：prepare 资格 CAS 后立即调用 commit 二查，成功进入 Completed；
- Rejected：以 exact fence 追加最多一条 `transport.rejected` same-state 事件，返回 `RejectedAuthorizationRequired`；没有 W18 显式授权就不产生新 attempt；
- Uncertain：以 exact fence CAS 到 `ResolutionRequired`，reason=`transport.uncertain`，返回 `ManualResolutionRequired`；
- ManualConfirmedNotDelivered：没有 W18 operator audit capability，不自动完成，返回 `OperatorAuditRequired`；
- authority missing/pending/unavailable/binding invalid：最多追加一条 `finalizer.terminal_ref_invalid` same-state 事件，返回 `AuthorityBlocked`；
- state/version/fence 竞争：读取 current 后重新分类，不使用旧引用。

“最多一条”按当前 head reason 实现；fixed-point 下一轮仍可重新查询 authority 发现恢复，但相同阻断不会每轮制造新版本。

### 5.3 AwaitingFinalizer

该状态证明第一次接受资格已落业务库，但不能复用重启前内存引用。W11 重新调用 W10 prepare，再由 commit 做 final requery：

- fresh accepted binding 一致：完成到 Completed；
- authority 仍失效：W10 追加一次 nonterminal invalid evidence；下一轮若 head 已是相同 invalid，先做只读 W09 probe，相同失败不再写，恢复后才重新进入 W10；
- fresh disposition 变成 Rejected/Uncertain/NotDelivered：这是已资格化之后的处置冲突，CAS 到 ResolutionRequired，reason=`operator.resolution_conflict`；不能降级成 Rejected retry 或 NotDelivered；
- CAS 竞争按 W10 隔离规则处理。

### 5.4 ResolutionRequired

启动恢复只返回 `ManualResolutionRequired`：

- 不自动构造 `VerifiedResolutionClearance`；
- 不获取/续租；
- 不查询或调用 sink；
- 不创建新 decision/intent；
- 不追加重复 isolation 事件。

只有 W18 认证 inspect/resolve 后才能签发 clearance 或 operator audit capability。

## 6. Fixed Point

`reconcile_startup` 使用受校验配置：owner、actor、now、new lease until、page size 与最大迭代数。每次迭代：

1. cursor 从空开始，完整扫描全部 recovery states；
2. 每条 intent 最多调用一次状态恢复；
3. 以调用前后 version/state 的变化判断本轮 progress；
4. 有 progress 则从最早 key 开始下一轮，覆盖刚进入的新状态；
5. 零 progress 即返回 `StartupRecoveryReport`；
6. 超过上限仍有 progress，返回 `IterationLimitExceeded`，不得宣称 producer ready。

报告按 `(business_date,intent_id)` 去重并排序，保存最终观察到的 typed boundary、最终 state/version/fence generation 和本轮 transition 数。它不保存 subject、rendered bytes、receipt evidence、token、数据库路径或 SQL。

fixed point 表示“W11 本地可安全推进的事实已耗尽”，不表示 deployment ready：`DispatchPending`、`LiveForeignLease`、`AuthorityBlocked`、`RejectedAuthorizationRequired`、`OperatorAuditRequired`、`ManualResolutionRequired` 都必须留在报告中，W15 再决定 Core/Producer/Occurrence readiness。

## 7. 错误与失败关闭

| 场景 | W11 行为 | 禁止行为 |
| --- | --- | --- |
| scan/attestation/chain 失败 | 整次 startup error | 跳过坏 intent |
| binding 缺失或 policy/template 错 | typed error，启动闸门失败 | 当作无数据或禁用 |
| SQLite busy/error | typed store error | 因 busy 重发 |
| live foreign lease | typed boundary，零写、零 authority query | 抢占或续租别人的 lease |
| expired lease CAS 输掉 | 重读 current，下一轮重分类 | 使用旧 generation/fence |
| terminal missing/pending/unavailable | invalid evidence 至多一次 + boundary | 创建新 decision 或发送 |
| Rejected | same-state evidence 至多一次 | 无授权 retry |
| Uncertain | ResolutionRequired | 自动重发/自动 Accepted |
| finalizer CAS/append fault | 继承 W10 rollback/error | 绕过事件提交完成 |
| 超过 fixed-point 上限 | `IterationLimitExceeded` | 启动 producer |

## 8. 测试与完成门禁

W11 至少覆盖：

1. page size=1 仍按日期/ID 覆盖多个历史业务日，不只扫描 today；
2. 完整终态不进入扫描，损坏链 fail closed；
3. 无 lease 获取 generation=1，过期 lease 接管 generation+1；
4. 同 owner live fence 复用，live foreign lease 零写且零 authority query；
5. PendingDispatch 只产生 DispatchPending，不调用 authority/sink、不改变发送状态；
6. AwaitingAuthority Accepted 从原业务日经 W10 二查完成一次；
7. AwaitingFinalizer 重启恢复不复用内存引用；
8. Rejected 只留一条阻断事件，多轮不重写且无 retry；
9. Uncertain 进入 ResolutionRequired，后续轮次不查询、不重发；
10. missing/pending/unavailable 留一次 invalid evidence，fixed point 不形成无限版本链；
11. ManualConfirmedNotDelivered 没有 operator audit 时保持未完成；
12. 已有 ResolutionRequired 零自动解封/零重复写；
13. state/version/lease 竞争只使用重读 current；
14. 有界迭代、防重复处理和报告脱敏。

Fresh 门禁包括 W11 目标、W07--W11 Foundation、push_job、rustdoc、check、strict/nonfatal Clippy attribution、定向 rustfmt、架构五组验证器、相对 W10 的生产 wiring/DDL/Cargo 零差异，以及原 release monitor 的只读运行观察。

W11 完成后仍不能宣称 production startup 已接线、durable physical recovery 已替换或任何 Unit 已迁移。下一步 W12 才实现 generic transport authority adapter。
