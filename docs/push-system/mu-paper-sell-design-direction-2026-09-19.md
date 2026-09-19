# MU-paper-sell：回退记录与正确设计方向

2026-09-19。**本文记录一次已被回退的实现尝试，以及下一轮应采用的正确设计。** 当前工作树已回到 `587b8b6`（干净），没有任何本文所述的改动残留，线上也从未部署过。

## 1. 为什么回退

一次"把卖出通知接进持久投递"的实现被独立复审否决，主控复核后确认存在两个阻断项，用户裁定整体回退（选择"乙"）。

### 阻断项 1：该实现会**静默关掉所有卖出通知**

把 `PaperSell` 注册为 counted kind（映射进 `durable_delivery_runtime.rs` 的 kind 表）会使 `is_counted_kind(PaperSell) == true`；而 `DeliveryEnvelope::new`（`src/durable_delivery/model.rs:820-831`）**强制要求**一条已编译 policy 行：

```rust
let policy = compiled_policy_catalog().into_iter()
    .find(|row| row.push_kind == push_kind && row.sub_kind == sub_kind)
    .ok_or_else(|| DurableDeliveryError::PolicyMismatch(
        format!("no registered policy for {push_kind}/{sub_kind}")))?;
```

`compiled_policy_catalog()`（`model.rs:413-547`，内部 `use PushKind::*;` 故写裸变体名）**含 `PaperTrade`、不含 `PaperSell`**（主控本人复核：`PaperTrade` 命中 1、`PaperSell` 命中 0）。

⇒ `deliver_counted_binding` 必然返回 `Denied("no registered policy for PaperSell/None")`：**不发卡、不写 durable 行**。原本"失败时才丢"会变成"永远不发"，而 `already_sold_today`（`paper_sell.rs:306-323`）仍使该卖出无法再生 ⇒ 丢失依然永久，只是原因改成了这次改动。

> **方法论教训**：主控第一次核对时用 `PushKind::` 前缀 grep，得到"11 个且无 PaperTrade"，与复审矛盾——**是模式错了**（catalog 写裸变体名）。差点据此错误地推翻一个正确的复审结论。**grep 的零命中不等于不存在。**

### 阻断项 2：BR-196 验收路径被打断

单元门 23/23 通过是**假绿**：`main.rs` 的验收路径会因缺少 `T-18-paper-sell` 的 rendered preview 而失败；验收矩阵与 `EXPECTED_CATALOG_TOTAL=56` 也仍是旧值。门之所以过，是因为相关测试使用**合成 summary**（旧常量），从不与 manifest 交叉校验。归档中**没有任何** `monitor --test` 的实跑记录。

## 2. 该 Unit 的真实形态（此前被主控误述两次）

| 主张 | 实际 |
|---|---|
| "卖出成交后用户永远收不到卡" | **错**。P-04（`push_templates.rs` 的 `prepare_paper_trade_daily`）的 loader **同时接受 `buy` 与 `sell`**（`:6374`），走**已经是持久化**的 `PaperTrade` 路径 |
| "P-04 的卡不透露卖出语义" | **错**。卖出行的 `virtual_reason` 写作 `BR-234四大铁律卖出:{reason}`（`paper_sell.rs:615`），P-04 在「主理由」里原样渲染 |
| 那张 `[虚拟盘卖出]` 卡独有什么 | **只有「收益率」**（`return_rate_pct`） |

**结论：它是补充性卡片，不是"卖出成交"这一事实的唯一通知。** 这个事实把该缺口的优先级显著降低——对一个只多带收益率的补充卡，去承担"新增预算抑制路径 + 验证未跑过的验收链路 + 两轮成本"的风险，交换比为负。

## 3. 为什么"投递前 prepare"这条路走不通

本次尝试在**投递前**构造 `CountedDeliveryBinding`（镜像 `holding_plan.rs:106-135`）。复审指出这**覆盖不到主要失效模式**：

- durable 只覆盖**投递事务内**的失败（sink 报错 / 被门拒绝）；
- 真正高频的失效是**进程在 fill 与 send 之间被杀**（本项目已知的 Mac 睡眠模式）——那时**什么都还没持久化**，没有任何东西可对账。

也就是说：这条路修的是一个较窄的窗口，却引入了一个新的无条件抑制向量。

## 4. 下一轮的正确设计方向

**核心：把"待投递"持久化在成交落库的同一事务里，而不是在投递前才构造。**

1. **同事务写入待投递记录**：`paper_sell` 写 `paper_trades(status='Filled')` 的那次提交里，同时写入一条待投递意图（occurrence = `(business_date, code)`，与 `already_sold_today` 的"当日一票一卖"不变式一致）。这样**进程在 fill 与 send 之间被杀也能对账**——这是本次尝试没做到的。
2. **消费端**：投递成功后标记该意图完成；失败保留待投递，由 tick/启动对账补偿。
3. **不要**把卖出卡直接注册成普通 counted kind 就完事——那会踩阻断项 1。若确要走 counted 路径，必须**同时**补 policy 行，并且**先决策**下面第 5 节的问题。
4. **验收必须先跑真实路径**：改完要实跑 `monitor --test`（含 preview 与验收矩阵），不能只看单元门的合成 summary。

## 5. 必须先做的产品决策

**卖出卡是否计入 30 条/日预算？**

`counts_against_daily_budget` 对任何不在 BR-237 豁免名单里的 kind 默认为 `true`（`model.rs:473-488`）。若卖出卡计入，它会**新进入 30 槽竞争**，可被 `DailyBudgetFull` 拒绝（`coordinator.rs:4929-4932`）——**正是 8/13 那 7 路复盘被饿死的事故机制**。

对一个涉及真实资金动作的卡片，被无关的日预算饿死是比原缺陷更糟的结果。故此项须显式决策，不能默认。

## 6. 顺带记录的既有维护风险

为新增**一个**呈现单元，本次牵动了 **7 处耦合的计数常量**（分散在 4 个位置、两种形态：数组类型 + 测试字面量），且只能靠**逐个跑测试失败**发现：

- `presentation_registry.rs`：数组类型 58（2 处）
- `br196_test_delivery.rs`：`ACTIVE_PRESENTATIONS` 56、family 总数 72、`ALL_PUSH_KINDS` 63、kind cover 63、lifecycle 矩阵（两分支 × 2 组）、descriptor 计数 58
- 另有 `project_push_kind_lifecycle` 内部**硬编码的 `total`**，与上方 `projected.len()` 校验本就会漂移

若 52 个 Unit 都要走这条路，**建议收敛为单一事实源**（由 `descriptors()` 派生），而不是各处各写一遍字面量。

## 7. 边界

- 本文不含任何未回退的源码改动；工作树为 `587b8b6`，干净。
- 未部署、未运行 monitor、未写生产库、未调用真实 provider。
- 本次尝试的全部过程日志（含被否决的 BR-196 门记录与回归）保留在 `.superpowers/sdd/2026-09-14-chain-macro-recovery/paper-sell-*.log`，供下一轮对照。
- 本文档位于 `docs/push-system/`，该目录在 `.gitignore` 中为**白名单模式**（`/docs/push-system/*` 全忽略 + 逐条 `!` 放行），故本文**默认不被 git 跟踪**。是否纳入版本控制需另行决定。
