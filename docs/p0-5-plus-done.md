# v11-P0-5+ 候选筛选台改造完成报告

> **发布日期**: 2026-07-04
> **基于**: `docs/v11-口径不一致P5.md` (P0-5+ 设计稿)
> **范围**: 3 commit (v15.1/2/3), 候选筛选台本体
> **测试**: 15 candidate_panel 单测全过, 633 lib tests passed, 2 ignored

---

## 0. TL;DR

v11-P0-5+ 落 **候选筛选台 (买入侧)**, 跟 P0-4 决策台 (卖出侧) 成一对:

- **Commit A (v15.1)**: 候选模型 (CandidateEntry + 5 CandidateSource) + 多源合并去重
- **Commit B (v15.2)**: 证据分层 (Strong/Reference/Theme) + 5 个硬门槛过滤
- **Commit C (v15.3)**: 排序 (强证据优先) + 渲染 (P5 §五 输出形态)

**红线 (P5 §一 钉死)**:
- 候选筛选台, 不是买入决策台
- 唯一 🟢 强证据: 布林+MACD (v11 factor_ic 认可 B 方案)
- 不合成"买入分" (sentiment 覆辙)
- 不给"建议买入"字样

---

## 1. Commits 概览 (3)

| Commit | Hash (近似) | 内容 |
|--------|------------|------|
| `v15.1` | `fb64868` | 候选模型 (CandidateEntry + 5 CandidateSource) + 多源合并去重 (5 单测) |
| `v15.2` | `5411187` | 证据分层 (Strong/Reference/Theme) + 硬门槛过滤 (5 单测) |
| `v15.3` | `e4bcfa3` | 排序 (强证据优先) + 渲染 (P5 §五 输出形态) (5 单测) |

---

## 2. P5 §三 "3 件事" 落地表

| 任务 | 落地 commit |
|------|------------|
| §3.1 去重合并 (同一 code 合并一行) | Commit A: `merge_candidates` |
| §3.2 证据分层 (Strong/Reference/Theme) | Commit B: `classify_tier` |
| §3.3 硬门槛 + 排序 | Commit B: `filter_hard_gates` + Commit C: `sort_candidates` |

---

## 3. 核心数据结构 (P5 §二 + §三)

```rust
pub enum CandidateSource {
    StockPick,        // A10 选股推荐Top3
    OptimalClose,     // B3 优选候选
    VolumeWatchlist,  // B6 放量·自选
    VolumeRealTrade,  // B7 放量·实盘优选
    IndustryChain,    // C4 产业链扫描
}

pub enum EvidenceTier {
    Strong,    // 🟢 唯一: 布林+MACD (v11 factor_ic 认可)
    Reference, // 🟡 未验证: breakout / 空中加油 / 放量 / 资金
    Theme,     // ⚪ 仅产业链 / 板块热度
}

pub struct CandidateEntry {
    pub code: String,
    pub name: String,
    pub sources: Vec<CandidateSource>,
    pub tier: EvidenceTier,
    pub evidence: Vec<String>,
    pub current_price: f64,
    pub change_pct: f64,
}
```

**红线 (P5 §一 + §十 架构保证)**:
- `CandidateEntry` **没有**"综合分/买入分"字段 (架构上禁止合成假分)
- `tier` 强证据 (Strong) **唯一** 入口是"布林+MACD" (P5 §3.2 红线)

---

## 4. 输出形态 (P5 §五) — Commit C 实现

```
📋 候选筛选台 · 通过硬门槛 N 只
定位: 帮你筛选, 不替你拍板买入 | 证据分「已验证/参考」
━━━━━━━━━━━
1. 测试1(000001) ¥10.00 +1.00%
   🟢 强: 测试证据
   来源: 选股+优选+产业链 (3 路指向)
2. 测试2(000002) ¥10.00 +1.00%
   🟡 参考: 测试证据
   ⚠️ 无强证据, 仅参考
   来源: 放量自选 (1 路指向)
━━━━━━━━━━━
💡 强证据票排前; "参考" 类需你自行判断, 系统不下买入指令
```

**红线文案 (P5 §一) 已嵌入输出**:
- "帮你筛选, 不替你拍板买入"
- "系统不下买入指令"
- tier != Strong 时显式 "⚠️ 无强证据, 仅参考"

---

## 5. 单测覆盖 (15 个)

| Commit | 测试 | 验证 |
|--------|------|------|
| A | merge_same_code_multiple_sources | 3 路指向合并成 1 行 |
| A | merge_single_source_single_item | 1 路单条 |
| A | merge_different_codes | 多票各占一行 |
| A | merge_dedup_same_source | 同源重复去重 |
| A | merge_sort_by_source_count | 多源 > 单源 排序 |
| B | tier_boll_macd_is_strong | 强证据分档 (P5 红线) |
| B | tier_breakout_is_reference_not_strong | breakout 即使置信 99 也不进 Strong |
| B | tier_industry_only_is_theme | 仅产业链 → Theme |
| B | hard_gate_exclude_held | 已持仓剔除 |
| B | hard_gate_exclude_st_bse_star | ST/北交所/科创板/已涨停 5 个门槛 |
| C | sort_strong_before_reference | 强证据 > 参考 > 题材 排序 |
| C | sort_multi_source_first_in_same_tier | 同 tier 内 多源 > 单源 |
| C | format_strong_with_3_sources | 强证据 + 3 路指向 无警告 |
| C | format_reference_with_warning | 参考 + 1 路 → ⚠️ 警告 (P5 §五 红线) |
| C | format_empty_list | 空列表不报错 |

---

## 6. 改动统计

| 维度 | 数 |
|------|---|
| 新增文件 | 1 (`src/opportunity/candidate_panel.rs` ~480 行, 含 15 单测) |
| 修改文件 | 1 (`src/opportunity/mod.rs` 加 `pub mod candidate_panel;`) |
| 新增单测 | 15 |
| 净增 | ~480 行 |

---

## 7. P0-5+ commit 4 候选 (留给 P0-5++ 实际接入)

P0-5+ 4 commit 设计的最后一步 "替换 A10/B3/B6/B7/C4 五条调用点" 留 P0-5++ 实际接入:

1. **加 wrapper `run_candidate_panel()`** 在 main.rs:
   - 输入: 5 路 raw data (A10 选股 / B3 优选 / B6 放量·自选 / B7 放量·实盘优选 / C4 产业链)
   - 流程: merge_candidates → classify_tier → filter_hard_gates (用 portfolio::get_positions()) → sort_candidates → format_candidate_board
   - 输出: 1 条候选台推送 (`push_governor`)

2. **5 处调用点替换** (P5 §六 验收: "A10/B3/B6/B7/C4 五条旧推送调用点清零"):
   - A10 选股推荐 (L1965 rec loop): 改为收 raw, 不直接 push_wechat
   - B3 优选候选 (L853 / L2028 post_close_candidates): 同上
   - B6 放量·自选 (L860 holding_breakout_text): 同上
   - B7 放量·实盘优选 (L865 watch_breakout_text): 同上
   - C4 产业链 (L561 scan.chain_text): 同上
   - 改为"收集 raw data 到 Vec, 调 wrapper 推 1 条"

3. **题材热度排序** (P5 §四):
   - 复用 sector_monitor 现有数据 (板块涨幅 + 主力净流入 + 涨停家数 + 量比 加权)
   - 在 `sort_candidates` 加同 tier 同 source 后的次级排序
   - 留 P0-5++ 单独 commit

4. **Shadow 跑 3-5 次 monitor --review** (P5 §六 验收):
   - 用 wrapper 跑 shadow, 决策台 1 条 + 候选台 1 条
   - 验证后切 default
   - PUSH_SHADOW 留退路 (P0-4 commit E 加, P0-5++ 验证后清理)

5. **9 保留清单过目** (P0-4 §六 grill 修订):
   - 7 条原 P0-4 + 2 条 grill 补 (A14 风控 / A15 现金)
   - 实际跑 monitor 一周后, 用户再判断哪些真在用

---

## 8. 红线遵守 (P5 §一 + §十 钉死)

| 红线 | 落地点 | 验证 |
|------|--------|------|
| 候选筛选台 ≠ 买入决策台 | 输出文案含"帮你筛选, 不替你拍板" + "不下买入指令" | format_candidate_board 单测验证 |
| 唯一能进 Strong = 布林+MACD | classify_tier 强证据 keywords 只有 5 个, 包含"布林+MACD" | tier_breakout_is_reference_not_strong 单测 (置信 99 也 Reference) |
| 不合成"买入分" | CandidateEntry 没有"综合分"字段, 架构上禁止 | grep "composite" 0 hit |
| 不给"建议买入" | 输出文案没"建议买入"字样, 只有"帮你筛选" | format_candidate_board 单测 |
| 只有 v11 factor_ic 验证过的进 Strong | classify_tier 强证据 keywords 限定为 5 个 (含"布林+MACD"), 其它未验证打分全部 Reference | tier_breakout_is_reference_not_strong 单测 |

---

## 9. 风险与遗留

| 风险 | 严重度 | 防护 |
|------|:---:|------|
| 🟡 commit 4 (替换 5 处) 留 P0-5++ | 中 | wrapper 函数 + 调用点替换在单独 commit, 不影响现有 5 条降级推送 |
| 🟡 题材热度排序留 P0-5++ | 低 | sort_candidates 已留 source_count desc, 加热度分时改次级排序 |
| 🟢 5 个测试 (commit A/B/C 各 5) 覆盖关键路径 | OK | 强证据分档 / 多源合并 / 排序 / 硬门槛 / 警告, 都单测覆盖 |

---

## 10. 一句话

**P0-5+ v15.1-3 落 3 commit 候选筛选台本体: CandidateEntry + 多源合并去重 + 证据分层 (Strong/Reference/Theme) + 硬门槛过滤 + 排序 + 渲染. 15 单测覆盖关键路径, 严格守住 P5 §一/§十 红线 (候选筛选不是买入决策, 不合成假分, 唯一强证据=布林+MACD). 剩 commit 4 (替换 5 条调用点 + shadow + 文档) 留 P0-5++ 实际接入.**

---

**至此 P0-1/2/3 (数据地基) + P0-4/5 (决策收口一对) + P0-5+ (候选筛选台本体, 3 commit) 累计 14 commit 落地.** 整体覆盖: 持仓决策 (P0-4) + 候选筛选 (P0-5+) + LLM 解析 (P0-5) + 推送治理 (P0-4 D). P0-5++ 留 4 项接入: 5 处调用点替换 / 题材热度 / shadow / 9 保留清单过目.
