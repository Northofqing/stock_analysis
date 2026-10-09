//! Frozen real-account SELL preview; public DTOs are observations, never authority.
use super::{at, escaped, hash, Clock, Source};
use crate::calendar::{verified_a_share_trading_day, verified_next_a_share_trading_day};
use crate::pipeline::position_tracker::{evaluate_sell_rules_with_net_return, SellEvaluation};
use crate::strategy::boll_macd::{detect_boll_macd_observations, BollMacdObservation};
use crate::trading::paper_book_v2_budget_v1::{checked, notional};
use crate::trend_analyzer::{StockData, StockTrendAnalyzer};
use chrono::{NaiveDate, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[path = "sell_reminder_producer.rs"]
pub mod producer;

pub const VERSION: &str = "sell-preview/v1-legacy-rule-units";
const SEMANTICS: &str = "逐批独立申报；不跨批合并；买入费用按建议股数比例截断分摊，余数留在剩余批次；卖出费用按该批本次独立申报。净收益分母为不含费买入金额，单位百分点。ATR14=最近14根high-low的元/股均值，但StopLoss按百分比使用；保留遗留单位差异，生产推广待明确解决。MA60不足时沿用MA20；少于35根不能确认Hold。";
const MANUAL: &str = "仅供人工在券商核对后申报，未连接券商、未下单或发送提醒。上交所盘后固定价格接受/撮合15:05–15:30（15:00–15:05尚未开始）；当日15:00停牌排除；限价不得高于确认收盘价；当日有效、时间优先且不保证成交。撤单以券商/交易所确认及受理时段为准。深交所须遵循其来源确认的时段与数量合同。";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub account_ref: String,
    pub ownership: String,
    pub environment: String,
    pub captured_at: Clock,
    pub observed_at: Clock,
    pub complete: bool,
    pub confirmed_empty: bool,
    pub source: Source,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lot {
    pub account_ref: String,
    pub instrument: String,
    pub lot_id: Option<String>,
    pub acquired: Option<NaiveDate>,
    pub sellable_from: Option<NaiveDate>,
    pub total: u32,
    /// Sellable before reservations. Reserved is a subset of this value.
    pub sellable: Option<u32>,
    pub reserved: Option<u32>,
    pub cost_micro_cny: Option<i64>,
    pub allocated_buy_fee_micro_cny: Option<i64>,
    pub sell_fees: Option<SellFees>,
    pub source: Source,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SellFees {
    pub shares: u32,
    pub commission_micro_cny: Option<i64>,
    pub stamp_micro_cny: Option<i64>,
    pub transfer_micro_cny: Option<i64>,
    pub other_micro_cny: Option<i64>,
    pub source: Source,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bar {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Security {
    pub instrument: String,
    /// Exact source-carried board; never inferred from code/name.
    pub board: String,
    pub quantity: QuantityContract,
    pub date: NaiveDate,
    pub close_micro_cny: Option<i64>,
    pub close_finalized: bool,
    pub close_source: Source,
    pub listed: Option<bool>,
    pub suspended_at_close: Option<bool>,
    pub status_source: Source,
    pub bars_source: Source,
    pub adjustment: String,
    /// Latest first, through the exact confirmed close.
    pub bars: Vec<Bar>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantityContract {
    pub minimum: u32,
    pub step: u32,
    pub max_per_order: u32,
    pub whole_remaining_odd_lot: bool,
    pub source: Source,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePack {
    pub schema: String,
    pub account: Option<Account>,
    pub lots: Vec<Lot>,
    pub securities: Vec<Security>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Candidate,
    NoSuggestion,
    Unavailable,
    Expired,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Row {
    instrument: String,
    account_ref: String,
    lot_id: Option<String>,
    acquired: Option<NaiveDate>,
    state: State,
    reason: String,
    observed_total_shares: u32,
    eligible_sellable: Option<u32>,
    suggested_shares: u32,
    observed_close_micro_cny: Option<i64>,
    reference_close_micro_cny: Option<i64>,
    net_scenario_pct: Option<f64>,
    covered_buy_fee_micro_cny: Option<i64>,
    sell_fees: Option<SellFees>,
    missing: Vec<String>,
    indicator_scope: String,
    evidence_hashes: Vec<String>,
    evidence: Vec<Source>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    schema: String,
    state: State,
    created_at: Clock,
    inspected_at: Clock,
    expires_at: Clock,
    input_hash: String,
    authority: String,
    account_state: String,
    account_snapshot: Option<Account>,
    missing: Vec<String>,
    rows: Vec<Row>,
    semantics: String,
    manual_notes: String,
    execution: String,
    next_open_comparison: String,
}
/// Declared previous-report bytes. No renderer or serializer; conversion always masks.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedPreview {
    schema: String,
    state: State,
    created_at: Clock,
    inspected_at: Clock,
    expires_at: Clock,
    input_hash: String,
    authority: String,
    account_state: String,
    account_snapshot: Option<Account>,
    missing: Vec<String>,
    rows: Vec<Row>,
    semantics: String,
    manual_notes: String,
    execution: String,
    next_open_comparison: String,
}
// Intentionally private, no Deserialize and no production issuer. A future reviewed
// adapter must bind independent real-lot, close, lifecycle and quantity contracts.
struct QualifiedSource;

pub fn preview_observed(pack: &EvidencePack, as_of: Clock) -> Preview {
    evaluate(pack, as_of, None)
}
fn evaluate(pack: &EvidencePack, as_of: Clock, authority: Option<&QualifiedSource>) -> Preview {
    let mut result = Preview {
        schema: VERSION.into(),
        state: State::Unavailable,
        created_at: as_of,
        inspected_at: as_of,
        expires_at: at(as_of.date_naive(), 15, 30),
        input_hash: hash(pack),
        authority: if authority.is_some() {
            "InternalQualifiedSource"
        } else {
            "ObservedOnly/NotAdmitted; ContractNotDelivered"
        }
        .into(),
        account_state: "missing".into(),
        account_snapshot: pack.account.clone(),
        missing: vec![],
        rows: vec![],
        semantics: SEMANTICS.into(),
        manual_notes: MANUAL.into(),
        execution:
            "Unavailable: no real order/fill receipts; coverage and human handling are separate"
                .into(),
        next_open_comparison:
            "Unavailable: requires actual fills/fees and qualified next-session prices".into(),
    };
    if as_of.offset().local_minus_utc() != 28800 {
        result.missing.push("Shanghai +08:00 as_of required".into());
    }
    if pack.schema != "sell-evidence-observed/v1" {
        result.missing.push("unsupported evidence schema".into());
    }
    if verified_a_share_trading_day(as_of.date_naive()) != Ok(true) {
        result
            .missing
            .push("verified trading day unavailable/nontrading".into());
    }
    if as_of.time().hour() < 15 || as_of >= result.expires_at {
        result.missing.push("outside 15:00 <= as_of <15:30".into());
    }
    if authority.is_none() {
        result.missing.push("ContractNotDelivered: independent real-account/lot ownership, final close, lifecycle/status, board/quantity and fee qualification".into());
    }
    if let Some(a) = &pack.account {
        result.account_state = if a.complete && a.confirmed_empty && pack.lots.is_empty() {
            "confirmed_complete_empty_observed"
        } else if !a.complete {
            "incomplete_observed"
        } else {
            "positions_observed"
        }
        .into();
        if a.account_ref.is_empty()
            || a.ownership != "self"
            || a.environment != "real"
            || !a.complete
        {
            result
                .missing
                .push("real account_ref/ownership=self/complete holdings required".into());
        }
        if a.confirmed_empty != pack.lots.is_empty() {
            result
                .missing
                .push("confirmed_empty conflicts with lot inventory".into());
        }
        for (name, time) in [("capture", a.captured_at), ("observation", a.observed_at)] {
            let age = as_of.signed_duration_since(time);
            if time > as_of || age > chrono::Duration::milliseconds(30_000) {
                result
                    .missing
                    .push(format!("account {name} must be nonfuture and <=30000ms"));
            }
        }
        if a.captured_at > a.observed_at {
            result.missing.push("capture after observation".into());
        }
        if let Err(e) = a.source.validate(as_of) {
            result.missing.push(format!("account {e}"));
        }
    } else {
        result
            .missing
            .push("account snapshot absent (not confirmed empty)".into());
    }
    let mut ids = BTreeSet::new();
    if pack
        .lots
        .iter()
        .filter_map(|l| l.lot_id.as_ref())
        .any(|id| !ids.insert(id))
    {
        result.missing.push("duplicate lot identity".into());
    }
    let mut codes = BTreeSet::new();
    if pack.securities.iter().any(|s| !codes.insert(&s.instrument)) {
        result
            .missing
            .push("duplicate/conflicting security evidence".into());
    }
    for lot in &pack.lots {
        let mut row = evaluate_lot(lot, pack, as_of);
        if !result.missing.is_empty() {
            row.state = State::Unavailable;
            mask_unqualified_numbers(&mut row);
            row.missing.extend(result.missing.clone());
            row.reason = "来源/账户/时点不可用".into();
        }
        result.rows.push(row);
    }
    result
        .rows
        .sort_by(|a, b| (&a.instrument, &a.lot_id).cmp(&(&b.instrument, &b.lot_id)));
    result.state = if as_of >= result.expires_at {
        State::Expired
    } else if result.rows.iter().any(|r| r.state == State::Candidate) {
        State::Candidate
    } else if !result.missing.is_empty()
        || result.rows.iter().any(|r| r.state == State::Unavailable)
    {
        State::Unavailable
    } else {
        State::NoSuggestion
    };
    if result.state == State::Expired {
        for r in &mut result.rows {
            r.state = State::Expired;
            r.suggested_shares = 0;
        }
    }
    result
}
/// Observed fields and raw provenance survive, but actionable/covered values do not.
fn mask_unqualified_numbers(row: &mut Row) {
    row.suggested_shares = 0;
    row.eligible_sellable = None;
    row.reference_close_micro_cny = None;
    row.net_scenario_pct = None;
    row.covered_buy_fee_micro_cny = None;
    row.sell_fees = None;
}
fn evaluate_lot(lot: &Lot, pack: &EvidencePack, as_of: Clock) -> Row {
    let mut row = Row {
        instrument: lot.instrument.clone(),
        account_ref: lot.account_ref.clone(),
        lot_id: lot.lot_id.clone(),
        acquired: lot.acquired,
        state: State::Unavailable,
        reason: "证据不完整".into(),
        observed_total_shares: lot.total,
        eligible_sellable: None,
        suggested_shares: 0,
        observed_close_micro_cny: pack
            .securities
            .iter()
            .find(|s| s.instrument == lot.instrument)
            .and_then(|s| s.close_micro_cny),
        reference_close_micro_cny: None,
        net_scenario_pct: None,
        covered_buy_fee_micro_cny: None,
        sell_fees: lot.sell_fees.clone(),
        missing: vec![],
        indicator_scope: String::new(),
        evidence_hashes: vec![lot.source.sha256.clone()],
        evidence: vec![lot.source.clone()],
    };
    let attempt = (|| -> Result<(), String> {
        lot.source.validate(as_of)?;
        if lot.lot_id.as_ref().is_none_or(|s| s.is_empty()) {
            return Err("lot_id missing".into());
        }
        if pack
            .account
            .as_ref()
            .is_none_or(|a| a.account_ref != lot.account_ref)
        {
            return Err("lot account ownership binding mismatch".into());
        }
        let acquired = lot.acquired.ok_or("buy/acquisition date missing")?;
        let sellable_from = lot.sellable_from.ok_or("sellable_from missing")?;
        if acquired > as_of.date_naive() || verified_a_share_trading_day(acquired) != Ok(true) {
            return Err("acquisition future/unverified trading date".into());
        }
        let next = verified_next_a_share_trading_day(acquired)?;
        if sellable_from < next {
            return Err("sellable_from violates verified T+1".into());
        }
        let sellable = lot.sellable.ok_or("sellable quantity missing")?;
        let reserved = lot.reserved.ok_or("reserved quantity missing")?;
        if lot.total == 0 || sellable > lot.total || reserved > sellable {
            return Err("total/sellable/reserved inconsistent".into());
        }
        let cost = lot
            .cost_micro_cny
            .filter(|c| *c > 0)
            .ok_or("positive lot cost basis missing")?;
        let buyfee = lot
            .allocated_buy_fee_micro_cny
            .filter(|f| *f >= 0)
            .ok_or("allocated original buy fee missing")?;
        if acquired == as_of.date_naive() || sellable_from > as_of.date_naive() {
            row.state = State::NoSuggestion;
            row.eligible_sellable = Some(0);
            row.reason = "T+1 锁定，本批不可卖".into();
            return Ok(());
        }
        let available = sellable - reserved;
        row.eligible_sellable = Some(available);
        if available == 0 {
            row.state = State::NoSuggestion;
            row.reason = "无可用可卖股数（含预留）".into();
            return Ok(());
        }
        let s = pack
            .securities
            .iter()
            .find(|s| s.instrument == lot.instrument)
            .ok_or("exact security close/lifecycle/status/quantity evidence missing")?;
        if !matches!(
            s.board.as_str(),
            "SSE.MainA" | "SSE.StarA" | "SZSE.MainA" | "SZSE.ChiNextA"
        ) {
            return Err("unsupported/unqualified A-share board (BSE/ETF excluded)".into());
        }
        for source in [
            &s.close_source,
            &s.status_source,
            &s.quantity.source,
            &s.bars_source,
        ] {
            source.validate(as_of)?;
            row.evidence_hashes.push(source.sha256.clone());
            row.evidence.push(source.clone());
        }
        if s.date != as_of.date_naive()
            || !s.close_finalized
            || s.close_source.known_at < at(s.date, 15, 0)
        {
            return Err("same-session independently finalized exact close missing".into());
        }
        if s.listed != Some(true) {
            return Err("independent Listed lifecycle unavailable".into());
        }
        match s.suspended_at_close {
            Some(true) => {
                row.state = State::NoSuggestion;
                row.reason = "15:00 停牌，排除".into();
                return Ok(());
            }
            None => return Err("independent suspension status missing".into()),
            Some(false) => {}
        }
        if s.status_source.known_at < at(s.date, 15, 0) {
            return Err("status does not cover 15:00 close".into());
        }
        let close = s
            .close_micro_cny
            .filter(|p| *p > 0)
            .ok_or("positive confirmed close missing")?;
        row.reference_close_micro_cny = Some(close);
        let q = &s.quantity;
        if q.minimum == 0
            || q.step == 0
            || q.max_per_order < q.minimum
            || q.max_per_order > 1_000_000
        {
            return Err("invalid source quantity contract".into());
        }
        let capped = available.min(q.max_per_order);
        let whole_instrument = pack
            .lots
            .iter()
            .filter(|l| l.instrument == lot.instrument)
            .count()
            == 1
            && available == lot.total;
        let shares = if capped >= q.minimum {
            q.minimum + (capped - q.minimum) / q.step * q.step
        } else if q.whole_remaining_odd_lot && whole_instrument {
            capped
        } else {
            0
        };
        if shares == 0 {
            row.state = State::NoSuggestion;
            row.reason = "不足独立申报数量；不跨批合并，不拆零股余额".into();
            return Ok(());
        }
        let fees = lot
            .sell_fees
            .as_ref()
            .ok_or("sell commission/stamp/transfer/other fee basis missing")?;
        fees.source.validate(as_of)?;
        if fees.shares != shares {
            return Err("sell fee scenario quantity differs from suggested order".into());
        }
        let mut totalfees = 0i128;
        for (key, fee) in [
            ("commission", fees.commission_micro_cny),
            ("stamp", fees.stamp_micro_cny),
            ("transfer", fees.transfer_micro_cny),
            ("other", fees.other_micro_cny),
        ] {
            totalfees += fee
                .filter(|f| *f >= 0)
                .ok_or_else(|| format!("sell {key} fee missing/invalid"))?
                as i128;
        }
        let sellfee = checked(totalfees).map_err(|e| e.to_string())?;
        let allocated = checked(buyfee as i128 * shares as i128 / lot.total as i128)
            .map_err(|e| e.to_string())?;
        let buy = notional(cost, shares).map_err(|e| e.to_string())?;
        let sell = notional(close, shares).map_err(|e| e.to_string())?;
        let profit = checked(sell as i128 - buy as i128 - allocated as i128 - sellfee as i128)
            .map_err(|e| e.to_string())?;
        let net = profit as f64 / buy as f64 * 100.0;
        if !net.is_finite() {
            return Err("finite net scenario unavailable".into());
        }
        row.net_scenario_pct = Some(net);
        row.covered_buy_fee_micro_cny = Some(allocated);
        row.evidence_hashes.push(fees.source.sha256.clone());
        row.evidence.push(fees.source.clone());
        let indicators = indicators(s, close)?;
        row.indicator_scope = indicators.scope;
        let signal = &indicators.boll;
        let evaluation = SellEvaluation {
            code: &lot.instrument,
            name: &lot.instrument,
            buy_price: cost as f64 / 1e6,
            buy_date: acquired,
            current_price: close as f64 / 1e6,
            quantity: shares as u64,
            ma5: indicators.ma5,
            ma20: indicators.ma20,
            ma60: indicators.ma60,
            atr: indicators.atr,
            boll_macd: signal.as_ref(),
            today: as_of.date_naive(),
        };
        if let Some(reason) = evaluate_sell_rules_with_net_return(&evaluation, net) {
            row.state = State::Candidate;
            row.reason = reason;
            row.suggested_shares = shares;
        } else if !indicators.complete {
            return Err("indicator coverage insufficient for confident Hold".into());
        } else {
            row.state = State::NoSuggestion;
            row.reason = "规则已评估，未触发卖出（Hold；含披露的MA60替代范围）".into();
        }
        Ok(())
    })();
    if let Err(e) = attempt {
        row.missing.push(e);
    }
    // Expose all missing raw lot fields together, useful with existing aggregate snapshots.
    for (key, missing) in [
        ("lot_id", lot.lot_id.is_none()),
        ("acquired", lot.acquired.is_none()),
        ("sellable_from", lot.sellable_from.is_none()),
        ("sellable", lot.sellable.is_none()),
        ("reserved", lot.reserved.is_none()),
        ("cost_micro_cny", lot.cost_micro_cny.is_none()),
        (
            "allocated_buy_fee_micro_cny",
            lot.allocated_buy_fee_micro_cny.is_none(),
        ),
        ("sell_fees", lot.sell_fees.is_none()),
    ] {
        if missing {
            row.missing.push(format!("missing {key}"));
        }
    }
    row
}
struct Indicators {
    ma5: Option<f64>,
    ma20: Option<f64>,
    ma60: Option<f64>,
    atr: Option<f64>,
    boll: Option<crate::strategy::BollMacdSignal>,
    complete: bool,
    scope: String,
}
fn indicators(s: &Security, close: i64) -> Result<Indicators, String> {
    if s.adjustment != "unadjusted" {
        return Err("indicator adjustment/corporate-action continuity not qualified".into());
    }
    if s.bars
        .first()
        .is_none_or(|b| b.date != s.date || (b.close - close as f64 / 1e6).abs() > 1e-9)
    {
        return Err("bars not frozen through exact confirmed close".into());
    }
    let mut prev = None;
    for b in &s.bars {
        if [b.open, b.high, b.low, b.close]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.)
            || !b.volume.is_finite()
            || b.volume < 0.
            || b.low > b.open.min(b.close)
            || b.high < b.open.max(b.close)
            || b.high < b.low
            || verified_a_share_trading_day(b.date) != Ok(true)
            || prev.is_some_and(|p| b.date >= p)
        {
            return Err("invalid/non-descending/future/duplicate indicator bars".into());
        }
        prev = Some(b.date);
    }
    let data = s
        .bars
        .iter()
        .rev()
        .map(|b| StockData {
            date: b.date.to_string(),
            open: b.open,
            high: b.high,
            low: b.low,
            close: b.close,
            volume: b.volume,
            ma5: None,
            ma10: None,
            ma20: None,
            ma60: None,
        })
        .collect::<Vec<_>>();
    let trend = StockTrendAnalyzer::new().analyze(&data, &s.instrument);
    let atr = (s.bars.len() >= 14)
        .then(|| s.bars.iter().take(14).map(|b| b.high - b.low).sum::<f64>() / 14.);
    if atr.is_some_and(|a| !a.is_finite())
        || [trend.ma5, trend.ma20, trend.ma60]
            .iter()
            .any(|m| !m.is_finite())
    {
        return Err("indicator arithmetic nonfinite".into());
    }
    let boll = detect_boll_macd_observations(
        &data
            .iter()
            .map(|b| BollMacdObservation {
                close: b.close,
                volume: b.volume,
            })
            .collect::<Vec<_>>(),
    )?;
    Ok(Indicators {ma5:(trend.ma5>0.).then_some(trend.ma5),ma20:(trend.ma20>0.).then_some(trend.ma20),ma60:(trend.ma60>0.).then_some(trend.ma60),
        atr,boll:(s.bars.len()>=35).then_some(boll),complete:s.bars.len()>=35,
        scope:format!("bars={}; ATR mean-range CNY interpreted as percent (legacy); ATR fallback={}; MA60 substitutes MA20={}; Boll>=35={}",s.bars.len(),atr.is_none_or(|a| a<=0.),s.bars.len()<60,s.bars.len()>=35)})
}
impl Preview {
    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn expires_at(&self) -> Clock {
        self.expires_at
    }
    /// Monotonic in-memory inspection. Persisted reports are observations; use the import boundary below.
    pub fn reinspect(&mut self, now: Clock) -> Result<(), String> {
        if now < self.inspected_at {
            return Err("inspection clock cannot move backwards".into());
        }
        self.inspected_at = now;
        if now >= self.expires_at || self.state == State::Expired {
            self.state = State::Expired;
            for r in &mut self.rows {
                r.state = State::Expired;
                r.suggested_shares = 0;
            }
        }
        Ok(())
    }
    pub fn markdown(&self) -> String {
        let title = match self.state {
            State::Candidate => "有卖出候选，请人工核对",
            State::NoSuggestion => "本次无卖出建议",
            State::Unavailable => "当前无法形成可用卖出建议",
            State::Expired => "预览已过期，不能据此申报",
        };
        let candidates = self
            .rows
            .iter()
            .filter(|r| r.state == State::Candidate)
            .count();
        let unknown = self
            .rows
            .iter()
            .filter(|r| r.state == State::Unavailable)
            .count();
        let mut out = format!(
            "{title}。\n\n到期：{}。候选批次：{candidates}；不可用批次：{unknown}；账户：{}。\n\n",
            self.expires_at,
            escaped(&self.account_state)
        );
        let missing = self
            .missing
            .iter()
            .chain(self.rows.iter().flat_map(|r| r.missing.iter()))
            .collect::<BTreeSet<_>>();
        if !missing.is_empty() {
            let summary = if self.authority.contains("NotAdmitted") {
                "来源尚未准入；需核验真实可卖批次、最终收盘价、交易状态与完整费用"
            } else {
                "部分账户、时点或逐批证据未通过校验；不可用行不构成继续持有建议"
            };
            out.push_str(&format!(
                "关键缺口：{summary}（{}项诊断见文末）。\n\n",
                missing.len()
            ));
        }
        if !self.rows.is_empty() {
            out.push_str("|代码|批次|状态|观察总股数|可卖股数|建议股数|确认收盘价(元)|原因|\n|---|---|---|---:|---:|---:|---:|---|\n");
            for r in &self.rows {
                out.push_str(&format!(
                    "|{}|{}|{:?}|{}|{}|{}|{}|{}|\n",
                    escaped(&r.instrument),
                    escaped(r.lot_id.as_deref().unwrap_or("缺失")),
                    r.state,
                    r.observed_total_shares,
                    r.eligible_sellable
                        .map(|x| x.to_string())
                        .unwrap_or("未知".into()),
                    r.suggested_shares,
                    r.reference_close_micro_cny
                        .map(|p| format!("{:.6}", p as f64 / 1e6))
                        .unwrap_or("未知".into()),
                    escaped(&r.reason)
                ));
            }
        }
        out.push_str(&format!("\n{}\n\n费用/规则：{}\n\n覆盖、人工处理与真实成交分别记录；真实成交及次日开盘价差比较当前不可用。\n\n输入哈希：{}；规则：{}；资格：{}\n",escaped(&self.manual_notes),escaped(&self.semantics),escaped(&self.input_hash),escaped(&self.schema),escaped(&self.authority)));
        if !missing.is_empty() {
            out.push_str("\n原始缺口诊断：\n\n");
            for gap in missing {
                out.push_str(&format!("- {}\n", escaped(gap)));
            }
        }
        out
    }
    pub fn handling_template(&self) -> serde_json::Value {
        serde_json::json!({"schema":"sell-human-observation/v1","preview_hash":hash(self),"not_settlement":true,"allowed_statuses":["declared","filled","partial","unfilled","handled-without-action"],"status":null,"observed_at":null,"broker_evidence_reference":null,"account_ref":null,"lot_ids":[],"declared_shares":null,"filled_shares":null,"actual_price":null,"commission":null,"stamp":null,"transfer":null,"other_fees":null,"notes":null,"execution_comparison":"unavailable until real receipts + qualified next prices + fees"})
    }
}
pub fn reinspect_imported(imported: ImportedPreview, now: Clock) -> Result<Preview, String> {
    let mut report = Preview {
        schema: imported.schema,
        state: imported.state,
        created_at: imported.created_at,
        inspected_at: imported.inspected_at,
        expires_at: imported.expires_at,
        input_hash: imported.input_hash,
        authority: imported.authority,
        account_state: imported.account_state,
        account_snapshot: imported.account_snapshot,
        missing: imported.missing,
        rows: imported.rows,
        semantics: imported.semantics,
        manual_notes: imported.manual_notes,
        execution: imported.execution,
        next_open_comparison: imported.next_open_comparison,
    };
    report.reinspect(now)?;
    let imported_state = if report.state == State::Expired {
        State::Expired
    } else {
        State::Unavailable
    };
    report.state = imported_state.clone();
    report.authority = "ImportedReport/NotAdmitted".into();
    report.account_state = "imported_untrusted_observation".into();
    report.execution = "Unavailable: imported observations do not establish real execution".into();
    report.next_open_comparison =
        "Unavailable: imported observations do not establish receipts/prices/fees".into();
    report.missing.push(
        "imported report cannot restore source authority; regenerate from qualified sources".into(),
    );
    for row in &mut report.rows {
        row.state = imported_state.clone();
        mask_unqualified_numbers(row);
    }
    Ok(report)
}

/// Existing aggregate snapshots preserve observed totals only. Never project paper lots.
pub fn diagnose_database(path: &std::path::Path, as_of: Clock) -> anyhow::Result<Preview> {
    use crate::database::attribution_reports::{
        AttributionDatabaseAccess, AttributionDatabaseSession,
    };
    use diesel::prelude::*;
    super::io::stable_snapshot(path)?;
    let before = super::io::file_hash(path)?;
    let session = AttributionDatabaseSession::open(path, AttributionDatabaseAccess::ReadOnly)?;
    let mut conn = session
        .database()
        .get_conn()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    #[derive(QueryableByName)]
    struct Head {
        #[diesel(sql_type=diesel::sql_types::Text)]
        snapshot_id: String,
        #[diesel(sql_type=diesel::sql_types::Text)]
        effective_at: String,
        #[diesel(sql_type=diesel::sql_types::Text)]
        confirmed_at: String,
        #[diesel(sql_type=diesel::sql_types::Text)]
        source: String,
        #[diesel(sql_type=diesel::sql_types::Text)]
        evidence_sha256: String,
        #[diesel(sql_type=diesel::sql_types::Integer)]
        confirm_empty: i32,
        #[diesel(sql_type=diesel::sql_types::Integer)]
        item_count: i32,
    }
    #[derive(QueryableByName)]
    struct Item {
        #[diesel(sql_type=diesel::sql_types::Text)]
        code: String,
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        quantity: i64,
        #[diesel(sql_type=diesel::sql_types::Double)]
        cost_price: f64,
    }
    let heads=diesel::sql_query("SELECT snapshot_id,effective_at,confirmed_at,source,evidence_sha256,confirm_empty,item_count FROM user_position_snapshot ORDER BY effective_at DESC,confirmed_at DESC,snapshot_id DESC").load::<Head>(&mut conn);
    let mut pack = EvidencePack {
        schema: "sell-evidence-observed/v1".into(),
        account: None,
        lots: vec![],
        securities: vec![],
    };
    let mut diagnostics = vec![];
    let mut account_state = "missing".to_string();
    match heads {
        Err(e) => diagnostics.push(format!("user_position_snapshot unreadable: {e}")),
        Ok(heads) => {
            let head = heads
                .into_iter()
                .filter_map(|h| {
                    let effective = super::shanghai_clock(&h.effective_at).ok()?;
                    let confirmed = super::shanghai_clock(&h.confirmed_at).ok()?;
                    (effective <= as_of && confirmed <= as_of).then_some((h, effective, confirmed))
                })
                .max_by_key(|(h, e, c)| (*e, *c, h.snapshot_id.clone()));
            if let Some((h, e, c)) = head {
                let items=diesel::sql_query("SELECT code,quantity,cost_price FROM user_position_snapshot_item WHERE snapshot_id=? ORDER BY code").bind::<diesel::sql_types::Text,_>(&h.snapshot_id).load::<Item>(&mut conn);
                match items {
                    Err(err) => diagnostics.push(format!("snapshot items unreadable: {err}")),
                    Ok(items) => {
                        account_state =
                            if h.confirm_empty == 1 && h.item_count == 0 && items.is_empty() {
                                "confirmed_complete_empty_observed"
                            } else if h.item_count >= 0
                                && h.item_count as usize == items.len()
                                && h.confirm_empty == 0
                                && !items.is_empty()
                            {
                                "complete_aggregate_positions_observed"
                            } else {
                                "incomplete_or_conflicting_snapshot"
                            }
                            .into();
                        if e > c {
                            diagnostics.push("snapshot effective after confirmation".into());
                        }
                        for item in items {
                            let total = match u32::try_from(item.quantity) {
                                Ok(value) if value > 0 => value,
                                _ => {
                                    diagnostics.push(format!(
                                        "{} aggregate quantity is not a positive u32 share count",
                                        item.code
                                    ));
                                    account_state = "incomplete_or_conflicting_snapshot".into();
                                    continue;
                                }
                            };
                            let cost =
                                crate::trading::paper_ledger::Money::from_cny(item.cost_price)
                                    .ok()
                                    .map(|v| v.micros());
                            pack.lots.push(Lot {
                                account_ref: String::new(),
                                instrument: item.code,
                                lot_id: None,
                                acquired: None,
                                sellable_from: None,
                                total,
                                sellable: None,
                                reserved: None,
                                cost_micro_cny: cost,
                                allocated_buy_fee_micro_cny: None,
                                sell_fees: None,
                                source: Source {
                                    source: h.source.clone(),
                                    revision: h.snapshot_id.clone(),
                                    sha256: h.evidence_sha256.clone(),
                                    known_at: c,
                                    conflicted: false,
                                    invalidated: false,
                                },
                            });
                        }
                    }
                }
            } else {
                diagnostics
                    .push("no snapshot known by as_of (future/malformed rows excluded)".into());
            }
        }
    }
    let mut result = preview_observed(&pack, as_of);
    result.input_hash = before.clone();
    result.account_state = account_state;
    result.missing.extend(diagnostics);
    result.missing.push("aggregate snapshot lacks account binding, lot IDs, acquisition/sellability dates, reservations and original fees; cost is aggregate diagnostic only".into());
    super::io::stable_snapshot(path)?;
    anyhow::ensure!(
        before == super::io::file_hash(path)?,
        "source snapshot changed during read"
    );
    Ok(result)
}

#[cfg(test)]
#[path = "sell_reminder_tests.rs"]
mod tests;
