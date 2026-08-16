//! Evidence-backed reverse-T observation plans.
//!
//! The evaluator is deterministic and side-effect free. It consumes one
//! strictly validated Magic TDX evidence record and a user-confirmed total
//! position. It does not claim broker-verified sellable shares and never
//! creates orders.
//!
//! Business rules: BR-151, BR-153.

use crate::data_gateway::{MagicTdxT0DailyBar, MagicTdxT0Evidence, MagicTdxT0FiveMinuteBar};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use crate::magic_compat::InstrumentId;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const MIN_GROSS_SPREAD_PCT: f64 = 1.5;
const MIN_LEVEL_DISTANCE_PCT: f64 = 0.003;
const SELL_VOLUME_RATIO_TRIGGER: f64 = 1.2;
const BUYBACK_VOLUME_RATIO_TRIGGER: f64 = 0.8;
const BOOK_RATIO_TRIGGER: f64 = 1.2;

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, PartialEq)]
pub struct T0Position {
    pub code: String,
    pub name: String,
    pub total_quantity: u64,
    pub cost_price: f64,
    pub snapshot: T0PositionSnapshotBindingV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct T0PositionSnapshotBindingV1 {
    pub snapshot_id: String,
    pub evidence_sha256: String,
    pub effective_at: DateTime<FixedOffset>,
    pub confirmed_at: DateTime<FixedOffset>,
}

impl T0PositionSnapshotBindingV1 {
    pub fn new(
        snapshot_id: impl Into<String>,
        evidence_sha256: impl Into<String>,
        effective_at: DateTime<FixedOffset>,
        confirmed_at: DateTime<FixedOffset>,
    ) -> Result<Self, String> {
        let snapshot_id = snapshot_id.into();
        let evidence_sha256 = evidence_sha256.into();
        if snapshot_id.trim().is_empty() {
            return Err("T0 position snapshot_id must be non-empty".to_owned());
        }
        if !is_sha256_hex(&evidence_sha256) {
            return Err("T0 position evidence_sha256 must be lowercase SHA-256 hex".to_owned());
        }
        if confirmed_at < effective_at {
            return Err(format!(
                "T0 position confirmed_at precedes effective_at effective_at={effective_at} confirmed_at={confirmed_at}"
            ));
        }
        Ok(Self {
            snapshot_id,
            evidence_sha256,
            effective_at,
            confirmed_at,
        })
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
pub enum TrendStatus {
    MainUpCore,
    MainUp,
    Range,
    Weak,
    Fade,
}

impl TrendStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::MainUpCore => "主升核心",
            Self::MainUp => "主升",
            Self::Range => "震荡",
            Self::Weak => "走弱",
            Self::Fade => "退潮",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
pub enum ZoneSource {
    DailyPivot,
    IntradayPivot,
    IntradayAverage,
    AtrProjection,
}

impl ZoneSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::DailyPivot => "日线确认拐点",
            Self::IntradayPivot => "5分钟确认拐点",
            Self::IntradayAverage => "TDX分时均价",
            Self::AtrProjection => "ATR投影",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PriceZone {
    pub low: f64,
    pub high: f64,
    pub source: ZoneSource,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
pub enum T0PlanState {
    WaitingSell,
    SellTriggered,
    WaitingBuyback,
    BuybackTriggered,
    SellInvalidated,
    BuybackInvalidated,
}

impl T0PlanState {
    pub fn label(self) -> &'static str {
        match self {
            Self::WaitingSell => "等待进入卖出区",
            Self::SellTriggered => "卖出观察触发",
            Self::WaitingBuyback => "等待进入接回区",
            Self::BuybackTriggered => "接回观察触发",
            Self::SellInvalidated => "卖出计划失效",
            Self::BuybackInvalidated => "接回计划失效",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct T0Metrics {
    pub trend: TrendStatus,
    pub pace_ratio: f64,
    pub last_bar_volume_ratio: f64,
    pub intraday_average_price: f64,
    pub atr14: f64,
    pub ask_bid_ratio: f64,
    pub bid_ask_ratio: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct T0StructuredPlan {
    pub code: String,
    pub name: String,
    pub source_at: chrono::DateTime<chrono::Utc>,
    pub batch_id: String,
    pub current_price: f64,
    pub cost_price: f64,
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct T0PlanDecisionBindingV1 {
    schema: &'static str,
    instrument: InstrumentId,
    position_snapshot: T0PositionSnapshotBindingV1,
    evidence_requested_at: DateTime<Utc>,
    evidence_source_at: DateTime<Utc>,
    evidence_observed_at: DateTime<Utc>,
    evidence_batch_id: String,
    plan: T0StructuredPlan,
}

impl T0PlanDecisionBindingV1 {
    pub fn new(
        position: &T0Position,
        evidence: &MagicTdxT0Evidence,
        plan: &T0StructuredPlan,
    ) -> Result<Self, String> {
        if evidence.code != evidence.instrument.code()
            || evidence.code != position.code
            || plan.code != position.code
        {
            return Err(format!(
                "T0 decision identity mismatch position={} evidence={} instrument={} plan={}",
                position.code,
                evidence.code,
                evidence.instrument.code(),
                plan.code
            ));
        }
        if !is_sha256_hex(&evidence.batch_id) || plan.batch_id != evidence.batch_id {
            return Err("T0 decision requires one canonical complete evidence batch ID".to_owned());
        }
        if evidence.observed_at < evidence.requested_at || evidence.source_at > evidence.observed_at
        {
            return Err(format!(
                "T0 decision evidence timestamps invalid requested_at={} source_at={} observed_at={}",
                evidence.requested_at, evidence.source_at, evidence.observed_at
            ));
        }
        if plan.source_at != evidence.source_at
            || plan.name != position.name
            || plan.total_quantity != position.total_quantity
            || plan.cost_price.to_bits() != position.cost_price.to_bits()
        {
            return Err("T0 decision plan does not preserve position/evidence inputs".to_owned());
        }
        Ok(Self {
            schema: "t0_plan_decision_binding_v1",
            instrument: evidence.instrument.clone(),
            position_snapshot: position.snapshot.clone(),
            evidence_requested_at: evidence.requested_at,
            evidence_source_at: evidence.source_at,
            evidence_observed_at: evidence.observed_at,
            evidence_batch_id: evidence.batch_id.clone(),
            plan: plan.clone(),
        })
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|error| format!("serialize T0 decision binding: {error}"))
    }

    pub fn decision_id(&self) -> Result<String, String> {
        let canonical = self.canonical_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(b"stock-analysis:t0-plan-decision:v1\0");
        hasher.update(canonical);
        Ok(hex::encode(hasher.finalize()))
    }

    pub fn delivery_subject_hash(&self) -> Result<String, String> {
        let instrument = serde_json::to_vec(&self.instrument)
            .map_err(|error| format!("serialize T0 delivery subject instrument: {error}"))?;
        let mut hasher = Sha256::new();
        hasher.update(b"stock-analysis:t0-advice-subject:v1\0");
        hasher.update(instrument);
        Ok(hex::encode(hasher.finalize()))
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn position_snapshot(&self) -> &T0PositionSnapshotBindingV1 {
        &self.position_snapshot
    }

    pub fn evidence_batch_id(&self) -> &str {
        &self.evidence_batch_id
    }

    pub fn evidence_observed_at(&self) -> DateTime<Utc> {
        self.evidence_observed_at
    }
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

fn no_plan(
    position: &T0Position,
    reason_code: &'static str,
    reason: impl Into<String>,
) -> T0NoPlan {
    T0NoPlan {
        code: position.code.clone(),
        name: position.name.clone(),
        reason_code,
        reason: reason.into(),
    }
}

fn observation_leg(total: u64) -> Option<u64> {
    let shares = (total / 3 / 100) * 100;
    (shares >= 100).then_some(shares)
}

fn simple_ma(values: &[f64], period: usize) -> Option<f64> {
    let tail = values.get(values.len().checked_sub(period)?..)?;
    Some(tail.iter().sum::<f64>() / period as f64)
}

fn atr14(bars: &[MagicTdxT0DailyBar]) -> Option<f64> {
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
    (!ranges.is_empty()).then(|| ranges.iter().sum::<f64>() / ranges.len() as f64)
}

fn grouped_five_minute(
    bars: &[MagicTdxT0FiveMinuteBar],
) -> BTreeMap<NaiveDate, Vec<&MagicTdxT0FiveMinuteBar>> {
    let mut grouped = BTreeMap::new();
    for bar in bars {
        grouped
            .entry(bar.at.date())
            .or_insert_with(Vec::new)
            .push(bar);
    }
    grouped
}

fn volume_ratios(evidence: &MagicTdxT0Evidence) -> Option<(f64, f64)> {
    let today = evidence
        .observed_at
        .with_timezone(&chrono::Local)
        .date_naive();
    let grouped = grouped_five_minute(&evidence.completed_five_minute);
    let today_bars = grouped.get(&today)?;
    let slots = today_bars.len();
    if slots == 0 {
        return None;
    }
    let mut historical = grouped
        .iter()
        .filter(|(date, bars)| **date < today && bars.len() >= slots)
        .collect::<Vec<_>>();
    historical.sort_by_key(|(date, _)| **date);
    let historical = historical.into_iter().rev().take(5).collect::<Vec<_>>();
    if historical.len() < 3 {
        return None;
    }

    let today_volume = today_bars.iter().map(|bar| bar.volume).sum::<f64>();
    let historical_cumulative = historical
        .iter()
        .map(|(_, bars)| bars.iter().take(slots).map(|bar| bar.volume).sum::<f64>())
        .collect::<Vec<_>>();
    let average_cumulative =
        historical_cumulative.iter().sum::<f64>() / historical_cumulative.len() as f64;
    let average_last = historical
        .iter()
        .map(|(_, bars)| bars[slots - 1].volume)
        .sum::<f64>()
        / historical.len() as f64;
    if average_cumulative <= 0.0 || average_last <= 0.0 {
        return None;
    }
    Some((
        today_volume / average_cumulative,
        today_bars[slots - 1].volume / average_last,
    ))
}

fn classify_trend(
    price: f64,
    closes: &[f64],
    pace_ratio: f64,
) -> Option<(TrendStatus, f64, f64, f64)> {
    let ma5 = simple_ma(closes, 5)?;
    let ma10 = simple_ma(closes, 10)?;
    let ma20 = simple_ma(closes, 20)?;
    let base = *closes.get(closes.len().checked_sub(6)?)?;
    let latest = *closes.last()?;
    let five_day_return_pct = (latest / base - 1.0) * 100.0;
    let trend = if price > ma5
        && ma5 > ma10
        && ma10 > ma20
        && five_day_return_pct >= 8.0
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
    Some((trend, ma5, ma10, ma20))
}

fn confirmed_daily_pivots(bars: &[MagicTdxT0DailyBar]) -> Vec<(f64, ZoneSource)> {
    bars.iter()
        .rev()
        .take(20)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .windows(5)
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

fn confirmed_intraday_pivots(
    bars: &[MagicTdxT0FiveMinuteBar],
    today: NaiveDate,
) -> Vec<(f64, ZoneSource)> {
    bars.iter()
        .filter(|bar| bar.at.date() == today)
        .cloned()
        .collect::<Vec<_>>()
        .windows(3)
        .flat_map(|window| {
            let middle = &window[1];
            let high = (middle.high > window[0].high && middle.high > window[2].high)
                .then_some((middle.high, ZoneSource::IntradayPivot));
            let low = (middle.low < window[0].low && middle.low < window[2].low)
                .then_some((middle.low, ZoneSource::IntradayPivot));
            high.into_iter().chain(low)
        })
        .collect()
}

fn zone(center: f64, source: ZoneSource, price: f64, atr: f64) -> PriceZone {
    let half_width = (atr * 0.1).clamp(price * 0.0015, price * 0.0035);
    PriceZone {
        low: center - half_width,
        high: center + half_width,
        source,
    }
}

fn nearest_level(
    candidates: &[(f64, ZoneSource)],
    price: f64,
    above: bool,
) -> Option<(f64, ZoneSource)> {
    let threshold = price * MIN_LEVEL_DISTANCE_PCT;
    candidates
        .iter()
        .copied()
        .filter(|(level, _)| {
            if above {
                *level >= price + threshold
            } else {
                *level <= price - threshold
            }
        })
        .min_by(|left, right| (left.0 - price).abs().total_cmp(&(right.0 - price).abs()))
}

fn distance_to_zone(price: f64, zone: &PriceZone) -> f64 {
    if price < zone.low {
        zone.low - price
    } else if price > zone.high {
        price - zone.high
    } else {
        0.0
    }
}

pub fn evaluate_structured(position: &T0Position, evidence: &MagicTdxT0Evidence) -> T0PlanDecision {
    if position.code != evidence.code {
        return T0PlanDecision::Rejected(no_plan(
            position,
            "identity_mismatch",
            format!(
                "position_code={} evidence_code={}",
                position.code, evidence.code
            ),
        ));
    }
    if !position.cost_price.is_finite() || position.cost_price <= 0.0 {
        return T0PlanDecision::Rejected(no_plan(
            position,
            "position_cost_invalid",
            format!("cost_price={}", position.cost_price),
        ));
    }
    let Some(leg) = observation_leg(position.total_quantity) else {
        return T0PlanDecision::Forbidden(no_plan(
            position,
            "leg_below_board_lot",
            format!(
                "总持仓{}股的三分之一向下取整后不足100股",
                position.total_quantity
            ),
        ));
    };
    let Some(atr) = atr14(&evidence.settled_daily) else {
        return T0PlanDecision::Rejected(no_plan(
            position,
            "atr_unavailable",
            format!("settled_daily_count={}", evidence.settled_daily.len()),
        ));
    };
    if !atr.is_finite() || atr <= 0.0 {
        return T0PlanDecision::Rejected(no_plan(position, "atr_invalid", format!("atr14={atr}")));
    }
    let Some((pace_ratio, last_bar_volume_ratio)) = volume_ratios(evidence) else {
        return T0PlanDecision::Rejected(no_plan(
            position,
            "volume_comparison_unavailable",
            "缺少至少3个同时间槽历史交易日",
        ));
    };
    let closes = evidence
        .settled_daily
        .iter()
        .map(|bar| bar.close)
        .collect::<Vec<_>>();
    let Some((trend, _, _, _)) = classify_trend(evidence.quote.price, &closes, pace_ratio) else {
        return T0PlanDecision::Rejected(no_plan(
            position,
            "trend_unavailable",
            format!("settled_daily_count={}", closes.len()),
        ));
    };
    if trend == TrendStatus::MainUpCore {
        return T0PlanDecision::Forbidden(no_plan(
            position,
            "main_up_core",
            "主升核心阶段禁止反T，避免卖飞",
        ));
    }
    if trend == TrendStatus::Fade {
        return T0PlanDecision::Forbidden(no_plan(position, "fade", "退潮趋势禁止反T"));
    }

    let bid_volume = evidence
        .quote
        .bids
        .iter()
        .map(|level| level.volume)
        .sum::<f64>();
    let ask_volume = evidence
        .quote
        .asks
        .iter()
        .map(|level| level.volume)
        .sum::<f64>();
    if bid_volume <= 0.0 || ask_volume <= 0.0 {
        return T0PlanDecision::Rejected(no_plan(
            position,
            "order_book_ratio_unavailable",
            format!("bid_volume={bid_volume} ask_volume={ask_volume}"),
        ));
    }
    let ask_bid_ratio = ask_volume / bid_volume;
    let bid_ask_ratio = bid_volume / ask_volume;

    let today = evidence
        .observed_at
        .with_timezone(&chrono::Local)
        .date_naive();
    let mut candidates = confirmed_daily_pivots(&evidence.settled_daily);
    candidates.extend(confirmed_intraday_pivots(
        &evidence.completed_five_minute,
        today,
    ));
    candidates.push((evidence.intraday_average_price, ZoneSource::IntradayAverage));
    let price = evidence.quote.price;
    let sell_center = nearest_level(&candidates, price, true)
        .unwrap_or((price + atr * 0.5, ZoneSource::AtrProjection));
    let buy_center = nearest_level(&candidates, price, false)
        .unwrap_or((price - atr * 0.5, ZoneSource::AtrProjection));
    let sell_zone = zone(sell_center.0, sell_center.1, price, atr);
    let buy_zone = zone(buy_center.0, buy_center.1, price, atr);
    if buy_zone.low <= 0.0 {
        return T0PlanDecision::Rejected(no_plan(
            position,
            "buy_zone_invalid",
            format!("low={}", buy_zone.low),
        ));
    }
    if sell_zone.low <= buy_zone.high {
        return T0PlanDecision::Forbidden(no_plan(
            position,
            "zones_overlap",
            format!(
                "sell_low={:.3} buy_high={:.3}",
                sell_zone.low, buy_zone.high
            ),
        ));
    }
    let gross_spread_pct = (sell_zone.low / buy_zone.high - 1.0) * 100.0;
    if gross_spread_pct < MIN_GROSS_SPREAD_PCT {
        return T0PlanDecision::Forbidden(no_plan(
            position,
            "spread_insufficient",
            format!("gross_spread_pct={gross_spread_pct:.3} required={MIN_GROSS_SPREAD_PCT:.3}"),
        ));
    }

    let today_bars = evidence
        .completed_five_minute
        .iter()
        .filter(|bar| bar.at.date() == today)
        .collect::<Vec<_>>();
    let Some(latest) = today_bars.last() else {
        return T0PlanDecision::Rejected(no_plan(
            position,
            "today_bar_missing",
            "没有当日已完成5分钟K线",
        ));
    };
    let last_two_above = today_bars.len() >= 2
        && today_bars[today_bars.len() - 2..]
            .iter()
            .all(|bar| bar.close > sell_zone.high);
    let sell_triggered = sell_zone.low <= price
        && price <= sell_zone.high
        && last_bar_volume_ratio >= SELL_VOLUME_RATIO_TRIGGER
        && ask_bid_ratio >= BOOK_RATIO_TRIGGER;
    let buyback_triggered = buy_zone.low <= price
        && price <= buy_zone.high
        && last_bar_volume_ratio <= BUYBACK_VOLUME_RATIO_TRIGGER
        && latest.close > latest.open
        && bid_ask_ratio >= BOOK_RATIO_TRIGGER;
    let state = if last_two_above {
        T0PlanState::SellInvalidated
    } else if latest.close < buy_zone.low {
        T0PlanState::BuybackInvalidated
    } else if sell_triggered {
        T0PlanState::SellTriggered
    } else if buyback_triggered {
        T0PlanState::BuybackTriggered
    } else if price > buy_zone.high {
        T0PlanState::WaitingSell
    } else {
        T0PlanState::WaitingBuyback
    };

    if matches!(
        state,
        T0PlanState::WaitingSell | T0PlanState::WaitingBuyback
    ) && distance_to_zone(price, &sell_zone).min(distance_to_zone(price, &buy_zone)) > atr * 0.5
    {
        return T0PlanDecision::Rejected(no_plan(
            position,
            "outside_observation_window",
            format!(
                "price={price:.3} nearest_distance={:.3} max_distance={:.3}",
                distance_to_zone(price, &sell_zone).min(distance_to_zone(price, &buy_zone)),
                atr * 0.5
            ),
        ));
    }

    let trigger_text = format!(
        "卖出需现价进入区间、末根5分钟量比≥{SELL_VOLUME_RATIO_TRIGGER:.1}x且五档卖/买≥{BOOK_RATIO_TRIGGER:.1}x；接回需进入区间、量比≤{BUYBACK_VOLUME_RATIO_TRIGGER:.1}x、5分钟收阳且五档买/卖≥{BOOK_RATIO_TRIGGER:.1}x"
    );
    let invalidation_text = format!(
        "连续两根已完成5分钟收盘>{:.2}取消卖出；最新已完成5分钟收盘<{:.2}取消接回",
        sell_zone.high, buy_zone.low
    );
    T0PlanDecision::Advice(T0StructuredPlan {
        code: position.code.clone(),
        name: position.name.clone(),
        source_at: evidence.source_at,
        batch_id: evidence.batch_id.clone(),
        current_price: price,
        cost_price: position.cost_price,
        total_quantity: position.total_quantity,
        sell_quantity: leg,
        buyback_quantity: leg,
        sell_zone,
        buy_zone,
        gross_spread_pct,
        metrics: T0Metrics {
            trend,
            pace_ratio,
            last_bar_volume_ratio,
            intraday_average_price: evidence.intraday_average_price,
            atr14: atr,
            ask_bid_ratio,
            bid_ask_ratio,
        },
        state,
        trigger_text,
        invalidation_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::{MagicTdxT0Quote, T0BookLevel};
    use chrono::{Duration, Local, TimeZone, Utc};

    fn position(quantity: u64) -> T0Position {
        T0Position {
            code: "TEST_CODE_600396".to_string(),
            name: "测试持仓".to_string(),
            total_quantity: quantity,
            cost_price: 14.20,
            snapshot: T0PositionSnapshotBindingV1::new(
                "TEST_CODE_POSITION_SNAPSHOT_20260723",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                chrono::DateTime::parse_from_rfc3339("2026-07-23T09:30:00+08:00").unwrap(),
                chrono::DateTime::parse_from_rfc3339("2026-07-23T09:31:00+08:00").unwrap(),
            )
            .unwrap(),
        }
    }

    fn daily_bars() -> Vec<MagicTdxT0DailyBar> {
        let start = NaiveDate::from_ymd_opt(2026, 6, 22).unwrap();
        (0..24)
            .map(|index| {
                let close = 15.0 + (index % 4) as f64 * 0.12;
                MagicTdxT0DailyBar {
                    date: start + Duration::days(index as i64),
                    open: close - 0.08,
                    high: close + 0.60,
                    low: close - 0.60,
                    close,
                    volume: 1_000_000.0,
                    amount: 15_000_000.0,
                }
            })
            .collect()
    }

    fn five_minute_bars(observed_at: chrono::DateTime<Utc>) -> Vec<MagicTdxT0FiveMinuteBar> {
        let today = observed_at.with_timezone(&Local).date_naive();
        let mut out = Vec::new();
        for day_offset in [5_i64, 4, 3, 2, 1] {
            let date = today - Duration::days(day_offset);
            for slot in 0..6 {
                let at =
                    date.and_hms_opt(9, 35, 0).expect("fixture time") + Duration::minutes(slot * 5);
                out.push(MagicTdxT0FiveMinuteBar {
                    at,
                    open: 15.70,
                    high: 15.82,
                    low: 15.58,
                    close: 15.72,
                    volume: 1_000.0,
                    amount: 15_700.0,
                });
            }
        }
        for slot in 0..6 {
            let at =
                today.and_hms_opt(9, 35, 0).expect("fixture time") + Duration::minutes(slot * 5);
            out.push(MagicTdxT0FiveMinuteBar {
                at,
                open: 15.70,
                high: 15.82,
                low: 15.58,
                close: 15.72,
                volume: 1_100.0,
                amount: 17_270.0,
            });
        }
        out
    }

    fn evidence() -> MagicTdxT0Evidence {
        let observed_at = Local
            .with_ymd_and_hms(2026, 7, 23, 10, 1, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let bids = std::array::from_fn(|index| T0BookLevel {
            price: 15.68 - index as f64 * 0.01,
            volume: 1_000.0,
        });
        let asks = std::array::from_fn(|index| T0BookLevel {
            price: 15.69 + index as f64 * 0.01,
            volume: 1_100.0,
        });
        MagicTdxT0Evidence {
            instrument: crate::magic_compat::InstrumentId::new(
                crate::magic_compat::Exchange::Shanghai,
                "TEST_CODE_600396",
                crate::magic_compat::AssetClass::Equity,
            )
            .unwrap(),
            code: "TEST_CODE_600396".to_string(),
            requested_at: observed_at - Duration::seconds(2),
            source_at: observed_at - Duration::seconds(1),
            observed_at,
            batch_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
            quote: MagicTdxT0Quote {
                price: 15.68,
                last_close: 15.20,
                open: 15.30,
                high: 15.90,
                low: 15.40,
                volume: 6_600.0,
                amount: 103_488.0,
                bids,
                asks,
            },
            settled_daily: daily_bars(),
            completed_five_minute: five_minute_bars(observed_at),
            intraday_average_price: 15.50,
        }
    }

    #[test]
    fn reverse_t_legs_use_same_one_third_board_lot() {
        let decision = evaluate_structured(&position(500), &evidence());

        let T0PlanDecision::Advice(plan) = decision else {
            panic!("expected structured plan: {decision:?}");
        };
        assert_eq!(plan.sell_quantity, 100);
        assert_eq!(plan.buyback_quantity, 100);
    }

    #[test]
    fn canonical_t0_decision_binding_is_stable_across_delivery_retries() {
        let position = position(500);
        let evidence = evidence();
        let T0PlanDecision::Advice(plan) = evaluate_structured(&position, &evidence) else {
            panic!("fixture must produce advice");
        };

        let first = T0PlanDecisionBindingV1::new(&position, &evidence, &plan).unwrap();
        let retry = T0PlanDecisionBindingV1::new(&position, &evidence, &plan).unwrap();

        assert_eq!(
            first.canonical_bytes().unwrap(),
            retry.canonical_bytes().unwrap()
        );
        assert_eq!(first.decision_id().unwrap(), retry.decision_id().unwrap());
        assert_eq!(first.decision_id().unwrap().len(), 64);
        assert_eq!(
            first.position_snapshot().snapshot_id,
            "TEST_CODE_POSITION_SNAPSHOT_20260723"
        );
        assert_eq!(
            first.position_snapshot().evidence_sha256,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(
            first.position_snapshot().effective_at,
            chrono::DateTime::parse_from_rfc3339("2026-07-23T09:30:00+08:00").unwrap()
        );
        assert_eq!(
            first.position_snapshot().confirmed_at,
            chrono::DateTime::parse_from_rfc3339("2026-07-23T09:31:00+08:00").unwrap()
        );
        assert_eq!(first.evidence_batch_id(), evidence.batch_id);
        assert_eq!(first.instrument(), &evidence.instrument);
    }

    #[test]
    fn canonical_t0_decision_binding_changes_when_real_snapshot_evidence_changes() {
        let original_position = position(500);
        let evidence = evidence();
        let T0PlanDecision::Advice(original_plan) =
            evaluate_structured(&original_position, &evidence)
        else {
            panic!("fixture must produce advice");
        };
        let original =
            T0PlanDecisionBindingV1::new(&original_position, &evidence, &original_plan).unwrap();

        let mut changed_position = original_position.clone();
        changed_position.snapshot = T0PositionSnapshotBindingV1::new(
            "TEST_CODE_POSITION_SNAPSHOT_20260723_B",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            chrono::DateTime::parse_from_rfc3339("2026-07-23T09:30:00+08:00").unwrap(),
            chrono::DateTime::parse_from_rfc3339("2026-07-23T09:32:00+08:00").unwrap(),
        )
        .unwrap();
        let T0PlanDecision::Advice(changed_plan) =
            evaluate_structured(&changed_position, &evidence)
        else {
            panic!("changed fixture must produce advice");
        };
        let changed =
            T0PlanDecisionBindingV1::new(&changed_position, &evidence, &changed_plan).unwrap();

        assert_ne!(
            original.canonical_bytes().unwrap(),
            changed.canonical_bytes().unwrap()
        );
        assert_ne!(
            original.decision_id().unwrap(),
            changed.decision_id().unwrap()
        );
    }

    #[test]
    fn incomplete_snapshot_or_noncanonical_batch_fails_closed() {
        let effective_at =
            chrono::DateTime::parse_from_rfc3339("2026-07-23T09:30:00+08:00").unwrap();
        let confirmed_at =
            chrono::DateTime::parse_from_rfc3339("2026-07-23T09:31:00+08:00").unwrap();
        assert!(T0PositionSnapshotBindingV1::new(
            "",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            effective_at,
            confirmed_at,
        )
        .is_err());
        assert!(T0PositionSnapshotBindingV1::new(
            "TEST_CODE_SNAPSHOT",
            "missing",
            effective_at,
            confirmed_at,
        )
        .is_err());

        let position = position(500);
        let mut evidence = evidence();
        let T0PlanDecision::Advice(mut plan) = evaluate_structured(&position, &evidence) else {
            panic!("fixture must produce advice");
        };
        evidence.batch_id = "not-a-complete-batch".to_owned();
        plan.batch_id.clone_from(&evidence.batch_id);
        assert!(T0PlanDecisionBindingV1::new(&position, &evidence, &plan).is_err());
    }

    #[test]
    fn position_below_one_observation_lot_is_forbidden() {
        let decision = evaluate_structured(&position(299), &evidence());

        assert!(matches!(
            decision,
            T0PlanDecision::Forbidden(ref value)
                if value.reason_code == "leg_below_board_lot"
        ));
    }

    #[test]
    fn main_up_core_is_forbidden() {
        let mut value = evidence();
        for (index, bar) in value.settled_daily.iter_mut().enumerate() {
            let close = 10.0 + index as f64 * 0.35;
            bar.open = close - 0.10;
            bar.high = close + 0.50;
            bar.low = close - 0.50;
            bar.close = close;
        }
        value.quote.price = 19.0;
        for bar in value
            .completed_five_minute
            .iter_mut()
            .filter(|bar| bar.at.date() == value.observed_at.with_timezone(&Local).date_naive())
        {
            bar.volume = 2_000.0;
        }

        let decision = evaluate_structured(&position(500), &value);

        assert!(matches!(
            decision,
            T0PlanDecision::Forbidden(ref value)
                if value.reason_code == "main_up_core"
        ));
    }

    fn reason_code(decision: &T0PlanDecision) -> &'static str {
        match decision {
            T0PlanDecision::Advice(_) => "advice",
            T0PlanDecision::Forbidden(value) | T0PlanDecision::Rejected(value) => value.reason_code,
        }
    }

    fn flatten_prices(value: &mut MagicTdxT0Evidence, close: f64, half_range: f64) {
        value.quote.price = close;
        value.intraday_average_price = close;
        for bar in &mut value.settled_daily {
            bar.open = close;
            bar.high = close + half_range;
            bar.low = close - half_range;
            bar.close = close;
        }
        for bar in &mut value.completed_five_minute {
            bar.open = close;
            bar.high = close + half_range;
            bar.low = close - half_range;
            bar.close = close;
        }
    }

    #[test]
    fn mismatched_identity_and_invalid_cost_are_rejected() {
        let mut mismatched = evidence();
        mismatched.code = "TEST_CODE_600397".to_string();
        assert_eq!(
            reason_code(&evaluate_structured(&position(500), &mismatched)),
            "identity_mismatch"
        );

        for cost_price in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut invalid = position(500);
            invalid.cost_price = cost_price;
            assert_eq!(
                reason_code(&evaluate_structured(&invalid, &evidence())),
                "position_cost_invalid"
            );
        }
    }

    #[test]
    fn daily_history_failures_remain_distinct() {
        let mut missing_atr = evidence();
        missing_atr.settled_daily.truncate(14);
        assert_eq!(
            reason_code(&evaluate_structured(&position(500), &missing_atr)),
            "atr_unavailable"
        );

        let mut missing_trend = evidence();
        missing_trend.settled_daily.truncate(15);
        assert_eq!(
            reason_code(&evaluate_structured(&position(500), &missing_trend)),
            "trend_unavailable"
        );

        let mut invalid_atr = evidence();
        flatten_prices(&mut invalid_atr, 15.0, 0.0);
        assert_eq!(
            reason_code(&evaluate_structured(&position(500), &invalid_atr)),
            "atr_invalid"
        );
    }

    #[test]
    fn volume_comparison_requires_positive_same_slot_history() {
        let mut value = evidence();
        let today = value.observed_at.with_timezone(&Local).date_naive();
        for bar in value
            .completed_five_minute
            .iter_mut()
            .filter(|bar| bar.at.date() < today)
        {
            bar.volume = 0.0;
        }
        assert_eq!(
            reason_code(&evaluate_structured(&position(500), &value)),
            "volume_comparison_unavailable"
        );
    }

    #[test]
    fn declining_trend_and_empty_book_fail_closed() {
        let mut declining = evidence();
        for (index, bar) in declining.settled_daily.iter_mut().enumerate() {
            let close = 20.0 - index as f64 * 0.2;
            bar.open = close + 0.1;
            bar.high = close + 0.5;
            bar.low = close - 0.5;
            bar.close = close;
        }
        declining.quote.price = 15.0;
        assert_eq!(
            reason_code(&evaluate_structured(&position(500), &declining)),
            "fade"
        );

        let mut empty_book = evidence();
        for level in &mut empty_book.quote.bids {
            level.volume = 0.0;
        }
        assert_eq!(
            reason_code(&evaluate_structured(&position(500), &empty_book)),
            "order_book_ratio_unavailable"
        );
    }

    #[test]
    fn narrow_valid_ranges_are_rejected_before_emitting_a_plan() {
        let mut overlap = evidence();
        flatten_prices(&mut overlap, 15.0, 0.005);
        assert_eq!(
            reason_code(&evaluate_structured(&position(500), &overlap)),
            "zones_overlap"
        );

        let mut insufficient = evidence();
        flatten_prices(&mut insufficient, 15.0, 0.05);
        assert_eq!(
            reason_code(&evaluate_structured(&position(500), &insufficient)),
            "spread_insufficient"
        );
    }

    #[test]
    fn labels_and_distance_expose_all_public_plan_states() {
        assert_eq!(TrendStatus::MainUp.label(), "主升");
        assert_eq!(TrendStatus::Range.label(), "震荡");
        assert_eq!(TrendStatus::Weak.label(), "走弱");
        assert_eq!(TrendStatus::Fade.label(), "退潮");
        assert_eq!(ZoneSource::DailyPivot.label(), "日线确认拐点");
        assert_eq!(ZoneSource::IntradayPivot.label(), "5分钟确认拐点");
        assert_eq!(ZoneSource::IntradayAverage.label(), "TDX分时均价");
        assert_eq!(ZoneSource::AtrProjection.label(), "ATR投影");
        assert_eq!(T0PlanState::WaitingSell.label(), "等待进入卖出区");
        assert_eq!(T0PlanState::SellTriggered.label(), "卖出观察触发");
        assert_eq!(T0PlanState::WaitingBuyback.label(), "等待进入接回区");
        assert_eq!(T0PlanState::BuybackTriggered.label(), "接回观察触发");
        assert_eq!(T0PlanState::SellInvalidated.label(), "卖出计划失效");
        assert_eq!(T0PlanState::BuybackInvalidated.label(), "接回计划失效");

        let zone = PriceZone {
            low: 10.0,
            high: 11.0,
            source: ZoneSource::DailyPivot,
        };
        assert_eq!(distance_to_zone(9.5, &zone), 0.5);
        assert_eq!(distance_to_zone(10.5, &zone), 0.0);
        assert_eq!(distance_to_zone(11.5, &zone), 0.5);
    }
}
