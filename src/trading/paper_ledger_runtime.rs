//! Production adapter: explicit epoch binding, outside-lock market reads, then
//! one locked ledger decision. Never reads user screenshots or the real ledger.
use super::paper_ledger::*;
use super::paper_trade::{
    PaperOutcome, PaperResult, PaperSignal, PaperTradePersistenceReceipt, PaperTradeStatus,
};
use crate::{broker::ExecutionQuote, database::DatabaseManager};
use chrono::{DateTime, Utc};
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
    PaperLedger::open(db, &Utc::now)
        .read(&binding)
        .map_err(|error| error.to_string())?;
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
    let mut signal = signal.clone();
    signal.plan_id = format!("paper:{}:{}", binding.epoch_id, signal.plan_id);
    if let Some(receipt) = ledger
        .recover_terminal(binding, &signal, PriceIntent::SignalQuoteMarketV1)
        .map_err(|error| error.to_string())?
    {
        return outcome(signal.plan_id, receipt);
    }
    let view = ledger.read(binding).map_err(|error| error.to_string())?;
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
    let set = PaperLedger::open(db, &Utc::now)
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
    let count:Count = diesel::sql_query("SELECT COUNT(*) AS n FROM paper_trades p JOIN paper_ledger_event e ON e.paper_trade_id=p.id WHERE e.account_id=? AND p.code=? AND p.direction='sell' AND p.status='Filled' AND date(p.ts)=?")
        .bind::<diesel::sql_types::Text,_>(&binding.account_id).bind::<diesel::sql_types::Text,_>(code).bind::<diesel::sql_types::Text,_>(today).get_result(&mut conn).map_err(|error|error.to_string())?;
    Ok(count.n > 0)
}
