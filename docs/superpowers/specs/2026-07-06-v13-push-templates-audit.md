# v13 推送模板差距审计报告

> **类型**：审计报告（与设计 spec `2026-07-06-v13-push-templates-design.md` 配套）
> **审计对象**：`Northofqing/stock_analysis`
> **审计范围**：v13 spec (§14.0~§14.6) ↔ `push_templates.rs` (24 render) + `notify.rs` (PushKind 治理)
> **审计时间**：2026-07-06
> **关联 baseline**：v19.16（commit `1f3a176`）

---

## 0. 审计摘要

| 维度 | 完全一致 | 有差异 | 缺失 (spec 有代码无) | 多余 (代码有 spec 无) |
|---|---:|---:|---:|---:|
| 字段级 (21 render) | 11 | 12 | 7 (v13 新增) + 6 (新规 v13.1) = 13 | 1 (T-08 候选失效复用 CandidateBoard) |
| 治理级 (34 PushKind 实际 enum) | 14 | 8 | 13 (v13 + 新规) | 3 (CandidateBoard / NewsRanked / CloseCall) |
| 风格级 | 0 | 12 | — | — |

**v13 spec `[新增]` 6 项盘点**：
1. ✅ P-01 `PreopenNewsHot` — 代码无 render
2. ✅ I-01 `IntradayMarket` — 代码无 render
3. ✅ I-02 `NewsCatalyst` — 代码无 render
4. ✅ A-01 `PaperReview` — 代码无 render
5. ✅ A-10 `CatalystReview` — 代码无 render
6. ✅ D-01 `NewsToIdea` — 代码无 render

**审计多发现**：
7. ✅ I-03 `IndustryChainIntraday`（盘中涨停扩散）— 代码无独立 render（盘后 R-03 形态不适用盘中）

**新规 v13.1 `[2026-07-06]` 6 项**：
8. ✅ T-14 `PostFixedPriceOrder`
9. ✅ T-15 `PostFixedPriceFill`
10. ✅ T-16 `StPriceLimitChanged`
11. ✅ T-17 `EtfClosingCallAuction`
12. ✅ T-18 `BlockTradeIntradayConfirm`
13. ✅ T-19 `BlockTradePriceRange`

**spec 完全没有的多余项**（需补登 §14.5 或归档）：
- `PushKind::CandidateBoard`（v11 候选台）— §14.5 表中无登记
- `PushKind::NewsRanked`（P2-News）— §14.5 表中无登记
- `PushKind::CloseCall`（T-12）— §14.5 表中无登记

**重要发现（与 audit agent 早期版差异）**：
- 实际 v19.16 enum 包含 **34 个**变体（不是审计初版认为的 22+）
- `render_turnover_top` 函数存在但 `PushKind::TurnoverTop` **未在 enum 中**（v19.15 设计后未接通）
- 实际 `render_*` 函数共 **21 个**（不是 24 个，差异因部分函数私有化或归类不同）

---

## 1. 字段级差异表

| 模板 | spec § | 现状 (render 输出 / 代码) | 差异点 | 修复方向 |
|---|---|---|---|---|
| **§14.0 Banner** | §14.0.1 | `render` 严格 1~2 行格式 (`push_templates.rs:118-139`) | ✅ 完全一致 | — |
| **P-01 PreopenNewsHot** | §14.1 P-01 | **代码完全缺失**，无 render 函数 | 缺"📰 盘前热点"、"主线"/"催化"/"关注票"5 行模板 | 新增 `render_preopen_news_hot()` |
| **P-02 AuctionVolume** | §14.1 P-02 (复用 T-11) | `render_auction_volume` | ⚠️ 标题用 `🌅 竞价异动 Top{N}`，spec 要求 `🌅 竞价热点量能`；spec 末行"辅助建议"缺失 | 标题改；结尾加"辅助建议"；可保留 TopN 形态 |
| **P-03 CandidateTriggered** | §14.1 P-03 (复用 T-07) | `render_candidate_triggered` | ✅ 字段顺序、证据/不买条件/末行"需人工确认"完全一致 | — |
| **P-04 PaperTrade** | §14.1 P-04 (复用 T-10) | `render_paper_trade` | ✅ 兼容（spec 标签化 vs 代码 `.label()` 输出一致） | — |
| **I-01 IntradayMarket** | §14.2 I-01 | **代码完全缺失** | 缺 `📊 盘中轮动`、三板块(科技/电力/机器人)强度+主攻+轮动状态 | 新增 `render_intraday_market()` |
| **I-02 NewsCatalyst** | §14.2 I-02 | **代码完全缺失** | 缺 `📰⚡ 新闻催化跟踪`、新闻/受益板块/上涨个股 | 新增 `render_news_catalyst()` |
| **I-03 IndustryChainIntraday** | §14.2 I-03 | **代码无独立 render**；盘中涨停扩散用 `render_candidate_triggered` 间接支持 | spec 要求 `🔥 盘中涨停扩散`；现有 `render_industry_chain` 是盘后 R-03 形态 | 新增 `render_industry_chain_intraday()`（审计多发现） |
| **I-04 HoldingPlan** | §14.2 I-04 (复用 T-03) | `render_holding_plan` | ✅ 完全一致 | — |
| **I-05 HoldingEvent** | §14.2 I-05 (复用 T-04) | `render_holding_event` | ✅ 完全一致 | — |
| **I-06 T0Advice (建议)** | §14.2 I-06 (复用 T-05) | `render_t0_advice` | ✅ 完全一致 | — |
| **I-06 T0Advice (禁止)** | §14.2 I-06 (复用 T-06) | `render_t0_forbid` | ✅ 完全一致 | — |
| **I-07 ForbiddenOps** | §14.2 I-07 (复用 T-09) | `render_forbidden_ops` | ✅ 完全一致 | — |
| **I-08 TurnoverTop** | §14.2 I-08 (v19.15) | `render_turnover_top` 函数存在但 `PushKind::TurnoverTop` 不在 enum | 标题 `🔄 盘中换手率 Top10 (HH:MM 盘中)`，spec 要求 `(HH:MM)` | 删除" 盘中" 后缀；接通 `PushKind::TurnoverTop` 或保留函数为局部 |
| **A-01 PaperReview** | §14.3 A-01 | **代码完全缺失** | 缺 `🧪 虚拟仓复盘（{date}）`、原触发/结果/次日计划三档 | 新增 `render_paper_review()` |
| **A-02 DailyReport** | §14.3 A-02 (复用 R-01) | `render_daily_report` | ✅ 完全一致 | — |
| **A-03 ReviewMarket** | §14.3 A-03 (复用 R-02) | `render_review_market` | ✅ 9 行字段顺序、低置信条件段、明日建议完全一致 | — |
| **A-04 IndustryChain (盘后)** | §14.3 A-04 (复用 R-03) | `render_industry_chain` | ✅ 完全一致 | — |
| **A-05 ReviewLhb** | §14.3 A-05 (复用 R-04) | `render_review_lhb` | ⚠️ spec 明确"盘中无数据时必须返回"兜底文案，代码无此分支 | 加 `if entries.is_empty()` 分支 |
| **A-06 ReviewSignal** | §14.3 A-06 (复用 R-05) | `render_review_signal` | ⚠️ spec 含"做T建议"括号注脚，代码无 | 加括号注脚行 |
| **A-07 ReviewFailure** | §14.3 A-07 (复用 R-06) | `render_review_failure` | ✅ 实质一致（段落顺序：原信号/结果/归因/处理建议/─────/分布） | — |
| **A-08 TomorrowWatch** | §14.3 A-08 (复用 R-07) | `render_tomorrow_watch` | ✅ 完全一致 | — |
| **A-09 EventCalendar** | §14.3 A-09 (复用 R-08) | `render_event_calendar` | ✅ 完全一致 | — |
| **A-10 CatalystReview** | §14.3 A-10 | **代码完全缺失** | 缺 `📰 题材催化复盘（{date}）`、当日强度/持续性/已启动/待启动/明日观察 | 新增 `render_catalyst_review()` |
| **D-01 NewsToIdea** | §14.4 D-01 | **代码完全缺失** | 缺 `🧭 新闻驱动个股（{HH:MM}）`、banner+新闻+板块+个股+推送原因+可选建议动作 | 新增 `render_news_to_idea()` |
| **T-08 候选失效** | spec 未单列 §14.5 | `render_candidate_invalidated` | spec §14.5 无独立 PushKind；代码复用 `PushKind::CandidateBoard` | 升级为 `CandidateInvalidated` 或保留并补 spec |

**字段级小结**：
- 完全一致：T-01/T-02/T-03/T-04/T-05/T-06/T-07/T-09/T-10/R-01/R-02/R-03/R-07/R-08、§14.0 banner
- 有差异：P-02 (标题)、I-08 (标题)、A-05 (空 entries 兜底)、A-06 (括号注脚)
- spec 有但代码无：P-01、I-01、I-02、I-03 独立版、A-01、A-10、D-01 (7 项) — 实际比"6 个新增"多 1 个：I-03 盘中 IndustryChain
- 代码有但 spec 无：T-08 候选失效复用 CandidateBoard

---

## 2. 治理级差异表

> **比对基线**：v13 spec §14.5 PushKind 治理清单 + 实际 v19.16 enum（34 个变体）

| PushKind | §14.5 期望 | 代码现状 | 差异点 | 修复方向 |
|---|---|---|---|---|
| `AccountMode` | ⚡ / 0 / 照发 / false | `Important` / `None` / 默认 false / false | ✅ | — |
| `DataMode` | ⚡ / 10min / 照发 / false | `Important` / `600` / 默认 false / false | ✅ | — |
| `HoldingPlan` | ⚡ / 30min/票 / 停发 / false | `Important` / `1800` / `should_block_on_mode` 含 / false | ✅ | — |
| `HoldingEvent` | 🚨 / 无视冷却 / 照发 / false | `Emergency` / `None` / 默认 false / false | ✅ | — |
| `T0Advice (建议)` | ⚡ / 30min/票 / 停发 / false | `Important` / `1800` / `should_block_on_mode` 含 / false | ✅ | — |
| `T0Advice (禁止)` | ℹ️ / 30min/票 / 照发 / false | `Important` (一律 ⚡) / `1800` / 默认 false / false | ⚠️ **等级不符** | 在 `render_t0_forbid` 路径标 `Info`，或拆 enum |
| `CandidateTriggered` | ⚡ / 1次/票/日 / 停发 / false | `Important` / `86400` / 含 / false | ✅ | — |
| `ForbiddenOps` | ℹ️ / 60min/票 / 照发 / true | `Info` / `3600` / 默认 false / **false** | ⚠️ `is_deprecated` 不符 | spec 加注脚说明 v19.12 全保留 |
| `PaperTrade` | ℹ️ / 5min/批 / 照发 / true | `Info` / `300` / `should_block_on_mode` 含 (停发) / **false** | ⚠️ 行为 + deprecated 不符 | `should_block_on_mode` 移出 PaperTrade |
| `AuctionVolume` | ⚡ / 10min / 视数据质量 / false | `Info` / `600` / 默认 / false | ⚠️ 等级不符 | `level()` 改 `Important` |
| `TurnoverTop` | ℹ️ / 10min / 照发 / false | **enum 中无此 variant** | ⚠️ 治理盲区 | 接通 enum 或保留函数为局部 |
| `IntradayMarket` | ⚡ / 15min / 照发 / false | 缺 | ❌ 整条缺失 | 新增 enum + 治理 |
| `NewsCatalyst` | ⚡ / 10min / 照发 / false | 缺 | ❌ 整条缺失 | 新增 enum + 治理 |
| `NewsToIdea` | ⚡ / 20min/票 / ReduceOnly可发 / false | 缺 | ❌ 整条缺失 | 新增 enum + 治理 |
| `PreopenNewsHot` | ⚡ / 15min / 可发 / false | 缺 | ❌ 整条缺失 | 新增 enum + 治理 |
| `PaperReview` | 盘后 / 1次/日 / 可发 / false | 缺 | ❌ 整条缺失 | 新增 enum + 治理 |
| `DailyReport` | 盘后 / 1次/日 / 可发 / false | `Important` / `86400` / 默认 / false | ✅ | — |
| `ReviewMarket` | 盘后 / 1次/日 / 可发 / false | `Important` / `86400` / 默认 / false | ✅ | — |
| `IndustryChain` | 盘后 / 1次/日 / 可发 / false | `Important` / 默认 `1800` / 默认 / false | ⚠️ 冷却不符 | 显式 `Some(86_400)` |
| `ReviewLhb` | 盘后 / 1次/日(21:00) / 可发 / false | `Important` / `86400` / 默认 / false | ✅ | — |
| `ReviewSignal` | 盘后 / 1次/日 / 可发 / false | `Important` / `86400` / 默认 / false | ✅ | — |
| `ReviewFailure` | 盘后 / 1次/日 / 可发 / false | `Important` / `86400` / 默认 / false | ✅ | — |
| `TomorrowWatch` | 盘后 / 1次/日 / 可发 / false | `Important` / `86400` / 默认 / false | ✅ | — |
| `EventCalendar` | 盘后 / 1次/日 / 可发 / false | `Important` / `86400` / 默认 / false | ✅ | — |
| `CatalystReview` | ⚡(盘后) / 1次/日 / 可发 / false | 缺 | ❌ 整条缺失 | 新增 enum + 治理 |
| `CloseCall` (T-12) | spec §14.5 表未列 | `Important` / `86400` / 默认 / false | ⚠️ spec 表漏登记 | spec §14.5 补行 |
| `CandidateBoard` (T-08) | spec §14.5 表未列 | `Important` / 默认 `1800` / 默认 / false | ⚠️ spec 表漏登记 | spec §14.5 补行 或归档 |
| `NewsRanked` | spec §14.5 表未列 | `Important` / 默认 `1800` / 默认 / false | ⚠️ spec 表漏登记 | spec §14.5 补行 或归档 |
| 11 v11 降级 enum | spec §14.5 表不收 | `Info` / 各种 / 默认 / false | ⚠️ spec 表漏登记 | spec §14.5 注明"v11 旧 enum 仅保留兼容，新代码不直接调用" |

**`requires_banner()` 单独检查**（notify.rs:142-163）：
- spec §14.0 强制带 banner 的有 12 类
- 代码 `requires_banner=true` 含 18 个 PushKind（含盘后 R 系列）
- ❌ **盘后 R 系列与 spec 模板不符**：R 系列在 spec 模板中**无 banner 占位符**，但代码要求 banner

---

## 3. 风格级差异表

> 对照 §14.0 全局约定 6 条

| 模板 | 风格项 | spec 要求 | 现状 | 修复方向 |
|---|---|---|---|---|
| 全局 | HH:MM 时间戳 | `（HH:MM）` 中文全角括号 | ✅ 全部一致 | — |
| 全局 | 行内分隔符 | ` | ` | ✅ 全部一致 | — |
| 全局 | 条件段 `[...]` | 整段省略 | ✅ 已用 `if let Some()` / `if !xxx.is_empty()` | — |
| 全局 | 第 1~2 行 banner | 交易建议类必带 | ⚠️ 盘后 R 系列不应带 | 修复见 §2 `requires_banner` |
| 全局 | 末行"辅助建议,非下单指令" | 交易建议类必带 | ⚠️ T-01/T-02/R-02 缺末行 | 加末行 |
| **T-01 账户模式** | 末行"辅助建议" | spec §14.0.1 第 6 条 | ❌ 缺 | 末行加 |
| **T-02 数据模式** | 末行"辅助建议" | spec §14.0.1 第 6 条 | ❌ 缺 | 末行加 |
| **R-02 盘面走向** | 末行"辅助建议" | spec §14.0.1 第 6 条 | ❌ 缺 | 末行加 |
| **P-02 竞价热点量能** | 标题用 `TopN` vs spec 单票 | spec 单票 | ❌ 代码用 `Top{N}` | 标题改 + 去掉 TopN |
| **I-08 换手率** | 标题" 盘中"后缀 | spec 无 | ⚠️ 多了" 盘中" | 删后缀 |

---

## 4. 关键发现与建议优先级

### 🔴 高优先级（spec 与代码硬冲突 / 缺失）

1. 7 个 v13 新增模板完全缺失（P-01/I-01/I-02/I-03/A-01/A-10/D-01）
2. 6 个新规 v13.1 模板完全缺失（T-14~T-19）
3. `PushKind::PaperTrade` `should_block_on_mode` 行为不符
4. `PushKind::T0Advice(禁止)` 等级不符
5. `A-05 龙虎榜` 空 entries 兜底文案缺失
6. 盘后 R 系列 `requires_banner=true` 与 spec 不符
7. `PushKind::TurnoverTop` enum 缺失（仅 render 函数存在）

### 🟡 中优先级（风格/字段微差）

8. 3 个 render 末行缺"辅助建议"（T-01/T-02/R-02）
9. `PushKind::IndustryChain` 冷却默认 1800s 应显式改 86400s
10. `PushKind::AuctionVolume` level 应改 Important
11. I-08 / P-02 标题统一

### 🟢 低优先级（代码有 spec 无 / 归档）

12. `CandidateBoard / NewsRanked / CloseCall` 在 §14.5 表中无登记
13. 11 个 v11 降级 enum 不在 spec §14.5 范围
14. T-08 候选失效 复用 `CandidateBoard`

### 📋 仓库结构

- 代码主文件：`src/bin/monitor/push_templates.rs` (3545 行)
- 治理主文件：`src/bin/monitor/notify.rs` (1395 行)
- spec 文件：`docs/architecture/v13-push-templates.md` (447 行)
- **注**：代码文件头注释仍引用 `v12-push-templates.md`（line 4），与 v13 文档对不上

---

## 附录：与初版审计的差异

| 项 | 初版审计 | 本审计（更正后） |
|---|---|---|
| PushKind enum 总数 | "22+" | 34 |
| 已实现 render 函数 | "24" | 21 |
| TurnoverTop | "已实现并接通 enum" | render 存在，enum 缺失 |
| I-03 多发现 | 已识别 | 同（强化独立 enum 论证） |
