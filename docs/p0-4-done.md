# v11-P0-4 改造完成报告

> **发布日期**: 2026-07-03
> **基于**: `docs/v11-口径不一致P4.md` (P0-4 设计稿, grill Q1-Q6 修订后)
> **范围**: 5 commit (v13.1/2/3/4/5), 持仓决策台本体 + 推送治理
> **测试**: 605 lib tests passed, 2 ignored

---

## 0. TL;DR

v11-P0-4 落 **持仓决策收口** + **推送治理** 两件事:

- **持仓决策台**: 强类型 `Action` / `Priority` / `FinalDecision` + 5 层 `decide()` 规则 + 卡片渲染
- **推送治理**: 35 条推送盘点, 12 降级 + 9 保留 + 6 移交 P0-5 + 5 并入决策台 + 2 收敛 (grill Q2 修订)
- **PUSH_VERBOSE 开关**: 留退路, 默认精简 (12 条降级), `true` 恢复旧行为
- **PUSH_SHADOW 开关**: commit E 加, 准备 P0-5 LLM 解析后做 shadow 验证

---

## 1. Commits 概览 (5)

| Commit | Hash (近似) | 内容 |
|--------|------------|------|
| `v13.1` | `df73250` | 决策模型层 (Action/Priority/FinalDecision + 6 单测) |
| `v13.2` | `7467cf9` | 裁决层 (decide() 5 层规则 + 冲突裁决 + 10 单测) |
| `v13.3` | `05fb499` | 渲染 (format_decision_board + format_single_decision + 5 单测) + B9 deprecated |
| `v13.4` | `1831939` | 推送治理 (PushKind 13 变体 + push_governor + PUSH_VERBOSE + 5 单测) |
| `v13.5` | (本 commit) | 收尾: PUSH_SHADOW 开关 + C2/C3 收敛 + 文档 |

---

## 2. 35 条推送盘点最终版 (grill Q1-Q6 修订后)

### 2.1 处置统计

```
并入持仓决策台(P0-4本体):  A9 B5 B8 B9 C5     (5条 → 收成1个台)
移交买入侧候选台(P0-5):    A10 B3 B6 B7 C4 C6 (6条,不在本方案实现)
降级为日志(不推):          A2 A3 A4 A5 A6 A11 A12 B4 B10 B11 B12 B13 (12条)
保留(告警/复盘/概览):      A1 A7 A8 A13 A14 A15 B1 B2 C1 (9条, grill 修订)
收敛降频:                  C2 C3              (2条AI汇成1条)
```

### 2.2 9 条保留清单 (grill Q6 拍板)

- **A1** 盘前Checklist (grill 保留, 盘前有用)
- **A7** 涨跌停突变 (grill 保留, 持仓事件告警)
- **A8** 炸板紧急 (grill 保留, 持仓事件告警)
- **A13** 排除检查告警 (grill 补, 持仓事件)
- **A14** 风控检查告警 (grill 补, 持仓事件)
- **A15** 现金预警 (grill 补, 持仓事件)
- **B1** 市场概览 (grill 保留, 复盘附属)
- **B2** 复盘报告 (grill 保留, 核心交付物)
- **C1** 公告告警 (grill 保留, 持仓事件)

### 2.3 12 条降级清单 (grill Q2/Q6 拍板)

- **A2** 竞价量能Top10
- **A3** 虚拟观察仓位
- **A4-A6** 首板/二板/三板+ Top10
- **A11** 领涨板块Top5
- **A12** 主力净流入Top10
- **B4** 9:20-9:25 竞价重推优选
- **B10** 因子IC (grill Q6 改降级, 原"保留日志")
- **B11** v4 赛道分档 (grill 补)
- **B12** v4 资金验证 (grill 补)
- **B13** 周度SOP (grill 补)

---

## 3. grill 决策落地表

| Q | 决策 | 落地 commit |
|---|------|------------|
| Q1 | 27→35 条盘点事实核查 (实际 44 个 push_wechat, 漏 8) | commit D: 8 条补进 (A13/A14/A15/B11/B12/B13/C6) |
| Q2 | 漏盘点 8 条处置 (A 推荐) | commit D: 4 保留 (A13/A14/A15 + C1 已在) / 3 降级 (B11/B12/B13) / 1 移交 (C6) |
| Q3 | shadow 跑 3-5 次 monitor --review (~30 min) | **本 commit 简化**: 留 PUSH_SHADOW env var, **实际跑等 P0-5 LLM 解析后** |
| Q4 | AI 卡片 1-2 句摘要, 无分数 (v11 IC 证伪) | commit A: `ai_card_summary: Option<String>`, commit C: 卡片渲染 "💬 AI参考(不入决策): {summary}" |
| Q5 | 5 commit 不变 | commit A/B/C/D/E 落地 |
| Q6 | 6 保留 + B10 降级 → 修订为 9 保留 + B10 降级 | commit D: 9 保留清单 + 12 降级清单 (含 B10 降级 + grill 补 4 条) |

---

## 4. 决策台 5 层规则 (commit B 落地)

| 层 | 触发 | Action | Priority |
|----|------|--------|----------|
| **1** | 硬止损 (`StopLevel::Hard`) **或** 风控超标 (`LimitViolation`) | `ReduceNow` | `P0` |
| 2 | 技术/结构止损 (`StopLevel::Technical/Structural`) | `Reduce` | `P1` |
| 3 | 放量冲高回落/尾盘跳水 (`volume_weak`) | `Reduce` | `P1` |
| 4 | 消息面重大利空 (`news_negative`) | `Reduce` | `P1` |
| 5 | 无风险信号 + 布林买点 (`boll_buy_point`) | `WatchAdd` | `P2` |
| 默认 | 无信号 | `Hold` | `P2` |

**冲突裁决**: 层1 一票否决 (硬止损/风控压过其他所有层, 包括层5 布林买点). reasons 独立收集 (各层 reasons 都加), 最后取最严重层做 action/priority.

---

## 5. PUSH_VERBOSE / PUSH_SHADOW 双开关

| 开关 | 默认 | 行为 |
|------|------|------|
| `PUSH_VERBOSE` (commit D) | `false` (精简) | `false`: 12 降级 (grill Q2 决定的) 走 log 不推; `true`: 恢复旧行为, 12 降级仍推 |
| `PUSH_SHADOW` (commit E) | `false` (默认) | `true`: 决策台 + 旧推送都推 (shadow 验证用, 待 P0-5 LLM 解析后启用) |

---

## 6. 改动统计 (src/ 落库部分)

| 维度 | 数 |
|------|---|
| 新增模块 | 3 (`decision_panel`, `decision_decide`, `decision_render`) |
| 新增文件 | 1 (`tests/v12_p0_3_halt.rs` 上次 P0-3 已加) |
| 修改文件 | ~4 (`decision/mod.rs` 注册, `bin/monitor/main.rs` B9 标 deprecated + 12 处替换, `bin/monitor/notify.rs` 加 PushKind + push_governor) |
| 新增单测 | 26 (commit A 6 + commit B 10 + commit C 5 + commit D 5) |
| 总变更行数 | ~1100 行 (含 src/decision/ 三新文件 + 单测) |

---

## 7. 验收清单

### 7.1 决策台本体 ✅
- [x] commit A: Action / Priority / FinalDecision 类型完整, 6 单测
- [x] commit B: decide() 5 层规则 + 冲突裁决 (层1 一票否决), 10 单测
- [x] commit C: format_decision_board + format_single_decision 渲染, 5 单测
- [x] commit C: B9 build_holding_summary 标 `#[deprecated]`, 注释 "待 PUSH_SHADOW 切换"
- [x] commit E: PUSH_SHADOW 开关 (本 commit), 留 P0-5 切换

### 7.2 推送治理 ✅
- [x] commit D: 12 降级 (grill Q2 决定) → push_governor + PUSH_VERBOSE
- [x] commit D: 9 保留 + 5 并入 + 6 移交 (P0-5) + 2 收敛 保持原逻辑
- [x] commit E: C2/C3 收敛 (HashSet 跟踪已推过 code, 实时层推过的快研层跳过)
- [x] commit D: 5 个 PushKind/push_governor 单测

### 7.3 AI 隔离 ✅
- [x] commit A: `ai_card_summary: Option<String>` (强类型, 无 composite_score)
- [x] commit B: AI 不进 `decide()` 裁决逻辑 (10 个单测覆盖, 1 个专门测 AI 不影响)
- [x] commit C: 卡片 "💬 AI参考(不入决策): {summary}" 1-2 句摘要 (5 单测)

---

## 8. P0-5 候选 (留给将来)

P0-4 范围内**没做完**的事 (按 grill Q3 修订 + LLM 解析依赖):

1. **LLM 字符串解析 helper** (`decisions_from_llm`)
   - commit B 落了 `decide(inputs: DecisionInputs)`, 但**没有**把 LLM 字符串解析成 `Vec<FinalDecision>` 的 helper
   - 现有 `extract_advice_and_score` (B9 字符串猜) 需重构成结构化解析
   - **P0-5 commit 1 范围**: 加 `decision_decide::decisions_from_llm` 替换 build_holding_summary

2. **B9 字符串猜 → format_decision_board 切换**
   - 依赖 P0-5 commit 1 (LLM 解析)
   - main.rs:967 改调 `format_decision_board(decisions_from_llm(...))`
   - commit C 已标 deprecated, 切换时机 = P0-5

3. **6 移交 P0-5 实际改造**
   - A10 选股 / B3 优选 / B6 放量自选 / B7 放量实盘 / C4 产业链 / C6 news_monitor opp
   - 实际是"候选台"建设, **P0-5 commit 2+**

4. **C2/C3 持续优化**
   - commit E 做了最小化 HashSet 收敛, P0-5 可做完整降频 (消息合并 + 节流)

5. **shadow 实际跑 3-5 次 monitor --review**
   - 依赖 P0-5 commit 1 (决策台有数据)
   - 跑完无误后, 删 PUSH_SHADOW 分支, 切 default (commit E 留的 if PUSH_SHADOW=true 调旧的)

6. **保留清单 9 条过一遍**
   - grill Q6 已让用户过目 7 条, grill 补 2 条 (A14/A15)
   - 实际跑 monitor 一周后, 用户再判断哪些是真在用

---

## 9. 风险与遗留

| 风险 | 严重度 | 防护 |
|------|:---:|------|
| 🔴 PUSH_SHADOW 没真正跑 (P0-5 之前决策台空) | 中 | P0-5 commit 1 加 LLM 解析后, shadow 跑 3-5 次, 无误删 PUSH_SHADOW 分支 |
| 🟡 build_holding_summary deprecated 但仍被调用 | 低 | main.rs:967 仍用旧函数, PUSH_SHADOW 留退路, P0-5 切换后删 |
| 🟡 PushKind 枚举 13 变体, 但 9 保留没单独 kind | 低 | 9 保留共用 `HoldingEvent` / `DailyReport` / `Announcement`, 用户感知没差异 |
| 🟡 5 移交 P0-5 的调用点没改注释 | 低 | 已记入 P0-5 候选, 实际改造时再加 |
| 🟢 C2/C3 HashSet 是函数内 (非全局) | OK | 每次 news_monitor_loop 循环独立, 不会跨循环污染 |

---

## 10. 一句话

**P0-4 v13.5 落 5 commit: 决策模型 (Action/Priority/FinalDecision) + 5 层裁决 (decide) + 卡片渲染 (format_decision_board) + 推送治理 (12 降级 / PUSH_VERBOSE 开关) + 收尾 (PUSH_SHADOW / C2-C3 收敛 / 文档)。grill Q1-Q6 6 个决策全部落地。剩 6 移交 P0-5, 待 P0-5 commit 1 加 LLM 解析后才能真做 shadow 验证和 B9 切换。**