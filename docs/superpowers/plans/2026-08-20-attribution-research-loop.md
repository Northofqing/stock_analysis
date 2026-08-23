# Attribution Research Loop 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立虚拟盘绩效归因闭环 (交付物 A: 每日 15:05 归因计算→落库→落盘→推送摘要) + 接回 G5a 盘中异动归因 (交付物 B)。

**Architecture:** 新增 `src/performance/attribution.rs` (纯逻辑: 信号族提取、带 lot 归属的 FIFO 匹配、日/窗口聚合、报告文本), 新表 `paper_attribution_daily`, main.rs 15:05 结算块接线, 新 PushKind::AttributionDaily 走标准注册链; G5a 在 main.rs 两处 `scan_stock` 循环内接线, 审计用 `alert_log::append_jsonl`。

**Tech Stack:** Rust, diesel (sqlite), tokio; 既有模式: `performance/snapshot.rs` FIFO、`notify.rs` PushKind 注册链、`push_governor_v3` 推送。

**Spec:** `docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md` (计划从 spec 论证; 执行者需同时读 spec 的 §3 证据与 §4/§5 设计)

## Global Constraints

- **提交纪律**: 只 `git add` 本分支文件 (src/performance/*, src/bin/monitor/notify.rs, push_templates.rs, v14_adapter.rs, main.rs, docs/operations/*)。**绝不 `git add` grpc WIP 文件**: `.claude/settings.json, build.rs, docs/business_rules.md, docs/operations/2026-08-18-data-grpc-known-issues.md, docs/superpowers/specs/2026-08-17-client-bundle-opening-readiness-design.md, docs/superpowers/specs/2026-08-18-newsflash-two-phase-source-audit-design.md, src/bin/grpc_bundle_probe.rs, src/data_gateway/global_news.rs, src/data_gateway/grpc_source.rs, src/data_gateway/grpc_source/convert.rs, src/grpc_client/client.rs, src/grpc_client/errors.rs, src/grpc_client/external_v1.rs, src/grpc_client/retry.rs, src/grpc_server/handlers.rs, src/news/aggregator/raw_v2.rs, findings.md, progress.md, task_plan.md`。提交前 `git status --short` 检查无 WIP 文件混入。
- **docs/ 被 .gitignore 忽略**: 遵守用户指令，不再用 `git add -f` 强制跟踪新文件；后续修订合并进已经被 Git 跟踪的设计、计划或业务规则文件。
- **v15 规则**: 默认值出声 — 归因空数据日仍生成报告并推送 (不静默跳过); 静默路径必须有注释说明。
- **测试字符串不进生产**: 单测里出现的 `first/mock/stub/test kept` 等字符串绝不出现在生产推送文本; 生产文本只由归因数据生成。
- **数据必须真实**: 不 mock, 不静默填零; 缺价格 → `unvalued` 计数, 缺数据 → 报告明示。
- **新 PushKind 必须完整注册**: notify.rs 5 个 match 块 (level/cooldown_secs/cooldown_scope/label/stable_template_id) + DISPATCH_TABLE + v14_adapter map — 遗漏任何一处编译报错 (non-exhaustive)。
- **commit 结尾**: `Co-Authored-By: Claude <noreply@anthropic.com>`。
- **分支**: `attribution-research-loop` (已创建 @ 3baa604)。全程不切换分支。

---

### Task 1: 信号族提取与可疑数据标注 (纯函数)

**Files:**
- Create: `src/performance/attribution.rs` (第一部分)
- Modify: `src/performance/mod.rs`
- Test: 同文件 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `SignalFamily` enum (`as_str() -> &'static str`), `signal_family_of(reason: &str) -> SignalFamily`, `parse_change_pct(reason: &str) -> Option<f64>`, `parse_volume_ratio(reason: &str) -> Option<f64>`, `is_suspicious_reason(reason: &str) -> bool` — 后续 Task 2/3 全部依赖。

- [ ] **Step 1: 声明子模块**

`src/performance/mod.rs` 全文替换为:
```rust
//! v16.4 #4: Performance module 入口

pub mod attribution;
pub mod snapshot;

pub use snapshot::{compute_snapshot, ensure_table, PerformanceEngine, PerformanceSnapshot};
```

- [ ] **Step 2: 写失败的测试** (追加到 `src/performance/attribution.rs` 底部)

创建 `src/performance/attribution.rs`:
```rust
//! 2026-08-20 Attribution Research Loop — 交付物 A 核心模块.
//!
//! 设计: docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md §4.
//! 数据来源: paper_trades (plan_id + virtual_reason), 证据 E3-E7.
//! 归因口径: 已实现 (FIFO 带 lot 归属) + 未实现浮盈 (未平仓 lot × 收盘价).

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 入场信号族 (归因维度). spec §4.1.
/// Ord 派生供 Task 3 的 BTreeMap 聚合排序使用.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SignalFamily {
    NewsCatalyst,
    VolumeSurge,
    MainNetInflow,
    Breakout,
    PostCloseFundInflow,
    ExitByRule,
    Unknown,
}

impl SignalFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            SignalFamily::NewsCatalyst => "NewsCatalyst",
            SignalFamily::VolumeSurge => "VolumeSurge",
            SignalFamily::MainNetInflow => "MainNetInflow",
            SignalFamily::Breakout => "Breakout",
            SignalFamily::PostCloseFundInflow => "PostCloseFundInflow",
            SignalFamily::ExitByRule => "ExitByRule",
            SignalFamily::Unknown => "Unknown",
        }
    }
}

/// virtual_reason → 信号族. 规则表见 spec §4.1; 未命中 → Unknown (报告明示, 不静默).
pub fn signal_family_of(reason: &str) -> SignalFamily {
    let r = reason.trim();
    if r.starts_with("NewsCatalyst") {
        return SignalFamily::NewsCatalyst;
    }
    if r.starts_with("VolumeSurge") {
        return SignalFamily::VolumeSurge;
    }
    if r.starts_with("MainNetInflow") {
        return SignalFamily::MainNetInflow;
    }
    if r.starts_with("Breakout") {
        return SignalFamily::Breakout;
    }
    if r.starts_with("盘后资金净流入") || r.contains("收盘价买入") {
        return SignalFamily::PostCloseFundInflow;
    }
    if r.starts_with("BR-") {
        return SignalFamily::ExitByRule;
    }
    SignalFamily::Unknown
}

/// 提取 `涨幅+X.X%` 数值; 无 → None.
pub fn parse_change_pct(reason: &str) -> Option<f64> {
    let (_, rest) = reason.split_once("涨幅")?;
    let value = rest.split('%').next()?.trim();
    value.parse::<f64>().ok()
}

/// 提取 `量比X.X` 数值; 无 → None.
pub fn parse_volume_ratio(reason: &str) -> Option<f64> {
    let (_, rest) = reason.split_once("量比")?;
    let value: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    value.parse::<f64>().ok()
}

/// 可疑数据: |涨幅| > 25 或 量比 ≤ 0 (spec §4.1; 证据 E6: 涨幅+858.9% ×27、量比0.0).
/// 可疑 lot 仍计入所属族 PnL, 由报告「数据质量」节单独标注 — 不删除, 不静默.
pub fn is_suspicious_reason(reason: &str) -> bool {
    if let Some(pct) = parse_change_pct(reason) {
        if pct.abs() > 25.0 {
            return true;
        }
    }
    if let Some(ratio) = parse_volume_ratio(reason) {
        if ratio <= 0.0 {
            return true;
        }
    }
    false
}
```

追加测试:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_from_reason_prefixes() {
        assert_eq!(signal_family_of("NewsCatalyst"), SignalFamily::NewsCatalyst);
        assert_eq!(signal_family_of("VolumeSurge"), SignalFamily::VolumeSurge);
        assert_eq!(signal_family_of("MainNetInflow"), SignalFamily::MainNetInflow);
        assert_eq!(signal_family_of("Breakout"), SignalFamily::Breakout);
        assert_eq!(signal_family_of("BR-234四大铁律卖出:结构止损（破中期趋势）"), SignalFamily::ExitByRule);
        assert_eq!(signal_family_of("盘后资金净流入Top10 收盘价买入: 主力+9.96亿 量比1.5 涨幅-2.9%"), SignalFamily::PostCloseFundInflow);
        assert_eq!(signal_family_of("未知原因"), SignalFamily::Unknown);
    }

    #[test]
    fn suspicious_rules_capture_garbage_but_keep_sane() {
        assert!(is_suspicious_reason("盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%"));
        assert!(is_suspicious_reason("... 涨幅+999.0%"));
        assert!(!is_suspicious_reason("... 涨幅+10.0% 量比1.5"));
        assert!(!is_suspicious_reason("NewsCatalyst"));
    }

    #[test]
    fn parse_helpers_extract_structured_fields() {
        let reason = "盘后资金净流入Top10 收盘价买入: 主力+9.96亿 量比1.5 涨幅-2.9%";
        assert_eq!(parse_change_pct(reason), Some(-2.9));
        assert_eq!(parse_volume_ratio(reason), Some(1.5));
        assert_eq!(parse_change_pct("NewsCatalyst"), None);
        assert_eq!(parse_volume_ratio("NewsCatalyst"), None);
    }

    #[test]
    fn family_names_are_stable_snake_case() {
        assert_eq!(SignalFamily::PostCloseFundInflow.as_str(), "PostCloseFundInflow");
        assert_eq!(SignalFamily::ExitByRule.as_str(), "ExitByRule");
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test --lib performance::attribution 2>&1 | tail -5`
Expected: FAIL (mod 不存在)

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib performance::attribution`
Expected: 4 tests passed

- [ ] **Step 5: Commit**

```bash
git add src/performance/attribution.rs src/performance/mod.rs
git commit -m "feat(performance): signal family extraction and suspicious data rules

交付物 A Task 1: SignalFamily 枚举 + signal_family_of / parse_change_pct /
parse_volume_ratio / is_suspicious_reason (spec §4.1, 证据 E4/E6)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: FIFO 匹配带 lot 归属 (TradeAttribution)

**Files:**
- Modify: `src/performance/attribution.rs` (追加)
- Test: 同文件

**Interfaces:**
- Consumes: Task 1 的 `SignalFamily / signal_family_of / is_suspicious_reason`
- Produces: `AttributionFillRow` (QueryableByName, 用于 SQL 查询), `OpenLot { code, plan_id, family, suspicious, remaining_qty, cost_price }`, `TradeAttribution { sell_id, code, pnl, entry_plan_id, entry_family, exit_reason, suspicious }`, `fifo_match(rows: &[AttributionFillRow], target_date: NaiveDate) -> Result<(Vec<TradeAttribution>, Vec<OpenLot>), String>` — Task 3 依赖。

- [ ] **Step 1: 写失败的测试** (追加到 tests mod)

```rust
    fn fill(
        id: i64,
        code: &str,
        direction: &str,
        price: f64,
        quantity: i64,
        local_ts: &str,
        plan_id: &str,
        virtual_reason: &str,
    ) -> AttributionFillRow {
        AttributionFillRow {
            id,
            code: code.to_string(),
            direction: direction.to_string(),
            fill_price: Some(price),
            quantity,
            local_ts: local_ts.to_string(),
            plan_id: plan_id.to_string(),
            virtual_reason: virtual_reason.to_string(),
        }
    }

    #[test]
    fn fifo_carries_lot_attribution() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-17 10:00:00", "news-1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "buy", 12.0, 200, "2026-07-18 09:31:00", "fund-2", "MainNetInflow"),
            fill(3, "TEST_CODE_600000", "sell", 15.0, 200, "2026-07-18 14:00:00", "sell-3", "BR-234四大铁律卖出:结构止损"),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");

        // 200 股卖出: 100 股归 NewsCatalyst lot (10.0→15.0 = +500), 100 股归 MainNetInflow lot (12.0→15.0 = +300)
        assert_eq!(attributions.len(), 2);
        let news: Vec<_> = attributions.iter().filter(|a| a.entry_family == SignalFamily::NewsCatalyst).collect();
        let fund: Vec<_> = attributions.iter().filter(|a| a.entry_family == SignalFamily::MainNetInflow).collect();
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].pnl, 500.0);
        assert_eq!(news[0].entry_plan_id, "news-1");
        assert_eq!(fund.len(), 1);
        assert_eq!(fund[0].pnl, 300.0);
        assert_eq!(fund[0].entry_plan_id, "fund-2");
        assert_eq!(attributions.iter().map(|a| a.pnl).sum::<f64>(), 800.0); // 与 snapshot.rs 已知结果一致
        assert_eq!(open.len(), 1); // MainNetInflow lot 剩 100 股
        assert_eq!(open[0].remaining_qty, 100);
        assert_eq!(open[0].cost_price, 12.0);
    }

    #[test]
    fn fifo_rejects_oversell_and_invalid_rows() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let oversell = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-18 10:00:00", "p1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "sell", 11.0, 200, "2026-07-18 14:00:00", "s1", "BR-234四大铁律卖出"),
        ];
        let err = fifo_match(&oversell, target).expect_err("oversell must fail");
        assert!(err.contains("exceeds matched buys"));

        let mut missing_price = fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-18 10:00:00", "p1", "NewsCatalyst");
        missing_price.fill_price = None;
        let err = fifo_match(&[missing_price], target).expect_err("missing price must fail");
        assert!(err.contains("fill_price missing/invalid"));
    }

    #[test]
    fn fifo_only_emits_target_date_sells() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 200, "2026-07-16 10:00:00", "p1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "sell", 11.0, 100, "2026-07-17 14:00:00", "s1", "BR-234四大铁律卖出"),
            fill(3, "TEST_CODE_600000", "sell", 12.0, 100, "2026-07-18 14:00:00", "s2", "BR-234四大铁律卖出"),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        assert_eq!(attributions.len(), 1); // 只归当日卖出
        assert_eq!(attributions[0].pnl, 200.0);
        assert_eq!(open.len(), 0);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib performance::attribution`
Expected: FAIL (fifo_match / AttributionFillRow / OpenLot / TradeAttribution 未定义)

- [ ] **Step 3: 实现 FIFO 匹配** (追加到 attribution.rs, `use chrono::NaiveDate;` 已有)

```rust
#[derive(diesel::QueryableByName, Debug)]
pub struct AttributionFillRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub fill_price: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub local_ts: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub plan_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub virtual_reason: String,
}

/// 已实现交易归因 — 每笔卖出按匹配到的入场 lot 拆分归属.
#[derive(Debug, Clone, PartialEq)]
pub struct TradeAttribution {
    pub sell_id: i64,
    pub code: String,
    pub pnl: f64,
    pub entry_plan_id: String,
    pub entry_family: SignalFamily,
    pub exit_reason: String,
    pub suspicious: bool,
    /// 卖出发生日期 (Task 3 compute_window 按此过滤窗口).
    pub sell_date: NaiveDate,
}

/// 未平仓 lot (FIFO 匹配剩余).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenLot {
    pub code: String,
    pub plan_id: String,
    pub family: SignalFamily,
    pub suspicious: bool,
    pub remaining_qty: i64,
    pub cost_price: f64,
}

/// FIFO 匹配: 语义与 performance/snapshot.rs::realized_pnls_for_date 逐条对齐
/// (id>0, code 非空, price>0 finite, qty>0 且 %100==0, 时间序校验, oversell 拒绝,
/// 非 finite PnL 拒绝), 区别: 匹配时携带入场 lot 的 plan_id/family/suspicious 归属.
/// 跨 lot 匹配时 PnL 按数量比例拆分 (每段生成一条 TradeAttribution).
/// 返回 (当日已实现归因列表, 未平仓 lot 列表).
pub fn fifo_match(
    rows: &[AttributionFillRow],
    target_date: NaiveDate,
) -> Result<(Vec<TradeAttribution>, Vec<OpenLot>), String> {
    use std::collections::{HashMap, VecDeque};

    #[derive(Clone)]
    struct Lot {
        remaining: u32,
        price: f64,
        plan_id: String,
        family: SignalFamily,
        suspicious: bool,
    }

    let mut lots: HashMap<String, VecDeque<Lot>> = HashMap::new();
    let mut realized = Vec::new();
    let mut previous_order: Option<(chrono::NaiveDateTime, i64)> = None;

    for row in rows {
        if row.id <= 0 || row.code.trim().is_empty() {
            return Err(format!(
                "attribution fill identity invalid: id={} code={:?}",
                row.id, row.code
            ));
        }
        let timestamp =
            chrono::NaiveDateTime::parse_from_str(&row.local_ts, "%Y-%m-%d %H:%M:%S")
                .map_err(|error| format!("attribution fill id={} timestamp invalid: {error}", row.id))?;
        if timestamp.date() > target_date {
            return Err(format!(
                "attribution fill id={} is later than settlement date {}",
                row.id, target_date
            ));
        }
        if previous_order.is_some_and(|previous| previous > (timestamp, row.id)) {
            return Err(format!("attribution fills are not ordered at id={}", row.id));
        }
        previous_order = Some((timestamp, row.id));
        let price = row
            .fill_price
            .filter(|price| price.is_finite() && *price > 0.0)
            .ok_or_else(|| format!("attribution fill id={} fill_price missing/invalid", row.id))?;
        let quantity = u32::try_from(row.quantity)
            .ok()
            .filter(|quantity| *quantity > 0 && quantity.is_multiple_of(100))
            .ok_or_else(|| {
                format!(
                    "attribution fill id={} quantity invalid: {}",
                    row.id, row.quantity
                )
            })?;
        let family = signal_family_of(&row.virtual_reason);
        let suspicious = is_suspicious_reason(&row.virtual_reason);

        match row.direction.as_str() {
            "buy" => lots.entry(row.code.clone()).or_default().push_back(Lot {
                remaining: quantity,
                price,
                plan_id: row.plan_id.clone(),
                family,
                suspicious,
            }),
            "sell" => {
                let queue = lots
                    .get_mut(&row.code)
                    .ok_or_else(|| format!("attribution sell id={} has no matched buy lots", row.id))?;
                let mut remaining = quantity;
                while remaining > 0 {
                    let lot = queue.front_mut().ok_or_else(|| {
                        format!(
                            "attribution sell id={} quantity {} exceeds matched buys",
                            row.id, quantity
                        )
                    })?;
                    let matched = remaining.min(lot.remaining);
                    let portion_pnl = (price - lot.price) * f64::from(matched);
                    if timestamp.date() == target_date {
                        realized.push(TradeAttribution {
                            sell_id: row.id,
                            code: row.code.clone(),
                            pnl: portion_pnl,
                            entry_plan_id: lot.plan_id.clone(),
                            entry_family: lot.family,
                            exit_reason: row.virtual_reason.clone(),
                            suspicious: lot.suspicious,
                            sell_date: timestamp.date(),
                        });
                    }
                    remaining -= matched;
                    lot.remaining -= matched;
                    if lot.remaining == 0 {
                        queue.pop_front(); // 与 snapshot.rs 同构: 已完成 lot 出队
                    }
                }
            }
            other => {
                return Err(format!(
                    "attribution fill id={} direction invalid: {other}",
                    row.id
                ));
            }
        }
    }
    // 非 finite 校验: 全部已实现 PnL 必须 finite (与 snapshot.rs 一致)
    for attribution in &realized {
        if !attribution.pnl.is_finite() {
            return Err(format!("attribution sell id={} PnL is non-finite", attribution.sell_id));
        }
    }
    let open = lots
        .into_iter()
        .flat_map(|(code, queue)| {
            queue.into_iter().map(move |lot| OpenLot {
                code: code.clone(),
                plan_id: lot.plan_id,
                family: lot.family,
                suspicious: lot.suspicious,
                remaining_qty: i64::from(lot.remaining),
                cost_price: lot.price,
            })
        })
        .collect();
    Ok((realized, open))
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib performance::attribution`
Expected: 7 tests passed

- [ ] **Step 5: Commit**

```bash
git add src/performance/attribution.rs
git commit -m "feat(performance): fifo attribution with lot carry

交付物 A Task 2: fifo_match 携带 plan_id/family/suspicious 归属,
跨 lot 拆分 PnL, 与 snapshot.rs 语义对齐 (spec §4.2)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 日/窗口聚合 (compute_daily / compute_window)

**Files:**
- Modify: `src/performance/attribution.rs` (追加)
- Test: 同文件

**Interfaces:**
- Consumes: Task 1 `SignalFamily`, Task 2 `fifo_match / TradeAttribution / OpenLot / AttributionFillRow`
- Produces: `FamilyAggregate`, `DailyAttribution`, `WindowAttribution`, `aggregate_families(attributions: &[TradeAttribution], open: &[OpenLot], prices: &HashMap<String, f64>) -> Vec<FamilyAggregate>`, `query_fills_until(date: NaiveDate) -> Result<Vec<AttributionFillRow>, String>` (SQL), `compute_daily(date: NaiveDate, prices: &HashMap<String, f64>) -> Result<DailyAttribution, String>`, `compute_window(end: NaiveDate, days: u32, prices: &HashMap<String, f64>) -> Result<WindowAttribution, String>` — Task 4/5/6 依赖。

- [ ] **Step 1: 写失败的测试**

```rust
    #[test]
    fn aggregate_families_sums_realized_and_unrealized() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-17 10:00:00", "news-1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "buy", 12.0, 200, "2026-07-18 09:31:00", "fund-2", "MainNetInflow"),
            fill(3, "TEST_CODE_600000", "sell", 15.0, 200, "2026-07-18 14:00:00", "sell-3", "BR-234四大铁律卖出:结构止损"),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        let mut prices = HashMap::new();
        prices.insert("TEST_CODE_600000".to_string(), 16.0);
        let families = aggregate_families(&attributions, &open, &prices);

        let news = families.iter().find(|f| f.family == SignalFamily::NewsCatalyst).expect("news family");
        assert_eq!(news.realized_pnl, 500.0);
        assert_eq!(news.realized_trades, 1);
        assert_eq!(news.wins, 1);
        assert_eq!(news.losses, 0);
        assert_eq!(news.win_rate, Some(1.0));
        assert_eq!(news.unrealized_pnl, 0.0);
        assert_eq!(news.open_lots, 0);

        let fund = families.iter().find(|f| f.family == SignalFamily::MainNetInflow).expect("fund family");
        assert_eq!(fund.realized_pnl, 300.0);
        // 剩余 100 股 × (16.0 - 12.0) = +400 浮盈
        assert_eq!(fund.unrealized_pnl, 400.0);
        assert_eq!(fund.open_lots, 1);
        assert_eq!(fund.total_pnl, 700.0);
    }

    #[test]
    fn missing_close_price_counts_unvalued_not_silent() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-17 10:00:00", "news-1", "NewsCatalyst"),
            fill(2, "TEST_CODE_600000", "buy", 12.0, 100, "2026-07-18 09:31:00", "news-2", "NewsCatalyst"),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        let prices = HashMap::new(); // 无任何收盘价
        let families = aggregate_families(&attributions, &open, &prices);
        let news = families.iter().find(|f| f.family == SignalFamily::NewsCatalyst).expect("news family");
        assert_eq!(news.open_lots, 2);
        assert_eq!(news.unvalued_lots, 2);
        assert_eq!(news.unrealized_pnl, 0.0); // 未估值不填零假装, 但计数出声
        assert_eq!(news.suspicious_lots, 0);
    }

    #[test]
    fn suspicious_lots_are_counted_per_family() {
        let target = NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let rows = vec![
            fill(1, "TEST_CODE_600000", "buy", 10.0, 100, "2026-07-17 10:00:00", "p1", "盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%"),
        ];
        let (attributions, open) = fifo_match(&rows, target).expect("valid FIFO fills");
        let families = aggregate_families(&attributions, &open, &HashMap::new());
        let fund = families.iter().find(|f| f.family == SignalFamily::PostCloseFundInflow).expect("fund family");
        assert_eq!(fund.suspicious_lots, 1);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib performance::attribution`
Expected: FAIL (FamilyAggregate 等未定义)

- [ ] **Step 3: 实现聚合与查询**

```rust
/// 单族聚合 (spec §4.2).
#[derive(Debug, Clone, PartialEq)]
pub struct FamilyAggregate {
    pub family: SignalFamily,
    pub realized_trades: i64,
    pub realized_pnl: f64,
    pub open_lots: i64,
    pub unrealized_pnl: f64,
    pub total_pnl: f64,
    pub wins: i64,
    pub losses: i64,
    pub win_rate: Option<f64>,
    pub unvalued_lots: i64,
    pub suspicious_lots: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyAttribution {
    pub date: NaiveDate,
    pub families: Vec<FamilyAggregate>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowAttribution {
    pub days: u32,
    pub end: NaiveDate,
    pub families: Vec<FamilyAggregate>,
}

/// 聚合: 已实现 (卖出归因) + 未实现浮盈 (未平仓 lot × close).
/// 缺失 close → unvalued_lots 计数, 浮盈记 0 (不静默: 计数与报告明示).
pub fn aggregate_families(
    attributions: &[TradeAttribution],
    open: &[OpenLot],
    prices: &HashMap<String, f64>,
) -> Vec<FamilyAggregate> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<SignalFamily, FamilyAggregate> = BTreeMap::new();
    let mut ensure = |family: SignalFamily| {
        map.entry(family).or_insert_with(|| FamilyAggregate {
            family,
            realized_trades: 0,
            realized_pnl: 0.0,
            open_lots: 0,
            unrealized_pnl: 0.0,
            total_pnl: 0.0,
            wins: 0,
            losses: 0,
            win_rate: None,
            unvalued_lots: 0,
            suspicious_lots: 0,
        })
    };
    for a in attributions {
        let row = ensure(a.entry_family);
        row.realized_trades += 1;
        row.realized_pnl += a.pnl;
        if a.pnl > 0.0 {
            row.wins += 1;
        } else {
            row.losses += 1;
        }
        if a.suspicious {
            row.suspicious_lots += 1;
        }
    }
    for lot in open {
        let row = ensure(lot.family);
        row.open_lots += 1;
        if lot.suspicious {
            row.suspicious_lots += 1;
        }
        match prices.get(&lot.code).copied().filter(|p| p.is_finite() && *p > 0.0) {
            Some(close) => row.unrealized_pnl += (close - lot.cost_price) * lot.remaining_qty as f64,
            None => row.unvalued_lots += 1,
        }
    }
    let mut families: Vec<FamilyAggregate> = map.into_values().collect();
    for row in &mut families {
        row.total_pnl = row.realized_pnl + row.unrealized_pnl;
        row.win_rate = (row.realized_trades > 0)
            .then_some(row.wins as f64 / row.realized_trades as f64);
    }
    families.sort_by_key(|f| f.family);
    families
}

const FILLS_UNTIL_SQL: &str = "SELECT id, code, direction, fill_price, quantity, \
     datetime(ts, 'localtime') AS local_ts, plan_id, virtual_reason \
     FROM paper_trades \
     WHERE datetime(ts, 'localtime') < datetime(?, '+1 day') AND status = 'Filled' \
     ORDER BY datetime(ts, 'localtime') ASC, id ASC";

/// 查询截至日期 (含) 的全部 Filled 成交 (与 snapshot.rs 查询同构, 多带 plan_id/virtual_reason).
pub fn query_fills_until(date: NaiveDate) -> Result<Vec<AttributionFillRow>, String> {
    let mut conn = crate::database::DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB: {e}"))?;
    let date_str = date.format("%Y-%m-%d").to_string();
    diesel::sql_query(FILLS_UNTIL_SQL)
        .bind::<diesel::sql_types::Text, _>(&date_str)
        .load::<AttributionFillRow>(&mut conn)
        .map_err(|e| format!("query paper_trades attribution: {e}"))
}

/// 当日归因: 已实现 (当日卖出 FIFO 全局匹配) + 浮盈 (截至当日未平仓 × close).
pub fn compute_daily(
    date: NaiveDate,
    prices: &HashMap<String, f64>,
) -> Result<DailyAttribution, String> {
    let rows = query_fills_until(date)?;
    let (attributions, open) = fifo_match(&rows, date)?;
    let families = aggregate_families(&attributions, &open, prices);
    Ok(DailyAttribution { date, families })
}

/// 30 天滚动窗口 (spec §4.5): 已实现 = 窗口内卖出 (FIFO 对历史全量 lot), 浮盈 = 期末未平仓 × close.
pub fn compute_window(
    end: NaiveDate,
    days: u32,
    prices: &HashMap<String, f64>,
) -> Result<WindowAttribution, String> {
    let rows = query_fills_until(end)?;
    let (all_attributions, open) = fifo_match(&rows, end)?;
    let start = end
        .checked_sub_signed(chrono::Duration::days(i64::from(days)))
        .unwrap_or(NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch"));
    let windowed: Vec<TradeAttribution> = all_attributions
        .into_iter()
        .filter(|a| a.sell_date >= start) // sell_date 字段由 Task 2 的 fifo_match 填充
        .collect();
    let families = aggregate_families(&windowed, &open, prices);
    Ok(WindowAttribution { days, end, families })
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib performance::attribution`
Expected: 10 tests passed

- [ ] **Step 5: Commit**

```bash
git add src/performance/attribution.rs
git commit -m "feat(performance): daily and window attribution aggregation

交付物 A Task 3: FamilyAggregate / compute_daily / compute_window,
未估值计数出声 (spec §4.2, §4.5)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: paper_attribution_daily 表与持久化

**Files:**
- Modify: `src/performance/attribution.rs` (追加)
- Test: 同文件 (纯 SQL 构造测试; DB 集成由 AC-A8/A9 验证)

**Interfaces:**
- Consumes: Task 3 `DailyAttribution / FamilyAggregate / SignalFamily`
- Produces: `ensure_attribution_table() -> Result<(), String>`, `persist_daily(daily: &DailyAttribution) -> Result<(), String>` (INSERT OR REPLACE, 幂等) — Task 6 依赖。

- [ ] **Step 1: 写失败测试** (SQL 以 const 形式存在, 用纯文本断言验证结构与幂等性, 不连库 — 与 snapshot.rs 无 DB 测试的仓库惯例一致; 真实落库由 AC-A6/A8 集成验证)

```rust
    #[test]
    fn ddl_const_declares_unique_per_date_and_family() {
        // 当日重算幂等锚点 (spec §4.3): UNIQUE(date, signal_family) + INSERT OR REPLACE
        assert!(DDL_SQL.contains("CREATE TABLE IF NOT EXISTS paper_attribution_daily"));
        assert!(DDL_SQL.contains("UNIQUE(date, signal_family)"));
        assert!(DDL_SQL.contains("unvalued_lots"));
        assert!(DDL_SQL.contains("suspicious_lots"));
    }

    #[test]
    fn persist_const_has_12_bind_slots_matching_12_columns() {
        // INSERT OR REPLACE (当日幂等, 与 snapshot 同模式) + 12 列 ↔ 12 个绑定占位
        assert!(PERSIST_SQL.contains("INSERT OR REPLACE INTO paper_attribution_daily"));
        let cols = PERSIST_SQL.split('(').nth(2).expect("column list").split(',').count();
        let binds = PERSIST_SQL.matches('?').count();
        assert_eq!(cols, binds, "columns ({cols}) must equal bind slots ({binds})");
    }
```
注: 真库幂等 (同一日期第二次调用不报错) 由 AC-A6/A8 集成验证; 若实现中发现可注入 conn 的既有模式 (如 `database::execution_tracking`), 采用之并补真测试。

- [ ] **Step 2: 实现表创建与持久化**

```rust
/// 建表 DDL (spec §4.3). const 供单测文本断言 (Step 1 测试依赖此 const).
const DDL_SQL: &str = "CREATE TABLE IF NOT EXISTS paper_attribution_daily (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            date            TEXT NOT NULL,
            signal_family   TEXT NOT NULL,
            realized_trades INTEGER NOT NULL DEFAULT 0,
            realized_pnl    REAL NOT NULL DEFAULT 0.0,
            open_lots       INTEGER NOT NULL DEFAULT 0,
            unrealized_pnl  REAL NOT NULL DEFAULT 0.0,
            total_pnl       REAL NOT NULL DEFAULT 0.0,
            wins            INTEGER NOT NULL DEFAULT 0,
            losses          INTEGER NOT NULL DEFAULT 0,
            win_rate        REAL,
            unvalued_lots   INTEGER NOT NULL DEFAULT 0,
            suspicious_lots INTEGER NOT NULL DEFAULT 0,
            created_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(date, signal_family)
        )";

/// 插入 SQL. const 供单测文本断言 (Step 1 测试依赖此 const).
const PERSIST_SQL: &str = "INSERT OR REPLACE INTO paper_attribution_daily \
             (date, signal_family, realized_trades, realized_pnl, open_lots, unrealized_pnl, \
              total_pnl, wins, losses, win_rate, unvalued_lots, suspicious_lots) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

/// 建表 (spec §4.3 DDL). 幂等, 与 paper_performance_snapshot 并行, 不 UPDATE 历史行.
pub fn ensure_attribution_table() -> Result<(), String> {
    let mut conn = crate::database::DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB: {e}"))?;
    diesel::sql_query(DDL_SQL)
        .execute(&mut conn)
        .map_err(|e| format!("create paper_attribution_daily: {e}"))?;
    Ok(())
}
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            date            TEXT NOT NULL,
            signal_family   TEXT NOT NULL,
            realized_trades INTEGER NOT NULL DEFAULT 0,
            realized_pnl    REAL NOT NULL DEFAULT 0.0,
            open_lots       INTEGER NOT NULL DEFAULT 0,
            unrealized_pnl  REAL NOT NULL DEFAULT 0.0,
            total_pnl       REAL NOT NULL DEFAULT 0.0,
            wins            INTEGER NOT NULL DEFAULT 0,
            losses          INTEGER NOT NULL DEFAULT 0,
            win_rate        REAL,
            unvalued_lots   INTEGER NOT NULL DEFAULT 0,
            suspicious_lots INTEGER NOT NULL DEFAULT 0,
            created_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(date, signal_family)
        )",
    )
    .execute(&mut conn)
    .map_err(|e| format!("create paper_attribution_daily: {e}"))?;
    Ok(())
}

/// 写入当日归因 (INSERT OR REPLACE, 当日重算幂等).
pub fn persist_daily(daily: &DailyAttribution) -> Result<(), String> {
    ensure_attribution_table()?;
    let mut conn = crate::database::DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB: {e}"))?;
    let date_str = daily.date.format("%Y-%m-%d").to_string();
    for row in &daily.families {
        diesel::sql_query(
            "INSERT OR REPLACE INTO paper_attribution_daily \
             (date, signal_family, realized_trades, realized_pnl, open_lots, unrealized_pnl, \
              total_pnl, wins, losses, win_rate, unvalued_lots, suspicious_lots) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind::<diesel::sql_types::Text, _>(&date_str)
        .bind::<diesel::sql_types::Text, _>(row.family.as_str())
        .bind::<diesel::sql_types::Integer, _>(row.realized_trades)
        .bind::<diesel::sql_types::Double, _>(row.realized_pnl)
        .bind::<diesel::sql_types::Integer, _>(row.open_lots)
        .bind::<diesel::sql_types::Double, _>(row.unrealized_pnl)
        .bind::<diesel::sql_types::Double, _>(row.total_pnl)
        .bind::<diesel::sql_types::Integer, _>(row.wins)
        .bind::<diesel::sql_types::Integer, _>(row.losses)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(row.win_rate)
        .bind::<diesel::sql_types::Integer, _>(row.unvalued_lots)
        .bind::<diesel::sql_types::Integer, _>(row.suspicious_lots)
        .execute(&mut conn)
        .map_err(|e| format!("insert paper_attribution_daily: {e}"))?;
    }
    Ok(())
}
```

- [ ] **Step 3: 编译+单测**

Run: `cargo test --lib performance::attribution && cargo build --lib`
Expected: 10 tests passed, build exit 0

- [ ] **Step 4: Commit**

```bash
git add src/performance/attribution.rs
git commit -m "feat(performance): paper_attribution_daily table and idempotent persist

交付物 A Task 4: spec §4.3 DDL + INSERT OR REPLACE 当日幂等

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 报告渲染 (全文 markdown + 推送摘要)

**Files:**
- Create: `src/performance/report.rs`
- Modify: `src/performance/mod.rs` (声明子模块)
- Test: `src/performance/report.rs` 内

**Interfaces:**
- Consumes: Task 3 `DailyAttribution / WindowAttribution / FamilyAggregate / SignalFamily`
- Produces: `render_full_markdown(daily: &DailyAttribution, window: &WindowAttribution) -> String`, `render_summary(daily: &DailyAttribution, window: &WindowAttribution) -> String` — Task 6 依赖。

- [ ] **Step 1: 写失败测试** (用固定 fixture 断言结构; 生产文本只由数据生成)

```rust
//! 归因报告渲染 — 全文 markdown + 推送摘要 (spec §4.4).

use super::attribution::{DailyAttribution, FamilyAggregate, SignalFamily, WindowAttribution};
use chrono::NaiveDate;

fn family(f: SignalFamily, realized: f64, unreal: f64, trades: i64, wins: i64, lots: i64, unvalued: i64, suspicious: i64) -> FamilyAggregate {
    FamilyAggregate {
        family: f,
        realized_trades: trades,
        realized_pnl: realized,
        open_lots: lots,
        unrealized_pnl: unreal,
        total_pnl: realized + unreal,
        wins,
        losses: trades - wins,
        win_rate: (trades > 0).then_some(wins as f64 / trades as f64),
        unvalued_lots: unvalued,
        suspicious_lots: suspicious,
    }
}

fn daily() -> DailyAttribution {
    DailyAttribution {
        date: NaiveDate::from_ymd_opt(2026, 8, 20).expect("date"),
        families: vec![
            family(SignalFamily::NewsCatalyst, -8120.0, -56000.0, 506, 192, 473, 0, 0),
            family(SignalFamily::PostCloseFundInflow, -3900.0, 1200.0, 270, 84, 135, 12, 27),
        ],
    }
}

fn window() -> WindowAttribution {
    WindowAttribution {
        days: 30,
        end: NaiveDate::from_ymd_opt(2026, 8, 20).expect("date"),
        families: daily().families.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_contains_family_lines_and_quality_section() {
        let text = render_summary(&daily(), &window());
        assert!(text.contains("📊 虚拟盘归因"));
        assert!(text.contains("NewsCatalyst"));
        assert!(text.contains("盘后资金流入"));
        assert!(text.contains("-8,120"));
        assert!(text.contains("数据存疑"));
        assert!(text.contains("27"));
        assert!(text.contains("未估值"));
    }

    #[test]
    fn full_markdown_has_sections() {
        let md = render_full_markdown(&daily(), &window());
        assert!(md.contains("# 虚拟盘归因"));
        assert!(md.contains("## 数据质量审计"));
        assert!(md.contains("## 今日归因"));
        assert!(md.contains("## 30 天滚动窗口"));
    }

    #[test]
    fn no_test_strings_leak_into_output() {
        // v15 规则: 测试文本不进生产路径 (spec Global Constraints)
        let text = render_summary(&daily(), &window());
        for forbidden in ["first", "second", "mock", "stub", "test kept", "placeholder", "fake", "sample"] {
            assert!(!text.contains(forbidden), "forbidden test string leaked: {forbidden}");
        }
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib performance::report`
Expected: FAIL (render_summary / render_full_markdown 未定义)

- [ ] **Step 3: 实现渲染**

```rust
/// 千分位 + 符号金额: -8120 → "-8,120"
fn fmt_money(v: f64) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let abs = v.abs().round();
    let digits = format!("{abs:.0}");
    let mut out = String::new();
    for (i, ch) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    format!("{sign}{}", out.chars().rev().collect::<String>())
}

/// 推送摘要 (~20 行, spec §4.4; 族按 |合计PnL| 降序, 序号由 ranks 数组索引 — 无硬编码)
pub fn render_summary(daily: &DailyAttribution, window: &WindowAttribution) -> String {
    let date = daily.date.format("%Y-%m-%d");
    let today_total: f64 = daily.families.iter().map(|f| f.total_pnl).sum();
    let win_total: f64 = window.families.iter().map(|f| f.total_pnl).sum();
    let win_realized: f64 = window.families.iter().map(|f| f.realized_pnl).sum();
    let win_unreal: f64 = window.families.iter().map(|f| f.unrealized_pnl).sum();
    let mut lines = vec![
        format!("📊 虚拟盘归因 {date}"),
        "━━━━━━━━━━━━━━━━━━━━".to_string(),
        format!("【今日】合计 {:<12}", fmt_money(today_total)),
        format!("【30天】已实现 {:<8} 期末浮盈 {}", fmt_money(win_realized), fmt_money(win_unreal)),
        "━━━━━━━━━━━━━━━━━━━━".to_string(),
    ];
    let mut families: Vec<&FamilyAggregate> = daily.families.iter().collect();
    families.sort_by(|a, b| b.total_pnl.abs().partial_cmp(&a.total_pnl.abs()).unwrap_or(std::cmp::Ordering::Equal));
    let ranks = ["①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩"];
    for (i, f) in families.iter().enumerate() {
        let rank = ranks.get(i).copied().unwrap_or("•");
        let label = match f.family {
            SignalFamily::PostCloseFundInflow => "盘后资金流入",
            SignalFamily::ExitByRule => "ExitByRule(卖)",
            other => other.as_str(),
        };
        let win = f.win_rate.map(|w| format!("胜率{:.0}%", w * 100.0)).unwrap_or_else(|| "胜率-".to_string());
        lines.push(format!(
            "{} {:<8} {:>6}笔 {:<10} {}",
            rank, label, f.realized_trades, fmt_money(f.total_pnl), win
        ));
    }
    lines.push("━━━━━━━━━━━━━━━━━━━━".to_string());
    let suspicious: i64 = daily.families.iter().map(|f| f.suspicious_lots).sum();
    let unvalued: i64 = daily.families.iter().map(|f| f.unvalued_lots).sum();
    let unknown: i64 = daily
        .families
        .iter()
        .filter(|f| f.family == SignalFamily::Unknown)
        .map(|f| f.open_lots + f.realized_trades)
        .sum();
    let mut quality = Vec::new();
    if suspicious > 0 {
        quality.push(format!("⚠ 数据存疑 {suspicious}笔"));
    }
    if unvalued > 0 {
        quality.push(format!("⚠ 未估值 {unvalued} lot"));
    }
    if unknown > 0 {
        quality.push(format!("⚠ Unknown {unknown}"));
    }
    if !quality.is_empty() {
        lines.push(quality.join("  |  "));
    }
    lines.join("\n")
}

/// 全文 markdown (spec §4.4 五节)
pub fn render_full_markdown(daily: &DailyAttribution, window: &WindowAttribution) -> String {
    let date = daily.date.format("%Y-%m-%d");
    let mut out = vec![format!("# 虚拟盘归因 {date}"), String::new()];
    out.push("## 数据质量审计".to_string());
    for f in &daily.families {
        if f.suspicious_lots > 0 || f.unvalued_lots > 0 {
            out.push(format!(
                "- {}: 存疑 {} lot / 未估值 {} lot",
                f.family.as_str(), f.suspicious_lots, f.unvalued_lots
            ));
        }
    }
    if !daily.families.iter().any(|f| f.suspicious_lots > 0 || f.unvalued_lots > 0) {
        out.push("- 无数据质量问题".to_string());
    }
    out.push(String::new());
    out.push("## 今日归因".to_string());
    out.push("| 信号族 | 已实现 | 浮盈 | 合计 | 笔数 | 胜率 |".to_string());
    out.push("|---|---|---|---|---|---|".to_string());
    for f in &daily.families {
        out.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            f.family.as_str(), fmt_money(f.realized_pnl), fmt_money(f.unrealized_pnl),
            fmt_money(f.total_pnl), f.realized_trades,
            f.win_rate.map(|w| format!("{:.0}%", w * 100.0)).unwrap_or_else(|| "-".to_string())
        ));
    }
    out.push(String::new());
    out.push("## 30 天滚动窗口".to_string());
    out.push("| 信号族 | 已实现累计 | 期末浮盈 | 合计 | 胜率 |".to_string());
    out.push("|---|---|---|---|---|".to_string());
    for f in &window.families {
        out.push(format!(
            "| {} | {} | {} | {} | {} |",
            f.family.as_str(), fmt_money(f.realized_pnl), fmt_money(f.unrealized_pnl),
            fmt_money(f.total_pnl),
            f.win_rate.map(|w| format!("{:.0}%", w * 100.0)).unwrap_or_else(|| "-".to_string())
        ));
    }
    out.push(String::new());
    out.join("\n")
}
```

`src/performance/mod.rs` 改为:
```rust
//! v16.4 #4: Performance module 入口

pub mod attribution;
pub mod report;
pub mod snapshot;

pub use snapshot::{compute_snapshot, ensure_table, PerformanceEngine, PerformanceSnapshot};
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib performance::`
Expected: 13 tests passed (attribution 10 + report 3)

- [ ] **Step 5: Commit**

```bash
git add src/performance/attribution.rs src/performance/report.rs src/performance/mod.rs
git commit -m "feat(performance): attribution report rendering (markdown + summary)

交付物 A Task 5: spec §4.4 全文五节 + ~20 行摘要, 质量审计段出声

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: main.rs 15:05 接线 (计算→落库→落盘→推送)

**Files:**
- Modify: `src/bin/monitor/main.rs` (15:05 块, 约 8196 行 `PERF_LAST_RUN` 块之后)
- Test: 集成验证 (AC-A6/A8/A9)

**Interfaces:**
- Consumes: Task 3 `compute_daily / compute_window`, Task 4 `persist_daily`, Task 5 `render_full_markdown / render_summary`, `market_data::fetch_position_quotes()`, `PushKind::AttributionDaily` (Task 7 先做! **Task 7 优先于 Task 6 或两者同批提交**, 否则 push_governor_v3 编译不过 — 实施时先完成 Task 7 的 enum/5-match/table 部分再写本接线)

**注意执行顺序**: 本 Task 依赖 `PushKind::AttributionDaily` 已存在。**先执行 Task 7 的 Step 1-5 (enum + 5 match + DISPATCH_TABLE + v14 map), 再执行本 Task**, 最后回到 Task 7 的 render 函数注册。或: 本 Task 与 Task 7 合并为一个提交 — 由执行者自行决定, 但**不得在 AttributionDaily 未注册时编译 main.rs**。

- [ ] **Step 1: 实现接线** (插入到 main.rs 约 8196 行 `*PERF_LAST_RUN...= Some(today);` 之后, 即 BR-226 提醒块之前)

```rust
            // Attribution Research Loop (2026-08-20 spec §4.6): 15:05 归因闭环。
            // 与 PerformanceEngine 同点运行, 当日一次, 失败出声 (30s 重试窗口沿用 PERF 模式)。
            if now.hour() == 15 && now.minute() == 5 {
                use std::collections::HashMap;
                use stock_analysis::performance::attribution::{
                    compute_daily, compute_window, persist_daily,
                };
                use stock_analysis::performance::report::{render_full_markdown, render_summary};
                static ATTRIBUTION_LAST_RUN: std::sync::Mutex<Option<chrono::NaiveDate>> =
                    std::sync::Mutex::new(None);
                let today = now.date_naive();
                let already_run = ATTRIBUTION_LAST_RUN
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|d| d == today)
                    .unwrap_or(false);
                if !already_run {
                    match (|| -> Result<String, String> {
                        let quotes = market_data::fetch_position_quotes()?;
                        // 生产价格映射 (build_price_map 是 cfg(test) 辅助, 生产内联同构构造)
                        let prices: HashMap<String, f64> = quotes
                            .iter()
                            .map(|q| (q.code.clone(), q.price))
                            .collect();
                        let daily = compute_daily(today, &prices)?;
                        persist_daily(&daily)?;
                        let window = compute_window(today, 30, &prices)?;
                        let md = render_full_markdown(&daily, &window);
                        std::fs::create_dir_all("data/attribution")
                            .map_err(|e| format!("create data/attribution: {e}"))?;
                        std::fs::write(format!("data/attribution/{}.md", today.format("%Y-%m-%d")), md)
                            .map_err(|e| format!("write attribution md: {e}"))?;
                        Ok(render_summary(&daily, &window))
                    })() {
                        Ok(text) => {
                            let outcome = push_governor_v3(&text, PushKind::AttributionDaily, None).await;
                            log::info!(
                                "[attribution] 15:05 归因推送完成: {:?}",
                                outcome
                            );
                            *ATTRIBUTION_LAST_RUN.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(today);
                        }
                        Err(e) => {
                            log::warn!("[attribution] 15:05 归因计算失败 (允许 30s 后重试): {e}");
                        }
                    }
                }
            }
```

注意: `market_data` 在 main.rs 中的引入方式 (现有代码 `market_data::fetch_position_quotes()` 直接可用, 见 main.rs:10798 用法)。若该处命名空间不可见, 用 `use crate::market_data;` 或在调用点写完整路径, 以现有编译为准。

- [ ] **Step 2: 编译**

Run: `cargo build --release --bin monitor`
Expected: exit 0 (若 `AttributionDaily` 未定义 → 先做 Task 7)

- [ ] **Step 3: --test 冒烟**

Run: `V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 | grep -E 'attribution' | head -5`
Expected: ≥1 行 (归因计算/推送日志; 15:05 定时块在 --test 下可能不触发 — 若触发则验证; 若 --test 不覆盖 15:05 块, 记录该事实并依赖生产 AC)

- [ ] **Step 4: Commit**

```bash
git add src/bin/monitor/main.rs
git commit -m "feat(monitor): wire attribution loop into 15:05 settlement

交付物 A Task 6: compute → persist → md 落盘 → push_governor_v3 (spec §4.6)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: PushKind::AttributionDaily 注册链

**Files:**
- Modify: `src/bin/monitor/notify.rs`, `src/bin/monitor/v14_adapter.rs`, `src/bin/monitor/push_templates.rs`, `src/bin/monitor/presentation_registry.rs`
- Test: 启动 audit (AC-B3) + `cargo build`

**Interfaces:**
- Consumes: Task 5 `render_summary` 的输出 (String) — 模板函数只做透传+注册
- Produces: `PushKind::AttributionDaily` (全链注册, 其他 Task 依赖), `render_attribution_daily(summary: &str) -> String`

- [ ] **Step 1: notify.rs 枚举加变体** (在 `SnapshotStale` 变体附近, 约 122 行)

```rust
    /// 虚拟盘绩效归因日推 (交付物 A, 每日 1 次, 默认出声) [2026-08-20]
    AttributionDaily,
```

- [ ] **Step 2: notify.rs 5 个 match 块各加一臂** (按现有 match 块的字段名; 用 grep 定位每个块)

`level()` 块加:
```rust
            PushKind::AttributionDaily => PushLevel::Info,
```
`cooldown_secs()` 加:
```rust
            PushKind::AttributionDaily => None,
```
`cooldown_scope()` 加:
```rust
            PushKind::AttributionDaily => CooldownScope::Global,
```
`label()` 加:
```rust
            PushKind::AttributionDaily => "虚拟盘归因",
```
`stable_template_id()` 加 (注意该方法当前是 `format!("daily_report_{}_v1", ...)` 的 DailyReport 特例; AttributionDaily 用通用规则):
```rust
            PushKind::AttributionDaily => "attribution_daily_v1".to_string(),
```
先 `grep -n "fn level\|fn cooldown_secs\|fn cooldown_scope\|fn label\|fn stable_template_id" src/bin/monitor/notify.rs` 定位 5 个 match 块的确切行, 逐一加臂。**编译报错为准**: non-exhaustive match 会列出每个缺臂的块。

- [ ] **Step 3: DISPATCH_TABLE 加行** (notify.rs:675 表内, 末尾追加)

```rust
    (
        PushKind::AttributionDaily,
        DispatchRow {
            level: PushLevel::Info,
            cooldown_secs: None,
            cooldown_scope: CooldownScope::Global,
            label: "虚拟盘归因",
            stable_template_id: "attribution_daily_v1",
        },
    ),
```

- [ ] **Step 4: v14_adapter.rs map_push_kind 加臂** (与 `PushKind::WeeklySOP` 行并列)

```rust
        PushKind::AttributionDaily => (HoldingHealth, "attribution_daily", Severity::Normal),
```

- [ ] **Step 5: push_templates.rs 渲染函数 + presentation_registry 注册**

push_templates.rs 追加 (模板函数只做透传, 文本由 report 模块生成):
```rust
/// 虚拟盘归因摘要 (交付物 A; 文本由 performance::report::render_summary 生成)
pub fn render_attribution_daily(summary: &str) -> String {
    summary.to_string()
}
```
presentation_registry.rs 注册条目 (仿照 `"render_intraday_alert"` 条目, presentation_registry.rs:81 附近):
```rust
        "render_attribution_daily",
```

- [ ] **Step 6: 编译 + 启动 audit**

Run: `cargo build --release --bin monitor && ./target/release/monitor --test 2>&1 | grep -iE 'dispatch|attribution' | head -10`
Expected: build exit 0; audit 输出含 AttributionDaily/attribution_daily_v1 (启动 audit 若报未注册即本链缺步, 按报错补齐)

- [ ] **Step 7: Commit**

```bash
git add src/bin/monitor/notify.rs src/bin/monitor/v14_adapter.rs src/bin/monitor/push_templates.rs src/bin/monitor/presentation_registry.rs
git commit -m "feat(monitor): register PushKind::AttributionDaily full chain

交付物 A Task 7: enum + 5 match 块 + DISPATCH_TABLE + v14 map +
render_attribution_daily + presentation_registry (spec §4.7)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: G5a 盘中异动归因接线

**Files:**
- Modify: `src/bin/monitor/main.rs` (两处 scan_stock 循环: ~8966 与 ~9472)
- Test: AC-B1 (--test 日志) + 编译

**Interfaces:**
- Consumes: `stock_analysis::monitor::attribution::apply_attribution`, `stock_analysis::monitor::alert_log::append_jsonl`
- Produces: 无新接口 (接线即产出)

- [ ] **Step 1: 接线第一处 (main.rs ~8966, 早盘限价循环)**

在 `for e in detector.scan_stock(&snap) { signal_count += 1;` 之后、`if let Some(event) = state_machine.process(e)` 之前插入:
```rust
                                // G5a (2026-08-20 spec §5): 同步归因, 2s 预算,
                                // 失败出声不折叠; ai_decision 随审计落库
                                // ({failure:?} 用 Debug 格式 — AttributionFailure 是否实现
                                // Display 未承诺, Debug 恒可用)
                                if let Err(failure) =
                                    stock_analysis::monitor::attribution::apply_attribution(&mut e)
                                {
                                    log::warn!("[G5a] attribution failed: {failure:?}");
                                }
                                if let Err(error) =
                                    stock_analysis::monitor::alert_log::append_jsonl(&e)
                                {
                                    log::warn!("[G5a] alert audit append failed: {error:?}");
                                }
```

- [ ] **Step 2: 接线第二处 (main.rs ~9472, 盘中循环)**

同样在 `for e in detector.scan_stock(&snap) { signal_count += 1;` 之后插入相同的归因+审计块 (该循环内 `e` 被后续 `match e.category` 借用为只读 + `signals.push(Signal::new(...))` — 归因块必须在任何借用之前, 且 `apply_attribution(&mut e)` 需要 `mut e`, 将 `for e in` 改为 `for mut e in`)。

- [ ] **Step 3: 编译 + --test 验证**

Run: `cargo build --release --bin monitor && V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 | grep -E 'G5a|attribution' | head -10`
Expected: build exit 0; ≥1 行 G5a/归因日志 (若 --test 未触发盘中循环, 记录并依赖 AC-B2 生产验证)

- [ ] **Step 4: Commit**

```bash
git add src/bin/monitor/main.rs
git commit -m "feat(monitor): wire G5a intraday attribution into alert path

交付物 B: apply_attribution 于两处 scan_stock 循环 (main.rs:8966/9472),
ai_decision 回写 + alert_log 单次审计, 失败出声 (spec §5)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: 文档 + 全量 AC 验证 + 收尾

**Files:**
- Create: `docs/operations/2026-08-20-attribution-research-loop.md`
- Test: 全量 AC 清单

- [ ] **Step 1: 写运维文档** (BR 注册 + 孤儿待办)

`docs/operations/2026-08-20-attribution-research-loop.md`:
```markdown
# 2026-08-20 Attribution Research Loop

## BR 注册 (待 grpc WIP 提交后合并进 business_rules.md — spec §8)
- BR-XXX: 虚拟盘绩效归因日推 (PushKind::AttributionDaily, 每日 15:05, 默认出声)
- BR-XXX: G5a 盘中异动归因接线 (apply_attribution + alert_log 审计)

## 已确认孤儿 (本分支不修, 留待后续)
- src/review/factor_ic.rs (637 行, 零生产调用者; PushKind::FactorIC 无生产者)
- src/review/failure_attribution.rs (136 行, R-06 设计, 零生产调用者)

## 生产验证清单 (AC)
... (复制 spec §6 的 AC-A8/A9/A10/B2/B3/B4 命令)
```

- [ ] **Step 2: 全量 AC 验证并粘贴输出** (逐条执行 spec §6 命令)

```bash
cargo test --lib performance:: 2>&1 | tail -3
cargo test --lib monitor::attribution 2>&1 | tail -3
cargo build --lib && cargo build --release --bin monitor
# 生产 AC (需交易日运行后验证; --test 可先跑):
V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 | grep -E 'attribution|G5a' | head -10
grep -RInE 'use stock_analysis::performance::|performance::attribution|PushKind::AttributionDaily' src/bin/monitor/ | wc -l   # 集成 grep (Completion Rule §2), 必须 ≥3
```
Expected: 单测全绿; build exit 0; 集成 grep ≥3; 生产 AC (A8/A9/A10/B2) 在下一交易日 15:05 后补验并记录日期。

- [ ] **Step 3: 提交文档**

```bash
git add docs/operations/2026-08-20-attribution-research-loop.md
git commit -m "docs: attribution loop BR registration and orphan backlog

交付物 A/B 收尾: spec §8 的 BR 注册 + 孤儿待办 (factor_ic / failure_attribution)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

- [ ] **Step 4: 最终检查**

Run: `git status --short`
Expected: 仅剩 grpc WIP 文件 (18 个) 未提交 — **确认本分支 9 个提交不含任何 WIP 文件**:
`git log --oneline 5d44613..HEAD | wc -l` → 9; 且每个提交的 `git show --stat <sha> | grep -E 'grpc|build.rs|settings.json|business_rules'` 均为空。

---

## Self-Review 记录

- **Spec 覆盖**: §4.1→T1, §4.2→T2/T3, §4.3→T4, §4.4→T5, §4.5→T3(compute_window), §4.6→T6, §4.7→T7, §5→T8, §7/§8→T9。AC-A1..B4 映射: A1-A3→T1/T2/T3 单测, A4/A5→各 Task build, A6/B1/B3→T6/T7/T8 冒烟, A7 一致性→T2 fifo_carries_lot_attribution 断言 800.0 (算法级), A8-A10→T9 生产清单, B4→T5 no_test_strings 测试。
- **占位符扫描**: 无 TBD/TODO; T6 内注释明确 `--test 不覆盖 15:05 块时记录事实` 属诚实边界, 非占位。
- **类型一致性**: `compute_daily/compute_window/persist_daily/render_full_markdown/render_summary/fifo_match/aggregate_families/query_fills_until` 签名在 T3-T6 间一致; `TradeAttribution.sell_date` 字段在 T3 明确要求回填 T2 结构体; `AttributionFillRow` 字段与 SQL 列一一对应 (id/code/direction/fill_price/quantity/local_ts/plan_id/virtual_reason)。
- **已知实施时需确认项**: (1) notify.rs 5 个 match 块的确切行号 (Step 2 已给 grep 命令); (2) `market_data` 命名空间可见性 (T6 Step 1 注); (3) --test 是否覆盖 15:05/盘中循环 (T6/T8 已注明记录事实)。

---

## 任务 10：2026-08-22 纸面卖出 T+1/FIFO 修订

**规格：** 本计划对应设计 §9，业务规则为 BR-134。实施在隔离分支 `codex/buy-sell-t1-fifo` 完成，不修改生产暂停闸、阈值、真实下单路径和 `monitor/main.rs`。

- [x] 新增纯 FIFO 批次账本，并按代码稳定排序。
- [x] 使用真实 `Filled.fill_price`，拒绝缺失、非正和非有限成交价。
- [x] 部分卖出按 FIFO 消耗原始批次，保留剩余价格、日期和数量。
- [x] 仅暴露隔夜可卖数量；当日批次锁定，同日卖出或超量消耗锁定批次整批失败。
- [x] 每一行在状态变更前校验不晚于评估日，未来卖出和未来买入后卖空均不能绕过。
- [x] 读取原始 `paper_trades.ts` 并严格解析，拒绝 SQLite `now`、仅时间及其他补造输入。
- [x] 多代码交错买卖保持 FIFO 隔离，输出按代码稳定排序。
- [x] 将评估日、成交 ID、剩余 lot、数量和精确价格绑定到防篡改订单审计，同时保持 `virtual_reason` 不变。
- [x] 数据库测试使用运行时唯一 `TEST_CODE` 并只清理自身代码。
- [x] 独立审查完成；Critical 时间补造和 Important 未来/同日绕过已修复。
- [ ] 结构性重建失败的独立持久审计事件：需要新的 Gate A，不能伪装成订单审计；在完成前生产暂停不解除。
- [ ] Gate B 全仓格式/Clippy：存在与本切片无关的既存失败，需按归因分批处理。
- [ ] Gate C 数据新鲜度：`stock_daily` 仍需受控真实回填后复验。
- [ ] Gate D：全仓覆盖率仍需达到 80%，核心交易/数据链路达到 95%。

已验证证据：

```text
da1c907  拒绝非法纸面成交时间线
041fe59  绑定纸面卖出 FIFO 审计证据
cargo test --lib trading::paper_ -- --test-threads=1
结果：67 passed, 0 failed
```

回滚：按新到旧顺序执行 `git revert 041fe59`、`git revert da1c907`，并继续回滚本分支更早的 FIFO 实施提交；禁止直接删除历史成交。

## 任务 11：2026-08-23 R-12 买入事件研究修订

**规格：** 对应设计 §10 与 BR-247。目标是消除策略来源/退出原因混淆和误导性胜率，不解除 BR-239 的 TechnicalBars 生产禁用。

- [x] 完成中文 Gate A：数据流、失败模式、旧模块关系、验收和回滚已登记。
- [x] RED：锁定早于首根 K 线不得映射到索引 0。
- [x] RED：锁定卖出行不进入买入事件统计，未知入场族整批失败。
- [x] RED：锁定逐事件路径 MFE/MAE，不再使用跨样本终点极值。
- [x] RED：锁定非法时间、真实 `fill_price` 缺失、坏/乱序 K 线整批失败。
- [x] GREEN：补全九个入场策略族并由 R-12 复用同一映射。
- [x] GREEN：把 R-12 收窄为买入事件研究，删除固定日期/`price < 1` 静默排除。
- [x] GREEN：报告改用“上涨比例/终点收益/路径 MFE/MAE”，固定输出非策略胜率声明和 200 样本门。
- [x] 回归：`cargo test --lib review::backtest::tests -- --test-threads=1`（12/12 PASS）。
- [x] 回归：`cargo test --lib performance::attribution -- --test-threads=1`（19/19 PASS）。
- [x] 回归：`cargo test --lib trading::paper_ -- --test-threads=1`（67/67 PASS）。
- [x] 编译/静态检查：`cargo check --bin monitor` 与 `cargo clippy --lib -- -D warnings`（PASS）。
- [ ] Gate B/C/D 与真实数据限制统一在最终证据节报告。
- [x] 双轴审查修正：精确边界对齐、连续时间栅格、high/low MFE/MAE、窄 close/volume 接口和全删失审计计数。

实现提交：`6f6a892`（入场策略族）、`73174c1`（R-12 买入事件研究）。
当前只证明实现和定向回归；全仓 Gate B/C/D 仍受任务 15 所列基线阻塞。BR-239 仍保持 Disabled，未发布 TechnicalBars
生产能力，也没有形成完整买入→卖出扣成本胜率结论。

回滚：文档、策略族映射和 R-12 实现分别 `git revert <sha>`；不得恢复硬编码日期删除、卖出胜率或边界 K 线补配。

## 任务 12：2026-08-23 经济仓位净收益归因

**规格：** 对应设计 §11 与 BR-248。先建立纯事实/计算层，不修改 `main.rs`、现有
归因生产接线、推送或交易闸。

- [x] Gate A：冻结空仓→再次空仓的主样本、开放仓位右删失、费用证据和失败边界。
- [x] RED：跨多个买入 lot 和多个部分卖单只形成一个闭合样本。
- [x] RED：归零后再次买入形成新的生命周期，代码间状态隔离。
- [x] RED：混合入场族保留组成，未知买入族整批失败。
- [x] RED：T+1、超卖、重复/乱序、非法身份/方向/价格/数量和未来行整批失败。
- [x] RED：费用缺失时净指标不可用；完整费用逐成交绑定，缺失/重复/未知引用失败。
- [x] 双轴审查 RED/GREEN：无真实费用适配器时任意字符串 Observed 失败关闭；Scenario 仍可显式计算。
- [x] RED：少于 200 个闭环或覆盖少于 84 天固定样本不足。
- [x] GREEN：新增 `performance::economic_position` 深模块及只读原始时间薄壳。
- [x] 回归：经济仓位 8/8、现有 attribution 19/19、paper FIFO/交易 67/67；
  `cargo check --bin monitor`、`cargo check --bin economic_position_probe` 与
  `cargo clippy --lib --bin economic_position_probe -- -D warnings` 均 PASS。
- [ ] 下一验证层：逐周期基准 Alpha、聚类不确定性和市场状态证据；完成前保持 ResearchOnly。
- [x] 真实历史只读探针已执行并按规则失败：`002594` 在 `id=520` 前只有 3,100 股
  隔夜批次，2026-08-11 当日买入 100 股后卖出 3,200 股，确定消费当日锁定批次。
- [ ] Gate C/D：历史批次当前不可采信，且费用、Alpha、聚类与市场状态证据仍缺失。

实现提交：`f120b6e`（经济仓位生命周期）、`dd4d0c4`（SQLite READ_ONLY 探针）。

真实只读命令与结果：

```text
cargo run --bin economic_position_probe -- \
  --db data/stock_analysis.db \
  --as-of 2026-08-23
结果：FAIL — economic sell id=520 violates A-share T+1 for 002594:
      buy_date=2026-08-11 sell_date=2026-08-11
只读复核：隔夜买入 3100 股 + 当日买入 100 股；id=520 卖出 3200 股。
```

该失败是当前策略验证结论的一部分：禁止删除/缩量 `id=520`、跳过该周期、假设部分
成交或用后续买入补平。历史事实继续保留，完整胜率不得发布。

回滚：设计、实现与证据分别 `git revert <sha>`；不得修改历史 `paper_trades`、补零费用
或恢复 lot 片段胜率。

## 任务 13：2026-08-23 纸面库存重建失败审计

**规格：** 对应设计 §12 与 BR-249。只补订单前失败审计，不修改 `main.rs`、推送、
卖出阈值、历史成交或生产暂停闸。

- [x] Gate A：冻结独立审计语义、来源快照、哈希链、五年留存、精确去重和回滚。
- [x] RED：解析失败与 FIFO/T+1 重建失败必须形成持久回执。
- [x] RED：完全相同失败重放不新增；来源或诊断变化必须新增。
- [x] RED：审计/链不可更新删除；篡改阻止追加；链写失败整笔回滚。
- [x] GREEN：新增独立 `paper_inventory_failure_audit` 深模块和启动校验。
- [x] GREEN：`paper_sell` 在原始行读取后的三个结构失败阶段调用审计。
- [x] 回归：审计模块 6/6、paper FIFO/交易 68/68、`cargo clippy --lib -- -D warnings`
  与 `cargo check --bin monitor` 均 PASS。
- [ ] Gate C/D：合规、全量测试、覆盖率和 PR 证据统一在最终证据节完成。

回滚：设计、实现与证据分别 `git revert <sha>`；数据库审计记录至少保留五年，不执行
破坏性 down migration，不改写或删除 `paper_trades`。

实现提交：`37dcf07`。集成测试已证明 T+1 失败在 `order_audit` 数量不变时取得 BR-249
回执；同一来源快照、评估日、阶段和诊断的第二次扫描返回 `disposition=existing`，审计
主表与链表仍各只有一行。`paper_sell_paused` 和 `src/bin/monitor/main.rs` 未修改。

## 任务 14：2026-08-23 真实验证证据盘点

**规格：** 对应设计 §13。目标是确认费用、基准、聚类和市场状态能否由现有事实支持，
避免为了“跑出胜率”新增无来源技术债。

- [x] SQLite READ_ONLY 盘点 898 条 `Filled` 纸面成交及时间覆盖。
- [x] 复核 `trades`：35 行费用全为 0，与纸面成交精确匹配和身份匹配均为 0。
- [x] 复核 `stock_daily`：5,006 行、57 个个股代码，没有沪深 300 历史序列。
- [x] 复核 `paper_performance_snapshot`：当前为零值空汇总，不作为成功证据。
- [x] 决定暂不实现默认费用、事后收盘基准或无来源市场状态适配器。
- [ ] 外部事实：建立不可变空仓确认后的干净验证纪元。
- [ ] 外部事实：提供逐成交 Observed 费用或明确批准的版本化 Scenario 费用。
- [ ] 外部事实：提供逐开平仓时点的历史基准与市场广度批次证据。
- [ ] 数据齐备后：另立 Gate A，实现周期超额、聚类不确定性和多状态报告。

当前结论：代码层不再有合理的“补一个计算函数即可得到成功案例”工作；剩余是数据采集
与用户/账户事实授权。禁止用旧 898 行选择性截断、新增默认常量或全 0 快照跨过该门。

## 任务 15：2026-08-24 双轴审查修正与全仓验证证据

**规格：** 对应设计 §10/§11 与 BR-247/BR-248。只修买卖策略验证边界，不修改
`src/bin/monitor/main.rs`、Unsafe 重复推送、交易阈值、生产订单路径或历史成交。

- [x] Gate A 文档提交：`da8fed5`。
- [x] 实现提交：`f94b680`。
- [x] R-12 定向测试：17/17 PASS；覆盖起点/终点栅格、内部与跨日缺口、栅格切换、
  精确时间边界、午休/非边界不对齐、high/low MFE/MAE 和原始 bars 共享未来路径。
- [x] Boll/MACD 窄接口测试：3/3 PASS；坏 close/volume 失败关闭，且与旧入口逐字段
  对比动作、原因、布林带、MACD 和量比，确认算法语义未改变。
- [x] 经济仓位测试：8/8 PASS；Observed 无真实来源能力时失败，Scenario 未回归。
- [x] R-12 dispatcher 审计测试：1/1 PASS；无统计分组但有退出排除、未对齐或截尾计数时
  仍可审计，只有分组与计数全部为零才返回 NoData。
- [x] `cargo check --bin monitor` 与 `cargo clippy --lib -- -D warnings` PASS；四个改动
  源文件的 `rustfmt --check` 与 `git diff --check` PASS。
- [ ] Gate B 全仓：同步 2026-08-24 最新 `master` 后，`cargo test -- --test-threads=1`
  的 lib 为 2796/0/7，monitor 为 683/2/4；仅失败 `main.rs` 既有 BR-139、BR-241
  源码计数断言，且该文件相对最新基线零差异。
  `cargo fmt --check` 仍命中多个既有文件；全目标 Clippy 仍命中 `t0_replay.rs` 两条
  `doc_lazy_continuation` 与 `hbars_probe.rs` 一条 `let_unit_value`。
- [ ] Gate C：`check_fake_impl.sh` 与设计矛盾检查 PASS；总合规因 worktree 无生产数据库
  导致 freshness FAIL，并因 60 条既有 BR 引用仓库中缺失的旧文档而 FAIL。补齐 BR-250
  两个 active path 的规则号后，最新业务规则复验为 60 errors/157 warnings；BR-247/248/250
  没有新增登记错误。未执行会写生产数据的回填脚本。
- [ ] Gate D：插桩测试仍只被相同两条 monitor 基线断言阻断。新鲜报告的全局行覆盖为
  175112/236807（73.95%，要求 80%）；核心覆盖为 140311/189071（74.21%，要求 95%，
  215 个文件）。BR-250 修复后官方检查器正确识别 worktree 核心文件并以真实低覆盖
  exit 1 退出，不再错误返回“没有核心模块”的 exit 2。

改动文件覆盖证据：`backtest.rs` 84.14%、`boll_macd.rs` 93.58%、
`economic_position.rs` 90.89%、`push_templates.rs` 65.93%。这些数字不满足 Gate D，不能
据此发布“策略已验证成功”或解除 ResearchOnly/TechnicalBars Disabled。回滚按新到旧执行
`git revert f94b680`、`git revert da8fed5`；不得删除历史成交或伪造生产数据。

## 任务 16：2026-08-24 修复 worktree 覆盖率路径识别

**规格：** 对应设计 §14 与 BR-250。只修验证工具的 checkout 路径归一化，不修改覆盖
报告、80%/95% 阈值、核心目录集合或任何生产路径。

- [x] 复现：真实报告在 worktree 中返回 exit 2，错误声称没有核心模块。
- [x] 根因：固定 `/stock_analysis/` 截断早于 cwd-relative 解析，产生 `.worktrees/.../src`。
- [x] RED：隔离 worktree 绝对路径 fixture 在旧检查器上返回 exit 2，未进入阈值判断。
- [x] GREEN：优先相对当前 checkout，保留外部 CI 重复仓库路径回退；4/4 工具测试通过。
- [x] 回归：真实报告正确返回 exit 1；全局 175112/236807（73.95%），核心
  140311/189071（74.21%，215 个文件），证明 Gate D 仍真实未达标。

实现提交：`f7419dd`。同步最新 `master` 的合并提交为 `38ba06e`；相对最新基线
`src/bin/monitor/main.rs` 仍为零差异。BR-250 只修验证工具，不改变任何策略、订单、阈值、
覆盖率报告或覆盖率门槛。

回滚：设计与实现分别 `git revert <sha>`；禁止修改 coverage JSON 或降低阈值。

## 任务 17：2026-08-24 公开未来路径 OHLC 失败关闭

**规格：** 对应设计 §10.2/§10.5 与 BR-247。`forward_observation/forward_return` 是
公开计算缝，不能依赖调用方必然先执行整批 K 线门禁；本任务只补路径 OHLC 自校验，
不改变时间对齐、窗口、指标公式、阈值、provider 或生产能力状态。

- [x] 缺口审计：当前路径只检查 `close/high/low` 正有限，不检查 `open` 或 OHLC 关系。
- [ ] RED：未来路径存在非有限 open 或 `high < close` 时必须返回 `None`。
- [ ] GREEN：逐根复用与整批门禁同义的正有限与 OHLC 关系校验。
- [ ] 回归：BR-247 定向测试、lib Clippy、monitor check、特定文件 rustfmt。

回滚：设计与实现分别 `git revert <sha>`；不得用坏 OHLC 继续计算 MFE/MAE。
