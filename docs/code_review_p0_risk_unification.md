# P0 风控统一改进 — 代码审查报告 (Step 5)

> 日期: 2026-06-21 | 审查范围: P0-1 ~ P0-4 全部变更

## 审查摘要

| 指标 | 数值 |
|------|------|
| 审查文件数 | 14 (7 新增 + 7 修改) |
| 发现 Bug 数 | 7 (3 严重 + 3 中等 + 1 轻微) |
| 数据红线违规 | 0 |
| 旧模块接入完整性 | 通过 (无遗漏) |

---

## 发现清单

### 🔴 严重 (需立即修复)

| # | 文件 | 行 | 问题 | 触发场景 |
|---|------|-----|------|---------|
| B1 | `review/factor_ic.rs` | 230,237 | 滚动窗口 20 < Spearman 要求 30 → FactorIC 分析不产生任何输出 | 任何 `--review` 运行, `compute_rolling_ic` 始终返回 None |
| B2 | `pipeline/position_tracker.rs` | 174 | T+1 锁仓仅阻止止损卖出,不阻止铁律3/4/5卖出 → 当日买入可当日平仓 | 同日买入触发跌破5日线/14天超时/布林减仓时错误平仓 |
| B3 | `pipeline/mod.rs` | 1447-1448 | ATR 使用 max(high-low) 而非均值 → 止损过宽 | use_dynamic=true 时止损价比预期宽,保护减弱 |

### 🟡 中等

| # | 文件 | 行 | 问题 |
|---|------|-----|------|
| B4 | `risk/veto_rules_live.rs` | 250-251 | bias_rate_enabled/bearish_alignment_enabled 是 OR 关系,无法独立禁用 |
| B5 | `pipeline/position_tracker.rs` | 106,362 | `data[0]` 在空切片时 panic,且未被 catch_unwind 覆盖 |
| B6 | `review/factor_report.rs` | 83-93 | 因子被过滤后相关性矩阵索引错位 (当某因子样本 <30) |

### 🟢 轻微

| # | 文件 | 行 | 问题 |
|---|------|-----|------|
| B7 | `review/factor_report.rs` | 143 | `truncate_name` 在 max_len < 2 时 usize 下溢 |

---

## 旧模块接入检查表

依照 AGENTS.md 第 5 步 MUST 要求:

- [x] 列出所有与新能力同类/相关的现有模块 (9 个模块已列)
- [x] 对每个旧模块回答是否应升级接入新能力:
  - `pipeline/veto_rules.rs` → 不接入 (互补,非重复)
  - `monitor/risk.rs` → 不接入 (卖出侧 vs 买入侧)
  - `risk/stop_loss.rs` → 不接入 (已通过 position_tracker 接入)
  - `risk/limits.rs` → 不接入 (组合级 vs 个股级)
  - `pipeline/position_tracker.rs` → ✅ 已升级 (P0-2)
  - `review/falsify.rs` → 不适用
  - 其余 → 独立关注点
- [x] 确认无"应接入却遗漏"的旧模块

---

## 数据红线检查表

依照 AGENTS.md 第 5 步 MUST 要求:

- [x] **2.1 生产路径禁 mock** — 通过。所有数据来自真实 DB/API,无 mock 残留
- [x] **2.2 缺失数据不静默填充** — 通过。VetoChain 规则数据缺失时 pass-through + warn
- [x] **2.3 坏数据校验** — 通过。价格 >0 检查,NaN/Inf 过滤
- [x] **2.4 数据时效** — 待后续验证。hardcode 的 5s/30s 阈值需逐链路核查
- [x] **2.5 测试实盘隔离** — 通过。所有测试使用 TEST_CODE 前缀
- [x] **2.6 写入/下单防护** — 通过。PositionSizer 有单票上限,但无券商实盘接口 (后续)
- [x] **2.7 审计留痕** — 通过。VetoChain 拦截记录到日志,position 交易留痕到 DB

---

*下一步: Step 6 修复审查问题*
