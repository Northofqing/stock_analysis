# Magic TDX T0 Price Zones Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the percentage-based T0 notification with a Magic TDX-only reverse-T observation plan derived from fresh quote, settled daily bars, completed 5-minute bars, intraday average price, volume pace, and five-level order-book evidence.

**Architecture:** `magic_tdx_t0` owns transport normalization and strict source validation, `decision::t0_advisor` owns deterministic indicator/zone/trigger decisions, and the monitor loop owns user-position association plus governed delivery. The production path fails closed per ticket, never substitutes Eastmoney/Sina/default percentages, and does not claim broker-verified sellable shares or place orders.

**Tech Stack:** Rust 2021, `magic-tdx-rs`, `chrono`, `sha2`, Tokio `spawn_blocking`, existing L4/L5 notification governor, Cargo test/clippy/llvm-cov, repository compliance scripts.

---

## File Map

- Create `src/data_provider/magic_tdx_t0.rs`: T0-specific Magic TDX source types, validation, batch acquisition, and pure normalization tests.
- Modify `src/data_provider/magic_tdx_provider.rs`: expose one thin `get_t0_evidence_batch` entry point while retaining existing daily/quote behavior.
- Modify `src/data_provider/mod.rs`: export the new evidence and rejection types.
- Rewrite `src/decision/t0_advisor.rs`: deterministic trend, ATR, volume pace, price-zone, trigger, invalidation, and equal-leg quantity logic.
- Modify `src/bin/monitor/push_templates.rs`: render the structured observation plan and explicit forbidden result.
- Modify `src/bin/monitor/main.rs`: replace detector/Eastmoney/Sina/symmetric-percentage logic with the Magic TDX batch and deterministic evaluator.
- Modify `src/bin/monitor/blocking_market_data.rs`: source audit proving the async loop uses the blocking boundary and contains no retired T0 sources.
- Modify `docs/business_rules.md`: keep BR-151 and BR-153 aligned with the implemented filter/sort/limit semantics.
- Modify `docs/superpowers/specs/2026-07-23-magic-tdx-t0-price-zones-design.md`: record any implementation-discovered clarification before code adopts it.

### Task 1: Define and validate Magic TDX T0 evidence

**Files:**
- Create: `src/data_provider/magic_tdx_t0.rs`
- Modify: `src/data_provider/magic_tdx_provider.rs`
- Modify: `src/data_provider/mod.rs`

- [ ] **Step 1: Write failing normalization and rejection tests**

Add fixture constructors and tests in `src/data_provider/magic_tdx_t0.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn observed_at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 1).unwrap()
    }

    #[test]
    fn quote_older_than_five_seconds_is_rejected() {
        let result = validate_quote_freshness(
            "TEST_CODE_600396",
            Utc.with_ymd_and_hms(2026, 7, 23, 1, 59, 55).unwrap(),
            observed_at(),
        );
        assert_eq!(result.unwrap_err().reason_code, "quote_stale");
    }

    #[test]
    fn completed_five_minute_bars_reject_duplicate_slots() {
        let at = Local
            .with_ymd_and_hms(2026, 7, 23, 9, 35, 0)
            .single()
            .unwrap()
            .naive_local();
        let bars = vec![five_minute(at), five_minute(at)];
        let result = validate_five_minute_bars("TEST_CODE_600396", &bars);
        assert_eq!(result.unwrap_err().reason_code, "five_minute_duplicate");
    }

    #[test]
    fn quote_batch_id_is_stable_for_same_source_payload() {
        let source_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let first = batch_id(source_at, &[("TEST_CODE_600396", 16.12)]);
        let second = batch_id(source_at, &[("TEST_CODE_600396", 16.12)]);
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn fewer_than_twenty_settled_daily_bars_is_rejected() {
        let result = validate_settled_daily("TEST_CODE_600396", &[]);
        assert_eq!(result.unwrap_err().reason_code, "daily_insufficient");
    }
}
```

The local helpers use only `TEST_CODE` symbols and deterministic timestamps.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --lib data_provider::magic_tdx_t0 -- --nocapture
```

Expected: compilation fails because `magic_tdx_t0` and its evidence functions do not exist.

- [ ] **Step 3: Implement source contracts and validators**

Create these public contracts in `src/data_provider/magic_tdx_t0.rs`:

```rust
use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use magic_tdx_rs::protocol::constants::{fq_type, KLINE_5MIN, KLINE_RI_K};
use magic_tdx_rs::protocol::types::{MinuteTimePrice, SecurityBar, SecurityQuote};
use magic_tdx_rs::TdxHqClient;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const T0_QUOTE_MAX_AGE_SECS: i64 = 5;
pub const T0_DAILY_MIN_BARS: usize = 20;
pub const T0_TODAY_MIN_COMPLETED_BARS: usize = 6;
pub const T0_HISTORY_MIN_SESSIONS: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct T0BookLevel {
    pub price: f64,
    pub volume: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MagicTdxT0Quote {
    pub price: f64,
    pub last_close: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    pub amount: f64,
    pub bids: [T0BookLevel; 5],
    pub asks: [T0BookLevel; 5],
}

#[derive(Clone, Debug, PartialEq)]
pub struct MagicTdxT0DailyBar {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MagicTdxT0FiveMinuteBar {
    pub at: NaiveDateTime,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MagicTdxT0Evidence {
    pub code: String,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub quote: MagicTdxT0Quote,
    pub settled_daily: Vec<MagicTdxT0DailyBar>,
    pub completed_five_minute: Vec<MagicTdxT0FiveMinuteBar>,
    pub intraday_average_price: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MagicTdxT0Rejection {
    pub code: String,
    pub reason_code: &'static str,
    pub detail: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MagicTdxT0Batch {
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub records: Vec<MagicTdxT0Evidence>,
    pub rejections: Vec<MagicTdxT0Rejection>,
}
```

Implement these pure checks:

```rust
fn valid_price(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn valid_nonnegative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn validate_ohlc(open: f64, high: f64, low: f64, close: f64) -> bool {
    [open, high, low, close].into_iter().all(valid_price)
        && high >= open.max(close)
        && low <= open.min(close)
        && high >= low
}

fn validate_quote_freshness(
    code: &str,
    source_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
) -> Result<(), MagicTdxT0Rejection> {
    let age = observed_at.signed_duration_since(source_at).num_seconds();
    if !(0..=T0_QUOTE_MAX_AGE_SECS).contains(&age) {
        return Err(MagicTdxT0Rejection {
            code: code.to_string(),
            reason_code: "quote_stale",
            detail: format!("age_secs={age} max_secs={T0_QUOTE_MAX_AGE_SECS}"),
            retryable: true,
        });
    }
    Ok(())
}

fn batch_id(source_at: DateTime<Utc>, quotes: &[(&str, f64)]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_at.timestamp_millis().to_be_bytes());
    for (code, price) in quotes {
        hasher.update(code.as_bytes());
        hasher.update(price.to_bits().to_be_bytes());
    }
    format!("{:x}", hasher.finalize())
}
```

`validate_settled_daily` sorts ascending, requires at least 20 unique trading dates, rejects current-date bars before settlement, rejects invalid OHLC/volume/amount, and rejects adjacent close moves whose absolute value exceeds 20%. `validate_five_minute_bars` sorts ascending, rejects duplicate timestamps, rejects invalid OHLC/volume/amount, keeps only bars whose end time is no later than the current completed 5-minute slot, requires six completed bars today, and requires at least three prior dates for the same first-N slot window.

- [ ] **Step 4: Implement one-connection batch acquisition**

Implement:

```rust
pub fn fetch_magic_tdx_t0_batch(
    codes: &[String],
    observed_at: DateTime<Utc>,
) -> Result<MagicTdxT0Batch> {
    let client = TdxHqClient::new();
    client
        .connect_to_any(Some(5.0))
        .map_err(|error| anyhow!("magic-tdx T0 connect failed: {error}"))?;

    let identities = codes
        .iter()
        .map(|code| normalized_identity(code))
        .collect::<Result<Vec<_>>>()?;
    let request = identities
        .iter()
        .map(|(market, code)| (*market, code.as_str()))
        .collect::<Vec<_>>();
    let quotes = client
        .get_security_quotes(&request)
        .map_err(|error| anyhow!("magic-tdx T0 quote batch failed: {error}"))?;
    if quotes.len() != request.len() {
        return Err(anyhow!(
            "magic-tdx T0 quote batch incomplete expected={} actual={}",
            request.len(),
            quotes.len()
        ));
    }

    let source_at = quotes
        .iter()
        .map(source_time)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .min()
        .ok_or_else(|| anyhow!("magic-tdx T0 quote batch empty"))?;
    let mut identity_prices = quotes
        .iter()
        .map(|quote| (quote.code.as_str(), quote.price))
        .collect::<Vec<_>>();
    identity_prices.sort_by_key(|(code, _)| *code);
    let batch_id = batch_id(source_at, &identity_prices);

    let mut records = Vec::new();
    let mut rejections = Vec::new();
    for quote in quotes {
        match evidence_for_quote(&client, quote, observed_at, &batch_id) {
            Ok(record) => records.push(record),
            Err(rejection) => rejections.push(rejection),
        }
    }
    records.sort_by(|left, right| left.code.cmp(&right.code));
    rejections.sort_by(|left, right| left.code.cmp(&right.code));
    Ok(MagicTdxT0Batch {
        source_at,
        observed_at,
        batch_id,
        records,
        rejections,
    })
}
```

`evidence_for_quote` calls:

```rust
client.get_security_bars(KLINE_RI_K, market, &code, 0, 40, fq_type::NONE)
client.get_security_bars(KLINE_5MIN, market, &code, 0, 240, fq_type::NONE)
client.get_minute_time_data(market, &code)
```

It maps five bid and five ask levels directly from `SecurityQuote`, derives the intraday average from the newest valid `MinuteTimePrice.avg_price`, and returns a per-ticket `MagicTdxT0Rejection` for each validation failure. It never substitutes process time for invalid provider time and never creates price/volume values.

- [ ] **Step 5: Add the provider façade and exports**

In `src/data_provider/magic_tdx_provider.rs`:

```rust
use super::magic_tdx_t0::{fetch_magic_tdx_t0_batch, MagicTdxT0Batch};

impl MagicTdxProvider {
    pub fn get_t0_evidence_batch(
        &self,
        codes: &[String],
        observed_at: DateTime<Utc>,
    ) -> Result<MagicTdxT0Batch> {
        fetch_magic_tdx_t0_batch(codes, observed_at)
    }
}
```

In `src/data_provider/mod.rs`:

```rust
pub mod magic_tdx_t0;
pub use magic_tdx_t0::{
    MagicTdxT0Batch, MagicTdxT0DailyBar, MagicTdxT0Evidence, MagicTdxT0FiveMinuteBar,
    MagicTdxT0Quote, MagicTdxT0Rejection, T0BookLevel,
};
```

- [ ] **Step 6: Run tests and commit**

Run:

```bash
cargo test --lib data_provider::magic_tdx_t0 -- --nocapture
cargo check --lib
```

Expected: all focused tests pass and the library checks cleanly.

Commit:

```bash
git add src/data_provider/magic_tdx_t0.rs src/data_provider/magic_tdx_provider.rs src/data_provider/mod.rs
git commit -m "feat(t0): add strict Magic TDX evidence batch"
```

### Task 2: Build the deterministic reverse-T evaluator

**Files:**
- Modify: `src/decision/t0_advisor.rs`

- [ ] **Step 1: Replace percentage-band tests with structured decision tests**

Add deterministic fixtures for 24 settled daily bars and four sessions of completed 5-minute bars. Cover:

```rust
#[test]
fn observation_leg_is_one_third_rounded_down_to_board_lot() {
    assert_eq!(observation_leg(500), Some(100));
    assert_eq!(observation_leg(299), None);
}

#[test]
fn main_up_core_is_forbidden_even_when_sell_trigger_matches() {
    let decision = evaluate_structured(&position(500), &main_up_core_evidence());
    assert!(matches!(
        decision,
        T0PlanDecision::Forbidden(ref value)
            if value.reason_code == "main_up_core"
    ));
}

#[test]
fn reverse_t_legs_use_the_same_quantity() {
    let decision = evaluate_structured(&position(500), &range_evidence());
    let T0PlanDecision::Advice(plan) = decision else {
        panic!("expected structured advice");
    };
    assert_eq!(plan.sell_quantity, 100);
    assert_eq!(plan.buyback_quantity, 100);
}

#[test]
fn spread_below_one_point_five_percent_is_forbidden() {
    let decision = evaluate_structured(&position(500), &narrow_range_evidence());
    assert!(matches!(
        decision,
        T0PlanDecision::Forbidden(ref value)
            if value.reason_code == "spread_insufficient"
    ));
}

#[test]
fn no_trigger_and_farther_than_half_atr_returns_no_advice() {
    let decision = evaluate_structured(&position(500), &far_from_zones_evidence());
    assert!(matches!(
        decision,
        T0PlanDecision::Rejected(ref value)
            if value.reason_code == "outside_observation_window"
    ));
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --lib decision::t0_advisor -- --nocapture
```

Expected: compilation fails because `evaluate_structured`, `T0PlanDecision`, and structured fixtures are not implemented.

- [ ] **Step 3: Define the structured decision model**

Replace the old `T0Input`/`T0Verdict` percentage-band API with:

```rust
use crate::data_provider::MagicTdxT0Evidence;

#[derive(Clone, Debug, PartialEq)]
pub struct T0Position {
    pub code: String,
    pub name: String,
    pub total_quantity: u64,
    pub cost_price: Option<f64>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrendStatus {
    MainUpCore,
    MainUp,
    Range,
    Weak,
    Fade,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ZoneSource {
    DailyPivot,
    IntradayPivot,
    IntradayAverage,
    AtrProjection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PriceZone {
    pub low: f64,
    pub high: f64,
    pub source: ZoneSource,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum T0PlanState {
    WaitingSell,
    SellTriggered,
    WaitingBuyback,
    BuybackTriggered,
    SellInvalidated,
    BuybackInvalidated,
}

#[derive(Clone, Debug, PartialEq)]
pub struct T0Metrics {
    pub trend: TrendStatus,
    pub pace_ratio: f64,
    pub last_bar_volume_ratio: f64,
    pub intraday_average_price: f64,
    pub atr14: f64,
    pub ask_bid_ratio: f64,
    pub bid_ask_ratio: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct T0StructuredPlan {
    pub code: String,
    pub name: String,
    pub source_at: chrono::DateTime<chrono::Utc>,
    pub batch_id: String,
    pub current_price: f64,
    pub total_quantity: u64,
    pub sell_quantity: u64,
    pub buyback_quantity: u64,
    pub sell_zone: PriceZone,
    pub buy_zone: PriceZone,
    pub gross_spread_pct: f64,
    pub metrics: T0Metrics,
    pub state: T0PlanState,
    pub trigger_text: String,
    pub invalidation_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct T0NoPlan {
    pub code: String,
    pub name: String,
    pub reason_code: &'static str,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum T0PlanDecision {
    Advice(T0StructuredPlan),
    Forbidden(T0NoPlan),
    Rejected(T0NoPlan),
}
```

- [ ] **Step 4: Implement indicators and trend classification**

Implement:

```rust
fn observation_leg(total: u64) -> Option<u64> {
    let shares = (total / 3 / 100) * 100;
    (shares >= 100).then_some(shares)
}

fn simple_ma(values: &[f64], period: usize) -> Option<f64> {
    let tail = values.get(values.len().checked_sub(period)?..)?;
    Some(tail.iter().sum::<f64>() / period as f64)
}

fn atr14(evidence: &MagicTdxT0Evidence) -> Option<f64> {
    let bars = &evidence.settled_daily;
    let start = bars.len().checked_sub(15)?;
    let ranges = bars[start + 1..]
        .iter()
        .zip(&bars[start..bars.len() - 1])
        .map(|(bar, previous)| {
            (bar.high - bar.low)
                .max((bar.high - previous.close).abs())
                .max((bar.low - previous.close).abs())
        })
        .collect::<Vec<_>>();
    Some(ranges.iter().sum::<f64>() / ranges.len() as f64)
}
```

The evaluator computes:

```rust
let trend = if price > ma5
    && ma5 > ma10
    && ma10 > ma20
    && settled_five_day_return_pct >= 8.0
    && pace_ratio >= 1.5
{
    TrendStatus::MainUpCore
} else if price > ma10 && ma5 > ma10 && ma10 > ma20 {
    TrendStatus::MainUp
} else if price < ma20 && ma5 < ma10 && ma10 < ma20 {
    TrendStatus::Fade
} else if price < ma10 || ma5 < ma10 {
    TrendStatus::Weak
} else {
    TrendStatus::Range
};
```

For the first `N` completed bars today:

```rust
pace_ratio =
    today_first_n_volume / mean(last_five_available_sessions_first_n_volume);
last_bar_volume_ratio =
    today_last_completed_volume / mean(same_slot_volume_from_last_five_available_sessions);
```

Both ratios require at least three valid historical sessions. The order-book ratios use the sum of all five levels and reject a zero denominator:

```rust
ask_bid_ratio = total_ask_volume / total_bid_volume;
bid_ask_ratio = total_bid_volume / total_ask_volume;
```

- [ ] **Step 5: Implement pivot selection, zones, triggers, and invalidation**

Use confirmed pivots only:

```rust
fn confirmed_daily_pivots(bars: &[MagicTdxT0DailyBar]) -> Vec<(f64, ZoneSource)> {
    bars.windows(5)
        .flat_map(|window| {
            let middle = &window[2];
            let high = (middle.high > window[0].high
                && middle.high > window[1].high
                && middle.high > window[3].high
                && middle.high > window[4].high)
                .then_some((middle.high, ZoneSource::DailyPivot));
            let low = (middle.low < window[0].low
                && middle.low < window[1].low
                && middle.low < window[3].low
                && middle.low < window[4].low)
                .then_some((middle.low, ZoneSource::DailyPivot));
            high.into_iter().chain(low)
        })
        .collect()
}
```

Add intraday pivots with one neighbor on each side, and the real Magic TDX intraday average. Choose the nearest sell candidate at least `price * 0.003` above current price and the nearest buy candidate at least `price * 0.003` below. Use `price ± 0.5 * ATR` with `ZoneSource::AtrProjection` only for a missing side.

Build each zone with:

```rust
let half_width = (atr * 0.1).clamp(price * 0.0015, price * 0.0035);
let zone = PriceZone {
    low: center - half_width,
    high: center + half_width,
    source,
};
```

Forbid when:

```rust
sell_zone.low <= buy_zone.high
    || (sell_zone.low / buy_zone.high - 1.0) * 100.0 < 1.5
```

Determine state in this order:

```rust
let state = if last_two_completed_closes_above_sell_high {
    T0PlanState::SellInvalidated
} else if latest_completed_close_below_buy_low {
    T0PlanState::BuybackInvalidated
} else if sell_zone.low <= price
    && price <= sell_zone.high
    && last_bar_volume_ratio >= 1.2
    && ask_bid_ratio >= 1.2
{
    T0PlanState::SellTriggered
} else if buy_zone.low <= price
    && price <= buy_zone.high
    && last_bar_volume_ratio <= 0.8
    && latest_completed_close > latest_completed_open
    && bid_ask_ratio >= 1.2
{
    T0PlanState::BuybackTriggered
} else if price > buy_zone.high {
    T0PlanState::WaitingSell
} else {
    T0PlanState::WaitingBuyback
};
```

For `WaitingSell` and `WaitingBuyback`, return `Rejected("outside_observation_window")` when the current price is farther than `0.5 * ATR` from both zones. `MainUpCore` and `Fade` always return `Forbidden`. The trigger and invalidation strings contain the exact numeric thresholds used by the decision.

- [ ] **Step 6: Run tests and commit**

Run:

```bash
cargo test --lib decision::t0_advisor -- --nocapture
cargo check --lib
```

Expected: all evaluator tests pass.

Commit:

```bash
git add src/decision/t0_advisor.rs
git commit -m "feat(t0): derive reverse T price zones"
```

### Task 3: Render actionable observation messages

**Files:**
- Modify: `src/bin/monitor/push_templates.rs`

- [ ] **Step 1: Write failing template contract tests**

Replace the old symmetric-band assertions with:

```rust
#[test]
fn t05_renders_magic_tdx_evidence_and_price_zones() {
    let text = render_t0_advice(&banner(), T0AdviceParams::from(&structured_plan()));
    assert!(text.contains("Magic TDX"));
    assert!(text.contains("批次: 0123456789ab"));
    assert!(text.contains("趋势: 震荡"));
    assert!(text.contains("量能节奏: 1.36x"));
    assert!(text.contains("末根5分钟量比: 1.28x"));
    assert!(text.contains("分时均价: ¥15.88"));
    assert!(text.contains("ATR14: ¥0.72"));
    assert!(text.contains("卖出观察区: ¥16.05~¥16.19"));
    assert!(text.contains("接回观察区: ¥15.20~¥15.34"));
    assert!(text.contains("观察腿: 100股卖出/100股接回"));
    assert!(text.contains("不代表券商已验证可卖数量"));
}

#[test]
fn t06_forbid_renders_evidence_reason_without_trade_instruction() {
    let text = render_t0_forbid(&banner(), T0ForbidParams::from(&forbidden()));
    assert!(text.contains("主升核心"));
    assert!(!text.contains("卖出100股"));
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --bin monitor t05_ -- --nocapture
```

Expected: test fails because the current template omits evidence metrics and state.

- [ ] **Step 3: Implement the structured renderer**

Change `T0AdviceParams` to borrow `T0StructuredPlan` fields and render:

```rust
pub fn render_t0_advice(banner: &BannerCtx, p: T0AdviceParams<'_>) -> String {
    format!(
        "{}\n🔁 做T观察【真实持仓】 {}({})（{}）\n\
         数据: Magic TDX | 批次: {} | 源时间: {}\n\
         状态: {} | 趋势: {}\n\
         现价: {} | 分时均价: {} | ATR14: {}\n\
         量能节奏: {:.2}x | 末根5分钟量比: {:.2}x | 五档卖/买: {:.2}x\n\
         卖出观察区: {}~{}（{}）\n\
         接回观察区: {}~{}（{}）\n\
         毛价差: {:.2}% | 观察腿: {}股卖出/{}股接回\n\
         触发: {}\n失效: {}\n\
         说明: 总持仓{}股；观察腿由用户确认持仓计算，不代表券商已验证可卖数量；执行前必须另取≤30秒券商可用持仓并校验T+1。",
        banner.render(),
        p.name,
        p.code,
        p.hhmm,
        &p.batch_id[..12],
        p.source_time,
        p.state,
        p.trend,
        fmt_price(p.current_price),
        fmt_price(p.intraday_average_price),
        fmt_price(p.atr14),
        p.pace_ratio,
        p.last_bar_volume_ratio,
        p.ask_bid_ratio,
        fmt_price(p.sell_lo),
        fmt_price(p.sell_hi),
        p.sell_source,
        fmt_price(p.buy_lo),
        fmt_price(p.buy_hi),
        p.buy_source,
        p.gross_spread_pct,
        p.sell_quantity,
        p.buyback_quantity,
        p.trigger_text,
        p.invalidation_text,
        p.total_quantity,
    )
}
```

Map every enum to a fixed Chinese label in `label()` methods; do not infer prose from missing values. The forbidden renderer includes source time, batch prefix, trend, and the explicit reason code/reason.

- [ ] **Step 4: Run tests and commit**

Run:

```bash
cargo test --bin monitor t05_ -- --nocapture
cargo test --bin monitor t06_ -- --nocapture
```

Expected: both template groups pass.

Commit:

```bash
git add src/bin/monitor/push_templates.rs
git commit -m "feat(t0): render evidence-backed observation zones"
```

### Task 4: Replace the production T0 producer

**Files:**
- Modify: `src/bin/monitor/main.rs`
- Modify: `src/bin/monitor/blocking_market_data.rs`

- [ ] **Step 1: Add failing source-audit tests**

In `src/bin/monitor/blocking_market_data.rs` add:

```rust
#[test]
fn br153_t0_path_uses_magic_tdx_only() {
    let source = include_str!("main.rs");
    let start = source.find("BR-151 / BR-153").expect("T0 start marker");
    let end = source[start..]
        .find("BR-153 T0 END")
        .map(|offset| start + offset)
        .expect("T0 end marker");
    let t0 = &source[start..end];
    assert!(t0.contains("MagicTdxProvider"));
    assert!(t0.contains("run_blocking_market_data"));
    assert!(t0.contains("evaluate_structured"));
    assert!(!t0.contains("fetch_eastmoney_quotes"));
    assert!(!t0.contains("fetch_sina_quotes"));
    assert!(!t0.contains("monitor::detector"));
    assert!(!t0.contains("change_pct.abs().max"));
}
```

Add an orchestration unit test for timer semantics around a new pure helper:

```rust
#[test]
fn t0_timer_advances_only_for_completed_confirmed_batch() {
    assert!(t0_batch_confirmed(true, &[]));
    assert!(t0_batch_confirmed(
        true,
        &[PushOutcome::Pushed, PushOutcome::Deduped]
    ));
    assert!(!t0_batch_confirmed(false, &[]));
    assert!(!t0_batch_confirmed(true, &[PushOutcome::Failed]));
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test --bin monitor br153_ -- --nocapture
```

Expected: source audit fails because the old detector and Eastmoney/Sina calls remain.

- [ ] **Step 3: Replace the old producer with one Magic TDX batch**

Within explicit markers `BR-151 / BR-153 T0 START` and `BR-153 T0 END`:

```rust
if last_t0_scan.elapsed().as_secs() >= 30
    && matches!(
        stock_analysis::calendar::current_session(),
        stock_analysis::calendar::MarketSession::Morning
            | stock_analysis::calendar::MarketSession::Afternoon
    )
{
    let snapshot = match stock_analysis::database::user_position_snapshot::
        latest_user_position_snapshot()
    {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            log::warn!("[做T-持仓][BR-153] skipped reason=user_snapshot_missing");
            last_t0_scan = std::time::Instant::now();
            continue;
        }
        Err(error) => {
            log::error!(
                "[做T-持仓][BR-153] user snapshot unavailable: {}",
                error
            );
            continue;
        }
    };
    let positions = snapshot
        .items
        .into_iter()
        .map(|item| T0Position {
            code: item.code,
            name: item.name,
            total_quantity: item.quantity,
            cost_price: item.cost_price,
        })
        .collect::<Vec<_>>();
    let codes = positions
        .iter()
        .map(|position| position.code.clone())
        .collect::<Vec<_>>();
    let observed_at = chrono::Utc::now();
    let batch = run_blocking_market_data(
        "BR-153 magic TDX T0 evidence",
        move || MagicTdxProvider::new()?.get_t0_evidence_batch(&codes, observed_at),
    )
    .await;
```

After a successful batch:

```rust
let positions_by_code = positions
    .iter()
    .map(|position| (position.code.as_str(), position))
    .collect::<HashMap<_, _>>();
let mut messages = Vec::new();
for rejection in &batch.rejections {
    log::warn!(
        "[做T-持仓][BR-153] code={} isolated reason_code={} retryable={} detail={}",
        rejection.code,
        rejection.reason_code,
        rejection.retryable,
        rejection.detail
    );
}
for evidence in &batch.records {
    let Some(position) = positions_by_code.get(evidence.code.as_str()) else {
        log::error!(
            "[做T-持仓][BR-153] source returned non-position code={}",
            evidence.code
        );
        continue;
    };
    match evaluate_structured(position, evidence) {
        T0PlanDecision::Advice(plan) => messages.push((
            plan.code.clone(),
            push_templates::render_t0_advice(
                &t0_banner(),
                push_templates::T0AdviceParams::from(&plan),
            ),
        )),
        T0PlanDecision::Forbidden(value) => {
            log::info!(
                "[做T-持仓][BR-153] code={} forbidden reason_code={} reason={}",
                value.code,
                value.reason_code,
                value.reason
            );
        }
        T0PlanDecision::Rejected(value) => {
            log::debug!(
                "[做T-持仓][BR-153] code={} no_plan reason_code={} reason={}",
                value.code,
                value.reason_code,
                value.reason
            );
        }
    }
}
```

Do not push forbidden/no-plan entries every 30 seconds. Push only evidence-backed `Advice`. Advance `last_t0_scan` for a successful complete batch when all attempted messages return `Pushed` or `Deduped`; retain immediate retry eligibility for quote-batch acquisition failure or an unconfirmed delivery.

- [ ] **Step 4: Remove retired T0 dependencies**

Delete from the T0 path:

```rust
market_data::fetch_eastmoney_quotes
market_data::fetch_sina_quotes
market_data::fetch_position_quotes
monitor::detector::Detector
monitor::detector::StockSnapshot
change_pct.abs().max(2.0)
hard_stop_value fallback rendering
```

Keep these modules elsewhere if other features use them. The change is scoped to the T0 producer.

- [ ] **Step 5: Run focused tests and commit**

Run:

```bash
cargo test --bin monitor br153_ -- --nocapture
cargo test --bin monitor t05_ -- --nocapture
cargo check --bin monitor
```

Expected: source audit and monitor check pass.

Commit:

```bash
git add src/bin/monitor/main.rs src/bin/monitor/blocking_market_data.rs
git commit -m "fix(monitor): use Magic TDX T0 observation plans"
```

### Task 5: Exercise all failure paths

**Files:**
- Modify: `src/data_provider/magic_tdx_t0.rs`
- Modify: `src/decision/t0_advisor.rs`
- Modify: `src/bin/monitor/push_templates.rs`

- [ ] **Step 1: Add the complete red-line test matrix**

Add tests with exact expected reason codes:

```rust
#[test]
fn invalid_quote_price_is_rejected() {
    let mut raw = valid_raw_quote();
    raw.price = 0.0;
    assert_eq!(
        normalize_quote("TEST_CODE_600396", raw, observed_at())
            .unwrap_err()
            .reason_code,
        "quote_invalid_price"
    );
}

#[test]
fn crossed_order_book_is_rejected() {
    let mut raw = valid_raw_quote();
    raw.bid1 = 16.13;
    raw.ask1 = 16.12;
    assert_eq!(
        normalize_quote("TEST_CODE_600396", raw, observed_at())
            .unwrap_err()
            .reason_code,
        "order_book_crossed"
    );
}

#[test]
fn daily_gap_above_twenty_percent_is_rejected() {
    let mut bars = settled_daily_fixture();
    bars[19].close = bars[18].close * 1.21;
    bars[19].high = bars[19].close;
    assert_eq!(
        validate_settled_daily("TEST_CODE_600396", &bars)
            .unwrap_err()
            .reason_code,
        "daily_change_over_20pct"
    );
}

#[test]
fn incomplete_current_five_minute_bar_is_excluded() {
    let now = Local
        .with_ymd_and_hms(2026, 7, 23, 10, 2, 0)
        .single()
        .unwrap();
    let completed = completed_only(five_minute_fixture(), now);
    assert!(completed.iter().all(|bar| bar.at.time() <= NaiveTime::from_hms_opt(9, 55, 0).unwrap()));
}

#[test]
fn fewer_than_one_board_lot_is_forbidden() {
    let decision = evaluate_structured(&position(299), &range_evidence());
    assert!(matches!(
        decision,
        T0PlanDecision::Forbidden(ref value)
            if value.reason_code == "leg_below_board_lot"
    ));
}

#[test]
fn sell_trigger_requires_volume_and_ask_pressure() {
    let mut evidence = sell_zone_evidence();
    evidence.quote.asks.iter_mut().for_each(|level| level.volume = 100.0);
    evidence.quote.bids.iter_mut().for_each(|level| level.volume = 100.0);
    let T0PlanDecision::Advice(plan) = evaluate_structured(&position(500), &evidence) else {
        panic!("expected waiting plan");
    };
    assert_eq!(plan.state, T0PlanState::WaitingSell);
}

#[test]
fn buyback_trigger_requires_contraction_green_bar_and_bid_support() {
    let decision = evaluate_structured(&position(500), &buyback_trigger_evidence());
    let T0PlanDecision::Advice(plan) = decision else {
        panic!("expected buyback plan");
    };
    assert_eq!(plan.state, T0PlanState::BuybackTriggered);
}

#[test]
fn two_closes_above_sell_zone_invalidate_sell() {
    let decision = evaluate_structured(&position(500), &sell_invalidated_evidence());
    let T0PlanDecision::Advice(plan) = decision else {
        panic!("expected invalidated plan");
    };
    assert_eq!(plan.state, T0PlanState::SellInvalidated);
}

#[test]
fn atr_projection_is_labeled_when_pivot_is_missing() {
    let decision = evaluate_structured(&position(500), &no_upper_pivot_evidence());
    let T0PlanDecision::Advice(plan) = decision else {
        panic!("expected projected plan");
    };
    assert_eq!(plan.sell_zone.source, ZoneSource::AtrProjection);
}
```

```rust
#[test]
fn empty_order_book_side_is_rejected() {
    let mut raw = valid_raw_quote();
    raw.ask_vol1 = 0.0;
    raw.ask_vol2 = 0.0;
    raw.ask_vol3 = 0.0;
    raw.ask_vol4 = 0.0;
    raw.ask_vol5 = 0.0;
    assert_eq!(
        normalize_quote("TEST_CODE_600396", raw, observed_at())
            .unwrap_err()
            .reason_code,
        "order_book_empty_side"
    );
}

#[test]
fn daily_duplicate_date_is_rejected() {
    let mut bars = settled_daily_fixture();
    bars[19].date = bars[18].date;
    assert_eq!(
        validate_settled_daily("TEST_CODE_600396", &bars)
            .unwrap_err()
            .reason_code,
        "daily_duplicate"
    );
}

#[test]
fn fewer_than_three_comparable_sessions_is_rejected() {
    let bars = five_minute_fixture_for_sessions(&[
        NaiveDate::from_ymd_opt(2026, 7, 22).unwrap(),
        NaiveDate::from_ymd_opt(2026, 7, 23).unwrap(),
    ]);
    assert_eq!(
        validate_five_minute_bars("TEST_CODE_600396", &bars)
            .unwrap_err()
            .reason_code,
        "history_slots_insufficient"
    );
}

#[test]
fn invalid_intraday_average_is_rejected() {
    let minute = MinuteTimePrice {
        time: "10:00".to_string(),
        price: 16.12,
        avg_price: 0.0,
        vol: 1_000.0,
    };
    assert_eq!(
        normalize_intraday_average("TEST_CODE_600396", &[minute])
            .unwrap_err()
            .reason_code,
        "intraday_average_invalid"
    );
}

#[test]
fn fade_is_forbidden() {
    let decision = evaluate_structured(&position(500), &fade_evidence());
    let T0PlanDecision::Forbidden(forbidden) = decision else {
        panic!("expected fade veto");
    };
    assert_eq!(forbidden.reason_code, "fade");
}

#[test]
fn close_below_buy_zone_invalidates_buyback() {
    let decision = evaluate_structured(&position(500), &buyback_invalidated_evidence());
    let T0PlanDecision::Advice(plan) = decision else {
        panic!("expected invalidated plan");
    };
    assert_eq!(plan.state, T0PlanState::BuybackInvalidated);
}
```

- [ ] **Step 2: Run tests and verify failures**

Run:

```bash
cargo test --lib data_provider::magic_tdx_t0 -- --nocapture
cargo test --lib decision::t0_advisor -- --nocapture
```

Expected: each newly added test fails before its precise validation branch exists.

- [ ] **Step 3: Implement missing explicit branches**

Add each reason-code branch at the validation boundary. No branch may replace a missing field with `0`, process time, a previous batch value, or a percentage default. Ensure `MagicTdxT0Rejection.detail` includes code, source timestamp when available, and the offending value/count.

- [ ] **Step 4: Run tests and commit**

Run:

```bash
cargo test --lib data_provider::magic_tdx_t0 -- --nocapture
cargo test --lib decision::t0_advisor -- --nocapture
cargo test --bin monitor t05_ -- --nocapture
```

Expected: all focused suites pass.

Commit:

```bash
git add src/data_provider/magic_tdx_t0.rs src/decision/t0_advisor.rs src/bin/monitor/push_templates.rs
git commit -m "test(t0): cover data and trigger failure paths"
```

### Task 6: Reconcile documentation and business rules

**Files:**
- Modify: `docs/business_rules.md`
- Modify: `docs/superpowers/specs/2026-07-23-magic-tdx-t0-price-zones-design.md`

- [ ] **Step 1: Compare code constants to BR-151/BR-153**

Run:

```bash
rg -n "BR-151|BR-153|1\\.5|1\\.2|0\\.8|0\\.5|0\\.3|one third|三分之一" \
  src/data_provider/magic_tdx_t0.rs \
  src/decision/t0_advisor.rs \
  src/bin/monitor/main.rs \
  docs/business_rules.md \
  docs/superpowers/specs/2026-07-23-magic-tdx-t0-price-zones-design.md
```

Expected: every implemented filter/sort/limit constant maps to the registered business rule.

- [ ] **Step 2: Update docs only for discovered clarification**

Keep the design values unchanged. If the provider’s 5-minute timestamp represents bar start rather than bar end, record the confirmed interpretation and exact completion conversion in the design and BR-153. If no clarification is needed, make no documentation edit in this step.

- [ ] **Step 3: Run business-rule compliance and commit any doc delta**

Run:

```bash
bash tools/compliance/lib/check_business_rules.sh
```

Expected: PASS.

If tracked docs changed:

```bash
git add docs/business_rules.md docs/superpowers/specs/2026-07-23-magic-tdx-t0-price-zones-design.md
git commit -m "docs(t0): align evidence rules with implementation"
```

### Task 7: Full Gate B and Gate C validation

**Files:**
- No production file changes unless a validation failure identifies a root cause.

- [ ] **Step 1: Format and reject formatting drift**

Run:

```bash
cargo fmt --all -- --check
```

Expected: exit 0. If it fails, run `cargo fmt --all`, inspect the diff, rerun the check, and commit only formatting changes with the owning implementation commit.

- [ ] **Step 2: Run strict Clippy**

Run:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: exit 0 with no warnings.

- [ ] **Step 3: Run the full test suite**

Run:

```bash
cargo test --workspace --all-targets --all-features -- --test-threads=1
```

Expected: all tests pass.

- [ ] **Step 4: Run repository compliance**

Run:

```bash
bash tools/compliance/check.sh
```

Expected: all checks pass, including freshness, fake implementation, design contradiction, and business-rule registration.

- [ ] **Step 5: Root-cause rollback on any failure**

- Architecture/data-flow conflict: update the design first, then return to Task 1.
- Red-line violation: fix the source boundary and re-run Tasks 1, 2, and 7.
- Implementation failure: add a reproducing test in the owning task and re-run Task 7.
- Freshness failure: run `bash tools/one_shot/backfill_daily.sh`, then re-run compliance; do not weaken the check.

### Task 8: Gate D coverage, live validation, release, and deployment

**Files:**
- Create evidence only under existing tracked evidence conventions if the repository already tracks that location.
- Do not edit production data or delete audit logs.

- [ ] **Step 1: Generate coverage**

Run:

```bash
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
```

Expected: total coverage at least 80% and the T0/core data path at least 95%. If the repository threshold tool reports a narrower core-path command, run that command and retain both results.

- [ ] **Step 2: Build release**

Run:

```bash
cargo build --release --bin monitor
```

Expected: exit 0.

- [ ] **Step 3: Run isolated monitor test mode**

Run with isolated state and no production push:

```bash
STOCK_DB=/private/tmp/stock_analysis_t0_test.db \
PUSH_ANALYTICS_DB=/private/tmp/stock_analysis_t0_push.db \
EVENT_AUDIT_PATH=/private/tmp/stock_analysis_t0_event_audit.jsonl \
V10_DRY_RUN_PUSH=1 \
RUST_BACKTRACE=1 \
target/release/monitor --test
```

Expected: exit 0, no Tokio runtime-drop panic, and no real-symbol order or production push.

- [ ] **Step 4: Run a read-only Magic TDX live evidence probe**

Use the production provider through a focused ignored test or existing probe command that prints only:

```text
code, source_at, observed_at, batch_prefix, daily_count,
completed_5m_count, historical_session_count, intraday_average,
bid_volume_5, ask_volume_5
```

Expected for a supported real holding during the trading session: quote age no more than five seconds, at least 20 settled daily bars, at least six completed 5-minute bars after 10:00, at least three comparable historical sessions, valid positive average price, and non-empty five-level book. This probe must not push or place orders.

- [ ] **Step 5: Review the diff**

Use the repository `review` skill against the pre-feature commit. Resolve all blocking Standards and Spec findings, then rerun Tasks 7 and 8 Steps 1–4.

- [ ] **Step 6: Commit validation evidence**

If the repository stores validation evidence in tracked docs, append:

```markdown
### Validation
- `cargo fmt --all -- --check`: PASS
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: PASS
- `cargo test --workspace --all-targets --all-features -- --test-threads=1`: PASS
- `bash tools/compliance/check.sh`: PASS
- coverage: copy the measured total and T0/core percentages from llvm-cov output
- isolated `target/release/monitor --test`: PASS
- Magic TDX read-only probe: copy the provider source timestamp printed by the probe
```

Use measured numbers and timestamps only. Do not write `PASS` before the command succeeds.

- [ ] **Step 7: Replace the running monitor safely**

Resolve the exact existing monitor PID and executable path:

```bash
pgrep -fl "target/(debug|release)/monitor"
ps -axo pid=,ppid=,etime=,state=,command= | rg "target/(debug|release)/monitor"
```

Gracefully terminate only the resolved old monitor, confirm it exited, then start exactly one `target/release/monitor` using the repository’s existing launch/supervisor mechanism. Do not use a broad process pattern for termination.

- [ ] **Step 8: Observe two completed T0 cycles**

For at least two 30-second cycles during a trading session, verify:

```text
no tokio blocking shutdown panic
no Eastmoney/Sina line inside [做T-持仓][BR-153]
no symmetric percentage high-sell/low-buy text
each evaluated record has Magic TDX source_at and batch prefix
isolated tickets log an explicit reason_code
advice appears only when trigger or near-zone waiting rule is satisfied
```

If no real trigger occurs, retain the status **In Progress** for production-push evidence. Do not generate a fake trigger or test push.

- [ ] **Step 9: Final commit and rollback record**

Run:

```bash
git status --short
git diff --check
git log --oneline --decorate -12
```

Expected: only unrelated pre-existing untracked files remain.

Rollback:

Use `git log --oneline --decorate -12` to identify the exact contiguous T0 implementation commits, revert those exact SHAs without reverting unrelated work, then run `cargo build --release --bin monitor`.

Restart the exact prior release using the same supervised process procedure. Preserve all event and delivery audit files.

## Self-Review

- Spec coverage: source exclusivity, freshness, daily/5-minute/average/book validation, ATR/MA/pace metrics, trend vetoes, pivot/projection zones, 1.5% spread, trigger/invalidation rules, near-zone waiting, equal reverse-T legs, user-position disclaimer, explicit rejection, governed delivery, coverage, and live evidence all map to Tasks 1–8.
- Placeholder scan: no deferred-value markers, unspecified error-handling step, or cross-task shortcut instruction remains.
- Type consistency: `MagicTdxT0Evidence` flows from Task 1 to `evaluate_structured` in Task 2; `T0StructuredPlan` flows from Task 2 to `T0AdviceParams::from` in Tasks 3–4; `T0PlanDecision` variants and reason-code fields are consistent in Tasks 2, 4, and 5.
- Safety consistency: the plan creates observations only, never broker availability, orders, mock production data, or stale/default fallbacks.
