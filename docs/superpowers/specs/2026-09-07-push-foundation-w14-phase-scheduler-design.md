# 推送 Foundation W14 PhaseScheduler 与 occurrence catch-up 设计

**状态：** 设计冻结，待 TDD 实现与 fresh 验证。

**决策日期：** 2026-09-07（Asia/Shanghai）

## 1. 目标与验收合同

W14 交付统一的时间调度状态机，使正常 tick、启动 catch-up 和完成关闭遵守同一套 occurrence 身份与生命周期规则。唯一 WBS 验收句是：

> 窗口半开区间和原业务日 catch-up 可回放；closed occurrence 不可重开。

权威依据：

- `docs/push-system/push-system-wbs.v1.json` W14：依赖 W01、W03、W06、W11；不含逐 Unit cutover；
- `docs/push-system/push-system-implementation-rfc.md:868-988`：ScheduleOccurrence 字段、version/CAS、身份、生命周期和四种 catch-up policy；
- `docs/Project_Architecture_Blueprint.md:1398-1431`：clock/calendar 注入、非 INACTIVE 定时 producer 登记、INACTIVE 零 scheduler、启动恢复先于 scheduler；
- W01 的 `ScheduleOccurrenceIdentityMaterial`、W03 的 `CompletionDirective`、W06 的 `MachineCatalog`、W11 的成功恢复报告。

## 2. 当前代码约束

### 2.1 W06 不是 schedule authority

W06 运行时目录只机械校验 kind、producer、Unit、completion owner、occurrence family 和 phase。机器目录里的 trigger/source/authority/policy 仍是自然语言，不包含结构化 schedule ID、calendar ID、source-contract ID/version 或精确窗口。因此 W14 不从描述文字解析时刻，也不声称已登记 102 个生产日程。

### 2.2 W07 v1 没有 occurrence registry

`docs/push-system/push-system-foundation.v1.sql` 冻结的是 intent、transition、activation manifest 和 promotion journal，共 25 个对象；没有 schedule occurrence 表。RFC 又明确 schedule occurrence 属于各业务库的同事务状态，独立于 `push_intents.version`。

W14 不修改 W07 v1 SQL，也不新建错误的中央 occurrence 数据库。各 Migration Unit 后续用本设计的 CAS proposal 在自己的业务库落地；W16 再绑定 generation/fence。

### 2.3 当前生产路径保持不变

现有 P01、N02、review、产业链和大量 `main.rs` timer 继续由旧 binary 执行。本切片不新增 `src/bin/monitor/phase_scheduler.rs` caller，不启动 timer，不迁移 physical owner，也不触碰生产 monitor。

## 3. 模块 seam

新增 crate-private `src/push_foundation/phase_scheduler.rs`。外部 seam 只暴露给同 crate 后续 Foundation composition：

```text
PhaseSchedule::try_bind(machine_catalog, identity, phase, window, catch_up_policy)

PhaseScheduler::tick(schedule, current_occurrence, market_observation)
PhaseScheduler::startup_catch_up(recovery_barrier, schedule, current_occurrence, market_observation)
PhaseScheduler::completion(schedule_occurrence, completion_directive, observed_at)

→ ScheduleStep
   ├─ NoOccurrence
   ├─ CreateExpected
   ├─ TransitionProposal
   ├─ RecoveryOnly
   └─ NoChange
```

模块返回结果，不执行网络、数据库、provider、LLM、render、sink、游标或订单副作用。调用方和测试通过同一 seam 验证行为。

## 4. 类型与不变量

### 4.1 `ScheduleWindow`

- `start: UtcMicros` 为含起点；
- `end: UtcMicros` 为不含终点；
- 构造时强制 `end > start`；
- 分类固定为 `now < start`、`start <= now < end`、`now >= end`。

这使边界无需依赖秒级 tick 精度：窗口起点必定 eligible，终点必定 expired。

### 4.2 `CatchUpPolicy`

闭集与 RFC 一致：

1. `ExpireWithoutCatchUp`；
2. `SameBusinessDayBeforeDeadline`；
3. `DeferToNextEligibleSession`；
4. `RecoverPersistedOnly`。

前两者在原窗口内均可进入 eligible，窗口结束后进入 Missed；第三种在原窗口结束后保存 typed `NextEligibleSessionRef` 并进入 Deferred；第四种禁止创建新 occurrence，只允许 W11 按既存 intent/decision 的原 identity/bytes 恢复。

### 4.3 `ScheduleStatus`

闭集与 RFC 一致：

`Expected / Eligible / Prepared / Closed / Missed / Deferred / BlockedOnInput`。

W14 时间判定负责以下边：

- `Expected → Eligible`；
- `Expected|Eligible|BlockedOnInput → Missed`；
- `Expected|Eligible|BlockedOnInput → Deferred`；
- `Deferred → Eligible`；
- W03 close directive 驱动 `Prepared → Closed`。

`Eligible → Prepared` 属于 future physical owner 在持久 prepared intent 后提交；`Expected|Eligible → BlockedOnInput` 与 `BlockedOnInput → Eligible` 的来源恢复细分属于 W15。W14 类型允许持久适配器 hydrate 这些状态，但不伪造对应 evidence。

`Closed` 与 `Missed` 没有任何出边。尤其 `Closed` 在正常 tick、启动 catch-up、时钟回拨和窗口变化输入下都只能返回 `NoChange(schedule.occurrence_closed)`。

### 4.4 `PhaseSchedule`

`PhaseSchedule` 持有：

- W01 `ScheduleOccurrenceIdentityMaterial`；
- W06 验证过的 phase；
- `ScheduleWindow`；
- `CatchUpPolicy`。

`try_bind` 必须从 W06 catalog 反查 producer，并 exact-match Unit、completion owner、occurrence family 和 phase。schedule/calendar/source-contract 值当前只能作为 typed binding 输入，不能假称来自 W06。模块为 crate-private 且没有 production caller，W16 才能用 versioned activation binding 构造运行能力。

### 4.5 `ScheduleOccurrenceSnapshot`

快照保存 schedule、派生的 `ScheduleOccurrenceId`、status、version、reason、created/updated time，以及 Deferred 时唯一允许存在的 next-session ref。

- ID 每次 hydrate 都从 W01 identity 重算；
- `updated_at >= created_at`；
- `Deferred` 必须有 next-session ref，其他状态禁止携带；
- 新建恒为 `Expected/version=0`；
- transition 的 result version 使用 checked add；
- wall-clock、normal/catch-up origin、generation、build、payload/evidence hash 都不进入 ID。

### 4.6 `ScheduleTransitionProposal`

proposal 绑定：

- exact occurrence ID；
- `from_status/to_status`；
- `expected_version/result_version`；
- stable `ReasonCode`；
- observed time；
- Deferred 所需 next-session ref。

它是不可直接写库的纯提案，不是 RFC 最终 `ScheduleOccurrenceTransitionRequest`。W16 仍须加入并在提交临界区重验 `expected_generation + ActivationFence`；各 Unit 持久适配器仍须用 exact ID/from/version CAS，在同一业务事务写状态与 append-only evidence。这个命名阻止调用方把未带 fence 的对象误当生产写权限。

## 5. 调度判定矩阵

| 当前事实 | 时间/策略 | 结果 | 原因 |
| --- | --- | --- | --- |
| 无 occurrence；authority 判非交易日 | 任意 | `NoOccurrence` | `schedule.not_trading_day` |
| 无 occurrence；`RecoverPersistedOnly` | 任意 | `RecoveryOnly`，不得创建 | 不补造新 identity |
| 无 occurrence；有效交易日 | 任意窗口位置 | `CreateExpected(version=0)` | 保存原业务日身份 |
| Expected；窗口前 | `now < start` | `NoChange` | `schedule.window_not_open` |
| Expected；窗口内 | `start <= now < end` | `Expected→Eligible` proposal | `schedule.window_open` |
| Expected/Eligible/Blocked；窗口后 | Expire/SameDay | `→Missed` proposal | `schedule.window_expired` |
| Expected/Eligible/Blocked；窗口后 | Defer | `→Deferred` proposal | `schedule.deferred` |
| Deferred；next window 前 | 任意 | `NoChange` | `schedule.window_not_open` |
| Deferred；next window 内 | 任意 | `Deferred→Eligible` proposal | `schedule.window_open` |
| Prepared | 任意时间 | `NoChange`，等待 W03 completion | 不因窗口过期丢已持久事实 |
| Prepared + W03 close directive | close | `Prepared→Closed` proposal | `schedule.occurrence_closed` |
| Closed | 任意 tick/catch-up/completion | `NoChange` | `schedule.occurrence_closed` |
| Missed | 任意 | `NoChange` | `schedule.window_expired` |

`BlockedOnInput` 在窗口仍开时保持原状态，W15 才有来源恢复/就绪证据；W14 不能把“又到了一个 tick”当作 source recovered。

## 6. 原业务日与可回放性

正常 tick 和启动 catch-up 调用同一个内部 `evaluate`：

- 两者使用同一 `PhaseSchedule`，所以派生同一 `ScheduleOccurrenceId`；
- origin、当前进程启动时间和 activation generation 不进入 identity；
- observation 必须引用 schedule 的原 business date；
- later catch-up 即使发生在另一个 calendar date，也不会把 identity 改成“今天”；
-同一 current snapshot + observation 重放得到逐字段相同的 `ScheduleStep`；
- 已存在 occurrence 时不会再次返回 `CreateExpected`。

启动 catch-up 额外要求 `StartupRecoveryBarrier`。该不可伪造 marker 只能由成功返回的 W11 `StartupRecoveryReport` 生成，确保顺序为“先恢复既存 intent/decision，再评估是否创建新 schedule occurrence”。

## 7. 失败关闭与信息安全

以下输入返回 typed error，且不产生 proposal：

- 非法或空窗口；
- identity 的 producer/Unit/owner/family/phase 与 W06 catalog 不一致；
- observation 的 target business date 与 schedule 原 business date 不一致；
- current snapshot 的 ID/schedule 与请求不一致；
- Deferred 缺 next-session ref，或非 Deferred 携带该 ref；
- next-session window 非法或不晚于原窗口；
- version 溢出；
- completion close 试图从 Prepared 之外的非终态出发。

错误与 Debug 只允许输出稳定类型、ID、状态、version 和检查名；不得输出 prepared/rendered/source/receipt bytes。W14 本身不持有这些 payload。

## 8. TDD 验收矩阵

1. `ScheduleWindow` 起点含、终点不含；非法窗口拒绝。
2. catalog binding exact-match producer/Unit/owner/family/phase；漂移逐项拒绝。
3. 非交易日无 occurrence；`RecoverPersistedOnly` 无新 occurrence。
4. 新交易日先创建 `Expected/version=0`，第二步才生成 `Expected→Eligible/version=1`。
5. 正常 tick 与 startup catch-up 对同一原业务日生成相同 ID 和逐字段相同 proposal。
6. catch-up 发生在后续 wall clock 时仍保存原 business date；observation 误换业务日拒绝。
7. Expire/SameDay 在 end 时进入 Missed；end 前 1 微秒仍 eligible。
8. Defer 保存 next-session ref，并在其半开窗口内 `Deferred→Eligible`，identity 不变。
9. Prepared 不因窗口结束变 Missed；W03 close directive 才能提案 Closed。
10. Closed 在 normal/catch-up/时钟回拨下均无 proposal；Missed 也无出边。
11. current ID/version/status/next ref 损坏及 version overflow fail closed。
12. Foundation、push_job、rustdoc、architecture docs 与 production-wiring diff 门禁保持通过。

## 9. 非目标与后续

- 不实现 52 个 Unit 的具体日程、holiday/calendar adapter 或业务库 occurrence table；
- 不实现 tokio long-running supervisor、timer registration、生产 `src/bin/monitor/phase_scheduler.rs` composition；
- 不实现 W15 readiness/source recovery、W16 activation fence、W17 shadow、W18 operator、W19 metrics 或 W20 fault harness；
- 不迁移/删除 P01、N02、review 或 `main.rs` 的现有 scheduler；
- 不把 Foundation GREEN 冒充任何 Unit 已灰度或线上推送已经改善。

W14 完成后，只能宣称统一调度语义和可持久化 CAS proposal 已有可执行证据；production 行为仍为零变化。
