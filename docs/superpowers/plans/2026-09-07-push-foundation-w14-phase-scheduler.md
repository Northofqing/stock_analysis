# 推送 Foundation W14 PhaseScheduler 实施计划

**状态：** 待执行。

**目标：** 以 TDD 实现半开窗口、原业务日 catch-up、W11 启动屏障、W03 completion close 和 closed 不可重开的 crate-private 调度模块；保持零 production wiring。

**权威设计：** `docs/superpowers/specs/2026-09-07-push-foundation-w14-phase-scheduler-design.md`

## Task 1：冻结窗口、策略、状态与 catalog binding

**文件：**

- 新增 `src/push_foundation/phase_scheduler_tests.rs`
- 新增 `src/push_foundation/phase_scheduler.rs`
- 修改 `src/push_foundation/mod.rs`
- 修改 `src/monitor/push_job/identity.rs`

### Step 1：写 RED

新增 `w14_` 测试：

- 窗口 `[start,end)` 的 start/end 精确边界和非法窗口；
- W06 catalog 的 producer/Unit/owner/family/phase exact binding；
- 五类 catalog 漂移 fail closed；
- 非交易日和 RecoverPersistedOnly 不创建 occurrence。

运行：

```bash
cargo test --lib w14_ -- --test-threads=1
```

Expected: 新接口不存在或行为测试失败，形成真实 RED。

### Step 2：最小 GREEN

实现 `ScheduleWindow`、`CatchUpPolicy`、`ScheduleStatus`、`PhaseSchedule::try_bind`、safe errors 和 W01 identity getters；只做到 Task 1 测试通过。

### Step 3：验证与提交

定向运行 `rustfmt --edition 2021`，再运行 W14 与 W01/W06 相邻测试。提交窗口/绑定切片。

## Task 2：实现 deterministic step 与原业务日 catch-up

**文件：**

- 修改 `src/push_foundation/phase_scheduler.rs`
- 修改 `src/push_foundation/phase_scheduler_tests.rs`
- 修改 `src/push_foundation/reconciler.rs`

### Step 1：写 RED

覆盖：

- 新 occurrence 第一步只创建 Expected/version=0；
- 第二步在窗口内提案 Expected→Eligible/version=1；
- normal tick 与 startup catch-up 的 ID/proposal exact 相同；
- W11 成功报告签发 catch-up barrier；
- later wall clock 不改变原 business date；错误业务日 observation 拒绝；
- 重放相同输入完全相等，已存在 occurrence 不重复 create。

### Step 2：最小 GREEN

实现 `ScheduleOccurrenceSnapshot`、`ScheduleTransitionProposal`、`ScheduleStep`、`PhaseScheduler::{tick,startup_catch_up}` 和 W11 barrier。所有 version 增量使用 checked add。

### Step 3：验证与提交

运行 W14、W11、W01 测试并提交 catch-up 切片。

## Task 3：完成 policy 矩阵与 closed sealing

**文件：**

- 修改 `src/push_foundation/phase_scheduler.rs`
- 修改 `src/push_foundation/phase_scheduler_tests.rs`

### Step 1：写 RED

覆盖：

- Expire/SameDay 在 end 精确进入 Missed，end-1 仍 Eligible；
- Defer 进入 Deferred 并保存 next-session ref，在 next `[start,end)` 内恢复 Eligible；
- BlockedOnInput 在窗口内不被 tick 伪装成 recovered，窗口后才按 policy Missed/Deferred；
- Prepared 越过窗口仍保留；
- W03 close directive 只允许 Prepared→Closed；
- Closed 在 tick、catch-up、时钟回拨、重复 completion 下均零 proposal；
- Missed 无出边；非法 next ref、快照绑定漂移、version overflow fail closed。

### Step 2：最小 GREEN

完成状态矩阵、`PhaseScheduler::completion`、reason 映射和安全 NoChange 投影。

### Step 3：验证与提交

运行 W14、W03/W11、Foundation 总测试并提交 sealing 切片。

## Task 4：双轴评审与修复

按 Spec 与 Standards 两轴逐项检查：

- 每条 W14 验收是否有行为测试；
- RFC 状态边、reason、identity exclusion 和 version 前提是否精确；
- 是否错误引入中央 DB、自然语言 schedule 解析、production timer 或 future W15/W16 权限；
- public/crate-private seam 是否最小，错误/Debug 是否泄漏；
- 是否存在生产 `unwrap/expect/panic/unreachable`；
- closed/replay 是否在竞争或损坏快照下 fail closed。

任何发现先写回归 RED，再修复并独立提交。

## Task 5：fresh gates、中文结果与收口

运行：

```bash
cargo test --lib w14_ -- --test-threads=1
cargo test --lib push_foundation:: -- --test-threads=1
cargo test --lib monitor::push_job::tests:: -- --test-threads=1
cargo test --doc
cargo check --lib
cargo clippy --lib
```

另运行五组 architecture docs tests、RFC/source/WBS 校验、`git diff --check`、目标 diff panic 扫描和 production-wiring 零 diff。strict Clippy 若仍被既有基线阻断，记录首错、总数和 W14 目标文件零新增，不伪装为通过。

新增 `docs/push-system/implementation-w14-results-2026-09-07.md`，更新本设计和计划状态，记录：验收矩阵、状态图、TDD commits、fresh 计数、基线例外、零生产接线、实际提升与 W15--W21/52 Unit 剩余边界。

最终提交文档并复验 W14 目标测试和 clean worktree；随后进入 W15，不提前接线或晋级 physical owner。
