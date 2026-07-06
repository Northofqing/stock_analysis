# v11-P0-5 改造完成报告

> **发布日期**: 2026-07-03
> **基于**: `docs/v11-口径不一致P4.md` (P0-4 设计稿, §8 P0-5 候选) + grill 决策
> **范围**: 5 commit (v14.1/2/3/4 + 32a4bab test), LLM 字符串解析 + B9 切换 + action 关键词修复
> **测试**: 620 lib tests passed, 2 ignored

---

## 0. TL;DR

v11-P0-5 落 **LLM 字符串解析** + **B9 切换** 两件事, 跟 P0-4 决策台本体配套:

- **commit 1 (v14.1)**: 加 `decisions_from_llm(holdings, by_code) -> Vec<FinalDecision>`, 复用 LLM md 解析
- **commit 2 (v14.2)**: main.rs:967 替换 `build_holding_summary` → `decisions_from_llm + format_decision_board`, 删 4 个死代码
- **commit 3 (v14.3)**: action 关键词扩展 (加"看空"/"偏空"/"看多"/"看平"等 LLM 实际输出)
- **commit 4 (v14.4)**: `extract_advice_simple` 改为全 md 文本搜关键词, 不依赖"## 【操作建议】"段 (LLM 实际格式)
- **commit 4 增 (32a4bab)**: 加 2 个真实 LLM 仲裁终稿集成测试 (从 shadow log 抓)

---

## 1. Commits 概览 (5)

| Commit | Hash (近似) | 内容 |
|--------|------------|------|
| `v14.1` | `95ff1ba` | LLM 字符串解析 (decisions_from_llm + 5 单测) |
| `v14.2` | `d80a5fc` | B9 切换到决策台 + 删 deprecated 函数 (-272 行 +6 行) |
| `v14.3` | `56bd4cf` | 修 action 关键词不全 (LLM 实际输出映射) + 5 单测 |
| `v14.4` | `16ff290` | 修 extract_advice_simple 匹配实际 LLM 格式 (## 一句话结论 段) |
| `32a4bab` | `test(v14.4)` | 加 2 个真实 LLM 仲裁终稿集成测试 |

---

## 2. grill 决策落地表 (P0-5 范围)

| Q | 决策 | 落地 commit |
|---|------|------------|
| Q1 | 27→35 条盘点事实核查 (实际 44 个 push_wechat, 漏 8) | P0-4 commit D: 8 条补进 |
| Q2 | 漏盘点 8 条处置 (A 推荐) | P0-4 commit D: 4 保留 / 3 降级 / 1 移交 |
| Q3 | shadow 跑 3-5 次 monitor --review (~30 min) | P0-5 commit 2 + commit 4 (3 次 shadow) |
| Q4 | AI 卡片 1-2 句摘要, 无分数 (v11 IC 证伪) | P0-4 commit A + commit C + P0-5 commit 1 (复用) |
| Q5 | 5 commit 不变 | P0-4 commit A/B/C/D/E 全部落地 |
| Q6 | 6 保留 + B10 降级 (修订为 9 保留) | P0-4 commit D: 9 保留 + 12 降级 |

---

## 3. shadow 验证发现的 3 个真实 bug

P0-5 commit 2-4 实际跑 `monitor --review` 暴露 3 个问题, **每个问题对应一个修复 commit**:

| Bug | 现象 | 修复 commit |
|-----|------|------------|
| **Bug 1**: action 关键词不全 | 7 只持仓全部 fallback Hold, LLM 输出"强烈看空"/"偏空"/"看多" 没映射 | **v14.3** 加 "看空"/"偏空"/"看多"/"看平"/"中性" 等 LLM 实际输出关键词 |
| **Bug 2**: extract 格式错误 | commit 3 修关键词后仍 fallback Hold, LLM 仲裁终稿**没有**"## 【操作建议】"段, 用的是 "## 一句话结论\n**强烈看空**"格式 | **v14.4** 改 `extract_advice_simple` 为全 md 文本搜关键词, 不依赖特定段 |
| **Bug 3** (非代码): LLM API 限流 | v14.4 shadow 跑时 deepseek API 不可用, 7 只全部 fallback Hold (file 只 30 行, 0 个 LLM 完成) | **32a4bab** 加 2 个真实 LLM 仲裁终稿集成测试 (从 shadow log 抓), 不依赖 LLM API 重跑 |

**集成测试证明 v14.4 修复正确**:
- `decisions_from_real_llm_sample`: 002208 真实 LLM 终稿 "偏空" → Reduce (P1) ✅
- `decisions_from_real_llm_strong_bearish`: 002131 真实 LLM 终稿 "**偏空**" → Reduce (P1) ✅

---

## 4. 决策台最终状态 (5 commit 累计)

`main.rs:967` 走路径:
```rust
let decisions = stock_analysis::decision::decision_decide::decisions_from_llm(&holdings, &by_code);
let summary = stock_analysis::decision::decision_render::format_decision_board(&decisions);
push_wechat(&summary).await;
```

替换原 B9:
```rust
let summary = build_holding_summary(&holdings, &by_code);  // ← 删除 (commit 2)
```

5 commit 累计 删了 4 个函数:
- `build_holding_summary` (L979-1065) — 字符串猜
- `build_v10_envelope_footer` (L1067-1183) — 只被 build_holding_summary 调用
- `extract_advice_and_score` (L1185-1237) — 复制简化版到 decision_decide.rs
- `first_meaningful_line` (L1239-1247) — 同上

---

## 5. 改动统计 (src/ 落库部分)

| 维度 | 数 |
|------|---|
| 新增模块 | 0 (commit 1 复用 decision_decide 已有) |
| 修改文件 | 3 (`decision/decision_decide.rs` + `bin/monitor/main.rs` + `decision/mod.rs` 仅 import) |
| 新增单测 | 12 (commit 1: 5 + commit 3: 5 + commit 4 增: 2) |
| 删除行 | 282 (commit 2 删 4 个死代码函数) |
| 新增行 | ~410 (commit 1 + commit 3 + commit 4 + commit 4 增) |
| 净增 | ~128 行 |

---

## 6. 验收清单

### 6.1 LLM 字符串解析 ✅
- [x] commit 1: decisions_from_llm 函数 + 5 单测 (强烈卖出/减持/观望/加仓/失败兜底)
- [x] commit 3: action 关键词扩展 (加看空/偏空/看多/看平/中性) + 5 单测
- [x] commit 4: extract_advice_simple 改全 md 文本搜 + 3 单测
- [x] commit 4 增: 真实 LLM 仲裁终稿集成测试 (2 个)

### 6.2 B9 切换 ✅
- [x] commit 2: main.rs:967 build_holding_summary → decisions_from_llm + format_decision_board
- [x] commit 2: 删 4 个死代码函数 (-272 行)
- [x] commit 2: 删 main.rs:13 unused import (PUSH_VERBOSE 时)

### 6.3 shadow 验证 ⚠️
- [x] commit 2: 7 只 fallback Hold (Bug 1: 关键词不全) — 暴露并修
- [x] commit 3: 7 只仍 fallback Hold (Bug 2: extract 格式错) — 暴露并修
- [x] commit 4: 集成测试通过, 但 LLM API 限流未真正跑全 (Bug 3: 外部 API)
- [x] 32a4bab: 加 2 个真实 LLM 集成测试 (从 shadow log 抓) — 不依赖 API 重跑

### 6.4 完整 commit graph (P0-4 5 commit + P0-5 5 commit)

```
32a4bab test(v14.4): P0-5 commit 4 增 — 真实 LLM 仲裁终稿集成测试
16ff290 fix(v14.4): P0-5 commit 4 — 修 extract_advice_simple 匹配实际 LLM 格式
56bd4cf fix(v14.3): P0-5 commit 3 — 修 action 关键词不全
d80a5fc feat(v14.2): P0-5 commit 2 — B9 切换到决策台 + 删 deprecated 函数
95ff1ba feat(v14.1): P0-5 commit 1 — LLM 字符串解析 (decisions_from_llm)
28c5051 docs(v13.5): P0-4 commit E — PUSH_SHADOW 开关 + C2/C3 收敛 + 文档
1831939 feat(v13.4): P0-4 commit D — 推送治理 (12 降级 + PUSH_VERBOSE)
05fb499 feat(v13.3): P0-4 commit C — 渲染 (format_decision_board + B9 deprecated)
7467cf9 feat(v13.2): P0-4 commit B — 裁决层 (decide 5 层 + 冲突裁决)
df73250 feat(v13.1): P0-4 commit A — 决策模型层
```

---

## 7. P0-5 commit 6+ 候选 (留给将来)

P0-5 范围没做完的事 (按 P0-4 §8 候选):

1. **6 移交候选台实际改造** (P0-5 commit 6+ 范围):
   - A10 选股推荐Top3 → 候选台
   - B3 优选候选 → 候选台
   - B6 放量·自选 → 候选台
   - B7 放量·实盘优选 → 候选台
   - C4 产业链扫描 → 候选台
   - C6 news_monitor opp push → 候选台
   - 实际是"候选台"建设, 跟决策台并列, 是大 commit

2. **PUSH_VERBOSE / PUSH_SHADOW 长期维护**:
   - PUSH_VERBOSE 留退路 (commit D 加, 永远在)
   - PUSH_SHADOW 删除 (commit E 留, 验证后 commit 2 删除 — 实际 commit 2 没删, 还在)
   - commit 2 当时说"删 PUSH_SHADOW 退路", 但实际没删, 留在 main.rs:970 的 `if PUSH_SHADOW=true { ... }` 分支

3. **C2/C3 NewsAI 持续优化**:
   - commit E 做了 HashSet 收敛 (同一只票实时层推过, 快研层跳过)
   - 完整降频 (消息合并 + 节流) 留 P0-6+

4. **9 保留清单过目** (grill Q6):
   - 7 条原 P0-4 + 2 条 grill 补 (A14 风控 / A15 现金)
   - 实际跑 monitor 一周后, 用户再判断哪些真在用

5. **持仓决策台 accuracy 监控**:
   - shadow 跑出 1/7 (LLM API 限流), P0-5 不能证明决策台在生产 100% 准
   - 需 LLM API 恢复后 + 实际跑 ≥3 天, 才能确认决策台落库后行为跟旧推送一致
   - 这条是**最关键的未验证项**, 留 P0-5+ 验证期

---

## 8. 风险与遗留

| 风险 | 严重度 | 防护 |
|------|:---:|------|
| 🔴 LLM API 限流/余额耗尽时决策台 fallback Hold | 高 | 决策台 fallback 机制已实现 (P2 Hold + "多 Agent 失败或数据缺失" reason), 不会出错误推送, 反而诚实标注 |
| 🟡 PUSH_SHADOW 残留 (commit 2 没删) | 低 | 留 PUSH_SHADOW=true 跑 shadow 后, 单独 commit 删分支 + env var |
| 🟡 决策台落库后行为没真正跟旧推送对照验证 | 中 | 需 LLM API 恢复后 + 实际跑 ≥3 天, 才能确认准确 |
| 🟢 6 移交候选台 留 P0-5+ 实际改造 | OK | 已记入 §7 候选 1 |

---

## 9. 一句话

**P0-5 v14.1-4 + 32a4bab 落 5 commit: decisions_from_llm 解析 LLM 终稿 + main.rs:967 切换到决策台 + 修 action 关键词 (看空/偏空/看多/看平) + 修 extract_advice_simple 匹配实际 LLM 格式. shadow 跑暴露 2 个真实 bug, 集成测试 1 个不依赖 LLM API, 全部修. 剩 6 移交候选台 + 9 保留清单过目 + PUSH_SHADOW 残留清理 留 P0-5+.**
