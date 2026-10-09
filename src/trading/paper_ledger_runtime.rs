//! Production adapter: explicit epoch binding, outside-lock market reads, then
//! one locked ledger decision. User screenshots seed the account once; runtime
//! valuation and risk facts never reload real-account cash or inventory.
use super::paper_ledger::*;
use super::paper_trade::{
    PaperOutcome, PaperResult, PaperSignal, PaperTradePersistenceReceipt, PaperTradeStatus,
};
use crate::{broker::ExecutionQuote, database::DatabaseManager};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use std::sync::atomic::{AtomicBool, Ordering};

/// Task10 must explicitly seed and bind this manifest; absence is fail-closed.
pub const BINDING_ENV: &str = "PAPER_LEDGER_ACCOUNT_BINDING";
/// Frozen decision input; a new ruling invalidates a previously evaluated sell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryCheckpoint {
    pub binding: AccountBinding,
    pub version: i64,
    pub event_hash: String,
    pub inventory_fingerprint: String,
}

/// The loss period is explicit: a seed-day return is not a real-account daily
/// PnL, and an absent prior paper close cannot become a cumulative daily loss.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaperPnlPeriod {
    SinceCutover { cutover_at: DateTime<Utc> },
    PreviousClose { price_date: NaiveDate },
}

#[derive(Clone, Debug)]
pub struct PaperAccountMetrics {
    pub binding: AccountBinding,
    pub version: i64,
    pub event_hash: String,
    /// Actual price observation; rereading the head does not refresh this time.
    pub effective_at: DateTime<Utc>,
    pub valuation_source: String,
    pub cash: f64,
    pub market_value: f64,
    pub total_assets: f64,
    pub pnl_period: Option<PaperPnlPeriod>,
    pub metrics: crate::risk::account_mode::PortfolioMetrics,
}

/// Intraday account facts use independently admitted fresh quotes. Outside the
/// session the frozen paper valuation is disclosed with its original time.
pub fn account_metrics() -> Result<PaperAccountMetrics, String> {
    let binding = active_binding()?;
    let db = DatabaseManager::try_get().ok_or("DB not initialized")?;
    let live = super::paper_sell::intraday_session_open_at(Utc::now());
    account_metrics_on(
        db,
        &binding,
        &Utc::now,
        live.then_some(
            &crate::broker::execution_quote as &dyn Fn(&str) -> Result<ExecutionQuote, String>,
        ),
    )
}

pub(crate) fn account_metrics_on(
    db: &DatabaseManager,
    binding: &AccountBinding,
    clock: &(dyn Fn() -> DateTime<Utc> + Sync),
    quotes: Option<&dyn Fn(&str) -> Result<ExecutionQuote, String>>,
) -> Result<PaperAccountMetrics, String> {
    let ledger = PaperLedger::open(db, clock);
    let evaluated_at = clock();
    let today = china_day(evaluated_at);
    let (view, effective) = ledger
        .read_account_with_effective(binding, today)
        .map_err(|error| error.to_string())?;
    let sample = effective
        .opening_inventory_sample()
        .map_err(|error| error.to_string())?;
    let cutover_at = effective
        .receipt()
        .cutover_at
        .ok_or("paper cutover is missing")?;
    let held_codes = view
        .lots
        .iter()
        .map(|lot| lot.code.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut marks = view
        .marks
        .iter()
        .filter(|(code, _)| held_codes.contains(code.as_str()))
        .map(|(code, mark)| (code.clone(), mark.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    if let Some(quotes) = quotes {
        for code in held_codes {
            let quote = quotes(code)?;
            marks.insert(
                code.to_string(),
                Mark {
                    code: code.into(),
                    price: Money::from_cny(quote.price).map_err(|error| error.to_string())?,
                    observed_at: quote.observed_at,
                    source: "qualified_execution_quote_v1".into(),
                },
            );
        }
        let completed_at = clock();
        if marks.values().any(|mark| {
            let age = completed_at
                .signed_duration_since(mark.observed_at)
                .num_milliseconds();
            mark.price <= Money::ZERO || !(0..=5_000).contains(&age)
        }) {
            return Err("paper account valuation requires complete fresh qualified quotes".into());
        }
        // Quote acquisition occurs outside the storage lock. A concurrent fill
        // must force a retry rather than combine new prices with old funds.
        let current = ledger.read(binding).map_err(|error| error.to_string())?;
        if current.version != view.version || current.event_hash != view.event_hash {
            return Err(LedgerError::VersionChanged.to_string());
        }
    }
    let effective_at = view
        .lots
        .iter()
        .filter_map(|lot| marks.get(&lot.code).map(|mark| mark.observed_at))
        .min()
        .unwrap_or(view.as_of);
    if view.as_of > clock() || effective_at > clock() {
        return Err("paper account valuation is from the future".into());
    }
    let market_micros = view.lots.iter().try_fold(0_i64, |total, lot| {
        let mark = marks
            .get(&lot.code)
            .ok_or("paper account valuation is incomplete")?;
        let amount = mark
            .price
            .micros()
            .checked_mul(i64::from(lot.quantity))
            .ok_or("paper account valuation overflow")?;
        total
            .checked_add(amount)
            .ok_or("paper account valuation overflow")
    })?;
    let equity_micros = view
        .cash
        .micros()
        .checked_add(market_micros)
        .ok_or("paper account equity overflow")?;
    if view.cash < Money::ZERO || market_micros < 0 || equity_micros <= 0 {
        return Err("paper account cash/equity is invalid".into());
    }
    let (pnl_period, baseline) = if !view.lots.is_empty() && china_day(effective_at) != today {
        (None, None)
    } else if china_day(cutover_at) == today {
        (
            Some(PaperPnlPeriod::SinceCutover { cutover_at }),
            Some(view.seed_equity),
        )
    } else {
        let previous = crate::calendar::verified_prev_a_share_trading_day(today)?;
        match view.closes.get(&previous) {
            Some(baseline) => (
                Some(PaperPnlPeriod::PreviousClose {
                    price_date: previous,
                }),
                Some(*baseline),
            ),
            None => (None, None),
        }
    };
    let today_pnl_pct = baseline
        .map(|baseline| {
            if baseline <= Money::ZERO {
                return Err("paper account PnL baseline must be positive".to_string());
            }
            Ok((equity_micros as f64 / baseline.micros() as f64 - 1.0) * 100.0)
        })
        .transpose()?;
    let mut exits = sample
        .exit_pnls
        .iter()
        .map(|exit| {
            let row = effective
                .rows()
                .map_err(|error| error.to_string())?
                .iter()
                .find(|row| row.id == exit.fill_id)
                .ok_or("paper exit lacks effective source")?;
            let at = super::paper_lot_ledger::parse_paper_fill_timestamp(row.id, &row.occurred_at)?;
            Ok((at, exit.fill_id, exit.account_net_pnl))
        })
        .collect::<Result<Vec<_>, String>>()?;
    exits.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let consecutive_stop_loss_n = exits
        .iter()
        .take(5)
        .take_while(|(_, _, pnl)| *pnl < Money::ZERO)
        .count() as u32;
    let valuation_source = if view.lots.is_empty() {
        "纯现金余额（无持仓价格依赖）".to_string()
    } else if quotes.is_some() {
        "实时行情估值".to_string()
    } else if view.as_of == cutover_at {
        "初始快照估值".to_string()
    } else if !marks.is_empty()
        && marks.values().all(|mark| {
            serde_json::from_str::<serde_json::Value>(&mark.source).is_ok_and(|source| {
                source["kind"] == "qualified_daily_close_v1"
                    && source["price_date"] == china_day(view.as_of).to_string()
            })
        })
    {
        format!("收盘价估值，价格日={}", china_day(view.as_of))
    } else {
        "模拟成交行情估值".to_string()
    };
    Ok(PaperAccountMetrics {
        binding: binding.clone(),
        version: view.version,
        event_hash: view.event_hash.clone(),
        effective_at,
        valuation_source,
        cash: view.cash.cny(),
        market_value: Money::from_micros(market_micros).cny(),
        total_assets: Money::from_micros(equity_micros).cny(),
        pnl_period,
        metrics: crate::risk::account_mode::PortfolioMetrics {
            today_pnl_pct,
            consecutive_stop_loss_n: Some(consecutive_stop_loss_n),
            total_pos_cheng: Some(
                (market_micros as f64 / equity_micros as f64 * 10.0)
                    .round()
                    .clamp(0.0, 10.0) as u8,
            ),
        },
    })
}

fn china_day(at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).expect("China offset"))
        .date_naive()
}

/// Establish one immutable paper closing baseline from the paper inventory and
/// admitted same-day closes. No user-account ledger or position table is read.
pub fn settle_closing_valuation(price_date: NaiveDate) -> Result<Option<PaperReceipt>, String> {
    let binding = active_binding()?;
    let db = DatabaseManager::try_get().ok_or("DB not initialized")?;
    settle_closing_valuation_on(db, &binding, price_date, &Utc::now, &|code, date| {
        let batch = crate::data_gateway::HistoricalBarsGateway::new()
            .required_daily_bars(code, 10)
            .map_err(|error| error.to_string())?;
        close_from_admitted(code, date, &batch)
    })
}

fn close_from_admitted(
    code: &str,
    date: NaiveDate,
    batch: &crate::data_gateway::AdmittedDailyBars,
) -> Result<(Money, String), String> {
    if batch.target_code() != code {
        return Err("paper close batch instrument mismatch".into());
    }
    let mut bars = batch.records().iter().filter(|bar| bar.date == date);
    let bar = bars
        .next()
        .ok_or_else(|| format!("paper close {code} lacks exact price_date={date}"))?;
    if bars.next().is_some()
        || bar.adjust != crate::data_provider::AdjustType::None
        || !bar.settled
        || !bar.close.is_finite()
        || bar.close <= 0.0
    {
        return Err(format!(
            "paper close {code} requires one settled unadjusted close for {date}"
        ));
    }
    let evidence = batch.evidence();
    let source = serde_json::json!({
        "kind": "qualified_daily_close_v1", "price_date": date.to_string(),
        "provider": format!("{:?}", evidence.provider), "source": evidence.source,
        "source_at": evidence.source_at, "observed_at": evidence.observed_at,
        "batch_id": evidence.batch_id,
    })
    .to_string();
    Ok((
        Money::from_cny(bar.close).map_err(|error| error.to_string())?,
        source,
    ))
}

pub(crate) fn settle_closing_valuation_on(
    db: &DatabaseManager,
    binding: &AccountBinding,
    price_date: NaiveDate,
    clock: &(dyn Fn() -> DateTime<Utc> + Sync),
    closes: &dyn Fn(&str, NaiveDate) -> Result<(Money, String), String>,
) -> Result<Option<PaperReceipt>, String> {
    let ledger = PaperLedger::open(db, clock);
    ledger
        .require_active_v1_owner(binding)
        .map_err(|error| error.to_string())?;
    let view = ledger.read(binding).map_err(|error| error.to_string())?;
    let command_id = format!("paper-close-v1:{}:{price_date}", binding.epoch_id);
    // An existing close is authoritative; a reread must not overwrite its
    // prices or fetch another batch under an already settled business identity.
    if view.closes.contains_key(&price_date) {
        return Ok(None);
    }
    let now = clock();
    let china = chrono::FixedOffset::east_opt(8 * 3600).expect("China offset");
    let close_at = china
        .from_local_datetime(&price_date.and_hms_opt(15, 0, 0).expect("valid close"))
        .single()
        .expect("fixed offset")
        .with_timezone(&Utc);
    if china_day(now) != price_date
        || now < close_at
        || !crate::calendar::verified_a_share_trading_day(price_date)?
    {
        return Err("paper close requires the completed current verified trading day".into());
    }
    let mut marks = Vec::new();
    for code in view
        .lots
        .iter()
        .map(|lot| lot.code.as_str())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let (price, source) = closes(code, price_date)?;
        if price <= Money::ZERO || source.trim().is_empty() {
            return Err("paper close price/source is invalid".into());
        }
        marks.push(Mark {
            code: code.into(),
            price,
            observed_at: now,
            source,
        });
    }
    // `as_of` is the completed valuation observation, while each source keeps
    // the actual price day and its original provider timestamps. It is never
    // execution-quote evidence. The locked Mark checks version and inventory.
    let observed_at = clock();
    if china_day(observed_at) != price_date || observed_at < now {
        return Err("paper close observation crossed the price day or moved backward".into());
    }
    for mark in &mut marks {
        mark.observed_at = observed_at;
    }
    ledger
        .apply(PaperCommand::Mark(ValuationBatch {
            binding: binding.clone(),
            command_id,
            expected_version: view.version,
            inventory_fingerprint: view
                .inventory_fingerprint()
                .map_err(|error| error.to_string())?,
            as_of: observed_at,
            closing: true,
            marks,
        }))
        .map(Some)
        .map_err(|error| error.to_string())
}
pub fn active_binding() -> Result<AccountBinding, String> {
    let raw = std::env::var(BINDING_ENV).map_err(|_| {
        "PaperLedger is not activated: explicit seed/cutover binding required".to_string()
    })?;
    let binding: AccountBinding = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid paper account binding: {error}"))?;
    if binding.account_id.trim().is_empty()
        || binding.epoch_id.trim().is_empty()
        || binding.manifest_hash.len() != 64
        || !binding
            .manifest_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("invalid paper account/epoch/manifest identity".into());
    }
    let db = DatabaseManager::try_get().ok_or("DB not initialized")?;
    let ledger = PaperLedger::open(db, &Utc::now);
    ledger
        .require_active_v1_owner(&binding)
        .map_err(|error| error.to_string())?;
    ledger.read(&binding).map_err(|error| error.to_string())?;
    Ok(binding)
}

pub fn execute(
    binding: &AccountBinding,
    signal: &PaperSignal,
    quote: &ExecutionQuote,
    cancelled: &AtomicBool,
) -> Result<PaperOutcome, String> {
    let db = DatabaseManager::try_get().ok_or("DB not initialized")?;
    execute_on(
        db,
        binding,
        signal,
        quote,
        &Utc::now,
        &crate::broker::execution_quote,
        cancelled,
    )
}

pub fn execute_candidate(
    checkpoint: &InventoryCheckpoint,
    signal: &PaperSignal,
    quote: &ExecutionQuote,
    cancelled: &AtomicBool,
) -> Result<PaperOutcome, String> {
    let db = DatabaseManager::try_get().ok_or("DB not initialized")?;
    execute_checked_on(
        db,
        &checkpoint.binding,
        signal,
        quote,
        &Utc::now,
        &crate::broker::execution_quote,
        cancelled,
        Some(checkpoint),
    )
}

/// The only injectable dependencies are storage, clock and outside-lock quotes;
/// no caller-supplied financial balance can authorize a trade.
pub(crate) fn execute_on(
    db: &DatabaseManager,
    binding: &AccountBinding,
    signal: &PaperSignal,
    quote: &ExecutionQuote,
    clock: &(dyn Fn() -> DateTime<Utc> + Sync),
    quotes: &dyn Fn(&str) -> Result<ExecutionQuote, String>,
    cancelled: &AtomicBool,
) -> Result<PaperOutcome, String> {
    execute_checked_on(db, binding, signal, quote, clock, quotes, cancelled, None)
}

pub(crate) fn execute_checked_on(
    db: &DatabaseManager,
    binding: &AccountBinding,
    signal: &PaperSignal,
    quote: &ExecutionQuote,
    clock: &(dyn Fn() -> DateTime<Utc> + Sync),
    quotes: &dyn Fn(&str) -> Result<ExecutionQuote, String>,
    cancelled: &AtomicBool,
    checkpoint: Option<&InventoryCheckpoint>,
) -> Result<PaperOutcome, String> {
    if cancelled.load(Ordering::SeqCst) {
        return Err(LedgerError::Cancelled.to_string());
    }
    if signal.plan_id.trim().is_empty() || signal.plan_id.starts_with("paper:") {
        return Err("producer must supply a stable unqualified plan identity".into());
    }
    let ledger = PaperLedger::open(db, clock);
    ledger
        .require_active_v1_owner(binding)
        .map_err(|error| error.to_string())?;
    let mut signal = signal.clone();
    signal.plan_id = format!("paper:{}:{}", binding.epoch_id, signal.plan_id);
    if let Some(receipt) = ledger
        .recover_terminal(binding, &signal, PriceIntent::SignalQuoteMarketV1)
        .map_err(|error| error.to_string())?
    {
        return outcome(signal.plan_id, receipt);
    }
    let view = ledger.read(binding).map_err(|error| error.to_string())?;
    let price_qualification = ExecutionPriceQualification::acquire(
        &signal.code,
        quote
            .observed_at
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
            .date_naive(),
    )
    .map_err(|error| error.to_string())?;
    if let Some(checkpoint) = checkpoint {
        if checkpoint.binding != *binding
            || checkpoint.version != view.version
            || checkpoint.event_hash != view.event_hash
            || checkpoint.inventory_fingerprint
                != view.inventory_fingerprint().map_err(|e| e.to_string())?
        {
            return Err(LedgerError::VersionChanged.to_string());
        }
    }
    let mut marks = Vec::new();
    let mut codes = view
        .lots
        .iter()
        .map(|lot| lot.code.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    codes.insert(&signal.code);
    for code in codes {
        if cancelled.load(Ordering::SeqCst) {
            return Err(LedgerError::Cancelled.to_string());
        }
        let mark_quote = if code == signal.code {
            quote.clone()
        } else {
            quotes(code)?
        };
        marks.push(Mark {
            code: code.into(),
            price: Money::from_cny(mark_quote.price).map_err(|error| error.to_string())?,
            observed_at: mark_quote.observed_at,
            source: "execution_quote_v1".into(),
        });
    }
    // Stable per observed attempt/head; new evidence after a rejection gets an
    // explicit new command. Terminal plans stay idempotent independently of it.
    let command_id = format!(
        "{}:head={}:quote={}",
        signal.plan_id,
        view.version,
        signal.quote_observed_at.to_rfc3339()
    );
    let plan_id = signal.plan_id.clone();
    let result = ledger
        .apply_controlled(
            PaperCommand::Execute(ExecuteIntent {
                binding: binding.clone(),
                command_id,
                expected_version: view.version,
                inventory_fingerprint: view
                    .inventory_fingerprint()
                    .map_err(|error| error.to_string())?,
                signal,
                price_intent: PriceIntent::SignalQuoteMarketV1,
                quote_price: Money::from_cny(quote.price).map_err(|error| error.to_string())?,
                price_qualification,
                marks,
            }),
            cancelled,
        )
        .map_err(|error| error.to_string())?;
    outcome(plan_id, result)
}

fn outcome(plan_id: String, receipt: PaperReceipt) -> Result<PaperOutcome, String> {
    let status = match receipt.status {
        LedgerStatus::Filled => PaperTradeStatus::Filled,
        LedgerStatus::NotFilled => PaperTradeStatus::NotFilled,
        LedgerStatus::Invalidated => PaperTradeStatus::Invalidated,
        LedgerStatus::Rejected => {
            return Err(receipt
                .reason
                .unwrap_or_else(|| "paper risk rejected".into()))
        }
        _ => return Err("unexpected non-order receipt".into()),
    };
    let terminal_receipt = if receipt.already_applied {
        None
    } else {
        receipt.audit.map(|audit| PaperTradePersistenceReceipt {
            plan_id,
            order_audit_id: audit.id,
            audit_previous_hash: audit.previous_hash,
            audit_record_hash: audit.record_hash,
            terminal_at: audit.created_at,
        })
    };
    Ok(PaperOutcome {
        result: PaperResult {
            status,
            fill_price: receipt.fill_price.map(Money::cny),
            not_fill_reason: receipt.reason,
        },
        inserted: !receipt.already_applied,
        terminal_receipt,
    })
}

pub fn sellable_positions(
    today: chrono::NaiveDate,
) -> Result<Vec<super::paper_sell::PaperPosition>, String> {
    let binding = active_binding()?;
    let db = DatabaseManager::try_get().ok_or("DB not initialized")?;
    sellable_positions_on(db, &binding, today)
}

pub(crate) fn sellable_positions_on(
    db: &DatabaseManager,
    binding: &AccountBinding,
    today: chrono::NaiveDate,
) -> Result<Vec<super::paper_sell::PaperPosition>, String> {
    let ledger = PaperLedger::open(db, &Utc::now);
    ledger
        .require_active_v1_owner(binding)
        .map_err(|error| error.to_string())?;
    let set = ledger
        .verified_effective_fills(&EffectiveFillRequest {
            scope: EffectiveFillScope::Epoch(binding.clone()),
            history: EffectiveHistory::RestatedLatest,
            as_of: today,
        })
        .map_err(|error| error.to_string())?;
    let (version, event_hash) = set
        .receipt()
        .ledger_head
        .clone()
        .ok_or("effective inventory has no bound ledger head")?;
    let checkpoint = InventoryCheckpoint {
        binding: binding.clone(),
        version,
        event_hash,
        inventory_fingerprint: set
            .receipt()
            .inventory_fingerprint
            .clone()
            .ok_or("effective inventory has no CAS fingerprint")?,
    };
    let mut positions = super::paper_sell::positions_from_effective(&set)?;
    for position in &mut positions {
        position.checkpoint = Some(checkpoint.clone());
    }
    Ok(positions)
}

pub fn already_sold_today(code: &str, today: &str) -> Result<bool, String> {
    let binding = active_binding()?;
    let db = DatabaseManager::try_get().ok_or("DB not initialized")?;
    already_sold_on(db, &binding, code, today)
}
pub(crate) fn already_sold_on(
    db: &DatabaseManager,
    binding: &AccountBinding,
    code: &str,
    today: &str,
) -> Result<bool, String> {
    use diesel::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        n: i64,
    }
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    super::paper_ledger::require_v1_owner_on(&mut conn, binding)
        .map_err(|error| error.to_string())?;
    let count:Count = diesel::sql_query("SELECT COUNT(*) AS n FROM paper_trades p JOIN paper_ledger_event e ON e.paper_trade_id=p.id WHERE e.account_id=? AND p.code=? AND p.direction='sell' AND p.status='Filled' AND date(p.ts)=?")
        .bind::<diesel::sql_types::Text,_>(&binding.account_id).bind::<diesel::sql_types::Text,_>(code).bind::<diesel::sql_types::Text,_>(today).get_result(&mut conn).map_err(|error|error.to_string())?;
    Ok(count.n > 0)
}

#[cfg(test)]
#[path = "paper_account_metrics_tests.rs"]
mod account_metrics_tests;
