# 回测缺失分析 (from Explore agent)

**核心定性**: selection_backtest 故意是"raw shadow-outcome reporter"，不是 execution backtest
- src/bin/selection_backtest.rs:0-15,57-78 — 显式 read-only
- src/bin/selection_backtest.rs:127-142 — 禁止任何 order/write 路径
- src/selection/report.rs:443-460 — 报告拒绝 success-rate/recommendation
- docs/business_rules.md:8 (BR-157) — raw 必须 calibration 后才能渲染成功率

**Selection 报告**: 只聚合 count/missing + median/Q25/Q75 + MFE/MAE/volume
- src/selection/report.rs:19-44, 170-188, 199-235
- **缺**: portfolio/equity/Sharpe/Sortino/Calmar/drawdown/trade log/holding distribution/raw-vs-cost returns

**结论性"故意" vs "缺口"**:
| 维度 | 状态 |
|------|------|
| 滑点/佣金/印花税/过户费/spread/tick/集合竞价 | 缺口 |
| 仓位/现金/风险/板块/仓位数/再投入/T+1 强平 | 缺口 |
| D1 next trading day + 只用 settled 数据 | 故意正确 |
| missing bar → ExpectedWait | 故意 |
| 停牌 (suspension) 未单独分类 | 缺口 |
| 涨跌幅处理 (主板 ±10%, 创/科 ±20%, ST ±5%) | 缺口 (quality.rs:208-217 只手工 ±20% 邻接拒绝) |
| 未复权数据 (unadjusted) | 故意 (quality.rs:264-268) |
| 前收盘 mismatch 标识 split continuity | 有 (quality.rs:194-205) |
| **红利/拆细 total-return 调整** | 缺口 (返回受 corporate action 扭曲) |
| Point-in-time lineage (T0 immutable baseline + D1) | 强 (outcome.rs:239-250, 358-376) |
| stale/future quote 检查 | 有 (quality.rs:120-143) |
| visible-only DB join | 有 (selection.rs:1325-1337) |
| 哈希链审计 | 有 (audit.rs:151-159, 390-470) |
| **历史 universe replay / 退市股** | 缺口 (selection.rs:1313-1347, 只选 visible samples) |

**统计严谨性**:
- 报告只有 quantile (report.rs:379-393)，**缺** significance/CI/walk-forward/CV/OOS/baseline/random/buy-hold/版本对比/live reconciliation/失败归因
- 截断 selection.rs:1343-1347 + report.rs:105-111 → 文档未说明 >limit 有偏

**其他回测 binary**:

## boll_macd_backtest
- 复用已 close trades CSV + realized returns (boll_macd_backtest.rs:0-7, 21-29, 165-223)
- *conditional/on-policy selection bias* (用了已发生交易的标的)
- **无** simulated execution/costs/portfolio
- slice data[idx..] 避免 future indicator data (202-220) ✓
- 但 provider fetch 用 current history，无 PIT vintage (179-214)
- 报告: count / win rate / avg returns / P-L (243-318)
- baseline = original A sample, 无 stat test

## rsi_optimize
- 14 preset in-sample on handpicked 30-stock pool (rsi_optimize.rs:17-58, 279-307, 349-382)
- **无** correction/CV/OOS/survivorship correction
- 引擎真实: next-open 避 lookahead, 拒 next-day 涨跌停/停牌 (strategy/rsi/standard.rs:652-665, 731-751)
- 模拟 cash, position sizing, slippage, commission, stamp tax (702-706, 884-960, 967-1015)
- 仍**缺**: spread/tick/call-auction/transfer fee/板块 partial fill
- 涨跌停都认为不可成交 (双方向)
- 每只股独立账户 + 简单平均 (optimize.rs:147-195) → 无共享 portfolio / sector / concurrent / 再投入
- Win calc compare sell vs avg_cost **不计费** (optimize.rs:82-117)，但 final value **含费** → 内部不一致

## winrate_simulator
- retrospective blacklist filtering, **不是** trading simulation (winrate_simulator.rs:0-15, 171-215)
- min sample = 5 (110, 219-239) — 门槛过低
- 336-375 提示 operator 提供 95% CI，但**代码无 CI/test**
- 推荐 + 动态 priority 都在同一段历史优化 (258-375) → multiple-comparison/overfit
- produce_winrate_samples 只选 verified non-null prediction_tracker 行 (108-159) → attrition bias
- 校验 |change|>20 但**无** corporate-action 处理 (130-159)

---

## 综合定性 (Bottom line)

仓库里"回测"实际是 3 套不同东西：

1. **selection_backtest** — 故意保守的 fixed shadow-outcome audit (BR-157)
2. **boll_macd_backtest / winrate_simulator** — 快速回顾分析，selection bias 重
3. **rsi_optimize / RsiBacktest** — 最接近真实 sim (cash, sizing, next-open, slippage, commission, stamp tax, 涨跌停/停牌 filter)，但仍缺共享 portfolio + 严格 OOS

**最大的缺口不是某项费率，而是缺一个 unified PIT universe-level portfolio backtest harness**：
- replay 不同版本的 selection
- 模拟 A 股执行规则
- 共享 cash & positions
- 严格 baseline / CI / walk-forward

---

## 已有的正控制 (不要破)

1. Immutable T0 baseline + D1 lineage (outcome.rs:239-250, 358-376)
2. 只用 settled trading-day evidence (outcome.rs:334-352, quality.rs:147-220)
3. 显式 missing-outcome 计数 (report.rs:128-167)
4. visibility-gated samples (database/selection.rs:1325-1337)
5. Append-only hash chain audit (audit.rs:151-159, 390-470)
6. RSI engine next-open anti-look-ahead (standard.rs:652-665, 731-751)
7. RSI slippage + 基本费率 (standard.rs:884-960)
