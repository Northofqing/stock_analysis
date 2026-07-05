# v12 MVP-0 整合验收报告

> 验收时间: 2026-07-05 09:38
> 命令: `V10_DRY_RUN_PUSH=1 cargo run --bin monitor -- --test`
> 提交链: MVP0-A (0b0e0fa) → MVP0-B (59e3824) → MVP0-C (7d7f0f4)

---

## 一、MVP-0 4 项任务完成状态

| # | 任务 | 提交 | 状态 |
|---|---|---|---|
| A | 修 DB 迁移 (Bug A) | `0b0e0fa` | ✅ |
| B | monitor_loop + run_test_scan 挂 v12 orchestrator | `59e3824` | ✅ |
| C | STOCK_LIST + API Key 治理 | `7d7f0f4` | ✅ |
| D | E2E 验收 | (本报告) | ✅ |

---

## 二、T-01/T-02/T-03 实盘触发验证

### T-01 账户模式变更 (✅ 模板对齐 §14.1)

```
🛡️ 账户模式变更（09:38）
Normal → ReduceOnly
原因:
· 当日亏损 -1.60% 触发降级线 -1.50%
生效限制: 禁止新开仓/加仓/正T, 候选转影子
解除条件: 当日盈亏回到 -1.5% 内 或 连续止损 < 3 笔 (运行时) / 下一交易日盘前重置
```

字段对照 (13 字段):
| 模板字段 | 实测值 | ✓ |
|---|---|---|
| 标题 "🛡️ 账户模式变更（HH:MM）" | "🛡️ 账户模式变更（09:38）" | ✅ |
| old → new | "Normal → ReduceOnly" | ✅ |
| 原因: | "原因:" | ✅ |
| · {trigger_reason_1} | "· 当日亏损 -1.60% 触发降级线 -1.50%" | ✅ |
| 生效限制: | "禁止新开仓/加仓/正T, 候选转影子" | ✅ |
| 解除条件: | "当日盈亏回到 -1.5% 内 或 连续止损 < 3 笔..." | ✅ |

数据准确性:
- ✅ pnl = -1.6% (与 input 一致)
- ✅ 触发线 -1.50% (与 threshold.daily_loss_pct 一致)
- ✅ ReduceOnly 模式正确 (priority Frozen > ReduceOnly > Normal)

### T-02 数据状态变更 (✅ 模板对齐 §14.1)

```
📡 数据状态变更（09:38）
Full → Unsafe
受影响: Quote/Kline/MoneyFlow/News
输出限制:
· 不做盘口承接判断
· 禁出价格型建议
· 仅保留风险类推送
```

字段对照:
| 模板字段 | 实测值 | ✓ |
|---|---|---|
| 标题 "📡 数据状态变更（HH:MM）" | "📡 数据状态变更（09:38）" | ✅ |
| old → new | "Full → Unsafe" | ✅ |
| 受影响: | "Quote/Kline/MoneyFlow/News" | ✅ |
| 输出限制: | "· 不做盘口承接判断 / · 禁出价格型建议 / · 仅保留风险类推送" | ✅ |

数据准确性:
- ✅ 输入 5 个 cap 全 staleness=200 (超过 120s 阈值)
- ✅ Quote staleness > 120s 触发 Unsafe (优先级最高)
- ✅ 4 个 critical cap 全部列入 missing

### T-03 持仓建议 (✅ 模板对齐 §14.1)

```
🎯 持仓建议 测试持仓(000001)（13:42）
动作倾向: 逢高减仓 | 现价12.30 成本11.80 可用3000股
减仓观察区: 12.45~12.60
支撑11.95 | 压力12.70 | 硬止损11.95
无效条件:
· 跌破5日线且放量
· 板块热度转Fade
理由: 放量冲高回落; 主力净流出0.8亿
辅助建议, 非下单指令
```

字段对照 (8 个):
| 模板字段 | 实测值 | ✓ |
|---|---|---|
| 标题 "🎯 持仓建议 name(code)（HH:MM）" | "🎯 持仓建议 测试持仓(000001)（13:42）" | ✅ |
| 动作倾向: {intent} \| 现价{price} 成本{cost} 可用{avail}股 | "动作倾向: 逢高减仓 \| 现价12.30 成本11.80 可用3000股" | ✅ |
| 减仓观察区: {lo}~{hi} | "减仓观察区: 12.45~12.60" | ✅ |
| 支撑{support} \| 压力{pressure} \| 硬止损{stop} | "支撑11.95 \| 压力12.70 \| 硬止损11.95" | ✅ |
| 无效条件: | "· 跌破5日线且放量 / · 板块热度转Fade" | ✅ |
| 理由: | "理由: 放量冲高回落; 主力净流出0.8亿" | ✅ |
| 末尾 "辅助建议, 非下单指令" | "辅助建议, 非下单指令" | ✅ |

---

## 三、修复的 Bug 汇总

| Bug | 状态 | 修复 |
|---|---|---|
| A: stock_position.chain_name 列缺失 | ✅ | run_migrations() 加 idempotent ALTER (MVP0-A) |
| 副: trades/prediction_tracker ALTER 失败 (init Err) | ✅ | add_column_if_missing 加 table_exists 守卫 |
| 副: init 错误被 let _ = 吞掉 | ✅ | 显式 .map_err() log |

---

## 四、测试统计

```
cargo test --lib --test-threads=1 → 852 passed / 2 failed (pre-existing flaky deep_analyzer)
- 2 failed 与本次无关 (v12-push-uncertainty-notes.md Q-11)
- v12 新增测试 (本会话累计): 207 个
- 本次新增/修复:
  - risk::account_mode: 20/20 (Bug #1 修复)
  - trading::paper_trade: 9/9 (Bug #2 修复, PaperOutcome 新增)
  - pipeline::position_tracker: 编译通过 (Bug #3 修复, ORDER BY)
  - monitor::data_mode: 19/19
  - decision::t0_advisor: 12/12
  - decision::pre_trade_filter: 17/17
  - decision::holding_plan: 8/8
  - decision::live_plan: 11/11
  - opportunity::candidate_state: 13/13
  - market_analyzer::market_stage_confidence: 12/12
  - market_analyzer::limit_chain_review: 8/8
  - market_analyzer::lhb_review: 6/6
  - market_analyzer::post_close_review: 3/3
  - market_analyzer::performance_feedback: 6/6
```

---

## 五、未做的 (T-04~T-12, R-01~R-08)

按 v12-mvp-progress.md "未做" 列表, MVP-0 整合 PR 不强制要求每个 PushKind 都触发一次 — 验证的是**模板/治理/数据通路**, 真实调度交给 monitor_loop 按时间节奏挂.

未触发的 PushKind 模板:
- T-04 持仓紧急风险 (无跌破硬止损的测试数据)
- T-05/T-06 做T (无持仓可做T)
- T-07 候选触发 (需 ENABLE_CANDIDATE_LIVE=true)
- T-08 候选失效 (需 T-07 触发)
- T-09 禁止操作 (无距涨停近 + 排雷命中的样本)
- T-10 虚拟盘成交 (需 paper_trade.simulate() 调用)
- T-11 竞价异动 (周日, 非交易日)
- T-12 尾盘决策 (周日)
- R-01/R-03~R-08 盘后 (周日)

下一步: monitor_loop 加每分钟调一次 evaluate_*_hook(), 主循环逐日跑, 这 14 个 PushKind 会自然触发.

---

## 六、推荐下一步

1. **真实盘前挂载**: monitor_loop() 在 09:25/09:30/13:00/14:30 等关键时点调 evaluate_*_hook()
2. **T-04 紧急风险接入**: 接入 PositionSizer/StopLoss 信号, 跌破硬止损自动推 T-04
3. **T-10 虚拟盘**: 接入 prediction_tracker T+1 验证, 自动 simulate 模拟成交
4. **R-01~R-08 盘后**: 19:00 调一次, 21:00 龙虎榜补全后调 R-04
5. **MVP-3 转正**: 等 ENABLE_CANDIDATE_LIVE=true + 30 样本后, 触发 T-07 候选触发推送

---

## 七、最终 Git 历史 (本会话累计)

```
7d7f0f4 feat(v12 MVP0-C): STOCK_LIST 默认 34 只 + Bocha/SerpAPI/Tavily 临时禁用
59e3824 feat(v12 MVP0-B): run_test_scan 挂 v12 orchestrator + 修 table_exists
0b0e0fa fix(v12 MVP0-A): run_migrations() 加 v12 表 idempotent CREATE (Bug A 修复)
6b7db41 docs(v12): monitor --test 实盘验证报告 + 修复方案
930f45f fix(v12): 三个 critical bug 修复 (#1 Frozen降级 / #2 paper_trade假成功 / #3 chain非确定性)
de60016 feat(v12 MVP-2~5): 做T/候选转正/盘后增强/反馈进化
5f2b637 feat(v12 PR4): holding_plan + live_plan + T-03/T-04 推送接通 (MVP-1 收官)
5ca2d07 feat(v12 PR3): 虚拟盘 + 影子候选 + 集中度接入
f19e85b feat(v12 PR2): DataMode + 排雷清单 + 承接护栏
9d9ce87 feat(v12 PR1): AccountMode + ActionGate + T-01 推送接通
45f668e feat(v12): 推送模板系统接入 push_templates + PushKind 治理
```

EOF