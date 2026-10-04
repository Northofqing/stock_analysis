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
pub(crate) struct OrderFact {
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
    pub(super) before: Option<Lot>,
    pub(super) after: Option<Lot>,
}

fn fresh(at: DateTime<Utc>, now: DateTime<Utc>) -> Result<(), LedgerError> {
    if at > now || now.signed_duration_since(at) > chrono::Duration::seconds(5) {
        return Err(LedgerError::EvidenceUnavailable(
            "realtime quote/mark not fresh after lock acquisition".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod fresh_tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn quote_freshness_uses_exact_inclusive_five_second_window() {
        let now = Utc.with_ymd_and_hms(2026, 9, 29, 6, 30, 0).unwrap();
        assert!(fresh(now, now).is_ok());
        assert!(fresh(now - chrono::Duration::seconds(5), now).is_ok());
        assert!(matches!(
            fresh(now + chrono::Duration::nanoseconds(1), now),
            Err(LedgerError::EvidenceUnavailable(_))
        ));
        assert!(matches!(
            fresh(
                now - chrono::Duration::seconds(5) - chrono::Duration::nanoseconds(1),
                now
            ),
            Err(LedgerError::EvidenceUnavailable(_))
        ));
    }
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
fn admit_marks( state: &Projection, marks: &[Mark], extra: Option<&str>, ) -> Result<BTreeMap<String, Mark>, LedgerError> {
    fw::historical(admit_marks_with_work(state, marks, extra, &mut FinancialWork::Historical))
}
fn admit_marks_with_work( state: &Projection, marks: &[Mark], extra: Option<&str>, w: &mut FinancialWork<'_, '_>) -> fw::Result<BTreeMap<String, Mark>> {
    w.finish()?;
    let mut required=std::collections::BTreeSet::new();
    for lot in &state.lots{
        w.set(&mut required, lot.code.as_str())?;
    }
    if let Some(code)=extra{
        w.set(&mut required, code)?;
    }
    let mut admitted = BTreeMap::new();
    for mark in marks {
        if !required.contains(mark.code.as_str()) || mark.price <= Money::ZERO || mark.source.trim().is_empty() || {
            let key=w.copy(&mark.code)?;
            let value=w.copy(mark)?;
            let duplicate=admitted.contains_key(&key);
            w.insert(&mut admitted, key, value)?;
            duplicate
        }
        {
            return Err(w.error(Txt::V1(fw::V1Text::InvalidDuplicateExtraneousValuationMark))?);
        }
    }
    if required.len() != admitted.len() {
        return Err(w.error(Txt::V1(fw::V1Text::IncompleteWholeAccountValuation))?);
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
        view.require_available()?;
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
        view.require_available()?;
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
    fw::historical(apply_fact_with_work(state, fact, &mut FinancialWork::Historical))
}
pub(super) fn apply_fact_with_work(state: &mut Projection, fact: &Fact, work: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
    match fact {
        Fact::DerivedSnapshotV1(revision) => super::snapshot::validate_with_work(revision, work)?,
        Fact::AdjudicatedV1(ruling) => *state = work.copy(&ruling.projection)?,
        Fact::Seeded {
            ..
        } => return Err(work.error(Txt::V1(fw::V1Text::SecondGenesis))?),
        Fact::Marked(batch) => apply_marked_with_work(state, batch, work)?,
        Fact::Order(order) => apply_order_with_work(state, order, work)?,
    }
    Ok(())
}

// Finite replay DTO seeds stay with the owners of private fields.
#[allow(dead_code, non_camel_case_types)]
mod replay_codec_owner {
    use super::*;
    use crate::trading::paper_replay_codec_v1 as c;
    use crate::trading::paper_replay_shapes_v1 as s;
    use serde::de::{EnumAccess as _, VariantAccess as _};
    impl c::sealed::Value for PriceIntent {}
    impl c::Value for PriceIntent {
        const SHAPE: s::Shape = s::Shape::External(&[
            s::Variant {
                name: "FixedSignalPriceV1",
                body: s::Body::Unit,
            },
            s::Variant {
                name: "SignalQuoteMarketV1",
                body: s::Body::Unit,
            },
        ]);
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            struct Seed_FixedSignalPriceV1<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_FixedSignalPriceV1<'de, '_, '_, '_> {
                type Value = PriceIntent;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(PriceIntent::FixedSignalPriceV1)
                }
            }
            struct Seed_SignalQuoteMarketV1<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::DeserializeSeed<'de> for Seed_SignalQuoteMarketV1<'de, '_, '_, '_> {
                type Value = PriceIntent;
                fn deserialize<D: serde::Deserializer<'de>>(
                    self,
                    de: D,
                ) -> Result<Self::Value, D::Error> {
                    c::adjacent_unit(de)?;
                    Ok(PriceIntent::SignalQuoteMarketV1)
                }
            }
            struct EV<'de, 'w, 'loan, 'pool>(c::Input<'de, 'w, 'loan, 'pool>);
            impl<'de> serde::de::Visitor<'de> for EV<'de, '_, '_, '_> {
                type Value = PriceIntent;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    c::expected(f)
                }
                fn visit_enum<A: serde::de::EnumAccess<'de>>(
                    mut self,
                    a: A,
                ) -> Result<Self::Value, A::Error> {
                    let (tag, value) = a.variant_seed(c::KeySeed {
                        names: &["FixedSignalPriceV1", "SignalQuoteMarketV1"],
                    })?;
                    let object = self.0.bytes[self.0.span.start] == b'{';
                    let span = if object {
                        self.0
                            .span
                            .children(self.0.bytes)
                            .next()
                            .ok_or_else(c::span_error)?
                            .1
                    } else {
                        self.0.span
                    };
                    let origin = self.0.origin;
                    match tag {
                        "FixedSignalPriceV1" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(PriceIntent::FixedSignalPriceV1)
                        }
                        "SignalQuoteMarketV1" => {
                            if object {
                                value.newtype_variant_seed(c::UnitPayload(
                                    self.0.child(span, origin),
                                ))?;
                            } else {
                                value.unit_variant()?;
                            }
                            Ok(PriceIntent::SignalQuoteMarketV1)
                        }
                        _ => Err(self.0.error(c::K::UnknownVariant, "codec variant")),
                    }
                }
            }
            de.deserialize_enum(
                "codec",
                &["FixedSignalPriceV1", "SignalQuoteMarketV1"],
                EV(input),
            )
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(match self {
                PriceIntent::FixedSignalPriceV1 => PriceIntent::FixedSignalPriceV1,
                PriceIntent::SignalQuoteMarketV1 => PriceIntent::SignalQuoteMarketV1,
            })
        }
    }
    impl c::sealed::Value for ValuationBatch {}
    impl c::Value for ValuationBatch {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "binding",
                    shape: &<AccountBinding as c::Value>::SHAPE,
                    optional: <AccountBinding as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "command_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "expected_version",
                    shape: &<i64 as c::Value>::SHAPE,
                    optional: <i64 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "inventory_fingerprint",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "as_of",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "closing",
                    shape: &<bool as c::Value>::SHAPE,
                    optional: <bool as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "marks",
                    shape: &<Vec<Mark> as c::Value>::SHAPE,
                    optional: <Vec<Mark> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
            ],
            false,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,ValuationBatch,true,{binding:AccountBinding=>false,command_id:String=>false,expected_version:i64=>false,inventory_fingerprint:String=>false,as_of:DateTime<Utc> =>false,closing:bool=>false,marks:Vec<Mark> =>false},ValuationBatch{binding,command_id,expected_version,inventory_fingerprint,as_of,closing,marks})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(ValuationBatch {
                binding: c::Value::paid_copy(&self.binding, w)?,
                command_id: c::Value::paid_copy(&self.command_id, w)?,
                expected_version: c::Value::paid_copy(&self.expected_version, w)?,
                inventory_fingerprint: c::Value::paid_copy(&self.inventory_fingerprint, w)?,
                as_of: c::Value::paid_copy(&self.as_of, w)?,
                closing: c::Value::paid_copy(&self.closing, w)?,
                marks: c::Value::paid_copy(&self.marks, w)?,
            })
        }
    }
    impl c::sealed::Value for OrderFact {}
    impl c::Value for OrderFact {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "plan_id",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "intent_hash",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "code",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "direction",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "requested_price",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "price_intent",
                    shape: &<PriceIntent as c::Value>::SHAPE,
                    optional: <PriceIntent as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "quantity",
                    shape: &<u32 as c::Value>::SHAPE,
                    optional: <u32 as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "quote_price",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "quote_observed_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "account_mode",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "data_mode",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "decision_basis",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "source_evidence",
                    shape: &<String as c::Value>::SHAPE,
                    optional: <String as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "occurred_at",
                    shape: &<DateTime<Utc> as c::Value>::SHAPE,
                    optional: <DateTime<Utc> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "status",
                    shape: &<LedgerStatus as c::Value>::SHAPE,
                    optional: <LedgerStatus as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "reason",
                    shape: &<Option<String> as c::Value>::SHAPE,
                    optional: <Option<String> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "cash_delta",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "commission",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "stamp",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "realized_delta",
                    shape: &<Money as c::Value>::SHAPE,
                    optional: <Money as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "lot_changes",
                    shape: &<Vec<LotChange> as c::Value>::SHAPE,
                    optional: <Vec<LotChange> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "marks",
                    shape: &<BTreeMap<String, Mark> as c::Value>::SHAPE,
                    optional: <BTreeMap<String, Mark> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "paper_trade_id",
                    shape: &<Option<i64> as c::Value>::SHAPE,
                    optional: <Option<i64> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "audit",
                    shape: &<AuditLink as c::Value>::SHAPE,
                    optional: <AuditLink as c::Value>::OPTIONAL,
                    positional_default: false,
                },
            ],
            false,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,OrderFact,true,{plan_id:String=>false,intent_hash:String=>false,code:String=>false,direction:String=>false,requested_price:Money=>false,price_intent:PriceIntent=>false,quantity:u32=>false,quote_price:Money=>false,quote_observed_at:DateTime<Utc> =>false,account_mode:String=>false,data_mode:String=>false,decision_basis:String=>false,source_evidence:String=>false,occurred_at:DateTime<Utc> =>false,status:LedgerStatus=>false,reason:Option<String> =>false,cash_delta:Money=>false,commission:Money=>false,stamp:Money=>false,realized_delta:Money=>false,lot_changes:Vec<LotChange> =>false,marks:BTreeMap<String, Mark> =>false,paper_trade_id:Option<i64> =>false,audit:AuditLink=>false},OrderFact{plan_id,intent_hash,code,direction,requested_price,price_intent,quantity,quote_price,quote_observed_at,account_mode,data_mode,decision_basis,source_evidence,occurred_at,status,reason,cash_delta,commission,stamp,realized_delta,lot_changes,marks,paper_trade_id,audit})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(OrderFact {
                plan_id: c::Value::paid_copy(&self.plan_id, w)?,
                intent_hash: c::Value::paid_copy(&self.intent_hash, w)?,
                code: c::Value::paid_copy(&self.code, w)?,
                direction: c::Value::paid_copy(&self.direction, w)?,
                requested_price: c::Value::paid_copy(&self.requested_price, w)?,
                price_intent: c::Value::paid_copy(&self.price_intent, w)?,
                quantity: c::Value::paid_copy(&self.quantity, w)?,
                quote_price: c::Value::paid_copy(&self.quote_price, w)?,
                quote_observed_at: c::Value::paid_copy(&self.quote_observed_at, w)?,
                account_mode: c::Value::paid_copy(&self.account_mode, w)?,
                data_mode: c::Value::paid_copy(&self.data_mode, w)?,
                decision_basis: c::Value::paid_copy(&self.decision_basis, w)?,
                source_evidence: c::Value::paid_copy(&self.source_evidence, w)?,
                occurred_at: c::Value::paid_copy(&self.occurred_at, w)?,
                status: c::Value::paid_copy(&self.status, w)?,
                reason: c::Value::paid_copy(&self.reason, w)?,
                cash_delta: c::Value::paid_copy(&self.cash_delta, w)?,
                commission: c::Value::paid_copy(&self.commission, w)?,
                stamp: c::Value::paid_copy(&self.stamp, w)?,
                realized_delta: c::Value::paid_copy(&self.realized_delta, w)?,
                lot_changes: c::Value::paid_copy(&self.lot_changes, w)?,
                marks: c::Value::paid_copy(&self.marks, w)?,
                paper_trade_id: c::Value::paid_copy(&self.paper_trade_id, w)?,
                audit: c::Value::paid_copy(&self.audit, w)?,
            })
        }
    }
    impl c::sealed::Value for LotChange {}
    impl c::Value for LotChange {
        const SHAPE: s::Shape = s::Shape::Record(
            &[
                s::Field {
                    name: "before",
                    shape: &<Option<Lot> as c::Value>::SHAPE,
                    optional: <Option<Lot> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
                s::Field {
                    name: "after",
                    shape: &<Option<Lot> as c::Value>::SHAPE,
                    optional: <Option<Lot> as c::Value>::OPTIONAL,
                    positional_default: false,
                },
            ],
            false,
            true,
        );
        fn read<'de, D: serde::Deserializer<'de>>(
            de: D,
            input: c::Input<'de, '_, '_, '_>,
        ) -> Result<Self, D::Error> {
            c::record_read!(de,input,LotChange,true,{before:Option<Lot> =>false,after:Option<Lot> =>false},LotChange{before,after})
        }
        fn paid_copy(
            &self,
            w: &mut c::CodecMechanics<'_, '_>,
        ) -> Result<Self, c::ReplayTerminalFailure> {
            w.finish()?;
            Ok(LotChange {
                before: c::Value::paid_copy(&self.before, w)?,
                after: c::Value::paid_copy(&self.after, w)?,
            })
        }
    }
    impl c::sealed::Element for LotChange {}
    impl c::ArrayElement for LotChange {}
}
#[cfg(test)]
pub(crate) fn replay_codec_fixtures(
    case: crate::trading::paper_replay_codec_v1::CodecFixtureCase,
    work: &mut crate::database::global_schema_v1::replay_work::CodecMechanics<'_, '_>,
) {
}

fn apply_marked_with_work(state:&mut Projection, batch:&ValuationBatch, w:&mut FinancialWork<'_, '_>)->fw::Result<()>{
    w.finish()?;
    if state.inventory_fingerprint_with_work(w)? != batch.inventory_fingerprint {
        return Err(w.error(Txt::V1(fw::V1Text::MarkInventoryMismatch))?);
    }
    state.marks = admit_marks_with_work(state, &batch.marks, None, w)?;
    state.as_of = batch.as_of;
    if batch.closing {
        if state.closes.contains_key(&day(batch.as_of)) {
            return Err(LedgerError::IdentityConflict.into());
        }
        let equity=state.equity_with_work(w)?;
        w.insert(&mut state.closes, day(batch.as_of), equity)?;
    }
    Ok(())
}
fn apply_order_with_work(state:&mut Projection, order:&OrderFact, w:&mut FinancialWork<'_, '_>)->fw::Result<()>{
    w.finish()?;
    if order.status==LedgerStatus::Filled {
        state.cash = state.cash.add(order.cash_delta)?;
        state.fees = state.fees.add(order.commission)?.add(order.stamp)?;
        state.realized_pnl = state.realized_pnl.add(order.realized_delta)?;
        for change in &order.lot_changes {
            if let Some(before) = &change.before {
                let index=w.option(state.lots.iter().position(|lot|lot.lot_id==before.lot_id&&lot==before), Txt::V1(fw::V1Text::FIFOBeforeLotMismatch))?;
                if let Some(after) = &change.after {
                    state.lots[index] = w.copy(after)?;
                } else {
                    state.lots.remove(index);
                }
            } else if let Some(after) = &change.after {
                if state.lots.iter().any(|lot| lot.lot_id == after.lot_id) {
                    return Err(w.error(Txt::V1(fw::V1Text::DuplicateLotIdentity))?);
                }
                let incoming=w.copy(after)?;
                w.push(&mut state.lots, incoming)?;
            }
        }
        state.marks = w.copy(&order.marks)?;
        state .marks .retain(|code, _| state.lots.iter().any(|lot| &lot.code == code));
        state.as_of = order.occurred_at;
        if state.cash < Money::ZERO {
            return Err(w.error(Txt::V1(fw::V1Text::NegativePaperCash))?);
        }
        state.equity_with_work(w)?;
    } else{
        if order.cash_delta != Money::ZERO || order.commission != Money::ZERO || order.stamp != Money::ZERO || order.realized_delta != Money::ZERO || !order.lot_changes.is_empty() {
            return Err(w.error(Txt::V1(fw::V1Text::NonfillHasFinancialEffects))?);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn transition_fixture(mut paid:Projection, w:&mut FinancialWork<'_, '_>){
    let mut plain=paid.clone();
    let batch=ValuationBatch{
        binding:AccountBinding{
            account_id:"a".into(),
            epoch_id:"e".into(),
            manifest_hash:"h".into()
        },
        command_id:"mark".into(),
        expected_version:1,
        inventory_fingerprint:paid.inventory_fingerprint().unwrap(),
        as_of:paid.as_of,
        closing:true,
        marks:paid.marks.values().cloned().collect()
    };
    apply_marked_with_work(&mut paid, &batch, w).unwrap();
    apply_fact(&mut plain, &Fact::Marked(batch.clone())).unwrap();
    assert_eq!(paid, plain);
    assert_eq!(paid.closes.len(), 1);
    let before=paid.lots[0].clone();
    let mut after=before.clone();
    after.quantity+=100;
    let mut order=OrderFact{
        plan_id:"p".into(),
        intent_hash:"h".into(),
        code:before.code.clone(),
        direction:"Buy".into(),
        requested_price:before.basis_price,
        price_intent:PriceIntent::FixedSignalPriceV1,
        quantity:100,
        quote_price:before.basis_price,
        quote_observed_at:paid.as_of,
        account_mode:"test".into(),
        data_mode:"test".into(),
        decision_basis:"fixture".into(),
        source_evidence:"fixture".into(),
        occurred_at:paid.as_of,
        status:LedgerStatus::Filled,
        reason:None,
        cash_delta:Money::from_micros(-1_005_000_000),
        commission:Money::from_micros(5_000_000),
        stamp:Money::ZERO,
        realized_delta:Money::ZERO,
        lot_changes:vec![LotChange{
            before:Some(before),
            after:Some(after)
        } ],
        marks:paid.marks.clone(),
        paper_trade_id:None,
        audit:AuditLink{
            id:1,
            previous_hash:"p".into(),
            record_hash:"r".into(),
            created_at:"fixture".into()
        }
    };
    apply_order_with_work(&mut paid, &order, w).unwrap();
    apply_fact(&mut plain, &Fact::Order(order.clone())).unwrap();
    assert_eq!(paid, plain);
    assert_eq!(paid.lots[0].quantity, 200);
    let mut added=paid.lots[0].clone();
    added.lot_id="second".into();
    added.quantity=100;
    order.lot_changes=vec![LotChange{
        before:None,
        after:Some(added)
    } ];
    apply_order_with_work(&mut paid, &order, w).unwrap();
    apply_fact(&mut plain, &Fact::Order(order.clone())).unwrap();
    assert_eq!(paid, plain);
    assert_eq!(paid.lots.len(), 2);
    order.lot_changes=vec![LotChange{
        before:Some(paid.lots[1].clone()),
        after:None
    } ];
    order.cash_delta=Money::from_micros(995_000_000);
    apply_order_with_work(&mut paid, &order, w).unwrap();
    apply_fact(&mut plain, &Fact::Order(order.clone())).unwrap();
    assert_eq!(paid, plain);
    assert_eq!(paid.lots.len(), 1);
    order.status=LedgerStatus::NotFilled;
    order.cash_delta=Money::ZERO;
    order.commission=Money::ZERO;
    order.lot_changes.clear();
    let frozen=paid.clone();
    apply_order_with_work(&mut paid, &order, w).unwrap();
    assert_eq!(paid, frozen);
    order.cash_delta=Money::from_micros(1);
    let error=apply_order_with_work(&mut paid, &order, w).unwrap_err();
    assert!(matches!(error, FinancialFailure::Financial(LedgerError::IntegrityFailure(_))));
    assert_eq!(paid, frozen);
}
