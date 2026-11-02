# Paper 账本虚拟买卖执行缺口（2026-11-02）

> 与 `docs/ticket-lifecycle-plan.md` 分开维护。ticket 方案覆盖的是“单票生命周期建议”，
> 不覆盖 paper 账本的买入/卖出执行合同；本文只记录这个独立缺口，不改变 ticket 的范围。

## 结论

当前 V2 paper 账本**没有虚拟买入/卖出流水**。它不是“页面没显示”，而是账本里确实没有
`Executed` 事件：

- `paper_ledger_event` 共 25 条：1 条 `Seeded` + 12 条 `Marked` + 12 条
  `DerivedSnapshot` + **0 条 `Executed`**（截至 2026-11-02 10:56）。
- 当前账户 `paper-actual-20261009-2113-c6124ac14803` 的 5 只持仓是 2026-10-09
  用快照 `Seed` 进来的库存，之后只做每日收盘 `Mark`，没有下单/成交。
- 旧表 `paper_trades` 有 2,589 条，但时间只到 **2026-09-24 05:21:34**；当前 5 只里
  只有 `002131` 有 2 条 8/9 月的旧记录，其余 4 只为 0。V1 的纸面交易写入已停。

## 为什么会这样

1. **V1 → V2 迁移后的职责变化**：V2 `paper_ledger` 是“库存镜像 + 每日估值”，
   买入建仓由 `SeedManifest` 一次性写入，不产生逐笔买入事件。
2. **卖出入口存在但未成交落账**：`paper_sell` 走
   `PaperCommand::Execute(ExecuteIntent)`（`paper_ledger_runtime.rs:523`），
   成功后应写 `paper_ledger_event` 的 `Executed` 并投影到 `paper_trades`
   （`paper_ledger_execution.rs:549`）。当前账本 0 条 `Executed`，说明没有卖出成交落账；
   是“没有触发/没有运行/未接入”，需要单独排查，不能从这段文字推定。
3. **没有运行中的买入生产者**：当前活跃的 V2 链路没有把买入信号写成
   `PaperCommand::Execute`；V1 的 `paper_engine` / `paper_trade` 写入停在 9-24。
4. **Seed 不是买入**：10-09 的 5 只库存是账户期初，不是 5 笔买入；所以
   “看不到买入”是合同设计的结果，不是数据丢失。

## 用户能在哪里看到“真实买卖”

- 真实账户的持仓变化：`user_position_snapshot` / `user_position_snapshot_item`
  的前后数量差；
- 票级事件：`ticket_event` 的 `position_opened` / `position_added` /
  `position_reduced` / `position_closed`（由 `position_change_detect` 写）。
- 这些都不是 paper 成交，不应混在 `paper_trades` 里。

## 影响

- 虚拟账户只能显示 seed 库存和每日 Mark，不能解释“哪一天买了/卖了什么”。
- paper 累计 PnL 是估值结果，不是逐笔成交结果；与券商真实账户的 7 只、现金 367.36
  不一致（当前 paper 是 5 只、现金 7,254.94，见 ticket 方案 12.2.1 的边界）。
- 任何“虚拟盘买卖复盘/胜率”如果读 `paper_trades`，拿到的只会是 9-24 之前的历史，
  不能代表当前账户。

## 可选的修法（需要先定产品口径）

| 方案 | 做法 | 代价 | 适用 |
|---|---|---|---|
| A. 快照差额转虚拟成交 | 每日 `user_position_snapshot` 数量差 → `PaperCommand::Execute`；按当日价/费用生成买入/卖出，写 `paper_trades` 投影 | 要定成交价、费用、T+1、现金口径；快照缺日会漏笔 | 想让虚拟盘镜像真实账户 |
| B. V2 策略执行器 | 策略信号 → 虚拟委托 → `Execute`；完整的买入/卖出/拒单/滑点 | 代码量与测试最大；要定义纸上策略 | 想做真正的 paper trading |
| C. 只读镜像，不造成交 | 页面明确显示“期初库存 + 每日估值”，另列 `ticket_event` 的真实持仓变化 | 成本最低；但用户永远看不到虚拟买卖 | 接受 paper 只是估值镜像 |
| D. 读旧 `paper_trades` | 仅展示 9-24 前历史，标注 legacy | 不能覆盖当前账户 | 只做历史查询 |

建议先确认产品意图：

- 若 paper 只是“真实账户的估值镜像”→ 选 C，不要伪造买卖；
- 若要“虚拟盘能复盘买卖”→ 选 A（镜像）或 B（策略）；
- 无论 A/B，都必须让 `paper_ledger_event` 出现 `Executed`、`paper_trades` 有对应行，
  且现金/持仓投影与 replay 一致，才算完成。

## 未来验收（如果立项）

- 买入/卖出各产生一条 `Executed`，`paper_trades` 有对应 `buy`/`sell` 行；
- 现金、持仓数量、成本、已实现/未实现 PnL 在投影与 `paper_trades` 之间可对账；
- T+1 锁定、费用、滑点、拒单原因有明确记录；
- 页面/推送读的是 `paper_ledger_event` 的 `Executed`，不是只读 legacy `paper_trades`；
- 重放/幂等：同一 `command_id` 不重复成交。

## 2026-11-02 期初 rebind 已执行

用户明确要求先做 baseline，旧 epoch 保留归档，不删除、不改写：

- 旧活动账本（`paper-actual-20261009-2113-c6124ac14803`，version 25，25 条事件）
  已整体改名归档为 `paper_archive_v1_*`；
- 新账本：
  - account_id=`paper-actual-20261102-154611-687e69a6`
  - epoch_id=`paper-epoch-20261102-154611-687e69a6`
  - 来源：snapshot 59（2026-11-02 15:46:11）
  - 6 只持仓：001216 100 / 002131 1000 / 002421 2000 / 600396 1000 /
    600667 600 / 600703 500
  - 可交易现金：2,007.82
  - 券商资产：42,324.92
  - 明确排除、不可交易残差：7,205.10
  - seed equity：35,119.82
  - 新账本 head version 2（Seed + opening closing mark）
- 为支持这次 rebind：
  - activation 现在允许 `excluded_residual` 非零，但必须严格满足
    `original_total - (cash + marks) == excluded_residual`；
  - 新增 `paper_rebind_apply`，先 preview 再 apply，并支持重复 apply 的 already_applied 校验；
  - 运行根 `.env` 的 `PAPER_LEDGER_ACCOUNT_BINDING` 已改到新 account/epoch，
    备份为 `.env.bak-paper-rebind-20261102`。
- monitor 为迁移已停止；当前 gateway 禁止在本会话调用 `launchctl bootstrap/kickstart`，
  需要运维者在独立终端执行：

```bash
launchctl bootstrap gui/501 /Users/zhangzhen/Library/LaunchAgents/com.stockanalysis.monitor.plist
```

启动后按既有标准等 `数据库初始化完成` / `startup_health_check status=ok` /
`观测订阅已建立`，再确认 paper 读的是新 binding。

## 明确不做

- 不把 seed 库存伪造成买入流水；
- 不直接改 `paper_ledger` 的 projection 或 `paper_trades` 来“补”买卖；
- 不在 ticket 生命周期方案里把 paper 执行当作 ticket 的一部分——两者是独立合同。
