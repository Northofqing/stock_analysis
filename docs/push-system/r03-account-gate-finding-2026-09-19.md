# MU-review-r03 账户门：核实现状与定性更正

2026-09-19。**只读核实**，未修改任何源码，未启用任何推送。本文更正 [R03 调用链](review-r03-call-chain-2026-09-14.md) 的定性与其失效行号，并把"是否启用"列为待用户决策的产品问题。

## 1. 门在哪里（当前源码，非文档行号）

| 事实 | 当前位置 |
|---|---|
| R03 归类为账户依赖 | `src/bin/monitor/review_batch.rs:512` — `Self::R03 => ReviewTaskDependency::LegacyAccountGate` |
| 分组进 `account_required` | `review_batch.rs:1154-1156` |
| 生产期无条件失败 | `src/bin/monitor/push_templates.rs:11019-11028` |
| 派发函数**存在** | `push_templates.rs:13530` `dispatch_r03_industry_chain_outcome`、`:13735` `dispatch_r03_industry_chain_real` |

生产段原文（`push_templates.rs:11019-11028`）：

```rust
// BR-139/BR-194: account_required 任务在真实账户指标缺失时统一停在
// typed AccountMetricsIncomplete 边界；不得调用 provider、renderer 或 sink。
let mut account_required_outcomes = Vec::new();
for task in &phases.account_required {
    account_required_outcomes.push((
        *task,
        ReviewTaskOutcome::account_metrics_incomplete(observed_at),
    ));
}
```

该循环**不读取任何账户批次**，直接为每个 `account_required` 任务产出 `AccountMetricsIncomplete`。注释所述条件（"真实账户指标缺失时"）在代码中并未被求值。

## 2. **定性更正：这是刻意停用，不是意外缺陷**

上一轮主控曾把此称为"生产缺陷 / 永远发不出去"。核实后**该定性偏了**，理由：

- R03 的完整派发实现**存在且位于产区**（`push_templates.rs:13530/13735`，均早于其后的 `#[cfg(test)]` 边界 `:14325`）。
- 但它**没有任何生产调用者**：唯一调用点 `:14817` 落在 `#[cfg(test)]` 之内，仅测试可达。同文件 `:20437` 的 `r03_industry_chain_two` 亦为 `#[test]`。
- 反向证据更强：存在**主动断言"不得接线"**的守卫测试——
  - `:13254` `assert!(!account_phase.contains("dispatch_r03_industry_chain_outcome"));`
  - `:13285` `assert!(!dispatcher.contains("dispatch_r03_industry_chain_outcome"));`

即：功能已实现并被测试覆盖，但被**刻意**排除在 dispatcher 之外，并由测试锁住该状态。这与 [R03 调用链](review-r03-call-chain-2026-09-14.md) 所记待办"真实账户批次接线：用完整、可验证的账户输入替换无条件失败"一致——**门是保守占位，接线是未完成的工作**，而非写错的分支。

## 3. 真实账户批次接口（已定位，未接线）

`src/database/account_snapshot.rs`：

| API | 作用 |
|---|---|
| `latest_account_snapshot() -> Result<Option<AccountSnapshot>, String>`（`:542`） | 真实账户批次读取入口 |
| `AccountSnapshot::validate_fresh_for_action(now)`（`:104`） | BR-103 / AGENTS 2.4：账户事实**最多授权 30 秒**；拒绝过期与未来时间 |
| `daily_pnl` / `position_ratio_pct` / `withdrawable_cash`（`Option`） | 完整性判定依据 |
| `daily_pnl_status` / `account_ref_status` | 状态字段 |

表 `real_account_snapshot` 不可变、留存 ≥5 年（`trg_real_account_snapshot_no_update` / `_no_delete`）。

## 4. 启用 R03 需要做的三件事（**未实施**）

1. **账户批次接线**：以 `latest_account_snapshot()` + `validate_fresh_for_action` + 完整性判定替换无条件失败；**保留**缺失 / 过期 / 不完整的拒绝，**不得删除门或伪造健康值**。
2. **接线派发**：将 `dispatch_r03_industry_chain_outcome` 接到账户验证通过之后。
3. **同步守卫测试**：`:13254` / `:13285` 现断言"不得包含"，须改为"必须在账户验证之后"。

同时注意 `:13248` 处以**源码字符串** `"let mut account_required_outcomes = Vec::new()"` 定位账户段——任何改动该行的实现都要同步更新该守卫。

## 5. 为何未实施：这是产品决策

完成上述三步后，**R03 将在账户快照新鲜且完整时第一次真正推送用户可见消息**。在真实交易系统上打开一条被人为关闭的推送，属于产品决策而非缺陷修复。2026-09-19 用户裁定：**暂不启用**；本类"打开开关"的改动单独成批，另行统一决策。届时本类缺口应放弃"修复缺陷"的措辞。

## 6. 边界

- 本文不含任何源码修改、未部署、未运行 monitor、未写生产库、未调用真实 provider。
- 文档行号（如 `push_templates.rs:10072`、`review_batch.rs:418/1138`）写作于 2026-09-14，**已失效**，不得再引用；本文数字均为 2026-09-19 实测。
- 本文只覆盖 `MU-review-r03` 一个 Unit 的账户门，不代表其余 51 个 Unit 的迁移状态。
