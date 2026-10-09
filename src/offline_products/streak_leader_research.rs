//! Offline observed ranking and conservative research. No persisted DTO issues admission.
use super::{at, escaped, hash, Clock, Source};
use crate::calendar::{verified_a_share_trading_day, verified_next_a_share_trading_day};
use crate::performance::fee_policy::AShareFeePolicyV2;
use crate::trading::paper_book_v2_budget_v1::checked;
use crate::trading::paper_book_v2_fill_model::{self as fill, ModelOutcome, Side, WindowRecord};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: String,
    pub max_picks: usize,
    pub shares: u32,
    /// Entry limit relative to D close, integer floor; adverse actual window price.
    pub max_entry_slippage_bps: u32,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            version: "streak-leader/v1-next-session-window".into(),
            max_picks: 3,
            shares: 100,
            max_entry_slippage_bps: 100,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub instrument: String,
    pub streak: Option<u32>,
    pub amount_micro_cny: Option<i64>,
    pub close_micro_cny: Option<i64>,
    pub source: Source,
    /// Descriptive daily path. No price here can become an executable window.
    pub next_close_micro_cny: Option<i64>,
    pub next_close_date: Option<NaiveDate>,
    pub next_close_source: Option<Source>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Day {
    pub date: NaiveDate,
    pub universe_source: Option<Source>,
    pub observations: Vec<Observation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePack {
    pub schema: String,
    pub days: Vec<Day>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradeRow {
    pub decision_date: NaiveDate,
    pub decision_at: Clock,
    pub instrument: String,
    pub source_known_at: Clock,
    pub source_hash: String,
    pub streak: Option<u32>,
    pub amount_micro_cny: Option<i64>,
    pub observed_rank: Option<usize>,
    pub selected: bool,
    pub reason: String,
    pub entry_session: Option<NaiveDate>,
    pub entry_at: Option<Clock>,
    pub exit_at: Option<Clock>,
    pub exit_session: Option<NaiveDate>,
    pub entry_state: String,
    pub exit_state: String,
    pub entry_shares: u32,
    pub closed_shares: u32,
    pub censored_shares: u32,
    pub entry_price_micro_cny: Option<i64>,
    pub exit_price_micro_cny: Option<i64>,
    pub buy_commission_micro_cny: Option<i64>,
    pub buy_stamp_micro_cny: Option<i64>,
    pub sell_commission_micro_cny: Option<i64>,
    pub sell_stamp_micro_cny: Option<i64>,
    pub modeled_covered_net_micro_cny: Option<i64>,
    pub gross_observed_close_return_pct: Option<f64>,
    pub actual_settled_net: Option<i64>,
    pub evidence_hashes: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Denominators {
    pub decision_days: usize,
    pub qualified_days: usize,
    pub observed_rows: usize,
    pub picks: usize,
    pub executable_entries: usize,
    pub closed_trades: usize,
    pub unfilled_entries: usize,
    pub unknown_entries: usize,
    pub censored_entries: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Study {
    pub schema: String,
    pub as_of: Clock,
    pub policy: Policy,
    pub strategy_hash: String,
    pub input_hash: String,
    pub fee_hash: Option<String>,
    pub fee_descriptor: Option<String>,
    pub fee_source: Option<Source>,
    pub fill_hash: String,
    pub authority: String,
    pub headline: String,
    pub denominator: Denominators,
    pub unavailable_days: Vec<(NaiveDate, String)>,
    pub rows: Vec<TradeRow>,
    pub modeled_win_rate: Option<f64>,
    pub modeled_covered_net_micro_cny: Option<i64>,
    pub fee_scope: String,
    pub fill_scope: String,
    pub checked_store_artifacts: Vec<serde_json::Value>,
}
// This is a research-only source boundary, not an execution capability. No
// Deserialize or production constructor. Tests model independent contract proof;
// the real H08/PIT/contra-liquidity adapters are not delivered.
struct QualifiedHistory {
    days: BTreeSet<NaiveDate>,
    windows: BTreeMap<(NaiveDate, String), HistoricalWindow>,
    fee: AShareFeePolicyV2,
    fee_source: Source,
}
struct HistoricalWindow {
    record: WindowRecord,
    lifecycle_band_status: Source,
    executable_contra_liquidity: Source,
    corporate_action_scope: Source,
    queue_proven: bool,
}
const FILL_SCOPE:&str="D-close decision at15:00 known-by cutoff; source streak>=2; sort streak desc/amount desc/instrument asc. Entry next verified session09:30–09:35, expiry09:35 exclusive; one qualified adverse-price whole100 window; entry limit=floor(D-close*(10000+slippage_bps)/10000), must fit source tick/band. Exit next verified session after entry09:30–09:35, limit=qualified lower band; one window, no carry-forward fill. T+1, partial whole100 volume; missing/suspended/delisted exit remains censored. Upper-limit buy/lower-limit sell requires independent queue/contra proof. No daily OHLC fill.";

pub fn research_observed(
    pack: &EvidencePack,
    policy: Policy,
    as_of: Clock,
) -> Result<Study, String> {
    research(pack, policy, as_of, None)
}
fn research(
    pack: &EvidencePack,
    policy: Policy,
    as_of: Clock,
    qualified: Option<&QualifiedHistory>,
) -> Result<Study, String> {
    if pack.schema != "streak-observed/v1"
        || policy.version != "streak-leader/v1-next-session-window"
        || policy.max_picks == 0
        || policy.max_picks > 20
        || policy.shares == 0
        || policy.shares % 100 != 0
        || policy.shares > 1_000_000
        || policy.max_entry_slippage_bps > 1000
        || as_of.offset().local_minus_utc() != 28800
    {
        return Err("unsupported schema/policy/Shanghai clock or bounds".into());
    }
    let mut result = Study {
        schema: "streak-study/v1".into(), as_of,
        strategy_hash: hash(&policy), policy, input_hash: hash(pack),
        fee_hash: qualified.map(|q| q.fee.descriptor_hash()),
        fee_descriptor: qualified.map(|q| String::from_utf8(q.fee.canonical_bytes()).expect("ASCII fee descriptor")),
        fee_source: qualified.map(|q|q.fee_source.clone()),
        fill_hash: hash(&(FILL_SCOPE, fill::MODEL_VERSION)),
        authority: if qualified.is_some() {"InternalQualifiedHistoricalResearch (never live admission)"} else {"ObservedOnly/NotAdmitted/PIT NotCertified"}.into(),
        headline: "不可用：没有合格的可执行历史样本".into(),
        denominator: Denominators::default(), unavailable_days: vec![], rows: vec![],
        modeled_win_rate: None, modeled_covered_net_micro_cny: None,
        fee_scope: "ModeledComponentsOnly: explicit dated Shanghai MainA/StarA commission+stamp per fill; transfer/other excluded; actual settled net unavailable".into(),
        fill_scope: FILL_SCOPE.into(), checked_store_artifacts: vec![],
    };
    let mut dates = BTreeSet::new();
    let mut days = pack.days.iter().collect::<Vec<_>>();
    days.sort_by_key(|d| d.date);
    for day in days {
        if !dates.insert(day.date) {
            return Err("duplicate decision date".into());
        }
        result.denominator.decision_days += 1;
        let decision = at(day.date, 15, 0);
        let date_ok = verified_a_share_trading_day(day.date) == Ok(true) && decision <= as_of;
        let universe_ok = day
            .universe_source
            .as_ref()
            .is_some_and(|s| s.validate(decision).is_ok());
        let day_qualified =
            date_ok && universe_ok && qualified.is_some_and(|q| q.days.contains(&day.date));
        if day_qualified {
            result.denominator.qualified_days += 1;
        } else {
            result.unavailable_days.push((day.date,if !date_ok{"decision time future/nontrading/unverified calendar"}else{"historical PIT universe/pool/status ContractNotDelivered; empty observations do not prove no candidates"}.into()));
        }
        let mut seen = BTreeSet::new();
        let mut eligible = vec![];
        for o in &day.observations {
            if !seen.insert(&o.instrument) {
                return Err("duplicate instrument in decision day".into());
            }
            if date_ok
                && !o.instrument.is_empty()
                && o.source.validate(decision).is_ok()
                && o.streak.is_some_and(|s| s >= 2)
                && o.amount_micro_cny.is_some_and(|v| v >= 0)
                && o.close_micro_cny.is_some_and(|v| v > 0)
            {
                eligible.push(o);
            }
        }
        eligible.sort_by(|a, b| {
            b.streak
                .cmp(&a.streak)
                .then_with(|| b.amount_micro_cny.cmp(&a.amount_micro_cny))
                .then_with(|| a.instrument.cmp(&b.instrument))
        });
        for o in &day.observations {
            let rank = eligible
                .iter()
                .position(|e| std::ptr::eq(*e, o))
                .map(|i| i + 1);
            let selected = day_qualified && rank.is_some_and(|r| r <= result.policy.max_picks);
            let entry = verified_next_a_share_trading_day(day.date).ok();
            let exit = entry.and_then(|d| verified_next_a_share_trading_day(d).ok());
            let mut row = TradeRow {
                decision_date: day.date,
                decision_at: decision,
                instrument: o.instrument.clone(),
                source_known_at: o.source.known_at,
                source_hash: o.source.sha256.clone(),
                streak: o.streak,
                amount_micro_cny: o.amount_micro_cny,
                observed_rank: rank,
                selected,
                reason: if selected {
                    "source-qualified selected"
                } else if rank.is_some() && !day_qualified {
                    "descriptive observed rank only; PIT/authority unavailable"
                } else if rank.is_some() {
                    "excluded: max picks"
                } else {
                    "excluded: streak/amount/close missing, future/invalid source or calendar"
                }
                .into(),
                entry_session: entry,
                entry_at: None,
                exit_at: None,
                exit_session: exit,
                entry_state: "Unavailable".into(),
                exit_state: "Unavailable".into(),
                entry_shares: 0,
                closed_shares: 0,
                censored_shares: 0,
                entry_price_micro_cny: None,
                exit_price_micro_cny: None,
                buy_commission_micro_cny: None,
                buy_stamp_micro_cny: None,
                sell_commission_micro_cny: None,
                sell_stamp_micro_cny: None,
                modeled_covered_net_micro_cny: None,
                gross_observed_close_return_pct: None,
                actual_settled_net: None,
                evidence_hashes: vec![o.source.sha256.clone()],
            };
            // A raw price comparison is explicitly descriptive, even when captured late.
            if let (Some(p), Some(n)) = (o.close_micro_cny, o.next_close_micro_cny) {
                if p > 0
                    && n > 0
                    && o.next_close_date == entry
                    && o.next_close_source.as_ref().is_some_and(|s| {
                        s.validate(as_of).is_ok()
                            && entry.is_some_and(|d| s.known_at >= at(d, 15, 0))
                    })
                {
                    row.gross_observed_close_return_pct = Some((n as f64 / p as f64 - 1.) * 100.);
                }
            }
            if selected {
                result.denominator.picks += 1;
                if let Some(q) = qualified {
                    model_trade(&mut row, o, &result.policy, q, as_of);
                }
            }
            result.rows.push(row);
        }
    }
    result
        .rows
        .sort_by(|a, b| (&a.decision_date, &a.instrument).cmp(&(&b.decision_date, &b.instrument)));
    result.denominator.observed_rows = result.rows.len();
    let mut sum = 0i128;
    let mut wins = 0usize;
    for row in result.rows.iter().filter(|r| r.selected) {
        if row.entry_shares > 0 {
            result.denominator.executable_entries += 1;
        }
        if row.entry_state.starts_with("Unfilled") {
            result.denominator.unfilled_entries += 1;
        }
        if row.entry_state.starts_with("Unavailable") {
            result.denominator.unknown_entries += 1;
        }
        if row.censored_shares > 0 {
            result.denominator.censored_entries += 1;
        }
        if row.entry_shares > 0 && row.closed_shares == row.entry_shares {
            result.denominator.closed_trades += 1;
            if let Some(net) = row.modeled_covered_net_micro_cny {
                sum += net as i128;
                if net > 0 {
                    wins += 1;
                }
            }
        }
    }
    if result.denominator.closed_trades > 0 {
        result.modeled_covered_net_micro_cny = Some(checked(sum).map_err(|e| e.to_string())?);
        result.modeled_win_rate = Some(wins as f64 / result.denominator.closed_trades as f64);
        result.headline = "仅历史模型覆盖费用结果；非真实结算收益，缺失/截尾样本单列".into();
    }
    Ok(result)
}
fn checked_window<'a>(
    q: &'a QualifiedHistory,
    date: NaiveDate,
    code: &str,
    as_of: Clock,
    side: Side,
) -> Result<&'a WindowRecord, String> {
    let w = q
        .windows
        .get(&(date, code.into()))
        .ok_or("independent historical executable price/contra-liquidity window absent")?;
    let r = &w.record;
    let observed = r.observed_at.with_timezone(as_of.offset());
    if observed > as_of
        || observed < at(date, 9, 30)
        || observed >= at(date, 9, 35)
        || r.session_date != date
        || r.instrument_code != code
    {
        return Err("window not known by as_of/outside frozen entry-exit expiry".into());
    }
    for source in [
        &w.lifecycle_band_status,
        &w.executable_contra_liquidity,
        &w.corporate_action_scope,
        &q.fee_source,
    ] {
        source.validate(observed)?;
    }
    r.validate().map_err(|e| e.to_string())?;
    if !w.queue_proven
        && ((side == Side::Buy && r.price_micro_cny == r.upper_micro_cny)
            || (side == Side::Sell && r.price_micro_cny == r.lower_micro_cny))
    {
        return Err("limit queue unknown: cannot infer fill".into());
    }
    if r.fee_segment
        != match q.fee.scope().segment() {
            crate::performance::fee_policy::FeeListingSegment::ShanghaiMainA => "ShanghaiMainA",
            crate::performance::fee_policy::FeeListingSegment::ShanghaiStarA => "ShanghaiStarA",
            _ => "unsupported",
        }
    {
        return Err("fee market/board scope mismatch".into());
    }
    Ok(r)
}
fn model_trade(
    row: &mut TradeRow,
    o: &Observation,
    p: &Policy,
    q: &QualifiedHistory,
    as_of: Clock,
) {
    let attempt = (|| -> Result<(), String> {
        let entry = row.entry_session.ok_or("next verified session missing")?;
        let exit = row.exit_session.ok_or("T+1 exit calendar unavailable")?;
        let window = checked_window(q, entry, &o.instrument, as_of, Side::Buy)?;
        let limit = checked(
            o.close_micro_cny.unwrap() as i128 * (10_000 + p.max_entry_slippage_bps) as i128
                / 10_000,
        )
        .map_err(|e| e.to_string())?;
        let buy = match fill::model(
            Side::Buy,
            p.shares,
            limit,
            window.upper_micro_cny,
            window,
            &q.fee,
        )
        .map_err(|e| e.to_string())?
        {
            ModelOutcome::NoFill(reason) => {
                row.entry_state = format!("Unfilled:{reason:?}");
                return Ok(());
            }
            ModelOutcome::Fill(fill) => fill,
        };
        row.entry_state = if buy.quantity < p.shares {
            "Partial"
        } else {
            "ModeledCoveredFeeEntry"
        }
        .into();
        row.entry_shares = buy.quantity;
        row.entry_at = Some(window.observed_at.with_timezone(as_of.offset()));
        row.entry_price_micro_cny = Some(buy.price_micro_cny);
        row.buy_commission_micro_cny = Some(buy.commission_micro_cny);
        row.buy_stamp_micro_cny = Some(buy.stamp_tax_micro_cny);
        row.censored_shares = buy.quantity;
        row.evidence_hashes.push(hash(window));
        let exit_result = (|| -> Result<(), String> {
            if exit < buy.sellable_from {
                return Err("T+1 locked".into());
            }
            let window = checked_window(q, exit, &o.instrument, as_of, Side::Sell)?;
            let sell = match fill::model(
                Side::Sell,
                buy.quantity,
                window.lower_micro_cny,
                window.upper_micro_cny,
                window,
                &q.fee,
            )
            .map_err(|e| e.to_string())?
            {
                ModelOutcome::NoFill(reason) => {
                    row.exit_state = format!("Censored:Unfilled:{reason:?}");
                    return Ok(());
                }
                ModelOutcome::Fill(fill) => fill,
            };
            row.closed_shares = sell.quantity;
            row.exit_at = Some(window.observed_at.with_timezone(as_of.offset()));
            row.censored_shares = buy.quantity - sell.quantity;
            row.exit_state = if row.censored_shares > 0 {
                "Partial/Censored"
            } else {
                "ModeledCoveredFeeClosed"
            }
            .into();
            row.exit_price_micro_cny = Some(sell.price_micro_cny);
            row.sell_commission_micro_cny = Some(sell.commission_micro_cny);
            row.sell_stamp_micro_cny = Some(sell.stamp_tax_micro_cny);
            // FIFO single entry: proportional cost/fee truncation, remainder retained in censored lot.
            let basis =
                buy.notional_micro_cny as i128 * sell.quantity as i128 / buy.quantity as i128;
            let fee =
                buy.total_fee_micro_cny as i128 * sell.quantity as i128 / buy.quantity as i128;
            row.modeled_covered_net_micro_cny = Some(
                checked(
                    sell.notional_micro_cny as i128
                        - basis
                        - fee
                        - sell.total_fee_micro_cny as i128,
                )
                .map_err(|e| e.to_string())?,
            );
            row.evidence_hashes.push(hash(window));
            Ok(())
        })();
        if let Err(e) = exit_result {
            row.exit_state = format!("Censored:{e}");
        }
        Ok(())
    })();
    if let Err(e) = attempt {
        row.entry_state = format!("Unavailable:{e}");
    }
}
impl Study {
    /// Integrity is supplied by the offline store reader, never source qualification.
    pub fn attach_checked_observation(&mut self, artifact: serde_json::Value) {
        self.checked_store_artifacts.push(artifact);
        self.input_hash = hash(&(self.input_hash.clone(), &self.checked_store_artifacts));
    }
    pub fn markdown(&self) -> String {
        let d = &self.denominator;
        let mut out=format!("{}。\n\n资格：{}。决策日{}；合格日{}；观察记录{}；选中{}；可执行入场{}；完整平仓{}；未知入场{}；截尾{}。\n\n",escaped(&self.headline),escaped(&self.authority),d.decision_days,d.qualified_days,d.observed_rows,d.picks,d.executable_entries,d.closed_trades,d.unknown_entries,d.censored_entries);
        if !self.checked_store_artifacts.is_empty() {
            out.push_str("已核对存档字节完整性；下列记录仅是历史观察，不能恢复准入或证明历史可用时点。\n\n|存档哈希|原始记录数|\n|---|---:|\n");
            for a in &self.checked_store_artifacts {
                out.push_str(&format!(
                    "|{}|{}|\n",
                    escaped(a["capture_sha256"].as_str().unwrap_or("unknown")),
                    a["records"].as_array().map_or(0, |r| r.len())
                ));
            }
        }
        for (day, reason) in &self.unavailable_days {
            out.push_str(&format!("- {day}：{}\n", escaped(reason)));
        }
        if !self.rows.is_empty() {
            out.push_str("\n|决策日|代码|观察排名|选中|入场|退出|观察收盘价差%（非收益）|原因|\n|---|---|---:|---|---|---|---:|---|\n");
            for r in &self.rows {
                out.push_str(&format!(
                    "|{}|{}|{}|{}|{}|{}|{}|{}|\n",
                    r.decision_date,
                    escaped(&r.instrument),
                    r.observed_rank.map(|r| r.to_string()).unwrap_or("—".into()),
                    r.selected,
                    escaped(&r.entry_state),
                    escaped(&r.exit_state),
                    r.gross_observed_close_return_pct
                        .map(|v| format!("{v:.4}"))
                        .unwrap_or("不可用".into()),
                    escaped(&r.reason)
                ));
            }
        }
        out.push_str(&format!("\n完整平仓样本模型胜率：{}；模型覆盖费用净额（微元）：{}；真实结算净额：不可用。\n\n费用：{}\n\n成交模型：{}\n\n策略哈希：{}；输入哈希：{}；费用哈希：{}；模型哈希：{}\n",self.modeled_win_rate.map(|v|format!("{v:.4}")).unwrap_or("不可用".into()),self.modeled_covered_net_micro_cny.map(|v|v.to_string()).unwrap_or("不可用".into()),escaped(&self.fee_scope),escaped(&self.fill_scope),self.strategy_hash,self.input_hash,self.fee_hash.as_deref().unwrap_or("Unavailable"),self.fill_hash));
        out
    }
    pub fn csv(&self) -> String {
        let row_keys = [
            "decision_date",
            "decision_at",
            "instrument",
            "source_known_at",
            "source_hash",
            "streak",
            "amount_micro_cny",
            "observed_rank",
            "selected",
            "reason",
            "entry_session",
            "entry_at",
            "exit_at",
            "exit_session",
            "entry_state",
            "exit_state",
            "entry_shares",
            "closed_shares",
            "censored_shares",
            "entry_price_micro_cny",
            "exit_price_micro_cny",
            "buy_commission_micro_cny",
            "buy_stamp_micro_cny",
            "sell_commission_micro_cny",
            "sell_stamp_micro_cny",
            "modeled_covered_net_micro_cny",
            "gross_observed_close_return_pct",
            "actual_settled_net",
            "evidence_hashes",
        ];
        let mut headers = vec![
            "strategy_hash",
            "fee_hash",
            "fill_hash",
            "input_hash",
            "authority",
            "fee_scope",
            "as_of",
            "policy_version",
            "max_picks",
            "requested_shares",
            "max_entry_slippage_bps",
            "decision_days",
            "qualified_days",
            "observed_rows",
            "picks",
            "executable_entries",
            "closed_trades",
            "unfilled_entries",
            "unknown_entries",
            "censored_entries",
            "headline",
            "modeled_win_rate",
            "modeled_total_covered_net_micro_cny",
        ];
        headers.extend(row_keys);
        headers.extend(["record_type", "raw_record_json"]);
        let line = |fields: Vec<String>| {
            fields
                .iter()
                .map(|s| super::io::csv_cell(s))
                .collect::<Vec<_>>()
                .join(",")
                + "\n"
        };
        let metadata = || {
            vec![
                self.strategy_hash.clone(),
                self.fee_hash.clone().unwrap_or("Unavailable".into()),
                self.fill_hash.clone(),
                self.input_hash.clone(),
                self.authority.clone(),
                self.fee_scope.clone(),
                self.as_of.to_rfc3339(),
                self.policy.version.clone(),
                self.policy.max_picks.to_string(),
                self.policy.shares.to_string(),
                self.policy.max_entry_slippage_bps.to_string(),
                self.denominator.decision_days.to_string(),
                self.denominator.qualified_days.to_string(),
                self.denominator.observed_rows.to_string(),
                self.denominator.picks.to_string(),
                self.denominator.executable_entries.to_string(),
                self.denominator.closed_trades.to_string(),
                self.denominator.unfilled_entries.to_string(),
                self.denominator.unknown_entries.to_string(),
                self.denominator.censored_entries.to_string(),
                self.headline.clone(),
                self.modeled_win_rate
                    .map(|v| v.to_string())
                    .unwrap_or("Unavailable".into()),
                self.modeled_covered_net_micro_cny
                    .map(|v| v.to_string())
                    .unwrap_or("Unavailable".into()),
            ]
        };
        let mut out = line(headers.iter().map(|s| s.to_string()).collect());
        let mut summary = metadata();
        summary.extend(row_keys.iter().map(|_| String::new()));
        summary.extend(["summary".into(), String::new()]);
        out.push_str(&line(summary));
        for row in &self.rows {
            let value = serde_json::to_value(row).unwrap();
            let mut fields = metadata();
            fields.extend(row_keys.iter().map(|k| match &value[k] {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Null => "Unavailable".into(),
                v => v.to_string(),
            }));
            fields.extend(["strategy_observation".into(), String::new()]);
            out.push_str(&line(fields));
        }
        for (date, reason) in &self.unavailable_days {
            let mut fields = metadata();
            fields.extend(row_keys.iter().map(|key| match *key {
                "decision_date" => date.to_string(),
                "reason" => reason.clone(),
                _ => String::new(),
            }));
            fields.extend(["unavailable_day".into(), String::new()]);
            out.push_str(&line(fields));
        }
        for artifact in &self.checked_store_artifacts {
            if let Some(records) = artifact["records"].as_array() {
                for record in records {
                    let mut fields = metadata();
                    fields.extend(row_keys.iter().map(|_| String::new()));
                    fields.push("checked_store_raw_observation".into());
                    fields.push(record.to_string());
                    out.push_str(&line(fields));
                }
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "streak_leader_research_tests.rs"]
mod tests;
