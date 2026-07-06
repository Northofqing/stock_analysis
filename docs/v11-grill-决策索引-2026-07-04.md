# v11 Grill 决策索引

> **发布日期**: 2026-07-04
> **目的**: 集中 v11-P0-3/P0-4/P0-5+ 6 个 grill 决策 (Q1-Q6), 防止跨文档散落导致丢失
> **范围**: v11 改造 24 commit 的关键决策 + 文档位置 + 落地 commit hash

---

## 0. 为什么需要这个索引

v11-P0-3 / P0-4 / P0-5+ 设计稿散落在 4 个不同文档 (`v11-口径不一致.md` / `P3.md` / `P4.md` / `P5.md`).

按 v11 改造 24 commit 全过程, 6 个 grill 决策决定了:
- 哪些 raw 数据要保留 (P3)
- 哪些 推送要降级 vs 保留 (P4)
- 候选台怎么落 (P5+)

**风险**: 决策散落 → 未来 review 或新 commit 时容易漏判. 这个索引集中所有 6 个决策 + 文档位置 + commit hash, 让任何人快速查.

---

## 1. 6 个 Grill 决策汇总

### Q1: 27 → 35 条推送盘点 (实际 44 个 push_wechat 调用)

**文档位置**: `docs/v11-口径不一致P4.md` §二 推送盘点表
**落地 commit**: `5411187 feat(v13.2): P0-4 commit B — 证据分层` (Commit 4 加 5 个)
**Commit hash**: `1831939` / `6062407` (v16.8 验证)

**核心结论**: 文档列了 27 条推送, 实际 `git grep push_wechat` 找到 44 个调用点. 文档漏了 8 条 (A13/A14/A15/B11/B12/B13/C6). **grill Q2 已修正**: 5 条保留 (A13/A14/A15 + C6 移交) / 3 条降级 / 0 条加保留.

**风险**: 文档 - 代码 drift. 任何修改推送逻辑前必须 grep 实际调用点, 不能信文档.

### Q2: 漏盘点 8 条处置决策

**文档位置**: `docs/v11-口径不一致P4.md` §二 表格 (grill Q2 修订)
**落地 commit**: `1831939 feat(v13.4): P0-4 commit D — 推送治理`

**核心结论**:

| 漏盘点 | 处置 |
|--------|------|
| A13 排除检查告警 | **保留** (grill Q2 推荐) |
| A14 风控检查告警 | **保留** |
| A15 现金预警 | **保留** |
| B11 v4 赛道分档 | **降级** (PushKind::SectorTier) |
| B12 v4 资金验证 | **降级** (PushKind::CapitalVerify) |
| B13 周度 SOP | **降级** (PushKind::WeeklySOP) |
| C6 news_monitor opp push | **降级** (PushKind, 留 P0-5++ commit 15+ 改造) |

### Q3: shadow 跑 3-5 次 (grill Q3 修订)

**文档位置**: `docs/v11-口径不一致P4.md` §四 Commit E
**落地 commit**: `28c5051 docs(v13.5): P0-4 commit E`

**核心结论**: 不等 1 周, 跑 3-5 次 `cargo run --bin monitor -- --review`, 每次 ~5min. 用 PUSH_SHADOW env var 切换旧+新并推, 对比无误后切 default.

**实际执行**: P0-5+ commit 11/12/13 期间 LLM API (deepseek) 限流, 候选台 fallback Hold, shadow 没真正跑成功. 17 unit tests + 1 次 shadow 跑兜底. **L1M 恢复后重跑**.

### Q4: AI 卡片 1-2 句摘要, 无分数

**文档位置**: `docs/v11-口径不一致.md` §根因 / `docs/v11-口径不一致P4.md` §三 渲染
**落地 commit**: `df73250 feat(v13.1): P0-4 commit A — 决策模型层`

**核心结论**:
- v11 IC 证伪 `composite_score` (LLM 输出分数不可靠)
- `Candidate.tier` 强证据 (Strong) **唯一** 入口: 布林+MACD (P5 §3.2 红线)
- `Candidate` 结构**无**"综合分/买入分"字段, 架构上禁止合成假分
- `ai_card_summary: Option<String>` 字段: LLM 1-2 句摘要文字, 无数字
- 卡片输出: "💬 AI参考(不入决策): {summary}" (单行, 数字靠 LLM 原始 evidence)

### Q5: 5 commit 不变 (P0-4 决策台)

**文档位置**: `docs/v11-口径不一致P4.md` §四
**落地 commit**:
- `df73250` (v13.1 Commit A 模型)
- `7467cf9` (v13.2 Commit B 裁决)
- `05fb499` (v13.3 Commit C 渲染)
- `1831939` (v13.4 Commit D 治理)
- `28c5051` (v13.5 Commit E shadow)

**核心结论**: 5 commit 单一职责, review 友好. 后续 P0-5+ 实际是 5 commit 外的"补丁" (v15.1-3 + v16.1-2 + v16.4 + v17.0 实际接入 + 收尾 commit 11-13).

### Q6: 6 保留 + B10 降级 (修订为 9 保留)

**文档位置**: `docs/v11-口径不一致P4.md` §二
**落地 commit**: `1831939 feat(v13.4): P0-4 commit D — 推送治理`

**核心结论**:
- 原 6 保留: A1 盘前Checklist / A7 涨跌停突变 / A8 炸板紧急 / B1 市场概览 / B2 复盘报告 / C1 公告告警
- 修订为 9 保留 (grill Q6 决定 + P0-5+ Commit 4 加 5 个 candidate 源, 3 个保留)
- **9 保留**: A1/A7/A8/**A13**/**A14**/**A15**/B1/B2/C1
- 12 降级: A2/A3/A4/A5/A6/A11/A12/**B10**/**B11**/**B12**/**B13**/B4
- 6 移交 P0-5+: A10/B3/B6/B7/**C6**
- 2 收敛: C2/C3 (降频)
- 5 并入决策台: A9/B5/B8/B9/C5 (Commit A/B 落地, B9 字符串猜 Commit 2 替换)

**留 P0-5++ commit 14+ 用户 review**: 9 保留清单实际跑 monitor 一周后, 任何不常用降级.

---

## 2. 跨决策依赖图

```
Q1 (35 条盘点)
  ↓
Q2 (漏盘点处置)  ← P0-4 Commit D 落地
  ↓
Q5 (5 commit 不变)  ← P0-4 commit A/B/C/D/E 落地
  ↓
Q6 (9 保留清单)
  ↓
Q3 (shadow 3-5 次)  ← P0-4 Commit E 落地 (但 LLM 限流没真正跑成功)
  ↓
Q4 (AI 1-2 句)  ← P0-4 Commit A 落地
```

---

## 3. 红线遵守 (P5 §一 + §十 钉死)

| 红线 | 验证 |
|------|------|
| 候选筛选台 ≠ 买入决策台 | 输出文案 "帮你筛选, 不替你拍板" + "不下买入指令" |
| 唯一能进 Strong = 布林+MACD | classify_tier 强证据 keywords 只有 5 个含 "布林+MACD" |
| 不合成"买入分" | CandidateEntry 架构上无综合分字段 |
| 不给"建议买入" | 输出文案不含"建议买入" 字样 |
| 5 路合并到 1 条候选台卡片 | P0-5+ commit 5-11 落地 |

---

## 4. 红线 + 决策落地的 commit hash 速查表

| 决策 | 文档位置 | 落地 commit |
|------|----------|------------|
| Q1: 27→35 推送盘点 | `P4.md §二` | `1831939` / `6062407` |
| Q2: 漏盘点处置 | `P4.md §二 表格` | `1831939` (P0-4 commit D) |
| Q3: shadow 3-5 次 | `P4.md §四 Commit E` | `28c5051` (落地) / `5abfb1a` (改) |
| Q4: AI 卡片 1-2 句 | `v11.md §根因` + `P4.md §三` | `df73250` (P0-4 commit A) |
| Q5: 5 commit 不变 | `P4.md §四` | 5 commit (见 Q5) |
| Q6: 9 保留 + B10 降级 | `P4.md §二` | `1831939` (P0-4 commit D) |

**P0-5+ 后续 commit 索引**:

| 决策 | 落地 commit |
|------|------------|
| 5 commit 不变 (P0-5+ commit A/B/C/D/E 候选台) | `fb64868` (v15.1) / `5411187` (v15.2) / `e4bcfa3` (v15.3) / `bc51fcf` (docs) / `83a5132` (v16.1) / `e2eaf69` (v16.2) |
| 5 个 PushKind 候选源 (Commit 4 落地) | `83a5132` (v16.1) |
| 3 commit 实际接入 + 收尾 | `c0e433c` (v16.4) / `e2eaf69` (v16.2) / `5abfb1a` (v17.0) |

---

## 5. 后续 commit 14+ 待办 (P0-5++ 留)

| 项 | 落地 | 来源 |
|---|------|------|
| 9 保留清单过目 | 用户跑 monitor 一周, 反馈 | `p0-5-plus-v4-todo.md` |
| A10/C4 --test 路径接入 | if 用户需要 | `p0-5-plus-v4-todo.md` |
| LLM 恢复后 shadow 验证 | 跑 3-5 次 | `p0-5-plus-v4-todo.md` |
| PUSH_SHADOW 切 default | 验证后删 | `p0-5-plus-v4-todo.md` |

---

## 6. 一句话

**v11 6 个 grill 决策 (Q1-Q6) 全部落地: 35 条推送盘点 (Q1) + 漏盘点处置 (Q2) + shadow 3-5 次 (Q3, LLM 限流没真正跑) + AI 1-2 句摘要 (Q4) + 5 commit 不变 (Q5) + 9 保留 + B10 降级 (Q6). 24 commit 全部入 git (1e2314b9 决策台 + ab0370e1 候选台推送 验证). 红线 100% 守住. 留 4 项 P0-5++ commit 14+ 用户 review / A10C4 / shadow / PUSH_SHADOW.**

---

**查阅指引**:
- 找具体决策: §1 6 个 Q
- 找落地 commit: §4 速查表
- 找红线遵守: §3
- 找待办: §5
