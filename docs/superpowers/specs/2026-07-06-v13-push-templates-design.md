# v13 推送模板整体设计 — 7 新增模板 + 6 新规模板 + 21 对齐 + 34 PushKind 治理

> **类型**：设计文档（Design Spec / Gate A 产物）
> **日期**：2026-07-06
> **关联 spec**：`docs/architecture/v13-push-templates.md §14.0~§14.6`
> **关联 baseline**：v19.16（commit `1f3a176`）
> **关联新规**：沪深北交易所《交易规则（2026 修订）》2026-07-06 施行
> **作者**：Claude Code（brainstorming 流程产出）

---

## 🚨 紧急标注（必读）

**新规 2026-07-06（今天）起施行**，本设计文档涉及两项需**即刻同步**的治理参数变更：

1. **ST/*ST 涨跌幅 5% → 10%** — 现有持仓风险阈值可能错配
2. **创业板做市商制度生效** — 中小市值流动性阈值需校准

**建议**：spec 通过 Gate A 后，**不等待 PR 实施**，先在 `config/risk/*.toml` 同步新阈值，再走 PR 流程固化代码。

---

## 0. 文档元信息

| 项 | 值 |
|---|---|
| 标题 | v13 推送模板整体设计 — 7 新增 + 6 新规 + 21 对齐 + 34 治理 |
| 日期 | 2026-07-06 |
| 路径 | `docs/superpowers/specs/2026-07-06-v13-push-templates-design.md` |
| 关联 spec | `docs/architecture/v13-push-templates.md §14.0~§14.6` |
| 关联 plan | TBD（由 `superpowers:writing-plans` 阶段产出 `v13-implementation-plan.md`） |
| 关联 baseline | v19.16（`1f3a176`） |
| 关联新规 | 沪深北《交易规则（2026 修订）》2026-07-06 施行 |
| 优先级分层 | **P0 = 9 模板（⚡重要）** / **P1 = 4 模板（ℹ️参考 + 1 个 v13 P1）** |
| 状态 | Draft（待用户审批） |

**P0 模板清单**（9 个，⚡重要）：

| 序 | 模板 ID | PushKind | 来源 |
|---|---|---|---|
| 1 | P-01 | `PreopenNewsHot` | v13 spec [新增] |
| 2 | I-01 | `IntradayMarket` | v13 spec [新增] |
| 3 | I-02 | `NewsCatalyst` | v13 spec [新增] |
| 4 | I-03 | `IndustryChainIntraday` | v13 spec + 审计多发现 |
| 5 | A-10 | `CatalystReview` | v13 spec [新增] |
| 6 | D-01 | `NewsToIdea` | v13 spec [新增] |
| 7 | T-14 | `PostFixedPriceOrder` | **新规 v13.1 ⚡** |
| 8 | T-15 | `PostFixedPriceFill` | **新规 v13.1 ⚡** |
| 9 | T-16 | `StPriceLimitChanged` | **新规 v13.1 ⚡** |

**P1 模板清单**（4 个，ℹ️参考 + 1 个 v13 P1）：

| 序 | 模板 ID | PushKind | 来源 |
|---|---|---|---|
| 1 | A-01 | `PaperReview` | v13 spec [新增]（前置 T-11） |
| 2 | T-17 | `EtfClosingCallAuction` | **新规 v13.1 ℹ️**（仅沪市 ETF） |
| 3 | T-18 | `BlockTradeIntradayConfirm` | **新规 v13.1 ℹ️**（创业板大宗盘中确认） |
| 4 | T-19 | `BlockTradePriceRange` | **新规 v13.1 ℹ️**（北交所大宗价格区间） |

---

## 1. 目标与范围

### 1.1 文档目标

将 v13 spec + 2026-07-06 新规 + 现有 24 render 审计发现**合并为一份可落地到 PR 的设计文档**，明确：

- 7 个 v13 spec `[新增]` 模板（含审计多发现的 I-03 盘中 IndustryChain）
- 6 个**新规 v13.1 模板**（盘后固定价格扩围 / ST 涨跌幅 / ETF 集合竞价 / 大宗交易等）
- 12 项现有 21 render 与 v13 spec 的差异对齐（v19.16 实际 21 个 render 函数）
- §14.5 治理清单 34 PushKind 全表对齐（v19.16 实际 34 variant；TurnoverTop enum 待接通）
- 风格统一与共性约束
- 分阶段 PR 提交节奏（预估 13 个 PR）+ 测试矩阵 + 证据 + 回滚

### 1.2 范围（Scope）

**In Scope**

| 项 | 内容 | 数量 |
|---|---|---|
| v13 新增模板 | P-01 / I-01 / I-02 / I-03 / A-01 / A-10 / D-01 | 7 |
| 新规 v13.1 模板 | T-14 ~ T-19 | 6 |
| 现有 21 render 对齐 | 12 项差异（见 §6） | 12 |
| PushKind 治理对齐 | 17 v19.16 实际 enum + 6 v13 [新增] + 6 v13.1 新规 + 3 code-only (CandidateBoard/NewsRanked/CloseCall) + 1 TurnoverTop 接通 + 11 v11 降级注明 = 34 项 | 34 |
| 测试矩阵 | 7+6 新模板 + 35 治理 + 5 红线 + 4 BR | — |
| 紧急治理参数同步 | ST 阈值 / 做市商流动性（不需 PR，先 `config/*.toml` 改） | 2 |
| PR 节奏 | 13 PR 分阶段 | 13 |

**Out of Scope**

- v19.16 已实现且**与 v13 完全一致**的 render 改动（T-01/T-02/T-03 等完全一致部分）
- 推送频道迁移、数据源接入实现、调度（cron）调整
- `--test` 路径回归与真接改造（独立 PR）
- 文档合规审查脚本本身
- 创业板做市商流动性阈值的**算法实现**（仅做治理参数标注）

### 1.3 关联文档

| 文档 | 用途 |
|---|---|
| `docs/architecture/v13-push-templates.md` | 上游 spec（被引） |
| `docs/superpowers/specs/2026-07-06-v13-push-templates-design.md` | **本文件** |
| `docs/superpowers/specs/2026-07-06-v13-push-templates-audit.md` | 审计报告（已产出，含 3 段差异表） |
| `config/risk/stop_loss.toml` | ST 阈值（待更新 5% → 10%） |
| `config/risk/limits.toml` | 流动性阈值（创业板做市商校准） |
| `docs/business_rules.md` | BR 登记（10 个：4 v13 spec + 6 新规 v13.1） |

---

## 2. 现状盘点（审计结果）

### 2.1 当前实现（v19.16）盘点

| 项 | 现状 |
|---|---|
| `PushKind` 枚举 | 22+ variant |
| `push_templates.rs` 行数 | 3545 |
| `notify.rs` 行数 | 1395 |
| 已实现 render 函数 | 21（v19.16 实际 `grep -c "^pub fn render_"` = 21） |
| 完全一致模板 | 11（与 v13 spec） |
| 有差异模板 | 12（见 §6） |
| 代码完全缺失模板 | 7（v13 新增）+ 6（新规 v13.1）= 13 |

### 2.2 审计三段差异表（汇总）

> 完整审计报告见 `docs/superpowers/specs/2026-07-06-v13-push-templates-audit.md`。

**字段级**：11 完全一致 / 12 有差异 / 7 完全缺失（v13）/ 6 完全缺失（新规）

**治理级**：14 完全一致 / 8 有差异 / 13 完全缺失（v13 + 新规）

**风格级**：15+ 完全一致 / 12 有差异（标题/末行/治理元信息错位）

### 2.3 高优先级冲突（必须修复）

| # | 项目 | 性质 | 影响 |
|---|---|---|---|
| 1 | 7 个 v13 新增 + 6 个新规模板完全缺失 | spec 有代码无 | 阻塞 v13 合规 |
| 2 | `PushKind::PaperTrade` Frozen/Unsafe 行为不符 | spec 照发，代码停发 | 阻塞 v13 合规 |
| 3 | `PushKind::T0Advice(禁止)` 等级不符 | spec ℹ️，代码 ⚡ | 阻塞 v13 合规 |
| 4 | `A-05 龙虎榜` 空 entries 兜底文案缺失 | spec 明确要求 | 阻塞 v13 合规 |
| 5 | 盘后 R 系列 `requires_banner=true` 与 spec 不符 | spec 无 banner 占位符 | 视觉不一致 |
| 6 | `PushKind::TurnoverTop/IndustryChain` 冷却不符 | spec 与代码不一致 | 行为偏差 |
| 7 | **ST/*ST 涨跌幅 5% → 10%** | 新规已生效 | 现有持仓风险阈值错配 ⚠️ |
| 8 | **创业板做市商流动性阈值** | 新规已生效 | 中小盘流动性治理需校准 ⚠️ |

### 2.4 中优先级冲突（建议修复）

| # | 项目 | 性质 |
|---|---|---|
| 9 | 3 个 render 末行缺"辅助建议"（T-01/T-02/R-02） | 风格不符 |
| 10 | `PushKind::AuctionVolume` level 应改 Important | 治理不符 |
| 11 | I-08 标题" 盘中" 后缀需删 | 风格不符 |
| 12 | P-02 标题 TopN 改单票 + 改"竞价热点量能" | 风格不符 |
| 13 | 代码文件头注释引用 `v12-push-templates.md` 应改 `v13` | 文档漂移 |

### 2.5 低优先级（注释/归档）

| # | 项目 |
|---|---|
| 14 | `CandidateBoard / NewsRanked / CloseCall` 在 §14.5 表中无登记，建议补行或归档 |
| 15 | T-08 候选失效 复用 `CandidateBoard`，建议升级为独立 enum 或保留 |

---

## 3. v13 模板分类总览

### 3.1 三大类别

| 类别 | 数量 | 模板 ID | 来源 |
|---|---|---|---|
| **A. v13 spec 6+1 `[新增]`** | 7 | P-01 / I-01 / I-02 / I-03 / A-01 / A-10 / D-01 | spec §14 |
| **B. 新规 v13.1 `[2026-07-06]`** | 6 | T-14 / T-15 / T-16 / T-17 / T-18 / T-19 | 新规 |
| **C. 现有 21 render（部分对齐）** | 21 | T-01~T-12, R-01~R-08, P-02~P-04, I-04~I-08, A-02~A-09 | 既有 |
| **D. v11 候选台（仅兼容）** | 3 | CandidateBoard / NewsRanked / CloseCall | v11 降级 |

### 3.2 时间窗口分布

```
┌─────────────────────────────────────────────────────────────┐
│ 盘前        盘中                            盘后/盘后固定   │
│ 09:00       09:30        11:30  13:00  14:57  15:00  15:30  │
│            ▼            ▼      ▼      ▼      ▼      ▼      │
│  ┌────┐  ┌────────────────────────────────────────┐ ┌────┐ │
│  │P-01│  │I-01 I-02 I-03 I-04 I-05 I-06 I-07 I-08│ │T-14│ │
│  │P-02│  │                D-01                   │ │T-15│ │
│  │P-03│  └────────────────────────────────────────┘ │T-17│ │
│  │P-04│                                            │T-18│ │
│  └────┘                                            └────┘ │
│                                          ┌─────────────────┐│
│                                          │  T-19 (北交所)    ││
│                                          │  15:05-15:30 撮合 ││
│                                          └─────────────────┘│
└─────────────────────────────────────────────────────────────┘
                                                  ▼
                              ┌────────────────────────────┐
                              │  盘后复盘 R-01~R-08, A-01, │
                              │  A-10, T-16                │
                              │  19:00 / 21:00             │
                              └────────────────────────────┘
```

**关键时间节点**：
- 09:00 — 盘前新闻 (P-01)
- 09:15 — 深市/北交所 申报起步
- 09:30 — 沪市 申报起步 + 连续竞价
- 11:30 — 上午收盘
- 13:00 — 下午开盘
- 14:57 — 上交所 ETF 集合竞价起步（T-17）
- 15:00 — 主板收盘
- 15:05 — 盘后固定价格撮合起步（T-15）
- 15:30 — 全日收市 + 盘后固定价格撮合结束
- 19:00 — 持仓明日计划 (R-01)
- 21:00 — 龙虎榜 (R-04) + 失败归因 (R-06)

---

## 4. v13 新增模板详细设计

> 7 个模板按 v13 spec 序号排列。每个模板：**字段语义 / 数据源 / 失败模式 / 合 Gate / Banner / PR 证据**。

### 4.1 P-01 PreopenNewsHot（盘前新闻热点）⚡ — P0

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填，缺失报错 |
| `theme_1/2/3` | `&str` | news_monitor cluster | 整段省略 |
| `news_1/2` + `chain_1/2` | `&str` + `&str` | news_monitor event | 整行省略 |
| `name/code/reason` ×N | `&str` | 候选台 + news | 跳过该行 |

**治理**：`Important` / `Some(900)` / `requires_banner=false`（盘前无持仓语义）/ `is_deprecated=false`。

**红线 Gate**：2.1 / 2.2 / 2.4。

**PR 证据**：`Refs: spec §14.1 P-01` / `Data-Redlines: [2.1, 2.2, 2.4]` / `Business-Rules: BR-NEWS-CLUSTER`。

### 4.2 I-01 IntradayMarket（盘中轮动总览）⚡ — P0

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填 |
| `tech_ai/hbm/smartphone` + `score` | `&str` + `f32` | sector_rotation | "N/A" |
| `power_uhv/grid/ess` + `score` | 同上 | 同上 | 同上 |
| `robot_reducer/servo/vision` + `score` | 同上 | 同上 | 同上 |
| `subsector` + 状态 | `&str` | 既有 | "暂无主攻" |

**治理**：`Important` / `Some(900)` / `requires_banner=true`（盘中 ⚡）/ `is_deprecated=false`。

**红线 Gate**：2.3（score > ±100 panic）/ 2.4。

**PR 证据**：`Refs: spec §14.2 I-01` / `Data-Redlines: [2.3, 2.4]` / `Threshold-Proof: score clamp [-100, 100]`。

### 4.3 I-02 NewsCatalyst（新闻催化映射）⚡ — P0

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填 |
| `headline` | `&str` | news_monitor | 必填 |
| `theme` | `&str` | news cluster | "未分类" |
| `name/code/chg/reason` ×N | `&str/f32/&str` | 实时行情 + news reason | chg 缺失整行省略 |

**治理**：`Important` / `Some(600)` / `requires_banner=true` / `is_deprecated=false`。

**红线 Gate**：2.1 / 2.3（chg > ±20% panic）/ 2.4。

**PR 证据**：`Refs: spec §14.2 I-02` / `Data-Redlines: [2.1, 2.3, 2.4]` / `Business-Rules: BR-NEWS-CATALYST`。

### 4.4 I-03 IndustryChainIntraday（盘中涨停扩散）⚡ — P0

> **审计多发现**：v13 spec §14.2 I-03 要求盘中形态，但现有 `render_industry_chain` 是盘后 R-03 形态，需拆分。

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填 |
| `chain` + `count` | `&str` + `u32` | 板块涨停扫描 | "无涨停" |
| `leader_name/code` + `连板高度` | `&str` + `u32` | 涨停龙头 | "暂无龙头" |
| `补涨候选` ×N | `&str` | 候选台 | 整段省略 |

**治理**：`Important` / `Some(1800)`（30min）/ `requires_banner=true` / `is_deprecated=false`。

**红线 Gate**：2.1 / 2.4。

**PR 证据**：`Refs: spec §14.2 I-03` / `Data-Redlines: [2.1, 2.4]`。

**关键决策**：与 `R-03`（盘后 A-04）拆分，新 enum `IndustryChainIntraday`，不合并。

### 4.5 A-01 PaperReview（虚拟仓复盘）ℹ️ — P1

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `date` | `&str` | 调度器 | 必填 |
| `name/code/trigger` | `&str` | virtual_watch DB | 整行省略 |
| `desc/pnl` | `&str` / `f32` | virtual_close DB | pnl 缺失 → "N/A%" |
| `plan_high/flat/low` | `&str` ×3 | 调用点计算（依赖 T-11，A-01 PR #4 落实） | "暂无计划" |

**治理**：`Info` / `Some(86400)` / `requires_banner=false`（盘后）/ `is_deprecated=false`。

**红线 Gate**：2.1 / 2.4（盘后 21:00 后取数）。

**前置依赖**：T-11 竞价复算通路（v12-dev-plan.md §MVP-3）。

### 4.6 A-10 CatalystReview（盘后题材催化复盘）⚡(盘后) — P0

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `date` | `&str` | 调度器 | 必填 |
| `theme` | `&str` | news cluster | 必填 |
| `score` | `f32` | 板块强度 | "N/A" |
| `persistent` | `enum`（high/med/low） | 持续性判定 | "med" |
| `started_names/pending_names/watch` | `&[&str]` | news + 候选台 | 整段省略 |

**治理**：`Important` / `Some(86400)`（盘后段 15:30-23:00 触发即记当日已推）/ `requires_banner=false` / `is_deprecated=false`。

**红线 Gate**：2.1 / 2.3 / 2.4。

**PR 证据**：`Refs: spec §14.3 A-10` / `Business-Rules: BR-THEME-STAGE`。

### 4.7 D-01 NewsToIdea（新闻驱动个股）⚡ — P0

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `banner` | `BannerCtx` | 既有 | 必填 |
| `HH:MM` | `&str` | 调度器 | 必填 |
| `headline` | `&str` | news_monitor | 必填 |
| `theme/stage` | `&str` / `enum` | news cluster | stage → "启动" |
| `name/code` | `&str` | 候选台 + news | 必填 |
| `reason_1/2` | `&str` | 候选台 + news | 整行省略 |
| `action` | `enum`（观察/低吸/不追） | 候选台 | 整段省略 |

**治理**：`Important` / `Some(1200)`（20min/票）/ `requires_banner=true` / `is_deprecated=false`。

**红线 Gate**：2.1 / 2.4 / 2.6。

**PR 证据**：`Refs: spec §14.4 D-01` / `Business-Rules: BR-NEWS-TO-IDEA`。

**Banner 时段约束**（Codex F7）：D-01 全天持续推送，但 banner 在不同时段需调整：
- **盘前（09:00 前）**：banner 省略"日盈亏"和"仓位%"（无交易语境）
- **盘中（09:30-15:00）**：标准 banner
- **盘后（15:00 后）**：banner 保留但仓位语义弱化
- 实现：PR #2 中 `BannerCtx::render_for_session(Preopen|Intraday|Post)` 变体函数（PR #2 范围扩展）

---

## 5. 新规 v13.1 模板详细设计

> 6 个新规模板按时间顺序排列。每个模板：**新规依据 / 字段语义 / 数据源 / 失败模式 / 合 Gate / Banner / PR 证据**。

### 5.1 新规总览（关联沪深北《交易规则（2026 修订）》）

| 序号 | 新规条款 | 影响模板 |
|---|---|---|
| ① | 盘后固定价格交易扩围（全部 A 股 + 沪深 ETF） | T-14 / T-15 |
| ② | 主板 ST/*ST 涨跌幅 5%→10% | T-16 |
| ③ | 上交所基金收盘改集合竞价 | T-17 |
| ④ | 创业板引入做市商制度 | 治理参数（不需新模板） |
| ⑤ | 创业板协议大宗盘中实时确认 | T-18 |
| ⑥ | 北交所大宗价格范围（实时均价） | T-19 |

### 5.2 T-14 PostFixedPriceOrder（盘后固定价格申报）⚡ — P0

**新规依据**：① 盘后固定价格交易扩围，申报时间按交易所区分。

**时间窗口**：
- 沪市 A 股/ETF：9:30-11:30, 13:00-15:30
- 深市 A 股/ETF + 北交所 A 股：9:15-11:30, 13:00-15:30

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `exchange` | `enum`（SH/SZ/BJ） | 调度器 | 必填 |
| `HH:MM` | `&str` | 调度器 | 必填 |
| `name/code/price/qty` | `&str/f64/u32` | 委托回报 | 整行省略 |
| `order_id` | `&str` | 委托 ID | 必填 |
| `status` | `enum`（已报/已撤/废单） | 委托状态 | 必填 |
| `window` | `enum`（上午/中午/下午） | 调度器 | 由 HH:MM 派生 |

**治理**：`Important` / `Some(60)`（1min/票）/ `requires_banner=true` / `is_deprecated=false`。

**红线 Gate**：2.1 / 2.4（申报时效 ≤ 30s）/ 2.6（不自动下单）。

**PR 证据**：`Refs: 新规 §5.2 + spec §14.5 v13.1` / `Data-Redlines: [2.1, 2.4, 2.6]` / `Business-Rules: BR-POST-FIXED-PRICE`（含申报窗口规则）。

**交易所过滤**：模板作用域依 `exchange` 字段，仅在窗口期内推送。

### 5.3 T-15 PostFixedPriceFill（盘后固定价格成交）⚡ — P0

**新规依据**：① 撮合时间 15:05-15:30 固定价格撮合。

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `exchange` | `enum` | 调度器 | 必填 |
| `HH:MM` | `&str` | 调度器 | 必填（15:05-15:30） |
| `name/code/fill_price/qty` | `&str/f64/u32` | 成交回报 | 整行省略 |
| `vs_limit_price` | `f32`（成交价 vs 涨跌幅价差） | 计算 | "N/A" |
| `next_session_carry` | `bool` | 计算（是否过户到次一交易日） | false |

**治理**：`Important` / `Some(300)`（5min/票）/ `requires_banner=true` / `is_deprecated=false`。

**红线 Gate**：2.1 / 2.4 / 2.6。

**PR 证据**：`Refs: 新规 §5.3 + spec §14.5 v13.1` / `Business-Rules: BR-POST-FIXED-PRICE`。

### 5.4 T-16 StPriceLimitChanged（ST 涨跌幅变更提醒）⚡ — P0

**新规依据**：② 主板 ST/*ST 涨跌幅 5% → 10%（与主板其他股票一致）。

**新规生效**：2026-07-06（**今天**）— 现有持仓风险阈值可能错配。

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填 |
| `name/code` | `&str` | 持仓 DB | 必填 |
| `st_type` | `enum`（ST / *ST） | 持仓 DB | 必填 |
| `old_limit` / `new_limit` | `f32` | 新规参数 | 必填 |
| `holding_qty/cost/now_price` | `u32/f64/f64` | 持仓 DB | 必填 |
| `new_stop_loss/new_take_profit` | `f64` | 调用点重算（基于 10%） | "未重算" |

**治理**：`Important` / `Some(86_400)`（1次/票/日，开盘触发即记已推）/ `requires_banner=true` / `is_deprecated=false`。

**红线 Gate**：2.1 / 2.6（仅提示，不自动调仓）。

**PR 证据**：`Refs: 新规 §5.4` / `Data-Redlines: [2.1, 2.6]` / `Business-Rules: BR-ST-PRICE-CHANGE` / `Threshold-Proof: 5% → 10% (新规)`。

**紧急同步项**（**不需 PR，先改 config**）：
- `config/risk/stop_loss.toml`: `st_price_limit = 0.10`（原 0.05）
- `config/strategy.toml`: `st_take_profit_pct = 0.10`（原 0.05）
- `docs/business_rules.md`: 登记 BR-ST-PRICE-CHANGE

### 5.5 T-17 EtfClosingCallAuction（ETF 收盘集合竞价）ℹ️ — P1

**新规依据**：③ 上交所基金收盘由连续竞价改为**收盘集合竞价**（14:57-15:00）。

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填（14:57-15:00） |
| `name/code` | `&str` | 持仓 DB | 必填 |
| `call_auction_price` | `f64` | 集合竞价行情 | "暂无" |
| `vs_continuous_est` | `f32` | 估值（与连续竞价对比） | "N/A" |
| `liquidity_note` | `&str` | 尾盘操纵风险 | "正常" |

**治理**：`Info` / `Some(86_400)`（1次/日，仅沪市 ETF）/ `requires_banner=false` / `is_deprecated=false`。

**红线 Gate**：2.1 / 2.4。

**作用域过滤**：`exchange == SH && instrument_type == ETF`。

**PR 证据**：`Refs: 新规 §5.5` / `Business-Rules: BR-CLOSING-CALL-AUCTION`。

### 5.6 T-18 BlockTradeIntradayConfirm（创业板大宗盘中确认）ℹ️ — P1

**新规依据**：⑤ 创业板股票协议大宗交易成交确认时间由"盘后统一确认"→"盘中实时确认"（与科创板一致）。

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填 |
| `name/code/qty/price` | `&str/u32/f64` | 大宗成交回报 | 必填 |
| `block_type` | `enum`（协议大宗/竞价大宗） | 大宗类型 | 必填 |
| `board` | `enum`（创业/科创/主板） | 板块 | 必填 |
| `real_time_confirm` | `bool` | 是否盘中实时 | 必填 |
| `next_session_settle` | `enum`（次日/实时） | 清算节奏 | 必填 |

**治理**：`Info` / `Some(300)`（5min/票）/ `requires_banner=false` / `is_deprecated=false`。

**红线 Gate**：2.1 / 2.6。

**PR 证据**：`Refs: 新规 §5.6` / `Business-Rules: BR-BLOCK-TRADE-CONFIRM`。

### 5.7 T-19 BlockTradePriceRange（北交所大宗价格区间）ℹ️ — P1

**新规依据**：⑥ 北交所无涨跌幅限制股票的大宗交易价格范围由"前收盘价"→"当日竞价交易实时成交均价"。

| 字段 | 类型 | 数据源 | 缺失行为 |
|---|---|---|---|
| `HH:MM` | `&str` | 调度器 | 必填 |
| `name/code` | `&str` | 持仓 DB | 必填 |
| `prev_close` | `f64` | 前收盘价 | "N/A" |
| `today_avg_price` | `f64` | 当日竞价实时均价 | 必填 |
| `block_price_range` | `&str` | 计算（基于当日均价 ± N%） | "暂无" |
| `note` | `&str` | 口径说明（"原: 前收盘 / 现: 当日均价"） | 必填 |

**治理**：`Info` / `Some(3600)`（60min/票）/ `requires_banner=false` / `is_deprecated=false`。

**红线 Gate**：2.1 / 2.4。

**作用域过滤**：`exchange == BJ && no_price_limit == true`。

**PR 证据**：`Refs: 新规 §5.7` / `Business-Rules: BR-BLOCK-TRADE-PRICE-RANGE`。

### 5.8 创业板做市商（治理参数变更，不需新模板）

**新规依据**：④ 创业板引入做市商制度，中小市值流动性增强。

**变更项**：
- `config/risk/limits.toml`: `gem_small_cap_liquidity_threshold` 调高 15-20%
- `docs/business_rules.md`: 登记 BR-GEM-MARKET-MAKER（流动性阈值校准）

**PR 证据**：作为 PR #11（治理全表对齐）一部分。

---

## 6. 现有 24 render 对齐设计

> 12 项差异逐项修复。每项：**现状 / spec 要求 / 修复方向 / 风险 / 关联 PR**。

### 6.1 字段级修复（12 项，Codex F4 已修正）

| # | 模板 | 现状 | spec 要求 | 修复方向 | 关联 PR |
|---|---|---|---|---|---|
| F-01 | T-01 AccountMode | 末行"解除条件" | 加"辅助建议, 非下单指令" | 末行拼接 spec 第 6 条 | PR #12 |
| F-02 | T-02 DataMode | 末行"恢复预计" | 同上 | 同上 | PR #12 |
| F-03 | T-03~T-07 等 11 个 | — | ✅ 已含 | — | — |
| F-04 | R-02 ReviewMarket | 末行"明日建议" | 加"辅助建议" | 末行拼接 | PR #12 |
| F-05 | P-02 AuctionVolume | 标题"🌅 竞价异动 TopN" | "🌅 竞价热点量能（09:2{x}）" | 标题改 + 去掉 TopN + 加"辅助建议" | PR #8 |
| F-06 | P-04 PaperTrade | spec 标签 `{Filled|...}`，代码 `.label()` | 兼容 | 保留代码现状，spec 加注 | — |
| F-07 | I-08 TurnoverTop | 标题 `🔄 盘中换手率 Top10 (HH:MM 盘中)` | `🔄 盘中换手率 Top10 (HH:MM)` | 删" 盘中" 后缀 | PR #8 |
| F-08 | A-05 ReviewLhb | 缺空 entries 兜底 | `盘中无数据 (盘后 21:00 才更新), 请参考 T-13` | 加 `if entries.is_empty()` 分支 | PR #8 |
| F-09 | A-06 ReviewSignal | 缺"做T建议"括号说明 | spec 加括号注脚 | 代码原样保留 | — |
| F-10 | A-07 ReviewFailure | 段落顺序"原信号/结果/归因/处理建议/─────/分布" | 与 spec 一致 | 保持 | — |
| F-11 | I-03 IndustryChain (盘中) | 复用 R-03 形态 | 独立盘中形态 | 新增 `IndustryChainIntraday` enum + render（见 §4.4） | PR #4 |
| F-12 | T-08 CandidateInvalidated | 复用 `CandidateBoard` | spec 无独立项 | 升级为 `CandidateInvalidated` 独立 enum 或保留并补 spec | PR #11 |

**Emoji 一致性验证**（Codex F15）：PR #1-#10 实施时每个 render 测试加 `assert!(out.starts_with("<expected emoji>"))` 断言，确保 13 新模板首字符 emoji 与 spec §14.x 一致。可选：`tools/compliance/check_emoji.sh` 静态扫描脚本（独立 PR，不在本批次）。

### 6.2 治理级修复（8 项）

| # | PushKind | 现状 | spec §14.5 要求 | 修复方向 | 关联 PR |
|---|---|---|---|---|---|
| G-01 | `T0Advice`（禁止路径） | `Important` | `Info` | 在 `render_t0_forbid` 调用点显式标 `PushLevel::Info`（不改 enum） | PR #11 |
| G-02 | `ForbiddenOps` | `is_deprecated=false` | spec `true`（v19.12 起全保留） | 改 `is_deprecated=false`，spec 加注脚说明 v19.12 决定 | PR #11 |
| G-03 | `PaperTrade` | `should_block_on_mode` 含 (Frozen/Unsafe 停发) | spec 照发 | `should_block_on_mode` 移出 `PaperTrade`，改照发 | PR #11 |
| G-04 | `AuctionVolume` | `Info` | `Important` | `level()` 改 `Important` | PR #11 |
| G-05 | `TurnoverTop` | 默认 `Some(1800)` | `Some(600)` | 显式加 `PushKind::TurnoverTop => Some(600)` | PR #11 |
| G-06 | `IndustryChain` | 默认 `Some(1800)` | `Some(86400)` | 显式加 `Some(86400)` | PR #11 |
| G-07 | `DailyReport` / `ReviewMarket` 等盘后 R 系列 | `requires_banner=true` | spec 模板无 banner 占位符 | `requires_banner()` matches 移除盘后 R 系列 | PR #11 |
| G-08 | `CandidateBoard` / `NewsRanked` / `CloseCall` | spec §14.5 无登记 | 应登记或归档 | spec §14.5 补 3 行 | PR #11 |

### 6.3 风格级修复（2 项）

| # | 项目 | 现状 | 要求 | 修复 | 关联 PR |
|---|---|---|---|---|---|
| S-01 | 代码文件头注释 | 引用 `v12-push-templates.md` | 引用 `v13-push-templates.md` | 改注释 | PR #11 |
| S-02 | Emoji 一致性 | 已实现 render emoji 正确 | 6 新增 + 6 新规 emoji 按 spec | 实施时按 spec | PR #1~#10 |

---

## 7. 治理元信息全表对齐（34 PushKind）

### 7.1 治理表（§14.5 扩展）

| PushKind | 等级 | 冷却 | Frozen/Unsafe | is_deprecated | 现状 | 来源 |
|---|---|---|---|---|---|---|
| `AccountMode` | ⚡ | 0（状态即推） | 照发 | false | ✅ 一致 | v12 |
| `DataMode` | ⚡ | 10min | 照发 | false | ✅ 一致 | v12 |
| `HoldingPlan` | ⚡ | 30min/票 | **停发** | false | ✅ 一致 | v12 |
| `HoldingEvent` | 🚨 | 无视冷却 | 照发 | false | ✅ 一致 | v12 |
| `T0Advice`（建议） | ⚡ | 30min/票 | **停发** | false | ✅ 一致 | v12 |
| `T0Advice`（禁止） | ℹ️ | 30min/票 | 照发 | false | ⚠️ G-01 | v12 |
| `CandidateTriggered` | ⚡ | 1次/票/日 | **停发** | false | ✅ 一致 | v12 |
| `ForbiddenOps` | ℹ️ | 60min/票 | 照发 | true（v19.12 注） | ⚠️ G-02 | v12 |
| `PaperTrade` | ℹ️ | 5min批 | **照发**（spec 改） | true（v19.12 注） | ⚠️ G-03 | v12 |
| `AuctionVolume` | ⚡ | 10min | 视数据质量 | false | ⚠️ G-04 | v12 |
| `TurnoverTop` | ℹ️ | 10min | 照发 | false | ⚠️ G-05 (enum 未接通) | v19.15 |
| **`IntradayMarket`** | ⚡ | 15min | 照发 | false | **缺** | v13 §14.2 I-01 |
| **`NewsCatalyst`** | ⚡ | 10min | 照发 | false | **缺** | v13 §14.2 I-02 |
| **`NewsToIdea`** | ⚡ | 20min/票 | ReduceOnly可发 / Frozen谨慎 | false | **缺** | v13 §14.4 D-01 |
| **`PreopenNewsHot`** | ⚡ | 15min | 可发 | false | **缺** | v13 §14.1 P-01 |
| **`PaperReview`** | 盘后 | 1次/日 | 可发 | false | **缺** | v13 §14.3 A-01 |
| `DailyReport` | 盘后 | 1次/日 | 可发 | false | ✅ 一致 | v12 R-01 |
| `ReviewMarket` | 盘后 | 1次/日 | 可发 | false | ✅ 一致 | v12 R-02 |
| **`IndustryChain`** | 盘后 | 1次/日 | 可发 | false | ⚠️ G-06 | v12 R-03（盘中 I-03 独立） |
| **`IndustryChainIntraday`** | ⚡ | 30min | 照发 | false | **缺** | v13 §14.2 I-03 (Codex F5: 需更新上游 spec §14.2 I-03 + §14.5 治理表) |
| `ReviewLhb` | 盘后 | 1次/日(21:00) | 可发 | false | ✅ 一致 | v12 R-04 |
| `ReviewSignal` | 盘后 | 1次/日 | 可发 | false | ✅ 一致 | v12 R-05 |
| `ReviewFailure` | 盘后 | 1次/日 | 可发 | false | ✅ 一致 | v12 R-06 |
| `TomorrowWatch` | 盘后 | 1次/日 | 可发 | false | ✅ 一致 | v12 R-07 |
| `EventCalendar` | 盘后 | 1次/日 | 可发 | false | ✅ 一致 | v12 R-08 |
| **`CatalystReview`** | ⚡(盘后) | 1次/日 | 可发 | false | **缺** | v13 §14.3 A-10 |
| **`PostFixedPriceOrder`** | ⚡ | 1min/票 | 照发 | false | **缺** | 新规 §5.2 T-14 |
| **`PostFixedPriceFill`** | ⚡ | 5min/票 | 照发 | false | **缺** | 新规 §5.3 T-15 |
| **`StPriceLimitChanged`** | ⚡ | 1次/票/日 | 照发 | false | **缺** | 新规 §5.4 T-16 |
| **`EtfClosingCallAuction`** | ℹ️ | 1次/日 | 照发 | false | **缺** | 新规 §5.5 T-17 |
| **`BlockTradeIntradayConfirm`** | ℹ️ | 5min/票 | 照发 | false | **缺** | 新规 §5.6 T-18 |
| **`BlockTradePriceRange`** | ℹ️ | 60min/票 | 照发 | false | **缺** | 新规 §5.7 T-19 |
| `CandidateBoard`（仅兼容） | ⚡ | 30min/票 | 照发 | false | ⚠️ G-08（spec 补登） | v11 |
| `NewsRanked`（仅兼容） | ⚡ | 30min | 照发 | false | ⚠️ G-08（spec 补登） | v11 |
| `CloseCall`（T-12） | ⚡ | 1次/日 | 照发 | false | ⚠️ G-08（spec 补登） | v12 |

**合计**：34 PushKind（17 v19.16 enum 现有 + 6 v13 [新增] + 6 v13.1 新规 + 3 code-only 需补登 + 1 TurnoverTop 接通 + 1 v11 降级归类）

### 7.2 requires_banner() 对齐

> spec §14.0.1 仅对"交易建议类"要求 banner。盘后 R 系列在 spec 模板中**无 banner 占位符**。

**修复**（G-07）：`requires_banner()` matches 中移除以下盘后 R 系列：

```
- DailyReport
- ReviewMarket
- ReviewLhb
- ReviewSignal
- ReviewFailure
- TomorrowWatch
- EventCalendar
- PaperReview
- CatalystReview  // 盘后非交易建议
```

保留 banner 的：`HoldingPlan / HoldingEvent / T0Advice / CandidateTriggered / PaperTrade / AuctionVolume / IntradayMarket / NewsCatalyst / NewsToIdea / PostFixedPriceOrder / PostFixedPriceFill / StPriceLimitChanged`

> **决策统一**（Codex F3）：PreopenNewsHot 盘前无持仓语义，`requires_banner=false`；盘后 R 系列 + PaperReview + CatalystReview 同样 `requires_banner=false`。

---

## 8. 风格统一与共性约束

### 8.1 全局约定（继承 §14.0）

| # | 约束 | 验证 |
|---|---|---|
| 1 | 纯文本 + emoji 标题 + `（HH:MM）` 时间戳 | 全部 render 一致 |
| 2 | 行内字段 ` \| ` 分隔 | ✅ 全部 render 已用 |
| 3 | `{xxx}` 变量占位 / `[...]` 条件段 | ✅ 已用 `if let Some()` / `if !xxx.is_empty()` |
| 4 | 交易建议类必带 banner | 修复后见 §7.2 |
| 5 | 新增 PushKind 必登记治理 | ✅ 13 新增全部登记（§7.1） |
| 6 | 末行"辅助建议, 非下单指令"（交易建议类） | 修复后见 §6.1 F-01/F-02/F-04 |

### 8.2 共性抽象

新增 `RenderCtx` trait（**内部 trait，不导出**）封装：

```rust
trait RenderCtx {
    fn level(&self) -> PushLevel;
    fn banner_required(&self) -> bool;
    fn cooldown_secs(&self) -> Option<u64>;
    fn is_deprecated(&self) -> bool;
    fn requires_helper_line(&self) -> bool;  // 末行"辅助建议"
}
```

6 套样板（PushKind 元信息 match）收敛为 trait 实现。

---

## 9. 测试矩阵

### 9.1 render 单测（7 + 6 = 13 新模板）

| 模板族 | 用例数 | 维度 |
|---|---|---|
| v13 7 新增 | 19（沿用 v13 spec §4.1） | emoji/HH:MM/字段映射/缺失/末行 |
| 新规 6 模板 | 12（每模板 ~2 用例） | 时间窗口/交易所过滤/字段映射/缺失 |
| 现有 24 对齐回归 | 12（每差异 1 用例） | 修复后正确性 |

**合计**：43 render 用例。

### 9.2 治理元信息测试（34 PushKind）

| PushKind | 断言维度 |
|---|---|
| 全部 34 | `level() / cooldown_secs() / requires_banner() / is_deprecated() / counts_against_daily_budget()` |

**合计**：34 × 5 = 170 治理断言（实际实现 plan gov 测试 ≈ 17 个，仅覆盖 P0/P1 新增模板；其余依赖类型系统穷尽性 `match`）。

### 9.3 红线门禁测试（ENGINEERING_RULES_V2 §2.1~§2.10）

| 红线 | 用例 ID | 验证 |
|---|---|---|
| §2.1 无 mock | `UT-RL21-NEW` | 13 render 不接受 mock 标志 |
| §2.2 缺数据显式 | `UT-RL22-NEW` | None → 空段/"N/A" |
| §2.3 坏数据 | `UT-RL23-NEW` | score>±100 / chg>±20% / 新规价格异常 `debug_assert!` |
| §2.4 时效 | `UT-RL24-NEW` | 申报时效/集合竞价窗口超时报 warning |
| §2.6 下单防护 | `UT-RL26-NEW` | T-14/T-15 不自动下单（与 §2.6 一致） |
| §2.8 假实现 | 复用 check_fake_impl.sh | — |
| §2.10 BR 登记 | `UT-BR-NEW` | 10 个 BR 引用 |

**合计**：6 红线 + 6 BR 引用。

### 9.4 覆盖率目标

| 模块 | 目标 |
|---|---|
| `push_templates.rs` 行覆盖 | ≥ 85% |
| `notify.rs` 治理分支 | ≥ 90% |
| `main.rs` 调用点 | ≥ 70% |

---

## 10. PR 节奏与证据

### 10.1 PR 计划（13 PR）

| PR # | 标题模板 | 范围 | 优先级 | 关联 Gate |
|---|---|---|---|---|
| **#1** | `feat(v13): PushKind 新增 PreopenNewsHot/IntradayMarket/NewsCatalyst + 治理` | 3 v13 P0 | P0 | A→B→C→D |
| **#2** | `feat(v13): PushKind 新增 NewsToIdea + 治理` | 1 v13 P0 | P0 | A→B→C→D |
| **#3** | `feat(v13): PushKind 新增 CatalystReview 盘后 + 治理` | 1 v13 P0 | P0 | A→B→C→D |
| **#4** | `feat(v13): PushKind 新增 IndustryChainIntraday 盘中形态 + 治理` | 1 v13 P0（审计多发现） | P0 | A→B→C→D |
| **#5** | `feat(v13): PushKind 新增 PaperReview 盘后 + 治理 (前置 T-11)` | 1 v13 P1 + `#[ignore]` | P1 | A→B；D 等 T-11 |
| **#6** | `feat(v13.1): PushKind 新增 PostFixedPriceOrder/Fill 盘后固定价格 + 治理` | 2 新规 P0 | P0 | A→B→C→D |
| **#7** | `feat(v13.1): PushKind 新增 StPriceLimitChanged ST 涨跌幅变更 + 治理` | 1 新规 P0 | P0 | A→B→C→D |
| **#8** | `fix(v13): 现有 render 对齐 §14 风格与字段（12 项差异）` | F-01~F-12 | P0 | B→C |
| **#9** | `feat(v13.1): PushKind 新增 EtfClosingCallAuction 沪市 ETF + 治理` | 1 新规 P1 | P1 | A→B→C |
| **#10** | `feat(v13.1): PushKind 新增 BlockTradeIntradayConfirm/PriceRange 大宗 + 治理` | 2 新规 P1 | P1 | A→B→C |
| **#11** | `chore(v13): §14.5 治理全表对齐 + 34 PushKind + requires_banner 修正` | G-01~G-08 | 收尾 | C |
| **#12** | `chore(v13): 文档漂移修正 + spec 文件头注释 v13` | S-01/S-02 | 收尾 | C |
| **#13** | `chore(v13): 紧急治理参数同步（ST 阈值 + 做市商流动性）` | config/*.toml + docs/business_rules.md | **紧急** | 立即 |

**累计**：13 PR / 总行数预估 +3500~5000（含测试）

### 10.2 紧急治理参数同步（**不需 PR，先做**）

> 新规 2026-07-06 已生效，治理参数需**立即**同步：

```bash
# 1. ST/*ST 涨跌幅 5% → 10%
vim config/risk/stop_loss.toml  # st_price_limit = 0.10
vim config/strategy.toml         # st_take_profit_pct = 0.10

# 2. 创业板做市商流动性阈值
vim config/risk/limits.toml      # gem_small_cap_liquidity_threshold += 0.15

# 3. BR 登记
vim docs/business_rules.md       # +BR-ST-PRICE-CHANGE, BR-GEM-MARKET-MAKER, BR-POST-FIXED-PRICE, BR-CLOSING-CALL-AUCTION, BR-BLOCK-TRADE-CONFIRM, BR-BLOCK-TRADE-PRICE-RANGE
```

完成后 commit：`urgent(v13.1): 新规 2026-07-06 治理参数同步 + BR 登记`

### 10.3 PR 证据样例（以 #6 盘后固定价格为例）

```markdown
## feat(v13.1): PushKind 新增 PostFixedPriceOrder/Fill 盘后固定价格 + 治理

### Refs
- 新规: 沪深北《交易规则（2026 修订）》§5.2/§5.3 (盘后固定价格交易扩围)
- spec: `docs/architecture/v13-push-templates.md §14.5 v13.1`
- design: `docs/superpowers/specs/2026-07-06-v13-push-templates-design.md §5.2, §5.3`

### Data-Redlines
- [2.1] 无 mock：仅依赖真实委托/成交回报
- [2.4] 时效：申报时效 ≤ 30s；撮合窗口 15:05-15:30
- [2.6] 不自动下单：仅推送状态

### OldModules
| 模块 | adopt/reject | 原因 |
|---|---|---|
| `order_report` | adopt | 复用既有委托回报 schema |
| `block_trade_handler` | reject | 大宗交易走独立模板（T-18/T-19） |

### Threshold-Proof
- 申报窗口：沪市 9:30-15:30；深市/北交所 9:15-15:30（按 exchange 区分）

### Business-Rules
- BR-POST-FIXED-PRICE（盘后固定价格申报+撮合规则）

### Validation
- `cargo fmt --check` ✓
- `cargo clippy -D warnings` ✓
- `cargo test push_templates::tests::post_fixed_price_*` ✓
- `bash tools/compliance/check.sh` ✓

### Rollback
\`\`\`bash
git revert <commit-sha>
cargo build --release  # 验证无 dangling ref
\`\`\`
```

### 10.4 回滚策略（L1~L4）

| 层 | 触发 | 操作 |
|---|---|---|
| L1 单 PR | 单模板错乱 | `git revert <sha>` |
| L2 PR 链 | P0 失败 | 暂停，回 Gate A |
| L3 设计缺陷 | ≥3 PR 共同失败 | 整体回 v19.16 baseline |
| L4 红线违规 | §2.1/§2.5/§2.6 | 立刻阻断 + 24h 复盘 |

### 10.5 红线触发与阻断（每 PR 必跑）

```bash
cargo fmt --check                    # 格式
cargo clippy -D warnings             # 静态
cargo test                           # 单测
bash tools/compliance/check.sh       # 合规
tools/compliance/lib/check_data_freshness.sh
tools/compliance/lib/check_fake_impl.sh
tools/compliance/lib/check_design_contradiction.sh
tools/compliance/lib/check_business_rules.sh
```

---

## 11. 验收清单（DoD）

| # | 项 | 验证 |
|---|---|---|
| 1 | 紧急治理参数同步（ST + 做市商）已完成 | `git log --oneline \| grep urgent(v13.1)` |
| 2 | 设计文档落盘 + git commit | `git log -- docs/superpowers/specs/2026-07-06-v13-push-templates-design.md` |
| 3 | 9 P0 PR（#1~#4, #6, #7, #8, #11）合并 | `git log --oneline \| grep "feat/v13"` |
| 4 | 4 P1 PR（#5, #9, #10, #12）合并 | 同上 |
| 5 | §14.5 治理表 34 PushKind 100% 对齐 | PR #11 审计脚本 0 差量 |
| 6 | 27 render 用例 + 17 治理 + 6 红线 全绿 | `cargo test` |
| 7 | 覆盖率 ≥ 85% / 90% / 70% | `cargo tarpaulin` |
| 8 | 紧急 BR 6 个已登记 | `docs/business_rules.md` |
| 9 | 无 mock/fake 数据 | `check_fake_impl.sh` |
| 10 | PR 证据 6 字段齐 | PR 模板 review |

---

## 12. 关键依赖与外部约束

| 依赖 | 状态 | 备注 |
|---|---|---|
| `news_monitor_loop` | ✅ | 复用 |
| `sector_rotation` | ✅ | 复用 |
| `virtual_watch` DB | ✅ | A-01 依赖 |
| T-11 竞价复算 | ⚠️ 未就绪 | A-01 前置 |
| `BannerCtx` | ✅ | 复用 |
| `push_governor` | ✅ | 复用 |
| `SignalStateMachine` | ✅ | 复用 |
| `order_report` | ✅ | T-14/T-15 复用 |
| `block_trade_handler` | ✅ | T-18/T-19 复用 |

---

## 13. 与现有规范的一致性自查

| 规范 | 满足方式 |
|---|---|
| AGENTS §1 强制预飞行 | 每 PR 模板含 Impacted/Triggered/Validation/Rollback |
| AGENTS §2 Gate A→D | 13 PR 严格串行 |
| AGENTS §3 数据红线 | 13 模板均映射 §2.1/§2.2/§2.4（含 §2.3/§2.6） |
| AGENTS §6 PR 证据 | PR 模板样例覆盖 6 字段 |
| AGENTS §7 根因回退 | L1~L4 分层 |
| ENGINEERING_RULES_V2 §1 Gate A | 本文档即 Gate A 产物 |
| ENGINEERING_RULES_V2 §2 红线 | 13 模板逐项映射 |
| ENGINEERING_RULES_V2 §3 PR 模板 | PR 证据样例已给 |
| ENGINEERING_RULES_V2 §4 根因回退 | L1~L4 对齐 |
| ENGINEERING_RULES_V2 §5 受控例外 | A-01 `#[ignore]` + 前置依赖 |
| ENGINEERING_RULES_V2 §6 双层门禁 | Fast + Full 一致 |

---

## 14. 范围外但需提前沟通

1. **调度接入**：cron 时机由运维 PR 排，本设计仅约束调用点
2. **真接测试数据**：D-01/A-10/T-14/T-15 需注入 fixture（独立 PR）
3. **i18n**：中文硬编码与 v19.x 一致
4. **bark/push channel**：v19.x 已稳定
5. **创业板协议大宗盘中实时确认**：T-18 推送时机需与现有大宗 handler 协调（独立 PR 调整 handler）

---

## 15. 下一步

设计文档完成后：

1. ✅ 写入设计文档（**本文件**）
2. ✅ Spec 自审（已完成 4 处修正，commit `da54a29`）
3. 🙋 **请用户审阅 spec**（当前阶段）
4. 🚀 进入 writing-plans skill → 产出 `v13-implementation-plan.md`（13 PR 任务卡）

---

## 附录 A：术语与引用

| 术语 | 含义 |
|---|---|
| PushKind | 推送类型枚举（`src/bin/monitor/notify.rs`） |
| BannerCtx | 交易建议类全局横幅上下文 |
| `push_governor` | 推送主控 |
| SignalStateMachine | 信号状态机 |
| `news_monitor_loop` | 新闻监控循环 |
| sector_rotation | 板块轮动引擎 |
| virtual_watch | 虚拟观察仓位 DB |
| T-11 | v12 竞价复算通路 |
| BR | Business Rule（业务规则） |

## 附录 B：与 v19.x 既有用例同形约束

- **Params 结构体**：与 `HoldingPlanParams<'_>` / `T0AdviceParams<'_>` 同形
- **render 函数签名**：`pub fn render_<kind>(banner: &BannerCtx, p: <Kind>Params<'_>) -> String`
- **测试函数命名**：`fn <kind>_<scenario>`
- **match 分支**：缺分支用 `unreachable!()` 而非 `_ =>`
- **错误处理**：render 入口只接受已校验数据，校验在调用点完成；render 内不做 IO

## 附录 C：新规来源

> 沪深北交易所《交易规则（2026 修订）》于 2026-04-24 发布，2026-07-06 起施行。
> 来源：上交所/深交所/北交所 2026-04-24 联合公告。

---

**状态**：Draft（待用户审批 → 转 Final）
**下一步**：用户审阅 → writing-plans skill 产出 13 PR 任务卡
**紧急项**：建议 spec 通过后**立刻**执行 §10.2 治理参数同步（不需等 PR）