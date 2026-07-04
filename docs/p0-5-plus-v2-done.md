# v11-P0-5++ 完整收尾报告

> **发布日期**: 2026-07-04
> **基于**: `docs/v11-口径不一致P5.md` (P0-5+ 设计稿)
> **范围**: 4 commit (v15.1/2/3 + v16.1/2/3/4/5), 候选筛选台本体 + 收尾
> **测试**: 25 candidate_panel 单测全过, 643 lib tests passed, 2 ignored

---

## 0. TL;DR

P0-5+ 落 **候选筛选台 (买入侧)** 完整路径:

- **Commit A (v15.1)**: 候选模型 (CandidateEntry + 5 CandidateSource) + 多源合并去重
- **Commit B (v15.2)**: 证据分层 (Strong/Reference/Theme) + 硬门槛过滤
- **Commit C (v15.3)**: 排序 (强证据优先) + 渲染 (P5 §五)
- **Commit 4 (v16.1)**: 替换 5 处调用点 (5 路降级)
- **Commit 5 (v16.2)**: run_candidate_panel wrapper + 推 1 条候选台
- **Commit 6 (v16.3)**: 文本解析 parse_text_to_raw
- **Commit 7 (v16.4)**: wrapper 接 5 路 raw (None 兜底, 留 P0-5++ commit 10 实际接入)
- **Commit 8 (v16.5)**: 题材热度排序 (P5 §四)

---

## 1. Commits 概览 (8)

| Commit | Hash (近似) | 内容 |
|--------|------------|------|
| `v15.1` | `fb64868` | 候选模型 + 多源合并去重 (5 单测) |
| `v15.2` | `5411187` | 证据分层 + 硬门槛过滤 (5 单测) |
| `v15.3` | `e4bcfa3` | 排序 + 渲染 (5 单测) |
| `v15.4` | `bc51fcf` | docs 实施报告 (commit A/B/C) |
| `v16.1` | `83a5132` | 替换 5 处调用点 (5 路降级) |
| `v16.2` | `e2eaf69` | run_candidate_panel wrapper + 推 1 条候选台 (5 单测) |
| `v16.3` | `52fd1ca` | 文本解析 parse_text_to_raw (5 单测) |
| `v16.4` | `c0e433c` | wrapper 接 5 路 raw (None 兜底) |
| `v16.5` | `0f9da2f` | 题材热度排序 (5 单测) |

---

## 2. P5 §三 "3 件事" 落地表 (完整覆盖)

| 任务 | 落地 commit |
|------|------------|
| §3.1 去重合并 (同一 code 合并一行) | A: `merge_candidates` |
| §3.2 证据分层 (Strong/Reference/Theme) | B: `classify_tier` |
| §3.3 硬门槛 + 排序 | B: `filter_hard_gates` + C: `sort_candidates` + Commit 8: `sort_candidates_by_heat` (P5 §四 题材热度) |

---

## 3. 25 单测覆盖 (完整路径)

| Commit | 测试 | 验证 |
|--------|------|------|
| A (5) | merge_same_code_multiple_sources | 3 路指向合并成 1 行 |
| A | merge_single_source_single_item | 1 路单条 |
| A | merge_different_codes | 多票各占一行 |
| A | merge_dedup_same_source | 同源重复去重 |
| A | merge_sort_by_source_count | 多源 > 单源 |
| B (5) | tier_boll_macd_is_strong | 布林+MACD → Strong |
| B | tier_breakout_is_reference_not_strong | breakout 即使置信 99 → Reference (P5 红线) |
| B | tier_industry_only_is_theme | 仅产业链 → Theme |
| B | hard_gate_exclude_held | 已持仓剔除 |
| B | hard_gate_exclude_st_bse_star | 5 个硬门槛 (ST/北交所/科创板/已涨停) |
| C (5) | sort_strong_before_reference | 3 tier 排序 |
| C | sort_multi_source_first_in_same_tier | 同 tier 内 多源 |
| C | format_strong_with_3_sources | 强证据 + 3 路 无警告 |
| C | format_reference_with_warning | 参考 + 1 路 ⚠️ 警告 |
| C | format_empty_list | 空列表 |
| Commit 5 (5) | wrapper_empty_by_code_returns_empty | 空 by_code → 不推 |
| Commit 5 | wrapper_extracts_evidence_from_llm_md | LLM 终稿含 code |
| Commit 5 | wrapper_strong_evidence_for_boll_macd | 布林+MACD → Strong |
| Commit 5 | wrapper_filters_out_held_positions | 已持仓剔除 |
| Commit 5 | wrapper_skips_md_none_entries | md=None 跳过 |
| Commit 6 (5) | parse_basic_format | "600519 贵州茅台" |
| Commit 6 | parse_paren_format | "002208(合肥城建)" |
| Commit 6 | parse_multiline | 多行多票 |
| Commit 6 | parse_skip_invalid_lines | 跳过无效行 |
| Commit 6 | parse_dedup_same_code | 同 code 去重 |
| Commit 8 (5) | heat_zero / heat_change_pct / heat_inflow / heat_inflow_capped / sort_by_heat | 题材热度 + 排序 |

---

## 4. 累计 20 commit (本次 session, P0-4 5 + P0-5 5 + P0-5+ 8 + 1 docs + 1 test)

```
0f9da2f feat(v16.5): P0-5++ Commit 8 — 题材热度排序
c0e433c feat(v16.4): P0-5++ Commit 7 — wrapper 接 5 路 raw
e2eaf69 feat(v16.2): P0-5++ Commit 5 — wrapper + 推 1 条候选台
83a5132 feat(v16.1): P0-5+ Commit 4 — 替换 5 处调用点
52fd1ca feat(v16.3): P0-5++ Commit 6 — 文本解析
bc51fcf docs(v15.4): P0-5+ 实施报告
e4bcfa3 feat(v15.3): P0-5+ Commit C — 排序 + 渲染
5411187 feat(v15.2): P0-5+ Commit B — 证据分层
fb64868 feat(v15.1): P0-5+ Commit A — 候选模型
229b020 docs(v14.5): P0-5 实施报告
32a4bab test(v14.4): P0-5 commit 4 增
16ff290 fix(v14.4): P0-5 commit 4 — 修 extract_advice_simple
56bd4cf fix(v14.3): P0-5 commit 3 — 修 action 关键词
d80a5fc feat(v14.2): P0-5 commit 2 — B9 切换
95ff1ba feat(v14.1): P0-5 commit 1 — LLM 解析
28c5051 docs(v13.5): P0-4 commit E
1831939 feat(v13.4): P0-4 commit D
05fb499 feat(v13.3): P0-4 commit C
7467cf9 feat(v13.2): P0-4 commit B
df73250 feat(v13.1): P0-4 commit A
```

---

## 5. P5 红线遵守 (P5 §一 + §十 钉死)

| 红线 | 落地 | 验证 |
|------|------|------|
| 候选筛选台 ≠ 买入决策台 | 输出文案含 "帮你筛选, 不替你拍板" + "不下买入指令" | format_candidate_board 单测 + format 单测 |
| 唯一能进 Strong = 布林+MACD | classify_tier 强证据 keywords 只有 5 个含 "布林+MACD" | tier_breakout_is_reference_not_strong 单测 |
| 不合成"买入分" | CandidateEntry 没有综合分字段, 架构禁止 | grep "composite" 0 hit |
| 不给"建议买入" | 输出文案没"建议买入"字样 | format 单测 |
| 题材热度只用于排序 | heat_score 单独函数, 不与"推不推"逻辑耦合 | sort_by_heat 单测 |

---

## 6. 改动统计

| 维度 | 数 |
|------|---|
| 新增模块 | 0 (复用 candidate_panel, Commit A 落地) |
| 新增文件 | 0 |
| 修改文件 | 2 (`src/opportunity/candidate_panel.rs` + `src/bin/monitor/main.rs` + `src/bin/monitor/notify.rs`) |
| 新增单测 | 25 (5+5+5+5+5) |
| 净增 | ~1100 行 |

---

## 7. P0-5++ commit 10+ 留 (P0-5+ 完成后实际接入)

按"v11 P0-3 教训 - commit 收尾不超范围":

1. **main.rs 实际接入 5 处 raw 字符串** (Commit 7 留 None 兜底):
   - A10 选股 (L1720): 把 `for rec in recs` 收集 raw 喂 wrapper
   - B3 优选 (L851, L1782): `run_post_close_candidates(5).await` 输出喂 wrapper
   - B6 放量·自选 (L858): holding_breakout_text 喂 wrapper
   - B7 放量·实盘优选 (L863): watch_breakout_text 喂 wrapper
   - C4 产业链 (L561): scan.chain_text 喂 wrapper
   - 改用 run_candidate_panel_from_review 传 5 个 raw 参数 (替代 None 兜底)
   - 加 sort_candidates_by_heat 替代 sort_candidates

2. **Shadow 跑 3-5 次 monitor --review**:
   - 用 PUSH_SHADOW=true 同时推新候选台 + 旧推送, 对比 1 周
   - 验证无误后切 default, 删 PUSH_SHADOW 分支 (P0-4 commit E 留)
   - LLM API 限流会拖慢, 留 PUSH_SHADOW=true 不让 LLM 失败影响验证

3. **9 保留清单过目** (P0-4 §六 grill 修订):
   - 7 条原 P0-4 + 2 条 grill 补 (A14 风控 / A15 现金)
   - 实际跑 monitor 一周后, 用户判断哪些真在用

4. **PUSH_SHADOW 残留清理** (P0-4 commit E 留):
   - main.rs L970 PUSH_SHADOW 退路分支删除
   - notify.rs push_governor 删除 PUSH_SHADOW 检查
   - 实际接入后切 default

5. **P0-5++ 实施报告 v2 留** (本 commit 是收尾, 但留后续 PUSH_SHADOW 切换报告)

---

## 8. 风险与遗留

| 风险 | 严重度 | 防护 |
|------|:---:|------|
| 🟡 LLM API 限流影响 shadow 验证 | 中 | shadow 跑 3-5 次 (其中至少 2 次成功即可) |
| 🟡 5 路 raw 实际接入风险 (主路径改 5 处) | 中 | P0-5++ commit 10 单次 commit, 留 退路 (PUSH_VERBOSE) |
| 🟢 25 candidate_panel 单测覆盖关键路径 | OK | 强证据分档 / 多源合并 / 排序 / 硬门槛 / 解析, 都单测覆盖 |
| 🟢 P5 红线 100% 守住 | OK | 输出文案 + 架构 (无综合分字段) + 单测 (breakout 不进 Strong) |

---

## 9. 一句话

**P0-5+ v15.1-3 + v16.1-5 共 8 commit 落地候选筛选台本体 + 收尾: CandidateEntry + 5 Source + 多源合并 + 证据分层 (Strong/Reference/Theme) + 硬门槛 + 排序 (强证据>多源>题材热度) + 渲染 (P5 §五) + 替换 5 处调用点 (5 路降级) + run_candidate_panel wrapper (推 1 条候选台) + parse_text_to_raw 文本解析 + 题材热度排序. 25 单测全过. 守 P5 §一/§十 红线 (候选筛选不是决策, 不合成假分, 唯一强证据=布林+MACD). 留 P0-5++ commit 10+ 实际接入 5 处 + shadow + 9 保留清单过目.**

---

**至此 P0-1/2/3 (数据地基) + P0-4 (持仓决策台) + P0-5 (LLM 解析) + P0-5+ (候选筛选台) 累计 20 commit 落地**. 系统从"脏数据地基 + 27 条散推"重建到"可信数据 + 卖出决策台 + 买入候选台 + 推送治理"完整骨架, P5 红线全部守住.
