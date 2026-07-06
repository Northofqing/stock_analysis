# v13 推送模板实现设计 — 6 新增 PushKind + §14.5 治理对齐

> **类型**：设计文档（Design Spec / Gate A 产物）
> **日期**：2026-07-06
> **关联 spec**：`docs/architecture/v13-push-templates.md` §14.0~§14.6
> **关联 baseline**：v19.16（commit `1f3a176`）
> **优先级**：P0 = 5 模板（⚡重要）/ P1 = 1 模板（ℹ️参考）
> **作者**：Claude Code（brainstorming 流程产出）

---

## 0. 文档元信息

| 项 | 值 |
|---|---|
| 标题 | v13 推送模板实现设计 — 6 新增 PushKind + §14.5 治理对齐 |
| 日期 | 2026-07-06 |
| 路径 | `docs/superpowers/specs/2026-07-06-v13-push-templates-design.md` |
| 关联 spec | `docs/architecture/v13-push-templates.md §14.0~§14.6` |
| 关联 plan | TBD（由 `superpowers:writing-plans` 阶段产出 `v13-implementation-plan.md`） |
| 关联 baseline | v19.16（`1f3a176`） |
| 优先级分层 | P0 = 5 模板（P-01 / I-01 / I-02 / D-01 / A-10）；P1 = 1 模板（A-01） |
| 状态 | Draft（待用户审批） |

---

## 1. 目标与范围

### 1.1 文档目标

将 `docs/architecture/v13-push-templates.md`（spec）转化为可落地到 PR 的设计文档，明确：

- 6 个 `[新增]` 模板的**数据源映射、字段语义、失败模式、合 Gate**
- §14.5 治理清单与 `PushKind` 枚举的**对齐差量**
- 分阶段 PR 提交节奏（P0 = 5 个 / P1 = 1 个）
- 测试矩阵、PR 证据、回滚命令

### 1.2 范围（Scope）

**In Scope**（本设计文档覆盖）

| 项 | 说明 |
|---|---|
| PushKind 枚举补齐 | 新增 6 个 variant：`PreopenNewsHot` / `IntradayMarket` / `NewsCatalyst` / `PaperReview` / `CatalystReview` / `NewsToIdea` |
| 治理元信息对齐 | §14.5 全表 → `level()` / `cooldown_secs()` / `requires_banner()` / `is_deprecated()` / `counts_against_daily_budget()` |
| 6 个新增 render 函数 | `render_preopen_news_hot` / `render_intraday_market` / `render_news_catalyst` / `render_paper_review` / `render_catalyst_review` / `render_news_to_idea` |
| 6 个 Params 结构体 | 与 v19.x 现存 `*Params` 同形（生命周期 + `&str` 借用） |
| 调用路径 | `bin/monitor/main.rs` 6 个新调用点 + `signal_state.rs` 注册 |
| 测试矩阵 | 6 render 单测 + 17 治理元信息单测 + 5 红线专项 + 4 BR 登记引用 |
| PR 证据样例 | 6 段 `Refs / Data-Redlines / OldModules / Threshold-Proof / Business-Rules / Rollback` |
| 回滚 | L1 单 PR revert / L2 PR 链暂停 / L3 整体回 baseline / L4 红线阻断 |

**Out of Scope**（不在本设计文档）

- v19.16 已实现模板的回归改动（`AuctionVolume` / `TurnoverTop` 等）
- 推送频道迁移、数据源接入实现、调度（cron）调整
- `--test` 路径回归与真接改造（独立 PR）
- 文档合规审查脚本本身（`tools/compliance/check.sh` 已存在，仅依赖）

### 1.3 与上游 spec 的引用关系

本设计**严格引用** `docs/architecture/v13-push-templates.md §14.0~§14.6`，不重复 spec 内容。所有设计决策必须可回溯到 spec 章节（满足 AGENTS §6 `Refs: spec §X.X` 与 ENGINEERING_RULES_V2 §3 PR 模板强制字段）。

---

## 2. 现状/差距 + 治理对齐

### 2.1 当前实现（v19.16）盘点

| 项 | 现状 |
|---|---|
| `PushKind` 枚举 | 22+ variant（含 v12 §14.3 新增 13 个 + v11-P0-5+ 5 个 + 降级 9 个） |
| `push_templates.rs` 行数 | 3545 |
| `notify.rs` 行数 | 1395 |
| 已实现 render 函数 | 24 个 |
| 既有 ⚡重要 PushKind | AccountMode / DataMode / HoldingPlan / HoldingEvent / T0Advice / CandidateTriggered / CloseCall / CandidateBoard / NewsRanked |
| 既有 ℹ️参考 PushKind | ForbiddenOps / PaperTrade / AuctionVolume / TurnoverTop / ReviewMarket / ReviewLhb / ReviewSignal / ReviewFailure / TomorrowWatch / EventCalendar |

### 2.2 现状 vs v13 spec 差距矩阵

| v13 spec § | PushKind | 治理（§14.5） | render 函数 | 当前状态 |
|---|---|---|---|---|
| §14.1 P-01 | `PreopenNewsHot` ⚡重要 | 待补齐 | **缺失** | **新增** |
| §14.1 P-02 | `AuctionVolume` ⚡重要 | 已存在 | `render_auction_volume` | v19.x 已实现 |
| §14.1 P-03 | `CandidateTriggered` ⚡重要 | 已存在 | `render_candidate_triggered` | v19.x 已实现 |
| §14.1 P-04 | `PaperTrade` ℹ️参考 | 已存在 | `render_paper_trade` | v19.x 已实现 |
| §14.2 I-01 | `IntradayMarket` ⚡重要 | 待补齐 | **缺失** | **新增** |
| §14.2 I-02 | `NewsCatalyst` ⚡重要 | 待补齐 | **缺失** | **新增** |
| §14.2 I-03 | `IndustryChain` + `CandidateTriggered` | 部分 | `render_industry_chain` | v19.x 已实现（盘后） |
| §14.2 I-04~I-07 | 已有 | 已存在 | 4 个 render | v19.x 已实现 |
| §14.2 I-08 | `TurnoverTop` ℹ️参考 | 已存在 | `render_turnover_top` | v19.15 已实现 |
| §14.3 A-01 | `PaperReview` ℹ️参考 | 待补齐 | **缺失** | **新增** |
| §14.3 A-02~A-09 | 已有 | 已存在 | 8 个 render | v19.x 已实现 |
| §14.3 A-10 | `CatalystReview` ⚡(盘后) | 待补齐 | **缺失** | **新增** |
| §14.4 D-01 | `NewsToIdea` ⚡重要 | 待补齐 | **缺失** | **新增** |

**结论**：6 个新模板均**纯增量**，不触碰现有 24 个 render。

### 2.3 §14.5 治理对齐差量

> 不只是新增 6 个 PushKind，还要把 §14.5 全表与 `notify.rs` 现有 `level()` / `cooldown_secs()` / `requires_banner()` 对齐。

| PushKind | 等级 | 冷却 | Frozen/Unsafe | is_deprecated | 现状 |
|---|---|---|---|---|---|
| `PreopenNewsHot` | ⚡ | 15min | 可发 | false | **缺** |
| `IntradayMarket` | ⚡ | 15min | 照发 | false | **缺** |
| `NewsCatalyst` | ⚡ | 10min | 照发 | false | **缺** |
| `PaperReview` | 盘后 | 1次/日 | 可发 | false | **缺** |
| `CatalystReview` | ⚡(盘后) | 1次/日 | 可发 | false | **缺** |
| `NewsToIdea` | ⚡ | 20min/票 | ReduceOnly可发 / Frozen谨慎 | false | **缺** |

**对齐方式**（满足 §14.0.1 全局横幅 + §14.5 治理表）：

- 等级 → `PushLevel::Important` / `Info`（⚡ → Important；ℹ️ → Info；盘后且 ⚡ → Important）
- 冷却 → 实现 `cooldown_secs()` 返回 `Some(n)`（None 仅 `AccountMode` / `HoldingEvent`）
- Frozen/Unsafe → 落到 `push_governor` 现有判定路径（不另写）
- `is_deprecated` → 默认 `false`，但 `PaperTrade` / `ForbiddenOps` 保留 v19.x 旧值

### 2.4 数据源与红线映射（设计原则）

> 满足 ENGINEERING_RULES_V2 §2.1~§2.8 红线 + AGENTS §3 数据红线。

| 模板 | 主要数据源 | 红线引用 |
|---|---|---|
| P-01 PreopenNewsHot | news_monitor (新闻源) | §2.1 / §2.4 |
| I-01 IntradayMarket | 板块轮动引擎（既有） | §2.4 / §2.3 |
| I-02 NewsCatalyst | news_monitor + 实时行情 | §2.1 / §2.4 |
| A-01 PaperReview | virtual_watch DB（既有） | §2.4 |
| A-10 CatalystReview | news_monitor + 板块 DB | §2.1 / §2.4 |
| D-01 NewsToIdea | news_monitor + 实时行情 + 候选台 | §2.1 / §2.4 / §2.6 |

**失败模式**：每个模板的 Params 必须为**显式 `Option`**，缺失字段渲染为空字符串 + warning，不静默填充（§2.2）。

### 2.5 模块边界（与现有架构对齐）

| 新增 | 位置 | 依赖 |
|---|---|---|
| 6 PushKind variant | `src/bin/monitor/notify.rs` | 无 |
| 6 render 函数 | `src/bin/monitor/push_templates.rs` | `BannerCtx` + 现有 helpers |
| 6 Params 结构体 | `src/bin/monitor/push_templates.rs` | 仅借用 `&str` |
| 6 调用点 | `src/bin/monitor/main.rs` | 现有 signal 触发器 |
| 6 单测 | `src/bin/monitor/push_templates.rs`（同文件 `#[cfg(test)]`） | 仅标准断言 |

**不引入新 crate / 不改模块边界**（满足 AGENTS §2.2 Gate B 最小变更）。

---

## 3. 6 个新增模板详细设计

> 6 个模板按 v13 spec 序号排列。每个模板给出：**字段语义 / 数据源 / 失败模式 / 合 Gate / Banner 行为 / PR 证据字段**。

### 3.1 P-01 PreopenNewsHot（盘前新闻热点）⚡重要 — P0

**字段映射**（spec §14.1 P-01）

| spec 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填，缺失 → 报错（AGENTS §3） |
| `theme_1/2/3` | `&str` | news_monitor cluster 输出 | 缺失 → 整段省略（§2.2） |
| `news_1/2` + `chain_1/2` | `&str` + `&str` | news_monitor event | 缺失 → 整行省略 |
| `name` / `code` / `reason` ×N | `&str` | 候选台 + news 关联 | 缺失 → 跳过该行 |

**治理元信息**（§14.5）

- `level() = Important`
- `cooldown_secs() = Some(900)`（15min）
- `requires_banner() = false`（盘前，banner 非强制）
- `is_deprecated() = false`
- `counts_against_daily_budget() = true`

**红线 Gate**：`2.1`（news 真数据，无 mock）/ `2.2`（缺字段显式）/ `2.4`（新闻时效）。

**PR 证据**：`Refs: spec §14.1 P-01` / `Data-Redlines: [2.1, 2.2, 2.4]` / `OldModules: news_monitor_loop.adopt` / `Threshold-Proof: N/A` / `Business-Rules: BR-NEWS-CLUSTER` / `Rollback: git revert <commit>`。

### 3.2 I-01 IntradayMarket（盘中轮动总览）⚡重要 — P0

**字段映射**（spec §14.2 I-01）

| spec 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填 |
| `tech_ai/hbm/smartphone` + `score` | `&str` + `f32` | 板块轮动引擎（既有 sector_score） | score 缺失 → "N/A"（§2.2） |
| `power_uhv/grid/ess` + `score` | 同上 | 同上 | 同上 |
| `robot_reducer/servo/vision` + `score` | 同上 | 同上 | 同上 |
| `subsector` + 状态 | `&str` | 既有 | 缺失 → "暂无主攻" |

**治理元信息**：`Important` / `Some(900)`（15min）/ `requires_banner() = true`（盘中 ⚡ 交易建议类）/ `is_deprecated() = false` / `counts_against_daily_budget() = true`。

**红线 Gate**：`2.3`（板块 score 坏数据校验，> ±20% 告警）/ `2.4`。

**PR 证据**：`Refs: spec §14.2 I-01` / `Data-Redlines: [2.3, 2.4]` / `OldModules: sector_rotation.adopt` / `Threshold-Proof: score clamp [-100, 100]` / `Rollback`。

### 3.3 I-02 NewsCatalyst（新闻催化映射）⚡重要 — P0

**字段映射**（spec §14.2 I-02）

| spec 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填 |
| `headline` | `&str` | news_monitor | 必填，缺失 → 报错 |
| `theme` | `&str` | news cluster | 缺失 → "未分类" |
| `name/code/chg/reason` ×N | `&str` / `&str` / `f32` / `&str` | 实时行情 + news reason | chg 缺失 → 整行省略 |

**治理元信息**：`Important` / `Some(600)`（10min）/ `requires_banner() = true` / `is_deprecated() = false` / `counts_against_daily_budget() = true`。

**红线 Gate**：`2.1` / `2.3`（chg 坏数据）/ `2.4`。

**PR 证据**：`Refs: spec §14.2 I-02` / `Data-Redlines: [2.1, 2.3, 2.4]` / `Business-Rules: BR-NEWS-CATALYST`。

### 3.4 A-01 PaperReview（虚拟仓复盘）ℹ️参考 — P1

**字段映射**（spec §14.3 A-01）

| spec 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `date` | `&str` | 调度器 | 必填 |
| `name/code/trigger` | `&str` / `&str` / `&str` | virtual_watch DB | 缺失 → 整行省略 |
| `desc/pnl` | `&str` / `f32` | virtual_close DB | pnl 缺失 → "N/A%" |
| `plan_high/flat/low` | `&str` ×3 | TBD 生成（与 T-11 同源） | 缺失 → "暂无计划" |

**治理元信息**：`Info`（盘后参考）/ `Some(86400)`（1次/日）/ `requires_banner() = false` / `is_deprecated() = false` / `counts_against_daily_budget() = false`。

**红线 Gate**：`2.1` / `2.4`（盘后 21:00 后取数）。

**PR 证据**：`Refs: spec §14.3 A-01` / `Data-Redlines: [2.1, 2.4]` / `OldModules: virtual_watch.adopt`。

**注**：本模板属 **P1 优先级**（ℹ️参考），但 `plan_*` 字段依赖 T-11 竞价复算，需在 T-11 数据通路就绪后实施；文档中标注前置依赖：`前置: T-11 竞价复算通路（v12-dev-plan.md §MVP-3）`。

### 3.5 A-10 CatalystReview（盘后题材催化复盘）⚡(盘后) — P0

**字段映射**（spec §14.3 A-10）

| spec 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `date` | `&str` | 调度器 | 必填 |
| `theme` | `&str` | news cluster | 必填 |
| `score` | `f32` | 板块强度 | 缺失 → "N/A" |
| `persistent` | `enum`（high/med/low） | 持续性判定 | 缺失 → "med" |
| `started_names/todo_names/watch` | `&[&str]` | news + 候选台 | 缺失 → 整段省略 |

**治理元信息**：`Important`（⚡盘后）/ `Some(86400)`（1次/日，盘后段 15:30-23:00 触发即记当日已推）/ `requires_banner() = false`（盘后非交易建议）/ `is_deprecated() = false` / `counts_against_daily_budget() = true`。

**红线 Gate**：`2.1` / `2.3`（score 校验）/ `2.4`。

**PR 证据**：`Refs: spec §14.3 A-10` / `Data-Redlines: [2.1, 2.3, 2.4]` / `Business-Rules: BR-THEME-STAGE`。

### 3.6 D-01 NewsToIdea（新闻驱动个股）⚡重要 — P0

**字段映射**（spec §14.4 D-01）

| spec 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `banner` | `BannerCtx` | 既有 | 必填，⚡交易建议类强约束 |
| `HH:MM` | `&str` | 调度器 | 必填 |
| `headline` | `&str` | news_monitor | 必填 |
| `theme/stage` | `&str` / `enum` | news cluster | 缺失 → stage="启动" |
| `name/code` | `&str` / `&str` | 候选台 + news 关联 | 必填 |
| `reason_1/2` | `&str` | 候选台 + news | 缺失 → 整行省略 |
| `action` | `enum`（观察/低吸/不追） | 候选台推荐 | 缺失 → 整段省略 |

**治理元信息**：`Important` / `Some(1200)`（20min/票）/ `requires_banner() = true` / `is_deprecated() = false` / `counts_against_daily_budget() = true`。

**红线 Gate**：`2.1` / `2.4` / `2.6`（建议类不强下单）。

**PR 证据**：`Refs: spec §14.4 D-01` / `Data-Redlines: [2.1, 2.4, 2.6]` / `Business-Rules: BR-NEWS-TO-IDEA`。

### 3.7 模板间共性约束

| 约束 | 来源 |
|---|---|
| 全部 emoji 标题 + `（HH:MM）` | spec §14.0.1 |
| 交易建议类必带 banner | spec §14.0.1 |
| 末尾 "辅助建议, 非下单指令" | spec §14.0.1 |
| 新增 PushKind 必登记治理 | spec §14.0 |
| 行内字段 ` \| ` 分隔 | spec §14.0 |
| `{xxx}` 变量 / `[...]` 条件段 | spec §14.0 |

**统一抽象**：新增 `RenderCtx` trait（**内部 trait，不导出**）封装 `level()` / `banner_required()` / `cooldown()` / `key()` 共性，避免 6 套重复样板。

---

## 4. 测试矩阵

> 所有测试置于 `src/bin/monitor/push_templates.rs` 末尾 `#[cfg(test)] mod tests`，**与 v19.x 既有用例同模块**（不另开文件）。

### 4.1 单测矩阵（6 render × 必测维度）

| 用例 ID | 模板 | 必测维度 | 期望 |
|---|---|---|---|
| `UT-PR01-01` | P-01 | 标题 emoji + `（HH:MM）` | `📰 盘前热点（09:05）` 匹配 |
| `UT-PR01-02` | P-01 | 3 主线 + 2 催化 + N 关注票 | 行数 = 1(标题) + 1(主线) + 1(催化label) + N(催化) + 1(关注label) + N(票) + 1(尾句) |
| `UT-PR01-03` | P-01 | theme/news 缺失 → 整段省略 | 不输出 "催化:" label |
| `UT-PR01-04` | P-01 | 末尾 "辅助建议, 非下单指令" | 字符串尾匹配 |
| `UT-IR01-01` | I-01 | 3 大板块 + score 格式 | `科技: {subsector}（强度{score}）` |
| `UT-IR01-02` | I-01 | score 缺失 → "N/A" | score == None 时渲染 "N/A" |
| `UT-IR01-03` | I-01 | 轮动状态 enum → 中文 | 扩散 / 分化 / 退潮 三态全覆盖 |
| `UT-NC02-01` | I-02 | banner 存在 + headline 必填 | 缺 headline → 报测试错 |
| `UT-NC02-02` | I-02 | theme 缺失 → "未分类" | 默认兜底字符串 |
| `UT-NC02-03` | I-02 | 票列表 chg 缺失 → 整行省略 | 不打印该行 |
| `UT-PV01-01` | A-01 | date + name/code/trigger | 标准盘后复盘行 |
| `UT-PV01-02` | A-01 | plan_xxx 三态全覆盖 | 高开> / 平开 / 低开 三段 |
| `UT-PV01-03` | A-01 | pnl 缺失 → "N/A%" | 显示占位 |
| `UT-PV01-04` | A-01 | **前置依赖**：T-11 通路未就绪 → `#[ignore]` 标记 | 文档注释引用 `v12-dev-plan.md §MVP-3` |
| `UT-CR10-01` | A-10 | theme + score + persistent 三态 | high / med / low 全覆盖 |
| `UT-CR10-02` | A-10 | 已启动 / 待启动段 | 缺失 → 整段省略 |
| `UT-NI01-01` | D-01 | banner 必填 → 缺则 panic（与 v19.x `HoldingPlan` 一致） | 测中显式校验 banner 行 |
| `UT-NI01-02` | D-01 | stage 三态 | 启动 / 发酵 / 分歧 |
| `UT-NI01-03` | D-01 | action 三态 | 观察 / 低吸 / 不追 |
| `UT-NI01-04` | D-01 | reason 缺失 → 整行省略 | 不打印该行 |

**合计**：19 个 render 用例。

### 4.2 治理元信息测试（§14.5 对齐）

> 复用 v19.x `cooldown_secs()` / `level()` / `requires_banner()` / `is_deprecated()` / `counts_against_daily_budget()` 既有断言模式（push_templates.rs:2317-2410 已存在 7 个治理断言）。

| 用例 ID | 断言 |
|---|---|
| `UT-GOV-01` | `PreopenNewsHot.cooldown_secs() == Some(900)` |
| `UT-GOV-02` | `IntradayMarket.cooldown_secs() == Some(900)` |
| `UT-GOV-03` | `NewsCatalyst.cooldown_secs() == Some(600)` |
| `UT-GOV-04` | `PaperReview.cooldown_secs() == Some(86_400)` |
| `UT-GOV-05` | `CatalystReview.cooldown_secs() == Some(86_400)` |
| `UT-GOV-06` | `NewsToIdea.cooldown_secs() == Some(1200)` |
| `UT-GOV-07` | `IntradayMarket.requires_banner() == true` |
| `UT-GOV-08` | `NewsCatalyst.requires_banner() == true` |
| `UT-GOV-09` | `NewsToIdea.requires_banner() == true` |
| `UT-GOV-10` | `PreopenNewsHot.requires_banner() == false` |
| `UT-GOV-11` | `PaperReview.requires_banner() == false` |
| `UT-GOV-12` | `CatalystReview.requires_banner() == false` |
| `UT-GOV-13` | `IntradayMarket.level() == PushLevel::Important` |
| `UT-GOV-14` | `PaperReview.level() == PushLevel::Info` |
| `UT-GOV-15` | `CatalystReview.level() == PushLevel::Important`（⚡盘后） |
| `UT-GOV-16` | `PaperReview.counts_against_daily_budget() == false` |
| `UT-GOV-17` | 6 新增均 `is_deprecated() == false` |

**合计**：17 个治理用例。

### 4.3 红线门禁测试（ENGINEERING_RULES_V2 §2.1~§2.10）

| 红线 | 用例 ID | 验证方式 |
|---|---|---|
| §2.1 无 mock | `UT-RL21-NEW` | render 6 函数均不接受任何 mock 标志；CI `tools/compliance/lib/check_fake_impl.sh` 通过 |
| §2.2 缺数据显式 | `UT-RL22-NEW` | 6 render 在字段为 None 时输出空段或 "N/A"，**不输出默认值数字** |
| §2.3 坏数据校验 | `UT-RL23-NEW` | score > ±100 / chg > ±20% 时 render 入口前置 `debug_assert!` 触发 panic |
| §2.4 时效 | `UT-RL24-NEW` | P-01 / I-02 / D-01 输入 timestamp 超过 spec 阈值 → 报 warning（不 panic） |
| §2.8 假实现 | 复用 | `check_fake_impl.sh` 已覆盖 |
| §2.9 设计矛盾 | 复用 | `check_design_contradiction.sh` 已覆盖（6 新增无阈值字段） |
| §2.10 业务规则 | `UT-BR-NEW` | 引用 BR-NEWS-CLUSTER / BR-NEWS-CATALYST / BR-THEME-STAGE / BR-NEWS-TO-IDEA（4 个），`docs/business_rules.md` 必先登记 |

**合计**：5 个红线专项 + 4 个 BR 登记引用。

### 4.4 测试覆盖率目标

> 满足 ENGINEERING_RULES_V2 §1 Gate D：单测 ≥ 80%，核心交易/数据链路 ≥ 95%。

| 指标 | 目标 | 测算依据 |
|---|---|---|
| `push_templates.rs` 行覆盖 | ≥ 85% | 19 render + 17 gov + 5 红线 = 41 新增覆盖 6 函数全部主路径 |
| `notify.rs` 行覆盖（治理分支） | ≥ 90% | 17 gov 用例覆盖 6 新增 variant 全部分支 |
| `main.rs` 调用点覆盖 | ≥ 70% | 6 新调用点通过 `--test` e2e 间接覆盖 |

### 4.5 `--test` 路径 e2e（v19.x 已确立的 e2e 验证）

| e2e ID | 验证 |
|---|---|
| `E2E-NEW-01` | `cargo run --bin monitor -- --test` 输出含 6 新增模板标题各 1 条 |
| `E2E-NEW-02` | `tools/compliance/check.sh` 全绿 |
| `E2E-NEW-03` | 数据源失败注入（关停 news_monitor 模拟）→ 6 render 报 warning，不静默 |

---

## 5. PR 节奏 + 证据 + 回滚

> 满足 AGENTS §2.2 Gate 序列（A → B → C → D）+ ENGINEERING_RULES_V2 §3 PR 模板强制字段 + §6 "小提交"。

### 5.1 分阶段 PR 计划（P0 = 5 / P1 = 1）

| PR # | 标题模板 | 范围 | 优先级 | 关联 spec | 关联 Gate |
|---|---|---|---|---|---|
| **#1** | `feat(v13): PushKind 新增 PreopenNewsHot/IntradayMarket/NewsCatalyst 治理元信息对齐` | 3 variant + 3 render + 3 Params + 治理 + 调用点 | **P0** ⚡ | §14.1 P-01 / §14.2 I-01 / §14.2 I-02 | A→B→C→D |
| **#2** | `feat(v13): PushKind 新增 NewsToIdea + 治理元信息` | 1 variant + 1 render + 1 Params + 治理 + 调用点 | **P0** ⚡ | §14.4 D-01 | A→B→C→D |
| **#3** | `feat(v13): PushKind 新增 CatalystReview 盘后复盘 + 治理元信息` | 1 variant + 1 render + 1 Params + 治理 + 调用点 | **P0** ⚡(盘后) | §14.3 A-10 | A→B→C→D |
| **#4** | `feat(v13): PushKind 新增 PaperReview 盘后复盘 (前置 T-11)` | 1 variant + 1 render + 1 Params + 治理 + 调用点（**`#[ignore]` 启动**） | **P1** ℹ️ | §14.3 A-01 | A→B；D 等 T-11 |
| **#5** | `chore(v13): §14.5 治理全表差量对齐审计` | 不新增功能，仅复核既有 22 PushKind 与 §14.5 表完全对齐 | 收尾 | §14.5 | C |

**累计**：

- 6 个新 variant → 3 PR（#1 三合一 / #2 / #3）+ 1 PR（#4，P1）
- 治理元信息 → 4 PR 各带 + #5 全表审计
- 总提交行数预估：每 PR +250~400 行（含测试），#5 +50 行

### 5.2 PR 证据样例（以 #1 中 PreopenNewsHot 为例）

> 直接套用 ENGINEERING_RULES_V2 §3 + AGENTS §6 字段。

```markdown
## feat(v13): PushKind 新增 PreopenNewsHot/IntradayMarket/NewsCatalyst 治理元信息对齐

### Refs
- spec: `docs/architecture/v13-push-templates.md §14.1 P-01 / §14.2 I-01 / §14.2 I-02`
- design: `docs/superpowers/specs/2026-07-06-v13-push-templates-design.md §3.1, §3.2, §3.3`

### Data-Redlines
- [2.1] 无 mock：render 入口仅依赖真实 news_monitor / sector_rotation 输出
- [2.2] 缺字段显式：theme/news/score 缺失 → 整段省略或 "N/A"
- [2.3] 坏数据：score > ±100 / chg > ±20% 时 `debug_assert!` 触发
- [2.4] 时效：P-01 依赖 news ≥ 09:00 拉取；I-02 依赖实时行情 ≤ 5s

### OldModules
| 模块 | adopt/reject | 原因 |
|---|---|---|
| `news_monitor_loop` | adopt | 复用现有 cluster 输出，不另起 schema |
| `sector_rotation` | adopt | 复用既有 score，不改计算口径 |
| `push_templates::render_auction_volume` | reject | 与 P-01 主题不同，不合并 |

### Threshold-Proof
- N/A（无阈值变更）

### Business-Rules
- BR-NEWS-CLUSTER（news cluster 聚类口径）
- BR-NEWS-CATALYST（news→个股映射规则）

### Validation
- `cargo fmt --check` ✓
- `cargo clippy -D warnings` ✓
- `cargo test push_templates::tests::preopen_*` ✓
- `cargo test push_templates::tests::intraday_*` ✓
- `cargo test push_templates::tests::news_catalyst_*` ✓
- `bash tools/compliance/check.sh` ✓

### Rollback
\`\`\`bash
git revert <commit-sha>
# 撤销 PushKind 三 variant + 治理元信息 + render
# 不影响 v19.x 既有 24 render
cargo build --release  # 验证无 dangling ref
\`\`\`
```

### 5.3 红线触发与阻断（每个 PR 必跑）

> 满足 AGENTS §5 + ENGINEERING_RULES_V2 §1 Gate C。

| 检查 | 命令 | FAIL 行为 |
|---|---|---|
| 格式 | `cargo fmt --check` | 阻断 PR |
| 静态 | `cargo clippy -D warnings` | 阻断 PR |
| 单测 | `cargo test` | 阻断 PR |
| 合规 | `bash tools/compliance/check.sh` | 阻断 PR |
| 数据时效 | `tools/compliance/lib/check_data_freshness.sh` | 阻断 PR |
| 假实现 | `tools/compliance/lib/check_fake_impl.sh` | 阻断 PR |
| 设计矛盾 | `tools/compliance/lib/check_design_contradiction.sh` | 阻断 PR |
| 业务规则 | `tools/compliance/lib/check_business_rules.sh` | 阻断 PR |

### 5.4 回滚策略（多层次）

| 层次 | 触发条件 | 操作 |
|---|---|---|
| **L1 单 PR** | 单模板错乱 | `git revert <sha>`（≤ 400 行，无跨 PR 依赖） |
| **L2 PR 链** | P0 任意 1 个不通过 | 暂停后续 PR 提交，回 Gate A 重审 spec 解读 |
| **L3 设计缺陷** | 3 个以上 PR 共同失败 | 整体回 v19.16 baseline，重新走 Gate A |
| **L4 红线违规** | 任何 §2.1 / §2.5 / §2.6 红线违反 | 立刻阻断，24h 内复盘补自动化防线（§5 受控例外） |

**关键不变量**：

- 6 新增 render **不依赖** v19.x 既有 24 render 的内部状态
- 任何 PR revert 后 `cargo build` 必须成功（无 dangling 引用）
- 治理元信息通过 `match` 表达，缺分支 = `unreachable!()`，不会"静默 fallback"

### 5.5 风险登记与缓解

| 风险 | 等级 | 缓解 |
|---|---|---|
| P-01 依赖 news_monitor cluster 输出稳定性 | 中 | 复用既有 `news_monitor_loop`，不另起 schema |
| I-01 板块 score 波动大 | 中 | `debug_assert!` 强校验 + UI "N/A" 兜底 |
| A-01 前置 T-11 未就绪 | 高 | PR #4 `#[ignore]` + 文档化前置依赖 |
| 6 新增同日推送冲击用户 | 中 | 各自 cooldown 已设（10min ~ 1次/日） |
| PushKind 增多导致 match 漏分支 | 中 | 全部用 `unreachable!()` + 治理元信息单测 |

---

## 6. 验收清单与下一步

### 6.1 验收清单（DoD）

> 满足 CLAUDE.md §6 Done Criteria + AGENTS §6 PR Evidence + ENGINEERING_RULES_V2 §1 Gate A→D。

| # | 项 | 验证方式 |
|---|---|---|
| 1 | 设计文档落盘 + git commit | `git log -- docs/superpowers/specs/2026-07-06-v13-push-templates-design.md` |
| 2 | 4 个 P0 PR 全部合并 + Gate C 全绿 | `git log --oneline \| grep "feat(v13)"` + `bash tools/compliance/check.sh` |
| 3 | 1 个 P1 PR（PaperReview）合并且 `#[ignore]` 待 T-11 解除 | PR #4 含 `#[ignore]` 与前置依赖注释 |
| 4 | §14.5 全表与代码治理元信息 100% 对齐 | PR #5 审计脚本输出 0 差量 |
| 5 | 19 render 用例 + 17 治理用例 + 5 红线用例 全绿 | `cargo test` |
| 6 | `--test` e2e 含 6 新增模板标题 | `cargo run --bin monitor -- --test` |
| 7 | 覆盖率 ≥ 85%（push_templates）/ ≥ 90%（notify 治理分支） | `cargo tarpaulin` 或 `llvm-cov` |
| 8 | 无 mock / fake 数据进入生产路径 | `check_fake_impl.sh` |
| 9 | PR 证据 6 字段齐 | PR 模板 review |

### 6.2 关键依赖与外部约束

| 依赖 | 状态 | 备注 |
|---|---|---|
| `news_monitor_loop` cluster 输出 | ✅ 已有 | 复用不另起 |
| `sector_rotation` score 计算 | ✅ 已有 | 复用不改口径 |
| `virtual_watch` DB | ✅ 已有 | A-01 依赖 |
| T-11 竞价复算通路 | ⚠️ **未就绪** | v12 MVP-3，A-01 前置 |
| `BannerCtx` | ✅ 已有 | 6 新增直接复用 |
| `push_governor` | ✅ 已有 | Frozen/Unsafe 判定落此处 |
| `SignalStateMachine` | ✅ 已有 | 6 新增注册即可 |

### 6.3 与现有规范的一致性自查

| 规范条款 | 满足方式 |
|---|---|
| AGENTS §1 强制预飞行 | 每个 PR 模板含 Impacted paths / Triggered rule IDs / Validation / Rollback |
| AGENTS §2 Gate A→D | 4 PR 严格串行；失败回对应 Gate |
| AGENTS §3 数据红线 | 6 模板均映射 §2.1 / §2.2 / §2.4（个别含 §2.3 / §2.6） |
| AGENTS §6 PR 证据 | PR 模板样例已覆盖 6 字段 |
| AGENTS §7 根因回退 | L1~L4 分层已设计 |
| ENGINEERING_RULES_V2 §1 Gate A | 本设计文档即 Gate A 产物 |
| ENGINEERING_RULES_V2 §2 红线 | 6 模板逐项映射（§2.1 / §2.2 / §2.3 / §2.4 / §2.6 / §2.8 / §2.9 / §2.10） |
| ENGINEERING_RULES_V2 §3 PR 模板 | PR 证据样例已给 |
| ENGINEERING_RULES_V2 §4 根因回退 | L1~L4 与 §4 对齐 |
| ENGINEERING_RULES_V2 §5 受控例外 | A-01 标注 `#[ignore]` + 前置依赖（非豁免红线） |
| ENGINEERING_RULES_V2 §6 双层门禁 | Fast (PR 提交时) + Full (合并前) 一致 |

### 6.4 范围外但需提前沟通的事项

1. **调度接入**：6 模板的 cron 时机（盘前 09:00 / 盘中 10/11/13/14 / 盘后 15:30/19:00/21:00）由运维 PR 排，本设计仅约束"调用点存在"。
2. **真接测试数据**：D-01 / A-10 需新闻+个股关联数据，依赖 `news_monitor` 真实拉取；测试环境需注入 fixture（独立 PR）。
3. **i18n**：模板中文硬编码，与 v19.x 一致；不另起 i18n 框架。
4. **bark / push channel**：推送通道配置不在本设计范围（v19.x 已稳定）。

### 6.5 下一步（流程交接）

设计文档完成后，按 brainstorming skill 流程：

1. ✍️ **写入设计文档** → `docs/superpowers/specs/2026-07-06-v13-push-templates-design.md`（本文件）
2. 🔍 **Spec 自审**（placeholder / 一致性 / 范围 / 歧义）
3. 🙋 **请用户审阅 spec**
4. 🚀 **进入 writing-plans skill** → 产出 `v13-implementation-plan.md`（PR #1~#5 任务卡）

---

## 附录 A：术语与引用

| 术语 | 含义 |
|---|---|
| PushKind | 推送类型枚举（`src/bin/monitor/notify.rs`） |
| BannerCtx | 交易建议类全局横幅上下文 |
| `push_governor` | 推送主控（冷却 / 治理 / 通道分发） |
| SignalStateMachine | 信号状态机（`src/monitor/signal_state.rs`） |
| `news_monitor_loop` | 新闻监控循环（盘前/盘中） |
| sector_rotation | 板块轮动引擎 |
| virtual_watch | 虚拟观察仓位 DB |
| T-11 | v12 竞价复算通路（v12-dev-plan.md §MVP-3） |

## 附录 B：与 v19.x 既有用例同形的约束

> 满足"读起来像现有代码"的本地化要求。

- **Params 结构体**：与 `HoldingPlanParams<'_>` / `T0AdviceParams<'_>` 同形（生命周期 + `&str` 借用）
- **render 函数签名**：`pub fn render_<kind>(banner: &BannerCtx, p: <Kind>Params<'_>) -> String`（⚡交易建议类必须带 banner）
- **测试函数命名**：`fn <kind>_<scenario>`（如 `preopen_news_hot_three_themes_two_news`）
- **match 分支**：所有 PushKind `match` 必须覆盖全部 variant，缺分支用 `unreachable!()` 而非 `_ =>`
- **错误处理**：所有 render 入口只接受已校验数据，校验在调用点完成；render 内不做 IO

---

**状态**：Draft（待用户审批 → 转 Final）
**下一步**：Spec 自审 → 用户审阅 → writing-plans skill 产出 PR 任务卡