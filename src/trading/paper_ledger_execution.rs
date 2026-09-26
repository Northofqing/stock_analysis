//! Locked financial decision and frozen event facts. No external I/O in this module.
use super::*;
use crate::database::order_audit::{insert_order_audit_with_receipt_query, OrderAuditRecord};
use crate::trading::paper_trade::{Direction, PaperSignal};
use diesel::sql_types::{Double, Nullable};

#[derive(Clone, Debug)]
pub struct ExecuteIntent {
    pub binding: AccountBinding,
    pub command_id: String,
    pub expected_version: i64,
    pub inventory_fingerprint: String,
    pub signal: PaperSignal,
    pub price_intent: PriceIntent,
    pub quote_price: Money,
    /// Whole current inventory plus the traded security, acquired outside the lock.
    pub marks: Vec<Mark>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriceIntent {
    FixedSignalPriceV1,
    SignalQuoteMarketV1,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValuationBatch {
    pub binding: AccountBinding,
    pub command_id: String,
    pub expected_version: i64,
    pub inventory_fingerprint: String,
    pub as_of: DateTime<Utc>,
    pub closing: bool,
    pub marks: Vec<Mark>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct OrderFact {
    pub plan_id: String,
    pub intent_hash: String,
    pub code: String,
    pub direction: String,
    pub requested_price: Money,
    pub price_intent: PriceIntent,
    pub quantity: u32,
    pub quote_price: Money,
    pub quote_observed_at: DateTime<Utc>,
    pub account_mode: String,
    pub data_mode: String,
    pub decision_basis: String,
    pub source_evidence: String,
    pub occurred_at: DateTime<Utc>,
    pub status: LedgerStatus,
    pub reason: Option<String>,
    pub cash_delta: Money,
    pub commission: Money,
    pub stamp: Money,
    pub realized_delta: Money,
    pub lot_changes: Vec<LotChange>,
    pub marks: BTreeMap<String, Mark>,
    pub paper_trade_id: Option<i64>,
    pub audit: AuditLink,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct LotChange {
    before: Option<Lot>,
    after: Option<Lot>,
}

fn fresh(at: DateTime<Utc>, now: DateTime<Utc>) -> Result<(), LedgerError> {
    if !(0..=5000).contains(&now.signed_duration_since(at).num_milliseconds()) {
        return Err(LedgerError::EvidenceUnavailable(
            "realtime quote/mark not fresh after lock acquisition".into(),
        ));
    }
    Ok(())
}
fn check_head(view: &PaperView, version: i64, fingerprint: &str) -> Result<(), LedgerError> {
    if view.version != version {
        return Err(LedgerError::VersionChanged);
    }
    if view.inventory_fingerprint()? != fingerprint {
        return Err(LedgerError::EvidenceUnavailable(
            "inventory fingerprint mismatch".into(),
        ));
    }
    Ok(())
}
fn admit_marks(
    state: &Projection,
    marks: &[Mark],
    extra: Option<&str>,
) -> Result<BTreeMap<String, Mark>, LedgerError> {
    let mut required = state
        .lots
        .iter()
        .map(|lot| lot.code.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(code) = extra {
        required.insert(code);
    }
    let mut admitted = BTreeMap::new();
    for mark in marks {
        if !required.contains(mark.code.as_str())
            || mark.price <= Money::ZERO
            || mark.source.trim().is_empty()
            || admitted.insert(mark.code.clone(), mark.clone()).is_some()
        {
            return Err(LedgerError::EvidenceUnavailable(
                "invalid/duplicate/extraneous valuation mark".into(),
            ));
        }
    }
    if required.len() != admitted.len() {
        return Err(LedgerError::EvidenceUnavailable(
            "incomplete whole-account valuation".into(),
        ));
    }
    Ok(admitted)
}
fn rate(amount: Money, numerator: i64, denominator: i64) -> Result<Money, LedgerError> {
    let rounded = (i128::from(amount.0) * i128::from(numerator) + i128::from(denominator / 2))
        / i128::from(denominator);
    i64::try_from(rounded)
        .map(Money)
        .map_err(|_| LedgerError::Overflow)
}
fn ratio_exceeds(part: Money, whole: Money, bps: u32) -> bool {
    i128::from(part.0) * 10000 > i128::from(whole.0) * i128::from(bps)
}

pub(super) fn order_intent_hash(
    binding: &AccountBinding,
    signal: &PaperSignal,
    price_intent: PriceIntent,
) -> Result<String, LedgerError> {
    if !signal
        .plan_id
        .starts_with(&format!("paper:{}:", binding.epoch_id))
    {
        return Err(LedgerError::InvalidInput(
            "plan lacks active epoch namespace".into(),
        ));
    }
    let semantic_price = match price_intent {
        PriceIntent::FixedSignalPriceV1 => Some(Money::from_cny(signal.price)?),
        PriceIntent::SignalQuoteMarketV1 => None,
    };
    Ok(digest(&encode(&(
        "PaperIntentV1",
        &binding.manifest_hash,
        price_intent,
        signal.direction.as_str(),
        &signal.code,
        signal.quantity,
        semantic_price,
    ))?))
}

impl PaperLedger<'_> {
    pub(super) fn mark(
        &self,
        conn: &mut SqliteConnection,
        batch: ValuationBatch,
        now: DateTime<Utc>,
    ) -> Result<PaperReceipt, LedgerError> {
        let view = load(conn, &batch.binding)?;
        if let Some(previous) = replay_command(
            conn,
            &batch.binding,
            Some(&batch.command_id),
            None,
            Some(&batch),
        )? {
            return Ok(previous);
        }
        check_head(&view, batch.expected_version, &batch.inventory_fingerprint)?;
        if batch.as_of < view.as_of
            || batch.as_of > now
            || batch.marks.iter().any(|m| m.observed_at != batch.as_of)
        {
            return Err(LedgerError::EvidenceUnavailable(
                "valuation time mismatch/future/backward".into(),
            ));
        }
        if batch.closing {
            use chrono::Timelike;
            if batch
                .as_of
                .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                .hour()
                < 15
                || !crate::calendar::verified_a_share_trading_day(day(batch.as_of))
                    .map_err(LedgerError::EvidenceUnavailable)?
            {
                return Err(LedgerError::EvidenceUnavailable(
                    "closing baseline requires completed verified trading day".into(),
                ));
            }
        }
        admit_marks(&view, &batch.marks, None)?;
        let binding = batch.binding.clone();
        let command = batch.command_id.clone();
        append(conn, &binding, &command, &view, Fact::Marked(batch))
    }

    pub(super) fn execute(
        &self,
        conn: &mut SqliteConnection,
        intent: ExecuteIntent,
        now: DateTime<Utc>,
    ) -> Result<PaperReceipt, LedgerError> {
        let view = load(conn, &intent.binding)?;
        let signal = &intent.signal;
        let intent_hash = order_intent_hash(&intent.binding, signal, intent.price_intent)?;
        if let Some(previous) = replay_command(
            conn,
            &intent.binding,
            Some(&intent.command_id),
            Some((&signal.plan_id, &intent_hash)),
            None,
        )? {
            return Ok(previous);
        }
        check_head(
            &view,
            intent.expected_version,
            &intent.inventory_fingerprint,
        )?;
        let price = Money::from_cny(signal.price)?;
        if price <= Money::ZERO
            || intent.quote_price <= Money::ZERO
            || signal.quantity == 0
            || !signal.quantity.is_multiple_of(100)
        {
            return Err(LedgerError::InvalidInput(
                "invalid order price/quantity".into(),
            ));
        }
        fresh(signal.quote_observed_at, now)?;
        if now < view.as_of {
            return Err(LedgerError::EvidenceUnavailable(
                "execution before cutover/head".into(),
            ));
        }
        let marks = admit_marks(&view, &intent.marks, Some(&signal.code))?;
        for mark in marks.values() {
            fresh(mark.observed_at, now)?;
        }
        let manifest: SeedManifest = decode(
            &account(conn, &intent.binding.account_id)?
                .ok_or(LedgerError::NotSeeded)?
                .manifest_bytes,
        )?;
        let notional = price.mul(signal.quantity)?;
        let commission = rate(notional, 3, 10000)?.max(Money(5_000_000));
        let stamp = if signal.direction == Direction::Sell {
            rate(notional, 1, 1000)?
        } else {
            Money::ZERO
        };
        let total_fee = commission.add(stamp)?;
        let mut marked_before = view.projection.clone();
        marked_before.marks = marks.clone();
        let equity = marked_before.equity()?;
        let risk = financial_check(
            signal,
            &view,
            equity,
            &marks,
            &manifest.policy,
            notional,
            commission,
            now,
        );
        let (status, reason) = if let Err(reason) = risk {
            (LedgerStatus::Rejected, Some(reason))
        } else if signal.is_suspended {
            (LedgerStatus::NotFilled, Some("停牌拒绝".into()))
        } else if (signal.direction == Direction::Buy && signal.is_limit_up)
            || (signal.direction == Direction::Sell && signal.is_limit_down)
        {
            (LedgerStatus::NotFilled, Some("涨跌停不可成交".into()))
        } else if i128::from((intent.quote_price.0 - price.0).abs()) * 10000
            > i128::from(price.0) * i128::from(manifest.policy.max_slippage_bps)
        {
            (LedgerStatus::Invalidated, Some("滑点超限".into()))
        } else {
            (LedgerStatus::Filled, None)
        };
        let mut after_lots = view.lots.clone();
        let mut cash_delta = Money::ZERO;
        let mut realized_delta = Money::ZERO;
        if status == LedgerStatus::Filled {
            match signal.direction {
                Direction::Buy => {
                    cash_delta = Money::ZERO.sub(notional.add(total_fee)?)?;
                    after_lots.push(Lot {
                        lot_id: format!("fill:{}", intent.command_id),
                        code: signal.code.clone(),
                        name: signal.name.clone(),
                        quantity: signal.quantity,
                        basis_price: price,
                        buy_fee_remaining: commission,
                        acquired_on: day(now),
                        sellable_from: crate::calendar::verified_next_a_share_trading_day(day(now))
                            .map_err(LedgerError::EvidenceUnavailable)?,
                        reported_cost: None,
                    });
                }
                Direction::Sell => {
                    cash_delta = notional.sub(total_fee)?;
                    let mut left = signal.quantity;
                    let mut basis = Money::ZERO;
                    let mut allocated_buy_fee = Money::ZERO;
                    for lot in &mut after_lots {
                        if lot.code != signal.code || lot.sellable_from > day(now) || left == 0 {
                            continue;
                        }
                        let taken = lot.quantity.min(left);
                        let fee = if taken == lot.quantity {
                            lot.buy_fee_remaining
                        } else {
                            Money(
                                i64::try_from(
                                    i128::from(lot.buy_fee_remaining.0) * i128::from(taken)
                                        / i128::from(lot.quantity),
                                )
                                .map_err(|_| LedgerError::Overflow)?,
                            )
                        };
                        basis = basis.add(lot.basis_price.mul(taken)?)?;
                        allocated_buy_fee = allocated_buy_fee.add(fee)?;
                        lot.buy_fee_remaining = lot.buy_fee_remaining.sub(fee)?;
                        lot.quantity -= taken;
                        left -= taken;
                    }
                    if left != 0 {
                        return Err(LedgerError::IntegrityFailure(
                            "inventory changed inside locked transaction".into(),
                        ));
                    }
                    after_lots.retain(|lot| lot.quantity > 0);
                    realized_delta = notional
                        .sub(basis)?
                        .sub(allocated_buy_fee)?
                        .sub(total_fee)?;
                }
            }
        }
        let mut changes = Vec::new();
        for before in &view.lots {
            let after = after_lots
                .iter()
                .find(|lot| lot.lot_id == before.lot_id)
                .cloned();
            if after.as_ref() != Some(before) {
                changes.push(LotChange {
                    before: Some(before.clone()),
                    after,
                });
            }
        }
        for after in &after_lots {
            if !view.lots.iter().any(|lot| lot.lot_id == after.lot_id) {
                changes.push(LotChange {
                    before: None,
                    after: Some(after.clone()),
                });
            }
        }
        let filled = status == LedgerStatus::Filled;
        let observed = signal.quote_observed_at.to_rfc3339();
        let decision_basis = format!(
            "{} | PaperLedgerV1 account={} epoch={} head={} inventory={} fee_model={FEE_MODEL}",
            signal.virtual_reason,
            intent.binding.account_id,
            intent.binding.epoch_id,
            view.version,
            intent.inventory_fingerprint
        );
        let audit = insert_order_audit_with_receipt_query(
            conn,
            &OrderAuditRecord {
                business_order_id: &signal.plan_id,
                source: "PaperTrade",
                decision_basis: &decision_basis,
                side: signal.direction.as_str(),
                code: &signal.code,
                requested_price: signal.price,
                execution_price: filled.then_some(price.cny()),
                quantity: i64::from(signal.quantity),
                quote_observed_at: Some(&observed),
                outcome: if filled { "Filled" } else { "Rejected" },
                failure_reason: reason.as_deref(),
            },
        )?;
        let paper_trade_id = if status != LedgerStatus::Rejected {
            let status_text = match status {
                LedgerStatus::Filled => "Filled",
                LedgerStatus::NotFilled => "NotFilled",
                _ => "Invalidated",
            };
            let ts = now
                .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                .format("%Y-%m-%d %H:%M:%S%.3f")
                .to_string();
            let rows = diesel::sql_query("INSERT INTO paper_trades(plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,virtual_reason,account_mode,data_mode,ts,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
                .bind::<Text,_>(&signal.plan_id).bind::<Text,_>(&signal.code).bind::<Text,_>(&signal.name).bind::<Text,_>(signal.direction.as_str())
                .bind::<Double,_>(signal.price).bind::<BigInt,_>(i64::from(signal.quantity)).bind::<Text,_>(status_text).bind::<Nullable<Double>,_>(filled.then_some(price.cny()))
                .bind::<Nullable<Text>,_>(reason.as_deref()).bind::<Text,_>(&signal.virtual_reason).bind::<Text,_>(signal.risk_context.account_mode.label()).bind::<Text,_>(signal.risk_context.data_mode.label())
                .bind::<Text,_>(&ts).bind::<Text,_>(&ts).execute(conn)?;
            if rows != 1 {
                return Err(LedgerError::IntegrityFailure(
                    "compatible trade insert affected zero".into(),
                ));
            }
            Some(
                diesel::sql_query("SELECT last_insert_rowid() AS value")
                    .get_result::<IntegerRow>(conn)?
                    .value,
            )
        } else {
            None
        };
        let fact = OrderFact {
            plan_id: signal.plan_id.clone(),
            intent_hash,
            code: signal.code.clone(),
            direction: signal.direction.as_str().into(),
            requested_price: price,
            price_intent: intent.price_intent,
            quantity: signal.quantity,
            quote_price: intent.quote_price,
            quote_observed_at: signal.quote_observed_at,
            account_mode: signal.risk_context.account_mode.label().into(),
            data_mode: signal.risk_context.data_mode.label().into(),
            decision_basis,
            source_evidence: encode(&(
                signal.price.to_bits(),
                signal.limit_down_price.map(f64::to_bits),
                signal.limit_up_price.map(f64::to_bits),
                signal.is_limit_down,
                signal.is_limit_up,
                signal.is_suspended,
                signal.secondary_confirmed,
            ))?,
            occurred_at: now,
            status,
            reason,
            cash_delta,
            commission: if filled { commission } else { Money::ZERO },
            stamp: if filled { stamp } else { Money::ZERO },
            realized_delta,
            lot_changes: changes,
            marks,
            paper_trade_id,
            audit: AuditLink {
                id: audit.order_audit_id,
                previous_hash: audit.previous_hash,
                record_hash: audit.record_hash,
                created_at: audit.created_at,
            },
        };
        append(
            conn,
            &intent.binding,
            &intent.command_id,
            &view,
            Fact::Order(fact),
        )
    }
}

fn financial_check(
    signal: &PaperSignal,
    view: &Projection,
    equity: Money,
    marks: &BTreeMap<String, Mark>,
    policy: &RiskPolicyV1,
    notional: Money,
    commission: Money,
    now: DateTime<Utc>,
) -> Result<(), String> {
    use crate::trading::order_safety::{OrderSafetyInput, SafetySide};
    crate::trading::order_safety::validate(&OrderSafetyInput {
        code: &signal.code,
        side: if signal.direction == Direction::Buy {
            SafetySide::Buy
        } else {
            SafetySide::Sell
        },
        order_price: signal.price,
        quantity: u64::from(signal.quantity),
        available_cash: Some(view.cash.cny()),
        limit_down_price: signal.limit_down_price,
        limit_up_price: signal.limit_up_price,
        secondary_confirmed: signal.secondary_confirmed,
    })?;
    if signal.risk_context.data_mode == crate::monitor::data_mode::DataMode::Unsafe {
        return Err("Unsafe 禁止成交".into());
    }
    use crate::risk::action_gate::{authorize, ActionKind, GateResult};
    if let GateResult::Deny(reason) = authorize(
        if signal.direction == Direction::Buy {
            ActionKind::OpenNew
        } else {
            ActionKind::Reduce
        },
        signal.risk_context.account_mode,
    ) {
        return Err(format!("account gate: {reason}"));
    }
    if signal.direction == Direction::Sell {
        let sellable: u64 = view
            .lots
            .iter()
            .filter(|lot| lot.code == signal.code && lot.sellable_from <= day(now))
            .map(|lot| u64::from(lot.quantity))
            .sum();
        return if sellable >= u64::from(signal.quantity) {
            Ok(())
        } else {
            Err("insufficient sellable inventory / T+1".into())
        };
    }
    let after_cash = view
        .cash
        .sub(notional)
        .and_then(|cash| cash.sub(commission))
        .map_err(|error| error.to_string())?;
    let after_fee_equity = equity.sub(commission).map_err(|error| error.to_string())?;
    if after_cash < Money::ZERO
        || after_fee_equity <= Money::ZERO
        || i128::from(after_cash.0) * 10000
            < i128::from(after_fee_equity.0) * i128::from(policy.cash_floor_bps)
    {
        return Err("post-trade cash floor / insufficient cash".into());
    }
    let existing = view
        .lots
        .iter()
        .filter(|lot| lot.code == signal.code)
        .try_fold(Money::ZERO, |total, lot| {
            total.add(marks[&lot.code].price.mul(lot.quantity)?)
        })
        .map_err(|error: LedgerError| error.to_string())?;
    if ratio_exceeds(
        existing.add(notional).map_err(|error| error.to_string())?,
        equity,
        policy.max_position_bps,
    ) {
        return Err("post-trade concentration exceeds pretrade-equity policy".into());
    }
    Ok(())
}

pub(super) fn apply_fact(state: &mut Projection, fact: &Fact) -> Result<(), LedgerError> {
    match fact {
        Fact::Seeded { .. } => return Err(LedgerError::IntegrityFailure("second genesis".into())),
        Fact::Marked(batch) => {
            if state.inventory_fingerprint()? != batch.inventory_fingerprint {
                return Err(LedgerError::IntegrityFailure(
                    "mark inventory mismatch".into(),
                ));
            }
            state.marks = admit_marks(state, &batch.marks, None)?;
            state.as_of = batch.as_of;
            if batch.closing {
                if state.closes.contains_key(&day(batch.as_of)) {
                    return Err(LedgerError::IdentityConflict);
                }
                state.closes.insert(day(batch.as_of), state.equity()?);
            }
        }
        Fact::Order(order) if order.status == LedgerStatus::Filled => {
            state.cash = state.cash.add(order.cash_delta)?;
            state.fees = state.fees.add(order.commission)?.add(order.stamp)?;
            state.realized_pnl = state.realized_pnl.add(order.realized_delta)?;
            for change in &order.lot_changes {
                if let Some(before) = &change.before {
                    let index = state
                        .lots
                        .iter()
                        .position(|lot| lot.lot_id == before.lot_id && lot == before)
                        .ok_or_else(|| {
                            LedgerError::IntegrityFailure("FIFO before-lot mismatch".into())
                        })?;
                    if let Some(after) = &change.after {
                        state.lots[index] = after.clone();
                    } else {
                        state.lots.remove(index);
                    }
                } else if let Some(after) = &change.after {
                    if state.lots.iter().any(|lot| lot.lot_id == after.lot_id) {
                        return Err(LedgerError::IntegrityFailure(
                            "duplicate lot identity".into(),
                        ));
                    }
                    state.lots.push(after.clone());
                }
            }
            state.marks = order.marks.clone();
            state
                .marks
                .retain(|code, _| state.lots.iter().any(|lot| &lot.code == code));
            state.as_of = order.occurred_at;
            if state.cash < Money::ZERO {
                return Err(LedgerError::IntegrityFailure("negative paper cash".into()));
            }
            state.equity()?;
        }
        Fact::Order(order) => {
            if order.cash_delta != Money::ZERO
                || order.commission != Money::ZERO
                || order.stamp != Money::ZERO
                || order.realized_delta != Money::ZERO
                || !order.lot_changes.is_empty()
            {
                return Err(LedgerError::IntegrityFailure(
                    "nonfill has financial effects".into(),
                ));
            }
        }
    }
    Ok(())
}
