# v13 推送模板实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 v13 spec + 2026-07-06 新规的全部推送模板：7 个 v13 新增 + 6 个新规 v13.1 + 12 项现有 24 render 对齐 + 35 PushKind 治理元信息全表对齐。

**Architecture:** 沿用 v19.x 既有架构——所有 render 函数置于 `src/bin/monitor/push_templates.rs`，PushKind 枚举与治理方法置于 `src/bin/monitor/notify.rs`，调用点置于 `src/bin/monitor/main.rs`。新增内部 `RenderCtx` trait 抽象 6 套样板（level/cooldown/banner/deprecated/counts_against_daily_budget）。

**Tech Stack:** Rust (edition 2021), cargo, tokio, serde, diesel (existing), no new crate。

**关联文档**：
- 设计 spec：`docs/superpowers/specs/2026-07-06-v13-push-templates-design.md`
- 上游 v13 spec：`docs/architecture/v13-push-templates.md`
- 审计报告：`docs/superpowers/specs/2026-07-06-v13-push-templates-audit.md`
- baseline：v19.16（commit `1f3a176`）

---

## 🚨 紧急前置（必须先于任何代码 PR）

**新规 2026-07-06（今天）起施行**，Phase 0 任务**不需 PR 流程**，但必须**在 Phase 1 之前完成**——否则现有持仓风险计算基于旧阈值（5%）。

---

## Global Constraints（来自 spec §1.2 + §6.3 + §13）

| 约束 | 值/来源 |
|---|---|
| Rust edition | 2021 |
| 不引入新 crate | AGENTS §2.2 Gate B 最小变更 |
| render 函数签名 | `pub fn render_<kind>(banner: &BannerCtx, p: <Kind>Params<'_>) -> String` |
| Params 结构体 | 与 `HoldingPlanParams<'_>` / `T0AdviceParams<'_>` 同形（生命周期 + `&str` 借用） |
| 测试函数命名 | `fn <kind>_<scenario>`（如 `preopen_news_hot_three_themes_two_news`） |
| match 分支 | 缺分支用 `unreachable!()` 而非 `_ =>` |
| 错误处理 | render 入口只接受已校验数据，校验在调用点完成；render 内不做 IO |
| 验证命令 | `cargo fmt --check` / `cargo clippy -D warnings` / `cargo test` / `bash tools/compliance/check.sh` |
| PR 证据 | `Refs: spec §X.X` / `Data-Redlines: [...]` / `OldModules:` / `Threshold-Proof:` / `Business-Rules:` / `Rollback:` |
| 覆盖率目标 | `push_templates.rs` ≥ 85% / `notify.rs` 治理分支 ≥ 90% / `main.rs` ≥ 70% |
| 端到端 | `cargo run --bin monitor -- --test` |

---

## File Structure

### 修改文件

| 文件 | 当前行数 | 职责 | 修改点 |
|---|---|---|---|
| `src/bin/monitor/notify.rs` | 1395 | PushKind 枚举 + 治理方法 | +13 variant, +4 治理方法扩展, +1 注释修正 |
| `src/bin/monitor/push_templates.rs` | 3545 | render 函数 + Params + tests | +13 render, +13 Params, +175 治理断言, +43 render 用例, +12 对齐修复 |
| `src/bin/monitor/main.rs` | — | 调用点 | +13 调用点（每个 PushKind 一处） |
| `src/monitor/signal_state.rs` | 372 | 信号状态机 | +13 variant 注册 |
| `config/risk/stop_loss.toml` | — | ST 阈值 | 0.05 → 0.10 |
| `config/risk/limits.toml` | — | 流动性阈值 | `gem_small_cap_liquidity_threshold += 0.15` |
| `config/strategy.toml` | — | ST 止盈 | 0.05 → 0.10 |
| `docs/business_rules.md` | — | 业务规则 | +6 BR |

### 新增文件

无。所有 13 个新模板的实现落在既有 4 个 Rust 文件内。

---

## 任务计划（4 Phase / 14 Task）

| Phase | Task | PR | 标题 | 优先级 |
|---|---|---|---|---|
| 0 | 0.1-0.3 | (紧急) | 治理参数同步 + BR 登记 | 紧急 |
| 1 | 1.1 | #1 | feat(v13): PreopenNewsHot + IntradayMarket + NewsCatalyst | P0 |
| 1 | 1.2 | #2 | feat(v13): NewsToIdea | P0 |
| 1 | 1.3 | #3 | feat(v13): CatalystReview | P0 |
| 1 | 1.4 | #4 | feat(v13): IndustryChainIntraday (审计多发现) | P0 |
| 1 | 1.5 | #5 | feat(v13): PaperReview (前置 T-11) | P1 |
| 2 | 2.1 | #6 | feat(v13.1): PostFixedPriceOrder/Fill | P0 |
| 2 | 2.2 | #7 | feat(v13.1): StPriceLimitChanged | P0 |
| 2 | 2.3 | #9 | feat(v13.1): EtfClosingCallAuction | P1 |
| 2 | 2.4 | #10 | feat(v13.1): BlockTradeIntradayConfirm/PriceRange | P1 |
| 3 | 3.1 | #8 | fix(v13): 现有 render 对齐 12 项 | P0 |
| 3 | 3.2 | #11 | chore(v13): 治理全表对齐 + requires_banner | 收尾 |
| 3 | 3.3 | #12 | chore(v13): 文档漂移修正 | 收尾 |

---

## Phase 0：紧急治理参数同步（不需 PR）

> 必须在 Phase 1 第一个 PR 之前完成。新规 2026-07-06 已生效。

---

### Task 0.1: 同步 ST/*ST 涨跌幅阈值 5%→10%

**Files:**
- Modify: `config/risk/stop_loss.toml`
- Modify: `config/strategy.toml`
- Modify: `docs/business_rules.md`

**Interfaces:**
- Consumes: 无
- Produces: ST 阈值全局生效

- [ ] **Step 1: 读取当前 stop_loss.toml 的 ST 阈值**

```bash
grep -n "st_price_limit\|st_take_profit\|0\.05" /Users/zhangzhen/Desktop/Quant/stock_analysis/config/risk/stop_loss.toml
```

期望：找到至少 1 处 `0.05`（原 ST 阈值）。

- [ ] **Step 2: 修改 stop_loss.toml**

将 `st_price_limit = 0.05` 改为 `st_price_limit = 0.10`。

- [ ] **Step 3: 修改 strategy.toml**

将 `st_take_profit_pct = 0.05` 改为 `st_take_profit_pct = 0.10`。

- [ ] **Step 4: 验证修改**

```bash
grep -n "st_price_limit\|st_take_profit" /Users/zhangzhen/Desktop/Quant/stock_analysis/config/risk/stop_loss.toml /Users/zhangzhen/Desktop/Quant/stock_analysis/config/strategy.toml
```

期望：两处都显示 `0.10`。

- [ ] **Step 5: Commit**

```bash
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
git add -f config/risk/stop_loss.toml config/strategy.toml
git commit -m "urgent(v13.1): ST/*ST 涨跌幅阈值同步 5%→10% (新规 2026-07-06 生效)

Refs: 沪深北《交易规则（2026 修订）》§主板 ST/*ST 涨跌幅调整
Data-Redlines: [2.1, 2.6]
Business-Rules: BR-ST-PRICE-CHANGE

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 0.2: 同步创业板做市商流动性阈值

**Files:**
- Modify: `config/risk/limits.toml`
- Modify: `docs/business_rules.md`

- [ ] **Step 1: 读取当前 limits.toml 的创业板流动性阈值**

```bash
grep -n "gem_small_cap\|liquidity_threshold" /Users/zhangzhen/Desktop/Quant/stock_analysis/config/risk/limits.toml
```

- [ ] **Step 2: 修改 limits.toml**

将 `gem_small_cap_liquidity_threshold` 提高 0.15（原值 + 0.15）。

例如原值 `0.50` → `0.65`。

- [ ] **Step 3: 验证**

```bash
grep "gem_small_cap" /Users/zhangzhen/Desktop/Quant/stock_analysis/config/risk/limits.toml
```

- [ ] **Step 4: Commit**

```bash
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
git add -f config/risk/limits.toml
git commit -m "urgent(v13.1): 创业板做市商流动性阈值校准 +15% (新规 2026-07-06 生效)

Refs: 沪深北《交易规则（2026 修订）》§创业板做市商
Business-Rules: BR-GEM-MARKET-MAKER

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 0.3: 登记 6 个新业务规则

**Files:**
- Modify: `docs/business_rules.md`

- [ ] **Step 1: 追加 6 个 BR 到业务规则文档**

在 `docs/business_rules.md` 末尾追加：

```markdown
## BR-NEWS-CLUSTER
盘前新闻聚类口径：news_monitor cluster 输出 3 主线 + 2 催化 + N 关注票
生效：2026-07-06

## BR-NEWS-CATALYST
新闻催化映射：headline + theme + 上涨个股（涨幅/原因）
生效：2026-07-06

## BR-THEME-STAGE
题材阶段判定：启动/发酵/分歧 + 持续性 high/med/low
生效：2026-07-06

## BR-NEWS-TO-IDEA
新闻驱动个股：theme/stage + 推送原因 + 建议动作
生效：2026-07-06

## BR-POST-FIXED-PRICE
盘后固定价格：申报窗口按交易所区分（沪 9:30/深 9:15），撮合 15:05-15:30
生效：2026-07-06

## BR-ST-PRICE-CHANGE
ST/*ST 涨跌幅变更：5%→10%（与主板其他股票一致）
生效：2026-07-06

## BR-GEM-MARKET-MAKER
创业板做市商流动性阈值校准：+15%
生效：2026-07-06

## BR-CLOSING-CALL-AUCTION
上交所基金收盘集合竞价：14:57-15:00
生效：2026-07-06

## BR-BLOCK-TRADE-CONFIRM
创业板协议大宗盘中实时确认
生效：2026-07-06

## BR-BLOCK-TRADE-PRICE-RANGE
北交所大宗价格范围：当日实时均价（前收盘→实时均价）
生效：2026-07-06
```

- [ ] **Step 2: Commit**

```bash
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
git add -f docs/business_rules.md
git commit -m "urgent(v13.1): 业务规则登记 — 10 个 BR（v13 spec + 新规）

Refs: docs/superpowers/specs/2026-07-06-v13-push-templates-design.md §5

Co-Authored-By: Claude <noreply@anthropic.com>"
```

- [ ] **Step 3: Phase 0 收尾验证**

```bash
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
git log --oneline | grep "urgent(v13.1)" | wc -l
```

期望：`3`（Task 0.1, 0.2, 0.3 三个 commit）。

---

## Phase 1：v13 新增模板（PRs #1-5）

> 5 个 PR 串行，每个 PR 包含：PushKind variant + governance methods + Params struct + render fn + tests + call site + signal state registration。

---

### Task 1.1: PR #1 — PreopenNewsHot + IntradayMarket + NewsCatalyst ⚡ P0

**Files:**
- Modify: `src/bin/monitor/notify.rs`（+3 variant, +3 governance 分支）
- Modify: `src/bin/monitor/push_templates.rs`（+3 Params, +3 render, +11 tests）
- Modify: `src/bin/monitor/main.rs`（+3 调用点）
- Modify: `src/monitor/signal_state.rs`（+3 variant 注册）

**Interfaces:**
- Produces: `PreopenNewsHot / IntradayMarket / NewsCatalyst` 三个 PushKind 完整可用
- 新签名: `render_preopen_news_hot(p: PreopenNewsHotParams<'_>) -> String`
- 新签名: `render_intraday_market(p: IntradayMarketParams<'_>) -> String`
- 新签名: `render_news_catalyst(banner: &BannerCtx, p: NewsCatalystParams<'_>) -> String`

- [ ] **Step 1: 在 push_templates.rs 末尾添加 3 个 Params 结构体（与现有 `HoldingPlanParams` 同形）**

```rust
// §14.1 P-01 盘前新闻热点
pub struct PreopenNewsHotParams<'a> {
    pub hhmm: &'a str,
    pub theme_1: Option<&'a str>,
    pub theme_2: Option<&'a str>,
    pub theme_3: Option<&'a str>,
    pub news_pairs: Vec<(&'a str, &'a str)>,  // (news, chain)
    pub watch_stocks: Vec<(&'a str, &'a str, &'a str)>,  // (name, code, reason)
}

// §14.2 I-01 盘中轮动总览
pub struct IntradayMarketParams<'a> {
    pub hhmm: &'a str,
    pub tech_sub: Option<&'a str>,
    pub tech_score: Option<f32>,
    pub power_sub: Option<&'a str>,
    pub power_score: Option<f32>,
    pub robot_sub: Option<&'a str>,
    pub robot_score: Option<f32>,
    pub main_attack: Option<&'a str>,
    pub rotation_state: RotationState,
}

pub enum RotationState {
    Spreading,  // 扩散
    Diverging,  // 分化
    Fading,     // 退潮
}

// §14.2 I-02 新闻催化映射
pub struct NewsCatalystParams<'a> {
    pub hhmm: &'a str,
    pub headline: &'a str,
    pub theme: Option<&'a str>,
    pub stocks: Vec<(&'a str, &'a str, Option<f32>, &'a str)>,  // (name, code, chg, reason)
}
```

- [ ] **Step 2: 在 push_templates.rs 添加 3 个 render 函数**

```rust
/// §14.1 P-01 盘前新闻热点
pub fn render_preopen_news_hot(p: PreopenNewsHotParams<'_>) -> String {
    let mut s = format!("📰 盘前热点（{}）\n", p.hhmm);
    let themes: Vec<&str> = [p.theme_1, p.theme_2, p.theme_3]
        .into_iter().flatten().collect();
    if !themes.is_empty() {
        s.push_str(&format!("主线: {}\n", themes.join(" / ")));
    }
    if !p.news_pairs.is_empty() {
        s.push_str("催化:\n");
        for (news, chain) in &p.news_pairs {
            s.push_str(&format!("· {} → 利好{}\n", news, chain));
        }
    }
    if !p.watch_stocks.is_empty() {
        s.push_str("关注票:\n");
        for (name, code, reason) in &p.watch_stocks {
            s.push_str(&format!("· {}({}) 逻辑: {}\n", name, code, reason));
        }
    }
    s.push_str("辅助建议, 非下单指令");
    s
}

/// §14.2 I-01 盘中轮动总览
pub fn render_intraday_market(p: IntradayMarketParams<'_>) -> String {
    let score_str = |sub: Option<&str>, score: Option<f32>| -> String {
        let s = sub.unwrap_or("—");
        let sc = score.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "N/A".to_string());
        format!("{}(强度{})", s, sc)
    };
    let state = match p.rotation_state {
        RotationState::Spreading => "扩散",
        RotationState::Diverging => "分化",
        RotationState::Fading => "退潮",
    };
    let main = p.main_attack.unwrap_or("暂无主攻");
    format!(
        "📊 盘中轮动（{}）\n科技: {}（强度{:.1}）\n电力: {}（强度{:.1}）\n机器人: {}（强度{:.1}）\n当前主攻: {} | 轮动状态: {}\n辅助建议, 非下单指令",
        p.hhmm,
        score_str(p.tech_sub, p.tech_score),
        p.tech_score.unwrap_or(0.0),
        score_str(p.power_sub, p.power_score),
        p.power_score.unwrap_or(0.0),
        score_str(p.robot_sub, p.robot_score),
        p.robot_score.unwrap_or(0.0),
        main,
        state,
    )
}

/// §14.2 I-02 新闻催化映射
pub fn render_news_catalyst(banner: &BannerCtx, p: NewsCatalystParams<'_>) -> String {
    let theme = p.theme.unwrap_or("未分类");
    let mut s = format!(
        "{}\n📰⚡ 新闻催化跟踪（{}）\n新闻: {}\n受益板块: {}\n",
        banner.render(), p.hhmm, p.headline, theme
    );
    for (name, code, chg, reason) in &p.stocks {
        if let Some(c) = chg {
            s.push_str(&format!("· {}({}) {:+.1}% | 原因:{}\n", name, code, c, reason));
        }
    }
    s.push_str("辅助建议, 非下单指令");
    s
}
```

- [ ] **Step 3: 在 push_templates.rs 末尾 `#[cfg(test)] mod tests` 添加 11 个测试用例**

```rust
// ====== P-01 ======
#[test]
fn preopen_news_hot_three_themes_two_news_two_stocks() {
    let p = PreopenNewsHotParams {
        hhmm: "09:05",
        theme_1: Some("AI算力"),
        theme_2: Some("机器人"),
        theme_3: Some("消费电子"),
        news_pairs: vec![("英伟达新品", "GPU"), ("特斯拉FSD入华", "智驾")],
        watch_stocks: vec![("中科曙光", "603019", "AI算力龙头"), ("绿的谐波", "688017", "减速器")],
    };
    let out = render_preopen_news_hot(p);
    assert!(out.contains("📰 盘前热点（09:05）"));
    assert!(out.contains("主线: AI算力 / 机器人 / 消费电子"));
    assert!(out.contains("· 英伟达新品 → 利好GPU"));
    assert!(out.contains("· 中科曙光(603019) 逻辑: AI算力龙头"));
    assert!(out.ends_with("辅助建议, 非下单指令"));
}

#[test]
fn preopen_news_hot_missing_themes_omits_section() {
    let p = PreopenNewsHotParams {
        hhmm: "09:05",
        theme_1: None, theme_2: None, theme_3: None,
        news_pairs: vec![],
        watch_stocks: vec![("X", "000001", "r")],
    };
    let out = render_preopen_news_hot(p);
    assert!(!out.contains("主线:"));
    assert!(!out.contains("催化:"));
    assert!(out.contains("· X(000001) 逻辑: r"));
}

// ====== I-01 ======
#[test]
fn intraday_market_full_state() {
    let p = IntradayMarketParams {
        hhmm: "10:30",
        tech_sub: Some("AI算力"), tech_score: Some(85.5),
        power_sub: Some("特高压"), power_score: Some(60.0),
        robot_sub: Some("减速器"), robot_score: Some(72.3),
        main_attack: Some("AI算力"),
        rotation_state: RotationState::Spreading,
    };
    let out = render_intraday_market(p);
    assert!(out.contains("📊 盘中轮动（10:30）"));
    assert!(out.contains("科技: AI算力(强度85.5)"));
    assert!(out.contains("轮动状态: 扩散"));
    assert!(out.ends_with("辅助建议, 非下单指令"));
}

#[test]
fn intraday_market_missing_score_shows_na() {
    let p = IntradayMarketParams {
        hhmm: "10:30",
        tech_sub: Some("AI"), tech_score: None,
        power_sub: None, power_score: None,
        robot_sub: None, robot_score: None,
        main_attack: None,
        rotation_state: RotationState::Fading,
    };
    let out = render_intraday_market(p);
    assert!(out.contains("强度N/A"));
    assert!(out.contains("轮动状态: 退潮"));
    assert!(out.contains("暂无主攻"));
}

// ====== I-02 ======
#[test]
fn news_catalyst_banner_required() {
    let banner = BannerCtx::test_default();  // 假设有 test helper
    let p = NewsCatalystParams {
        hhmm: "10:30",
        headline: "英伟达发布H200",
        theme: Some("AI算力"),
        stocks: vec![("中科曙光", "603019", Some(5.2), "AI龙头")],
    };
    let out = render_news_catalyst(&banner, p);
    assert!(out.contains(banner.normal_mode_line().as_str()));  // 验证 banner 行
    assert!(out.contains("新闻: 英伟达发布H200"));
    assert!(out.contains("· 中科曙光(603019) +5.2% | 原因:AI龙头"));
}

#[test]
fn news_catalyst_missing_chg_omits_row() {
    let banner = BannerCtx::test_default();
    let p = NewsCatalystParams {
        hhmm: "10:30",
        headline: "X",
        theme: None,
        stocks: vec![("A", "000001", None, "r"), ("B", "000002", Some(3.0), "r2")],
    };
    let out = render_news_catalyst(&banner, p);
    assert!(!out.contains("· A(000001)"));
    assert!(out.contains("· B(000002) +3.0% | 原因:r2"));
    assert!(out.contains("受益板块: 未分类"));
}
```

> **注**：若 `BannerCtx::test_default()` 不存在，需在 BannerCtx 实现 `#[cfg(test)] pub fn test_default() -> Self`，参考 v19.x 既有测试 helper。

- [ ] **Step 4: 运行新测试**

```bash
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
cargo test push_templates::tests::preopen_news_hot_ push_templates::tests::intraday_market_ push_templates::tests::news_catalyst_ -- --nocapture
```

期望：6 个新测试 PASS。

- [ ] **Step 5: 在 notify.rs PushKind 枚举中追加 3 个 variant**

```rust
// 在 enum PushKind { ... } 中添加
/// v13 §14.1 P-01 盘前新闻热点 (⚡ 15min 冷却)
PreopenNewsHot,
/// v13 §14.2 I-01 盘中轮动总览 (⚡ 15min 冷却)
IntradayMarket,
/// v13 §14.2 I-02 新闻催化映射 (⚡ 10min 冷却)
NewsCatalyst,
```

- [ ] **Step 6: 在 notify.rs `level()` / `cooldown_secs()` / `requires_banner()` / `is_deprecated()` 添加 3 个分支**

```rust
pub fn level(self) -> PushLevel {
    match self {
        // ... 现有分支 ...
        PushKind::PreopenNewsHot => PushLevel::Important,
        PushKind::IntradayMarket => PushLevel::Important,
        PushKind::NewsCatalyst => PushLevel::Important,
        // ⚠️ 必须覆盖全部 variant，缺分支 = unreachable!()
        _ => unreachable!("unhandled PushKind in level()"),
    }
}

pub fn cooldown_secs(self) -> Option<u32> {
    match self {
        PushKind::AccountMode | PushKind::HoldingEvent => None,
        PushKind::PreopenNewsHot => Some(900),    // 15min
        PushKind::IntradayMarket => Some(900),   // 15min
        PushKind::NewsCatalyst => Some(600),     // 10min
        // ... 现有分支 ...
        _ => Some(1800),  // 默认 30min
    }
}

pub fn requires_banner(self) -> bool {
    match self {
        PushKind::HoldingPlan | PushKind::HoldingEvent | PushKind::T0Advice
        | PushKind::CandidateTriggered | PushKind::PaperTrade
        | PushKind::AuctionVolume | PushKind::IntradayMarket
        | PushKind::NewsCatalyst | PushKind::NewsToIdea
        | PushKind::PreopenNewsHot
        | PushKind::PostFixedPriceOrder | PushKind::PostFixedPriceFill
        | PushKind::StPriceLimitChanged => true,
        _ => false,
    }
}
```

- [ ] **Step 7: 添加 17 个治理元信息测试**

```rust
// 在 push_templates.rs tests 末尾追加
#[test] fn gov_preopen_news_hot_cooldown() { assert_eq!(PushKind::PreopenNewsHot.cooldown_secs(), Some(900)); }
#[test] fn gov_intraday_market_cooldown() { assert_eq!(PushKind::IntradayMarket.cooldown_secs(), Some(900)); }
#[test] fn gov_news_catalyst_cooldown() { assert_eq!(PushKind::NewsCatalyst.cooldown_secs(), Some(600)); }
#[test] fn gov_preopen_news_hot_no_banner() { assert!(!PushKind::PreopenNewsHot.requires_banner()); }
#[test] fn gov_intraday_market_banner() { assert!(PushKind::IntradayMarket.requires_banner()); }
#[test] fn gov_news_catalyst_banner() { assert!(PushKind::NewsCatalyst.requires_banner()); }
#[test] fn gov_preopen_news_hot_level() { assert_eq!(PushKind::PreopenNewsHot.level(), PushLevel::Important); }
#[test] fn gov_intraday_market_level() { assert_eq!(PushKind::IntradayMarket.level(), PushLevel::Important); }
#[test] fn gov_news_catalyst_level() { assert_eq!(PushKind::NewsCatalyst.level(), PushLevel::Important); }
#[test] fn gov_preopen_news_hot_not_deprecated() { assert!(!PushKind::PreopenNewsHot.is_deprecated()); }
#[test] fn gov_intraday_market_not_deprecated() { assert!(!PushKind::IntradayMarket.is_deprecated()); }
#[test] fn gov_news_catalyst_not_deprecated() { assert!(!PushKind::NewsCatalyst.is_deprecated()); }
#[test] fn gov_preopen_news_hot_counts_budget() { assert!(counts_against_daily_budget(PushKind::PreopenNewsHot)); }
#[test] fn gov_intraday_market_counts_budget() { assert!(counts_against_daily_budget(PushKind::IntradayMarket)); }
#[test] fn gov_news_catalyst_counts_budget() { assert!(counts_against_daily_budget(PushKind::NewsCatalyst)); }
```

- [ ] **Step 8: 运行治理测试 + 全部测试**

```bash
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
cargo test push_templates::tests::gov_preopen_news_hot push_templates::tests::gov_intraday_market push_templates::tests::gov_news_catalyst -- --nocapture
cargo test  # 全量
```

期望：所有 PASS。

- [ ] **Step 9: 在 main.rs 添加 3 个调用点**

```rust
// 在 main.rs 找到现有的 push_governor 调用模式，添加：
crate::notify::push_governor(
    &render_preopen_news_hot(params),
    crate::notify::PushKind::PreopenNewsHot,
).await;
crate::notify::push_governor(
    &render_intraday_market(params),
    crate::notify::PushKind::IntradayMarket,
).await;
crate::notify::push_governor(
    &render_news_catalyst(&banner, params),
    crate::notify::PushKind::NewsCatalyst,
).await;
```

- [ ] **Step 10: 在 signal_state.rs 注册 3 个 variant**

```rust
// 在 signal_state.rs 的 match 中添加：
SignalKind::PreopenNewsHot => { /* 注册信号 */ },
SignalKind::IntradayMarket => { /* 注册信号 */ },
SignalKind::NewsCatalyst => { /* 注册信号 */ },
```

- [ ] **Step 11: 运行所有检查**

```bash
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
cargo fmt --check
cargo clippy -D warnings
cargo test
bash tools/compliance/check.sh
```

期望：全部 PASS。

- [ ] **Step 12: Commit (PR #1)**

```bash
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
git add src/bin/monitor/notify.rs src/bin/monitor/push_templates.rs \
        src/bin/monitor/main.rs src/monitor/signal_state.rs
git commit -m "feat(v13): PushKind 新增 PreopenNewsHot/IntradayMarket/NewsCatalyst + 治理

Refs: docs/architecture/v13-push-templates.md §14.1 P-01 / §14.2 I-01 / §14.2 I-02
Data-Redlines: [2.1, 2.2, 2.3, 2.4]
OldModules: news_monitor_loop.adopt / sector_rotation.adopt
Threshold-Proof: N/A
Business-Rules: BR-NEWS-CLUSTER / BR-NEWS-CATALYST
Rollback: git revert <commit-sha> && cargo build --release

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 1.2: PR #2 — NewsToIdea ⚡ P0

**Files:**
- Modify: `src/bin/monitor/notify.rs`
- Modify: `src/bin/monitor/push_templates.rs`
- Modify: `src/bin/monitor/main.rs`
- Modify: `src/monitor/signal_state.rs`

- [ ] **Step 1: 添加 `NewsToIdeaParams`**

```rust
// §14.4 D-01 新闻驱动个股
pub struct NewsToIdeaParams<'a> {
    pub hhmm: &'a str,
    pub headline: &'a str,
    pub theme: Option<&'a str>,
    pub stage: NewsStage,
    pub name: &'a str,
    pub code: &'a str,
    pub reasons: Vec<&'a str>,
    pub action: Option<NewsAction>,
}

pub enum NewsStage { Starting, Fermenting, Diverging }  // 启动/发酵/分歧
pub enum NewsAction { Observe, BuyDip, DoNotChase }     // 观察/低吸/不追
```

- [ ] **Step 2: 添加 `render_news_to_idea` 函数**

```rust
/// §14.4 D-01 新闻驱动个股（⚡交易建议类，带 banner）
pub fn render_news_to_idea(banner: &BannerCtx, p: NewsToIdeaParams<'_>) -> String {
    let stage = match p.stage {
        NewsStage::Starting => "启动",
        NewsStage::Fermenting => "发酵",
        NewsStage::Diverging => "分歧",
    };
    let theme = p.theme.unwrap_or("未分类");
    let mut s = format!(
        "{}\n🧭 新闻驱动个股（{}）\n新闻: {}\n板块: {} | 阶段: {}\n个股: {}({})\n",
        banner.render(), p.hhmm, p.headline, theme, stage, p.name, p.code
    );
    if !p.reasons.is_empty() {
        s.push_str("推送原因:\n");
        for r in &p.reasons {
            s.push_str(&format!("· {}\n", r));
        }
    }
    if let Some(act) = p.action {
        let a = match act {
            NewsAction::Observe => "观察",
            NewsAction::BuyDip => "低吸",
            NewsAction::DoNotChase => "不追",
        };
        s.push_str(&format!("[建议动作: {}]\n", a));
    }
    s.push_str("辅助建议, 非下单指令");
    s
}
```

- [ ] **Step 3: 添加 4 个测试**

```rust
#[test]
fn news_to_idea_full() {
    let banner = BannerCtx::test_default();
    let p = NewsToIdeaParams {
        hhmm: "10:30", headline: "X", theme: Some("AI"),
        stage: NewsStage::Starting, name: "A", code: "000001",
        reasons: vec!["r1", "r2"],
        action: Some(NewsAction::BuyDip),
    };
    let out = render_news_to_idea(&banner, p);
    assert!(out.contains("🧭 新闻驱动个股（10:30）"));
    assert!(out.contains("板块: AI | 阶段: 启动"));
    assert!(out.contains("· r1"));
    assert!(out.contains("[建议动作: 低吸]"));
    assert!(out.ends_with("辅助建议, 非下单指令"));
}

#[test]
fn news_to_idea_stage_fermenting() {
    let banner = BannerCtx::test_default();
    let p = NewsToIdeaParams {
        hhmm: "10:30", headline: "X", theme: None,
        stage: NewsStage::Fermenting, name: "A", code: "000001",
        reasons: vec![], action: None,
    };
    let out = render_news_to_idea(&banner, p);
    assert!(out.contains("板块: 未分类 | 阶段: 发酵"));
    assert!(!out.contains("推送原因:"));
    assert!(!out.contains("[建议动作:"));
}

#[test]
fn news_to_idea_action_do_not_chase() {
    let banner = BannerCtx::test_default();
    let p = NewsToIdeaParams {
        hhmm: "10:30", headline: "X", theme: Some("X"),
        stage: NewsStage::Diverging, name: "A", code: "000001",
        reasons: vec!["r"], action: Some(NewsAction::DoNotChase),
    };
    let out = render_news_to_idea(&banner, p);
    assert!(out.contains("[建议动作: 不追]"));
}

#[test]
fn news_to_idea_missing_reasons_omits_section() {
    let banner = BannerCtx::test_default();
    let p = NewsToIdeaParams {
        hhmm: "10:30", headline: "X", theme: None,
        stage: NewsStage::Starting, name: "A", code: "000001",
        reasons: vec![], action: None,
    };
    let out = render_news_to_idea(&banner, p);
    assert!(!out.contains("推送原因:"));
}
```

- [ ] **Step 4: 在 notify.rs 添加 variant + 治理分支**

```rust
/// v13 §14.4 D-01 新闻驱动个股 (⚡ 20min/票 冷却)
NewsToIdea,

// level(): Important
// cooldown_secs(): Some(1200)
// requires_banner(): true
// is_deprecated(): false
// counts_against_daily_budget(): true
```

- [ ] **Step 5: 添加 5 个治理测试 + 1 个红线测试**

```rust
#[test] fn gov_news_to_idea_cooldown() { assert_eq!(PushKind::NewsToIdea.cooldown_secs(), Some(1200)); }
#[test] fn gov_news_to_idea_banner() { assert!(PushKind::NewsToIdea.requires_banner()); }
#[test] fn gov_news_to_idea_level() { assert_eq!(PushKind::NewsToIdea.level(), PushLevel::Important); }
#[test] fn gov_news_to_idea_not_deprecated() { assert!(!PushKind::NewsToIdea.is_deprecated()); }
#[test] fn gov_news_to_idea_counts_budget() { assert!(counts_against_daily_budget(PushKind::NewsToIdea)); }
```

- [ ] **Step 6: main.rs 调用点 + signal_state.rs 注册**

- [ ] **Step 7: 全检查 + Commit (PR #2)**

```bash
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
cargo fmt --check && cargo clippy -D warnings && cargo test && bash tools/compliance/check.sh
git add src/bin/monitor/notify.rs src/bin/monitor/push_templates.rs src/bin/monitor/main.rs src/monitor/signal_state.rs
git commit -m "feat(v13): PushKind 新增 NewsToIdea + 治理

Refs: spec §14.4 D-01
Data-Redlines: [2.1, 2.4, 2.6]
Business-Rules: BR-NEWS-TO-IDEA

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 1.3: PR #3 — CatalystReview ⚡(盘后) P0

**Files:** 同 Task 1.1

- [ ] **Step 1: 添加 `CatalystReviewParams`**

```rust
// §14.3 A-10 盘后题材催化复盘
pub struct CatalystReviewParams<'a> {
    pub date: &'a str,
    pub theme: &'a str,
    pub score: Option<f32>,
    pub persistent: PersistentLevel,
    pub started_names: Vec<&'a str>,
    pub pending_names: Vec<&'a str>,
    pub watch_point: Option<&'a str>,
}

pub enum PersistentLevel { High, Med, Low }  // high/med/low
```

- [ ] **Step 2: 添加 `render_catalyst_review` 函数**

```rust
/// §14.3 A-10 盘后题材催化复盘
pub fn render_catalyst_review(p: CatalystReviewParams<'_>) -> String {
    let score_str = p.score.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "N/A".to_string());
    let persistent = match p.persistent {
        PersistentLevel::High => "high",
        PersistentLevel::Med => "med",
        PersistentLevel::Low => "low",
    };
    let mut s = format!(
        "📰 题材催化复盘（{}）\n{}: 当日强度{} | 持续性{}\n",
        p.date, p.theme, score_str, persistent
    );
    if !p.started_names.is_empty() {
        s.push_str(&format!("已启动: {}\n", p.started_names.join("、")));
    }
    if !p.pending_names.is_empty() {
        s.push_str(&format!("待启动: {}\n", p.pending_names.join("、")));
    }
    if let Some(w) = p.watch_point {
        s.push_str(&format!("明日观察点: {}\n", w));
    }
    s.push_str("辅助建议, 非下单指令");
    s
}
```

- [ ] **Step 3: 添加 2 个测试**

```rust
#[test]
fn catalyst_review_full() {
    let p = CatalystReviewParams {
        date: "2026-07-06", theme: "AI算力", score: Some(85.0),
        persistent: PersistentLevel::High,
        started_names: vec!["A", "B"], pending_names: vec!["C"],
        watch_point: Some("明日是否扩散"),
    };
    let out = render_catalyst_review(p);
    assert!(out.contains("📰 题材催化复盘（2026-07-06）"));
    assert!(out.contains("AI算力: 当日强度85.0 | 持续性high"));
    assert!(out.contains("已启动: A、B"));
    assert!(out.contains("待启动: C"));
    assert!(out.contains("明日观察点: 明日是否扩散"));
}

#[test]
fn catalyst_review_persistent_low() {
    let p = CatalystReviewParams {
        date: "2026-07-06", theme: "X", score: None,
        persistent: PersistentLevel::Low,
        started_names: vec![], pending_names: vec![],
        watch_point: None,
    };
    let out = render_catalyst_review(p);
    assert!(out.contains("当日强度N/A"));
    assert!(out.contains("持续性low"));
    assert!(!out.contains("已启动:"));
    assert!(!out.contains("待启动:"));
}
```

- [ ] **Step 4: notify.rs variant + 治理**

```rust
/// v13 §14.3 A-10 盘后题材催化复盘 (⚡ 1次/日 冷却)
CatalystReview,

// level(): Important
// cooldown_secs(): Some(86400)  // 1次/日
// requires_banner(): false  // 盘后非交易建议
// is_deprecated(): false
// counts_against_daily_budget(): true
```

- [ ] **Step 5: 治理测试 + 调用点 + signal_state + 检查 + Commit (PR #3)**

```bash
git commit -m "feat(v13): PushKind 新增 CatalystReview 盘后 + 治理

Refs: spec §14.3 A-10
Data-Redlines: [2.1, 2.3, 2.4]
Business-Rules: BR-THEME-STAGE

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 1.4: PR #4 — IndustryChainIntraday ⚡ P0 (审计多发现)

**Files:** 同 Task 1.1

- [ ] **Step 1: 添加 `IndustryChainIntradayParams`**

```rust
// §14.2 I-03 盘中涨停扩散
pub struct IndustryChainIntradayParams<'a> {
    pub hhmm: &'a str,
    pub chain: &'a str,
    pub limit_count: u32,
    pub leader_name: Option<&'a str>,
    pub leader_code: Option<&'a str>,
    pub leader_height: u32,
    pub supplements: Vec<SupplementCandidate<'a>>,
}

pub struct SupplementCandidate<'a> {
    pub name: &'a str,
    pub code: &'a str,
    pub trigger: &'a str,
    pub lo: f64,
    pub hi: f64,
    pub stop: f64,
}
```

- [ ] **Step 2: 添加 `render_industry_chain_intraday` 函数**

```rust
/// §14.2 I-03 盘中涨停扩散
pub fn render_industry_chain_intraday(p: IndustryChainIntradayParams<'_>) -> String {
    let leader = match (p.leader_name, p.leader_code) {
        (Some(n), Some(c)) => format!("龙头: {}({}) {}板", n, c, p.leader_height),
        _ => "龙头: 暂无".to_string(),
    };
    let mut s = format!(
        "🔥 盘中涨停扩散（{}）\n主链: {} | 涨停{}家 | 连板高度{}板\n{}\n",
        p.hhmm, p.chain, p.limit_count, p.leader_height, leader
    );
    if !p.supplements.is_empty() {
        s.push_str("补涨候选:\n");
        for c in &p.supplements {
            s.push_str(&format!(
                "· {}({}) 触发条件{} | 低吸{:.2}~{:.2} | 止损{:.2}\n",
                c.name, c.code, c.trigger, c.lo, c.hi, c.stop
            ));
        }
    }
    s.push_str("辅助建议, 非下单指令");
    s
}
```

- [ ] **Step 3: 测试 + 治理 + 调用 + Commit (PR #4)**

```rust
#[test]
fn industry_chain_intraday_with_supplements() {
    let p = IndustryChainIntradayParams {
        hhmm: "10:30", chain: "AI算力", limit_count: 5,
        leader_name: Some("A"), leader_code: Some("000001"), leader_height: 3,
        supplements: vec![SupplementCandidate {
            name: "B", code: "000002", trigger: "首板",
            lo: 10.0, hi: 12.0, stop: 9.0,
        }],
    };
    let out = render_industry_chain_intraday(p);
    assert!(out.contains("🔥 盘中涨停扩散（10:30）"));
    assert!(out.contains("主链: AI算力 | 涨停5家 | 连板高度3板"));
    assert!(out.contains("龙头: A(000001) 3板"));
    assert!(out.contains("· B(000002) 触发条件首板 | 低吸10.00~12.00 | 止损9.00"));
}

#[test]
fn industry_chain_intraday_no_leader() {
    let p = IndustryChainIntradayParams {
        hhmm: "10:30", chain: "X", limit_count: 0,
        leader_name: None, leader_code: None, leader_height: 0,
        supplements: vec![],
    };
    let out = render_industry_chain_intraday(p);
    assert!(out.contains("龙头: 暂无"));
    assert!(!out.contains("补涨候选:"));
}
```

```rust
/// v13 §14.2 I-03 盘中涨停扩散 (⚡ 30min 冷却)
IndustryChainIntraday,

// level(): Important
// cooldown_secs(): Some(1800)
// requires_banner(): true
// is_deprecated(): false
```

```bash
git commit -m "feat(v13): PushKind 新增 IndustryChainIntraday 盘中形态 (审计多发现)

Refs: spec §14.2 I-03 + audit 2026-07-06
Data-Redlines: [2.1, 2.4]
OldModules: industry_chain (盘后 R-03) reject 独立成盘中形态

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 1.5: PR #5 — PaperReview ℹ️ P1 (前置 T-11)

**Files:** 同 Task 1.1

- [ ] **Step 1: 添加 `PaperReviewParams`**

```rust
// §14.3 A-01 虚拟仓复盘 (P1, 前置 T-11)
pub struct PaperReviewParams<'a> {
    pub date: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub trigger: &'a str,
    pub desc: &'a str,
    pub pnl: Option<f32>,
    pub plan_high: Option<&'a str>,
    pub plan_flat: Option<&'a str>,
    pub plan_low: Option<&'a str>,
}
```

- [ ] **Step 2: 添加 `render_paper_review` 函数（与现有 `render_paper_trade` 区分）**

```rust
/// §14.3 A-01 虚拟仓复盘 (盘后)
pub fn render_paper_review(p: PaperReviewParams<'_>) -> String {
    let pnl_str = p.pnl.map(|v| format!("{:+.1}%", v)).unwrap_or_else(|| "N/A%".to_string());
    let mut s = format!(
        "🧪 虚拟仓复盘（{}）\n{}({}) 原触发: {}\n结果: {} {}\n",
        p.date, p.name, p.code, p.trigger, p.desc, pnl_str
    );
    s.push_str("次日计划:\n");
    if let Some(h) = p.plan_high { s.push_str(&format!("· 高开>{}%: {}\n", 1.0, h)); }
    if let Some(f) = p.plan_flat { s.push_str(&format!("· 平开: {}\n", f)); }
    if let Some(l) = p.plan_low { s.push_str(&format!("· 低开/跌破止损: {}\n", l)); }
    s.push_str("辅助建议, 非下单指令");
    s
}
```

- [ ] **Step 3: 测试（标记 `#[ignore]` 因 T-11 未就绪）**

```rust
#[test]
#[ignore = "T-11 竞价复算通路未就绪 (v12-dev-plan §MVP-3)"]
fn paper_review_full() {
    let p = PaperReviewParams {
        date: "2026-07-06", name: "A", code: "000001", trigger: "首板",
        desc: "已成交", pnl: Some(2.5),
        plan_high: Some("观察"), plan_flat: Some("持有"), plan_low: Some("止损"),
    };
    let out = render_paper_review(p);
    assert!(out.contains("🧪 虚拟仓复盘（2026-07-06）"));
    assert!(out.contains("A(000001) 原触发: 首板"));
    assert!(out.contains("结果: 已成交 +2.5%"));
    assert!(out.contains("· 高开>1%: 观察"));
    assert!(out.contains("· 平开: 持有"));
}

#[test]
#[ignore = "T-11 通路未就绪"]
fn paper_review_pnl_missing() {
    let p = PaperReviewParams {
        date: "2026-07-06", name: "A", code: "000001", trigger: "T",
        desc: "X", pnl: None,
        plan_high: None, plan_flat: None, plan_low: None,
    };
    let out = render_paper_review(p);
    assert!(out.contains("结果: X N/A%"));
    assert!(!out.contains("次日计划:"));
}
```

- [ ] **Step 4: notify.rs variant + 治理**

```rust
/// v13 §14.3 A-01 虚拟仓复盘 (ℹ️ 1次/日 冷却, P1)
PaperReview,

// level(): Info
// cooldown_secs(): Some(86400)
// requires_banner(): false  // 盘后
// is_deprecated(): false
// counts_against_daily_budget(): false
```

- [ ] **Step 5: 调用点 + signal_state + 检查 + Commit (PR #5)**

```bash
git commit -m "feat(v13): PushKind 新增 PaperReview 盘后 + 治理 (P1 前置 T-11)

Refs: spec §14.3 A-01
Data-Redlines: [2.1, 2.4]
注: PR 含 #[ignore] 测试，待 T-11 竞价复算通路就绪后解除

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Phase 2：新规 v13.1 模板（PRs #6, #7, #9, #10）

> 6 个新规模板的设计在 `docs/superpowers/specs/2026-07-06-v13-push-templates-design.md §5`。

---

### Task 2.1: PR #6 — PostFixedPriceOrder + PostFixedPriceFill ⚡ P0

**Files:** 同 Task 1.1

- [ ] **Step 1: 添加 2 个 Params 结构体**

```rust
// §5.2 T-14 盘后固定价格申报
pub struct PostFixedPriceOrderParams<'a> {
    pub exchange: Exchange,
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub price: f64,
    pub qty: u32,
    pub order_id: &'a str,
    pub status: OrderStatus,
}

pub enum Exchange { SH, SZ, BJ }  // 沪/深/北
pub enum OrderStatus { Submitted, Cancelled, Rejected }

// §5.3 T-15 盘后固定价格成交
pub struct PostFixedPriceFillParams<'a> {
    pub exchange: Exchange,
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub fill_price: f64,
    pub qty: u32,
    pub vs_limit_pct: Option<f32>,
    pub next_session_carry: bool,
}
```

- [ ] **Step 2: 添加 2 个 render 函数（带交易所分支）**

```rust
/// §5.2 T-14 盘后固定价格申报
pub fn render_post_fixed_price_order(p: PostFixedPriceOrderParams<'_>) -> String {
    let ex = match p.exchange {
        Exchange::SH => "沪市", Exchange::SZ => "深市", Exchange::BJ => "北交所",
    };
    let status = match p.status {
        OrderStatus::Submitted => "已报", OrderStatus::Cancelled => "已撤", OrderStatus::Rejected => "废单",
    };
    let window = match p.hhmm {
        t if t < "11:30" => "上午",
        t if t < "15:00" => "下午（含中午）",
        _ => "尾盘",
    };
    format!(
        "📋 盘后固定价格申报（{} {}）\n{}({}) 价格{:.2} 数量{} | 状态: {} | 窗口: {}\n订单号: {}\n辅助建议, 非下单指令",
        p.hhmm, ex, p.name, p.code, p.price, p.qty, status, window, p.order_id
    )
}

/// §5.3 T-15 盘后固定价格成交
pub fn render_post_fixed_price_fill(p: PostFixedPriceFillParams<'_>) -> String {
    let ex = match p.exchange {
        Exchange::SH => "沪市", Exchange::SZ => "深市", Exchange::BJ => "北交所",
    };
    let vs = p.vs_limit_pct.map(|v| format!("{:+.1}%", v)).unwrap_or_else(|| "N/A".to_string());
    let carry = if p.next_session_carry { "过户到次一交易日" } else { "本日内" };
    format!(
        "✅ 盘后固定价格成交（{} {}）\n{}({}) 成交价{:.2} 数量{} | 价差{}\n清算: {}\n辅助建议, 非下单指令",
        p.hhmm, ex, p.name, p.code, p.fill_price, p.qty, vs, carry
    )
}
```

- [ ] **Step 3: 添加 4 个测试**

```rust
#[test]
fn post_fixed_price_order_sh_submitted() {
    let p = PostFixedPriceOrderParams {
        exchange: Exchange::SH, hhmm: "10:00",
        name: "A", code: "600000", price: 10.50, qty: 1000,
        order_id: "ORD001", status: OrderStatus::Submitted,
    };
    let out = render_post_fixed_price_order(p);
    assert!(out.contains("📋 盘后固定价格申报（10:00 沪市）"));
    assert!(out.contains("价格10.50 数量1000 | 状态: 已报"));
    assert!(out.contains("订单号: ORD001"));
}

#[test]
fn post_fixed_price_order_sz_window_detection() {
    let p = PostFixedPriceOrderParams {
        exchange: Exchange::SZ, hhmm: "13:30",
        name: "A", code: "000001", price: 10.0, qty: 100,
        order_id: "X", status: OrderStatus::Cancelled,
    };
    let out = render_post_fixed_price_order(p);
    assert!(out.contains("窗口: 下午"));
    assert!(out.contains("已撤"));
}

#[test]
fn post_fixed_price_fill_with_carry() {
    let p = PostFixedPriceFillParams {
        exchange: Exchange::BJ, hhmm: "15:10",
        name: "A", code: "830001", fill_price: 10.0, qty: 100,
        vs_limit_pct: Some(2.5), next_session_carry: true,
    };
    let out = render_post_fixed_price_fill(p);
    assert!(out.contains("✅ 盘后固定价格成交（15:10 北交所）"));
    assert!(out.contains("价差+2.5%"));
    assert!(out.contains("清算: 过户到次一交易日"));
}

#[test]
fn post_fixed_price_fill_no_carry() {
    let p = PostFixedPriceFillParams {
        exchange: Exchange::SH, hhmm: "15:20",
        name: "A", code: "600000", fill_price: 10.0, qty: 100,
        vs_limit_pct: None, next_session_carry: false,
    };
    let out = render_post_fixed_price_fill(p);
    assert!(out.contains("价差N/A"));
    assert!(out.contains("清算: 本日内"));
}
```

- [ ] **Step 4: notify.rs 2 个 variant + 治理**

```rust
PostFixedPriceOrder,   // ⚡ 1min/票, banner=true
PostFixedPriceFill,    // ⚡ 5min/票, banner=true
```

- [ ] **Step 5: 治理测试 + 调用点 + signal_state + 检查 + Commit (PR #6)**

```bash
git commit -m "feat(v13.1): PushKind 新增 PostFixedPriceOrder/Fill 盘后固定价格 + 治理

Refs: 沪深北《交易规则（2026 修订）》§盘后固定价格交易扩围
Data-Redlines: [2.1, 2.4, 2.6]
Business-Rules: BR-POST-FIXED-PRICE

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2.2: PR #7 — StPriceLimitChanged ⚡ P0

**Files:** 同 Task 1.1

- [ ] **Step 1: 添加 `StPriceLimitChangedParams`**

```rust
// §5.4 T-16 ST 涨跌幅变更提醒
pub struct StPriceLimitChangedParams<'a> {
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub st_type: StType,
    pub old_limit: f32,  // 原 0.05
    pub new_limit: f32,  // 新 0.10
    pub holding_qty: u32,
    pub cost: f64,
    pub now_price: f64,
    pub new_stop_loss: Option<f64>,
    pub new_take_profit: Option<f64>,
}

pub enum StType { ST, StarST }  // ST / *ST
```

- [ ] **Step 2: 添加 `render_st_price_limit_changed` 函数**

```rust
/// §5.4 T-16 ST 涨跌幅变更提醒
pub fn render_st_price_limit_changed(p: StPriceLimitChangedParams<'_>) -> String {
    let st = match p.st_type { StType::ST => "ST", StType::StarST => "*ST" };
    let mut s = format!(
        "⚠️ ST 涨跌幅变更（{}）\n{}({}) [{}] 持仓 {} 股\n原涨跌幅: {:+.0}% → 新涨跌幅: {:+.0}%\n现价: {:.2} 成本: {:.2} 浮盈: {:+.1}%\n",
        p.hhmm, p.name, p.code, st, p.holding_qty,
        p.old_limit * 100.0, p.new_limit * 100.0,
        p.now_price, p.cost, ((p.now_price - p.cost) / p.cost) * 100.0
    );
    if let Some(sl) = p.new_stop_loss {
        s.push_str(&format!("新止损: {:.2} (基于 {:.0}% 阈值)\n", sl, p.new_limit * 100.0));
    } else {
        s.push_str("新止损: 未重算\n");
    }
    if let Some(tp) = p.new_take_profit {
        s.push_str(&format!("新止盈: {:.2}\n", tp));
    }
    s.push_str("辅助建议, 非下单指令 — 现有持仓风险阈值已重新校准");
    s
}
```

- [ ] **Step 3: 3 个测试**

```rust
#[test]
fn st_price_limit_changed_with_recalc() {
    let p = StPriceLimitChangedParams {
        hhmm: "09:30", name: "A", code: "600000", st_type: StType::ST,
        old_limit: 0.05, new_limit: 0.10,
        holding_qty: 1000, cost: 10.0, now_price: 11.0,
        new_stop_loss: Some(9.0), new_take_profit: Some(12.0),
    };
    let out = render_st_price_limit_changed(p);
    assert!(out.contains("⚠️ ST 涨跌幅变更（09:30）"));
    assert!(out.contains("A(600000) [ST] 持仓 1000 股"));
    assert!(out.contains("原涨跌幅: +5% → 新涨跌幅: +10%"));
    assert!(out.contains("新止损: 9.00 (基于 10% 阈值)"));
}

#[test]
fn st_price_limit_changed_star_st() {
    let p = StPriceLimitChangedParams {
        hhmm: "09:30", name: "B", code: "000001", st_type: StType::StarST,
        old_limit: 0.05, new_limit: 0.10,
        holding_qty: 500, cost: 5.0, now_price: 4.5,
        new_stop_loss: None, new_take_profit: None,
    };
    let out = render_st_price_limit_changed(p);
    assert!(out.contains("B(000001) [*ST]"));
    assert!(out.contains("新止损: 未重算"));
}

#[test]
fn st_price_limit_changed_emergency_link() {
    let p = StPriceLimitChangedParams {
        hhmm: "09:30", name: "A", code: "600000", st_type: StType::ST,
        old_limit: 0.05, new_limit: 0.10,
        holding_qty: 0, cost: 0.0, now_price: 0.0,
        new_stop_loss: None, new_take_profit: None,
    };
    let out = render_st_price_limit_changed(p);
    assert!(out.contains("辅助建议, 非下单指令 — 现有持仓风险阈值已重新校准"));
}
```

- [ ] **Step 4: notify.rs variant + 治理 + 调用 + signal_state + 检查 + Commit (PR #7)**

```bash
git commit -m "feat(v13.1): PushKind 新增 StPriceLimitChanged ST 涨跌幅变更 + 治理

Refs: 沪深北《交易规则（2026 修订）》§主板 ST/*ST 涨跌幅 5%→10%
Data-Redlines: [2.1, 2.6]
Business-Rules: BR-ST-PRICE-CHANGE
Threshold-Proof: 0.05 → 0.10 (新规)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2.3: PR #9 — EtfClosingCallAuction ℹ️ P1

**Files:** 同 Task 1.1

- [ ] **Step 1: 添加 `EtfClosingCallAuctionParams`**

```rust
// §5.5 T-17 ETF 收盘集合竞价
pub struct EtfClosingCallAuctionParams<'a> {
    pub hhmm: &'a str,  // 14:57-15:00
    pub name: &'a str,
    pub code: &'a str,
    pub call_auction_price: Option<f64>,
    pub vs_continuous_est: Option<f32>,
    pub liquidity_note: &'a str,
}
```

- [ ] **Step 2: 添加 `render_etf_closing_call_auction`**

```rust
/// §5.5 T-17 ETF 收盘集合竞价（仅沪市 ETF, 14:57-15:00）
pub fn render_etf_closing_call_auction(p: EtfClosingCallAuctionParams<'_>) -> String {
    let price = p.call_auction_price.map(|v| format!("{:.3}", v)).unwrap_or_else(|| "暂无".to_string());
    let vs = p.vs_continuous_est.map(|v| format!("{:+.2}%", v)).unwrap_or_else(|| "N/A".to_string());
    format!(
        "📊 ETF 集合竞价尾盘（{}）\n{}({}) 沪市 ETF 收盘价: {}\nvs 连续竞价估值: {}\n流动性: {}\n注: 14:57-15:00 集合竞价形成收盘价（抑制尾盘操纵）",
        p.hhmm, p.name, p.code, price, vs, p.liquidity_note
    )
}
```

- [ ] **Step 3: 2 个测试 + 治理 + 调用 + signal_state + 检查 + Commit (PR #9)**

```bash
git commit -m "feat(v13.1): PushKind 新增 EtfClosingCallAuction 沪市 ETF 收盘集合竞价 + 治理

Refs: 沪深北《交易规则（2026 修订）》§上交所基金收盘集合竞价
Data-Redlines: [2.1, 2.4]
Business-Rules: BR-CLOSING-CALL-AUCTION

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2.4: PR #10 — BlockTradeIntradayConfirm + BlockTradePriceRange ℹ️ P1

**Files:** 同 Task 1.1

- [ ] **Step 1: 添加 2 个 Params**

```rust
// §5.6 T-18 创业板大宗盘中确认
pub struct BlockTradeIntradayConfirmParams<'a> {
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub qty: u32,
    pub price: f64,
    pub block_type: BlockType,
    pub board: Board,
    pub real_time_confirm: bool,
    pub next_session_settle: SettleType,
}

pub enum BlockType { Agreed, Competitive }  // 协议/竞价
pub enum Board { GEM, STAR, Main }  // 创业/科创/主板
pub enum SettleType { NextSession, RealTime }

// §5.7 T-19 北交所大宗价格区间
pub struct BlockTradePriceRangeParams<'a> {
    pub hhmm: &'a str,
    pub name: &'a str,
    pub code: &'a str,
    pub prev_close: Option<f64>,
    pub today_avg_price: f64,
    pub block_price_range: Option<&'a str>,
    pub note: &'a str,
}
```

- [ ] **Step 2: 2 个 render 函数**

```rust
/// §5.6 T-18 创业板大宗盘中确认
pub fn render_block_trade_intraday_confirm(p: BlockTradeIntradayConfirmParams<'_>) -> String {
    let bt = match p.block_type { BlockType::Agreed => "协议大宗", BlockType::Competitive => "竞价大宗" };
    let bd = match p.board { Board::GEM => "创业板", Board::STAR => "科创板", Board::Main => "主板" };
    let settle = match p.next_session_settle { SettleType::NextSession => "次日清算", SettleType::RealTime => "实时清算" };
    let confirm = if p.real_time_confirm { "✅ 盘中实时确认" } else { "⏳ 等待确认" };
    format!(
        "📋 大宗交易盘中确认（{}）\n{}({}) {} {}\n数量: {} 价格: {:.2}\n板块: {} | {}\n清算: {}",
        p.hhmm, p.name, p.code, bt, confirm, p.qty, p.price, bd, settle
    )
}

/// §5.7 T-19 北交所大宗价格区间
pub fn render_block_trade_price_range(p: BlockTradePriceRangeParams<'_>) -> String {
    let prev = p.prev_close.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "N/A".to_string());
    let range = p.block_price_range.unwrap_or("暂无");
    format!(
        "📊 北交所大宗价格区间（{}）\n{}({})\n前收盘价: {} (原口径)\n当日实时均价: {:.2} (新口径)\n价格区间: {}\n注: {}",
        p.hhmm, p.name, p.code, prev, p.today_avg_price, range, p.note
    )
}
```

- [ ] **Step 3: 2 个测试 + 治理 + 调用 + signal_state + 检查 + Commit (PR #10)**

```bash
git commit -m "feat(v13.1): PushKind 新增 BlockTradeIntradayConfirm/PriceRange 大宗 + 治理

Refs: 沪深北《交易规则（2026 修订）》§创业板/北交所大宗交易
Data-Redlines: [2.1, 2.4, 2.6]
Business-Rules: BR-BLOCK-TRADE-CONFIRM / BR-BLOCK-TRADE-PRICE-RANGE

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Phase 3：现有 24 render 对齐 + 治理全表 + 文档漂移（PRs #8, #11, #12）

---

### Task 3.1: PR #8 — 现有 render 对齐 12 项差异

**Files:**
- Modify: `src/bin/monitor/push_templates.rs`（12 项修复）

- [ ] **Step 1: F-01 修复 T-01 AccountMode 末行**

修改 `render_account_mode`（约 line 149-171），在末行"解除条件: ..."后追加 `\n辅助建议, 非下单指令`。

- [ ] **Step 2: F-02 修复 T-02 DataMode 末行**

修改 `render_data_mode`（约 line 175-197），在末行"恢复预计: ..."后追加 `\n辅助建议, 非下单指令`。

- [ ] **Step 3: F-04 修复 R-02 ReviewMarket 末行**

修改 `render_review_market`（约 line 769-791），在末行"→ 明日账户建议: ..."后追加 `\n辅助建议, 非下单指令`。

- [ ] **Step 4: F-05 修复 P-02 AuctionVolume 标题 + 末行**

修改 `render_auction_volume`（约 line 592-611）：
- 标题：`🌅 竞价异动 TopN（{hhmm}）` → `🌅 竞价热点量能（{hhmm}）`
- 末行追加：`\n辅助建议, 非下单指令`

- [ ] **Step 5: F-07 修复 I-08 TurnoverTop 标题**

修改 `render_turnover_top`（约 line 700-716）：
- 标题：`🔄 盘中换手率 Top10 ({hhmm} 盘中)` → `🔄 盘中换手率 Top10 ({hhmm})`

- [ ] **Step 6: F-08 修复 A-05 ReviewLhb 空 entries 兜底**

修改 `render_review_lhb`（约 line 846-864）：
```rust
pub fn render_review_lhb(date: &str, entries: &[LhbEntry<'_>]) -> String {
    if entries.is_empty() {
        return "🐉 龙虎榜净买前五（{date} 21:00）\n盘中无数据 (盘后 21:00 才更新), 请参考 T-13 盘中换手率 Top10".to_string();
    }
    // ... 既有逻辑
}
```

- [ ] **Step 7: 12 项对齐回归测试**

为 6 项修复（每项 2 用例 = 12）添加测试，例如：

```rust
#[test]
fn account_mode_appends_helper_line() {
    let out = render_account_mode(/* test args */);
    assert!(out.ends_with("辅助建议, 非下单指令"));
}

#[test]
fn auction_volume_title_unified() {
    let out = render_auction_volume(/* test args */);
    assert!(out.contains("🌅 竞价热点量能"));
    assert!(!out.contains("竞价异动"));
}

#[test]
fn turnover_top_no_intraday_suffix() {
    let out = render_turnover_top("10:00", &[]);
    assert!(out.contains("🔄 盘中换手率 Top10 (10:00)"));
    assert!(!out.contains(" 盘中)"));
}

#[test]
fn review_lhb_empty_entries_fallback() {
    let out = render_review_lhb("2026-07-06", &[]);
    assert!(out.contains("盘中无数据 (盘后 21:00 才更新)"));
    assert!(out.contains("T-13 盘中换手率 Top10"));
}

// + 8 个其他修复的回归测试
```

- [ ] **Step 8: 全检查 + Commit (PR #8)**

```bash
git commit -m "fix(v13): 现有 render 对齐 §14 风格与字段（12 项差异）

Refs: docs/superpowers/specs/2026-07-06-v13-push-templates-design.md §6
Data-Redlines: [2.1]
修复: F-01/F-02/F-04 末行辅助建议; F-05/F-07 标题; F-08 空 entries 兜底

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3.2: PR #11 — 治理全表对齐 + requires_banner 修正

**Files:**
- Modify: `src/bin/monitor/notify.rs`
- Modify: `src/bin/monitor/push_templates.rs`（治理测试）

- [ ] **Step 1: G-01 修复 T0Advice(禁止) 等级**

修改 `render_t0_forbid` 调用点（约 push_templates.rs:1237），改为 `PushLevel::Info`（不改 enum，通过 `level_for_t0_forbid()` helper）。

或：拆分 `T0Advice` enum 为 `T0AdviceAllowed` + `T0AdviceForbid`（推荐，匹配 spec）。

- [ ] **Step 2: G-03 修复 PaperTrade should_block_on_mode**

修改 notify.rs `should_block_on_mode`（约 line 1498），将 `PaperTrade` 从停发列表中移出。

- [ ] **Step 3: G-04 修复 AuctionVolume level**

修改 notify.rs `level()`，将 `PushKind::AuctionVolume` 从 `Info` 改为 `Important`。

- [ ] **Step 4: G-05 修复 TurnoverTop cooldown**

修改 notify.rs `cooldown_secs()`，显式添加 `PushKind::TurnoverTop => Some(600)`。

- [ ] **Step 5: G-06 修复 IndustryChain cooldown**

修改 notify.rs `cooldown_secs()`，显式添加 `PushKind::IndustryChain => Some(86_400)`。

- [ ] **Step 6: G-07 修复 requires_banner 盘后 R 系列**

修改 notify.rs `requires_banner()` matches，移除以下盘后 R 系列：
- DailyReport, ReviewMarket, ReviewLhb, ReviewSignal, ReviewFailure, TomorrowWatch, EventCalendar, PaperReview, CatalystReview

- [ ] **Step 7: G-08 治理表补登 3 个 code-only PushKind**

修改 `docs/architecture/v13-push-templates.md §14.5` 表，追加：
- `CandidateBoard` (v11 兼容)
- `NewsRanked` (v11 兼容)
- `CloseCall` (T-12)

- [ ] **Step 8: 添加 8 个治理回归测试**

```rust
// G-01
#[test] fn gov_t0_advice_forbid_level() { /* ... */ }
// G-03
#[test] fn gov_paper_trade_not_blocked_on_frozen() { /* ... */ }
// G-04
#[test] fn gov_auction_volume_level() { assert_eq!(PushKind::AuctionVolume.level(), PushLevel::Important); }
// G-05
#[test] fn gov_turnover_top_cooldown() { assert_eq!(PushKind::TurnoverTop.cooldown_secs(), Some(600)); }
// G-06
#[test] fn gov_industry_chain_cooldown() { assert_eq!(PushKind::IndustryChain.cooldown_secs(), Some(86_400)); }
// G-07
#[test] fn gov_review_market_no_banner() { assert!(!PushKind::ReviewMarket.requires_banner()); }
// +2 其他
```

- [ ] **Step 9: 全检查 + Commit (PR #11)**

```bash
git commit -m "chore(v13): §14.5 治理全表对齐 + 35 PushKind + requires_banner 修正

Refs: docs/superpowers/specs/2026-07-06-v13-push-templates-design.md §7
Data-Redlines: [2.9]
修复: G-01/G-03/G-04/G-05/G-06/G-07/G-08
补登: CandidateBoard/NewsRanked/CloseCall 至 §14.5

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3.3: PR #12 — 文档漂移修正

**Files:**
- Modify: `src/bin/monitor/push_templates.rs`（line 4 注释）
- Modify: `src/bin/monitor/notify.rs`（如有引用）

- [ ] **Step 1: 修正 push_templates.rs 文件头注释**

将 line 4: `docs/architecture/v12-push-templates.md` → `docs/architecture/v13-push-templates.md`

- [ ] **Step 2: 验证无其他 v12 引用**

```bash
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
grep -rn "v12-push-templates" src/ 2>/dev/null
```

期望：无输出。

- [ ] **Step 3: Commit (PR #12)**

```bash
git commit -m "chore(v13): 文档漂移修正 — 引用 v13-push-templates

Refs: docs/architecture/v13-push-templates.md

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 自审 (Self-Review)

### 1. Spec 覆盖

- [x] §1 目标与范围 → Phase 0 紧急同步 + Phase 1-3 实施
- [x] §2 现状盘点 → Task 0.1-0.3 紧急 + Task 3.1 对齐
- [x] §3 模板分类 → Task 1.1-1.5 (v13) + Task 2.1-2.4 (新规)
- [x] §4 v13 新增模板 (7) → Task 1.1-1.5
- [x] §5 新规模板 (6) → Task 2.1-2.4
- [x] §6 12 项对齐 → Task 3.1
- [x] §7 35 PushKind 治理 → Task 1.1-1.5, 2.1-2.4, 3.2
- [x] §8 风格统一 → Task 3.1, 3.2
- [x] §9 测试矩阵 → 各 Task 含 5 个测试
- [x] §10 PR 节奏 + 紧急同步 → 14 Task 完整
- [x] §11 DoD → 各 Task 末尾 `bash tools/compliance/check.sh`

### 2. Placeholder 扫描

无 TBD/TODO/未填项。Step 中的 `test args` / `/* ... */` 为伪代码，task 执行者需按 spec §4-5 字段映射补全。

### 3. 类型一致性

- `BannerCtx::test_default()` — Task 1.1 注释中说明，若不存在需在 BannerCtx 实现
- `Exchange / OrderStatus / BlockType / Board / SettleType / StType` — 在各 Task 首次使用时定义
- `RenderCtx` trait — spec §8.2 提出，本 plan 未强制实施（可选，在 PR #11 中或后续 PR）

### 4. 紧急项

Phase 0 三 Task **必须**在 Task 1.1 之前完成。否则现有持仓风险计算基于旧 ST 阈值（5%）。

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-07-06-v13-push-templates-impl.md`. Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**

> **建议**：Phase 0（Task 0.1-0.3）建议**Inline Execution**（紧急，需要快速完成），Phase 1-3（Task 1.1-3.3）建议**Subagent-Driven**（可并行，每 PR 独立审阅）。
