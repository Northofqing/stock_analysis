//! Versioned economic rulings in the existing paper event chain.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillFingerprint {
    pub paper_trade_id: i64,
    pub plan_id: String,
    pub event_hash: String,
    pub raw_trade_hash: String,
    pub audit_hash: String,
    pub fact_at: DateTime<Utc>,
    /// No fake terminal hash: pre-audit historical rows are marked explicitly.
    pub legacy_before_cutover: bool,
    pub legacy_no_terminal: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdjudicationAction {
    Quarantine,
    CorrectionDeclared {
        price: Money,
        quantity: u32,
        fact_at: DateTime<Utc>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adjudication {
    pub binding: AccountBinding,
    pub request_id: String,
    pub expected_version: i64,
    pub expected_head: String,
    pub expected_predecessor: Option<String>,
    pub original: FillFingerprint,
    pub action: AdjudicationAction,
    pub reason: String,
    pub evidence: String,
    pub operator: String,
    pub source: String,
    pub decision_at: DateTime<Utc>,
}

impl PaperLedger<'_> {
    pub fn fill_fingerprint(
        &self,
        binding: &AccountBinding,
        id: i64,
    ) -> Result<FillFingerprint, LedgerError> {
        let mut conn = self
            .db
            .get_conn()
            .map_err(|e| LedgerError::Database(e.to_string()))?;
        conn.transaction(|conn| {
            effective::verify_catalog(conn)?;
            load(conn, binding)?;
            fingerprint(conn, binding, id)
        })
    }
    /// Explicit operator action only. Preview and apply use the identical validator.
    pub fn adjudicate(&self, request: Adjudication) -> Result<PaperReceipt, LedgerError> {
        self.apply(PaperCommand::Adjudicate(request))
    }
    pub(super) fn adjudicate_on(
        &self,
        conn: &mut SqliteConnection,
        request: Adjudication,
        now: DateTime<Utc>,
    ) -> Result<PaperReceipt, LedgerError> {
        effective::verify_catalog(conn)?;
        let view = load(conn, &request.binding)?;
        for row in events(conn, &request.binding.account_id)? {
            if row.command_id == request.request_id {
                let fact: Fact = decode(&row.payload)?;
                return match &fact {
                    Fact::AdjudicatedV1(original) if original.request == request => {
                        Ok(receipt(row.seq, row.event_hash, &fact, true))
                    }
                    _ => Err(LedgerError::IdentityConflict),
                };
            }
        }
        let fact = prepare(conn, &request, &view, now)?;
        append(
            conn,
            &request.binding,
            &request.request_id,
            &view,
            Fact::AdjudicatedV1(fact),
        )
    }
    pub fn preview_adjudication(
        &self,
        request: &Adjudication,
    ) -> Result<AdjudicationPreview, LedgerError> {
        let mut conn = self
            .db
            .get_conn()
            .map_err(|e| LedgerError::Database(e.to_string()))?;
        conn.transaction(|conn| {
            effective::verify_catalog(conn)?;
            let view = load(conn, &request.binding)?;
            let fact = prepare(conn, request, &view, (self.clock)())?;
            let historical_scope =
                if let Some((after_hash, unavailable)) = &fact.historical_projection {
                    let rows = events(conn, &request.binding.account_id)?;
                    let (before_hash, _) =
                        effective::legacy_result(conn, &request.binding, &rows, None)?;
                    Some(HistoricalProjectionImpact {
                        scope: EffectiveFillScope::LegacyBeforeCutover(request.binding.clone()),
                        identity_version: "LegacyEconomicV1".into(),
                        before_hash,
                        after_hash: after_hash.clone(),
                        unavailable: unavailable.clone(),
                    })
                } else {
                    None
                };
            Ok(AdjudicationPreview {
                current_account: AccountProjectionImpact {
                    changed: fact.projection != view.projection,
                    projection_hash: digest(&encode(&fact.projection)?),
                    unavailable: fact.projection.economic_unavailable.clone(),
                    cash: fact.projection.cash,
                    fees: fact.projection.fees,
                },
                historical_scope,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjudicationPreview {
    pub current_account: AccountProjectionImpact,
    pub historical_scope: Option<HistoricalProjectionImpact>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProjectionImpact {
    pub changed: bool,
    pub projection_hash: String,
    pub unavailable: Option<String>,
    pub cash: Money,
    pub fees: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoricalProjectionImpact {
    pub scope: EffectiveFillScope,
    /// This historical economic hash is not a current-account/receipt hash.
    pub identity_version: String,
    pub before_hash: String,
    pub after_hash: String,
    /// The same FIFO/T+1/dependent-sell diagnostic frozen by apply.
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdjudicatedFact {
    pub request: Adjudication,
    pub projection: Projection,
    pub historical_projection: Option<(String, Option<String>)>,
}

#[derive(QueryableByName)]
pub(super) struct RawBytes {
    #[diesel(sql_type = Text)]
    pub bytes: String,
}

pub(super) fn raw_bytes(conn: &mut SqliteConnection, id: i64) -> Result<String, LedgerError> {
    diesel::sql_query("SELECT json_array(id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,virtual_reason,account_mode,data_mode,CAST(ts AS TEXT),CAST(updated_at AS TEXT)) AS bytes FROM paper_trades WHERE id=?")
        .bind::<BigInt,_>(id).get_result::<RawBytes>(conn).optional()?.map(|r|r.bytes)
        .ok_or_else(||LedgerError::IntegrityFailure("original raw fill missing".into()))
}
pub(super) fn fingerprint(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    id: i64,
) -> Result<FillFingerprint, LedgerError> {
    fingerprint_with_audit_guard(conn, binding, id, &mut V1AuditReplayGuard::default())
}

fn fingerprint_with_audit_guard(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    id: i64,
    audit_guard: &mut V1AuditReplayGuard,
) -> Result<FillFingerprint, LedgerError> {
    audit_guard.ensure_validated(conn)?;
    for row in events(conn, &binding.account_id)? {
        if let Fact::Order(order) = decode(&row.payload)? {
            if order.paper_trade_id == Some(id) && order.status == LedgerStatus::Filled {
                let raw = raw_bytes(conn, id)?;
                let audit = diesel::sql_query(
                    "SELECT record_hash AS bytes FROM order_audit_chain WHERE order_audit_id=?",
                )
                .bind::<BigInt, _>(order.audit.id)
                .get_result::<RawBytes>(conn)?;
                if audit.bytes != order.audit.record_hash {
                    return Err(LedgerError::IntegrityFailure(
                        "original audit fingerprint mismatch".into(),
                    ));
                }
                let terminal = diesel::sql_query("SELECT id,business_order_id,source,decision_basis,side,code,requested_price,execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at FROM order_audit WHERE id=?")
                    .bind::<BigInt,_>(order.audit.id).get_result::<crate::database::order_audit::CanonicalOrderAuditRow>(conn)?;
                let raw_values: Vec<serde_json::Value> = decode(&raw)?;
                let raw_time = raw_values.get(13).and_then(|v| v.as_str()).ok_or_else(|| {
                    LedgerError::IntegrityFailure("raw fill timestamp missing".into())
                })?;
                use chrono::TimeZone;
                let at = crate::trading::paper_lot_ledger::parse_paper_fill_timestamp(id, raw_time)
                    .map_err(LedgerError::IntegrityFailure)?;
                let at = chrono::FixedOffset::east_opt(8 * 3600)
                    .unwrap()
                    .from_local_datetime(&at)
                    .single()
                    .ok_or_else(|| {
                        LedgerError::IntegrityFailure("raw fill timestamp ambiguous".into())
                    })?
                    .with_timezone(&Utc);
                let exact = raw_values.len() == 15
                    && raw_values[1].as_str() == Some(order.plan_id.as_str())
                    && raw_values[2].as_str() == Some(order.code.as_str())
                    && raw_values[4].as_str() == Some(order.direction.as_str())
                    && raw_values[5].as_f64().map(f64::to_bits) == Some(terminal.requested_price.to_bits())
                    && raw_values[6].as_u64() == Some(u64::from(order.quantity))
                    && raw_values[7].as_str() == Some("Filled")
                    && raw_values[8].as_f64().map(f64::to_bits) == Some(order.requested_price.cny().to_bits())
                    && raw_values[9].is_null()
                    && raw_values[10].as_str().is_some_and(|v| !v.is_empty() && order.decision_basis.starts_with(&format!("{v} | PaperLedgerV1 account=")))
                    && raw_values[11].as_str() == Some(order.account_mode.as_str())
                    && raw_values[12].as_str() == Some(order.data_mode.as_str())
                    // Compat row records milliseconds; event preserves clock precision.
                    && at.timestamp_millis() == order.occurred_at.timestamp_millis()
                    && terminal.business_order_id == order.plan_id && terminal.source == "PaperTrade"
                    && terminal.decision_basis == order.decision_basis && terminal.side == order.direction
                    && terminal.code == order.code && terminal.quantity == i64::from(order.quantity)
                    && terminal.execution_price.map(f64::to_bits) == Some(order.requested_price.cny().to_bits())
                    && terminal.outcome == "Filled" && terminal.failure_reason.is_none();
                if !exact {
                    return Err(LedgerError::IntegrityFailure(
                        "raw fill contradicts immutable event/terminal".into(),
                    ));
                }
                return Ok(FillFingerprint {
                    paper_trade_id: id,
                    plan_id: order.plan_id,
                    event_hash: row.event_hash,
                    raw_trade_hash: digest(&raw),
                    audit_hash: audit.bytes,
                    fact_at: order.occurred_at,
                    legacy_before_cutover: false,
                    legacy_no_terminal: false,
                });
            }
        }
    }
    let source = effective::legacy_source(conn, binding)?;
    let original = source
        .fills()
        .iter()
        .find(|fill| fill.fill().id == id)
        .ok_or_else(|| {
            LedgerError::InvalidInput(
                "original fill outside bound account/epoch/legacy prefix".into(),
            )
        })?;
    let row = original.fill();
    let at = crate::trading::paper_lot_ledger::parse_paper_fill_timestamp(id, &row.occurred_at)
        .map_err(LedgerError::IntegrityFailure)?;
    let at = at.and_utc();
    Ok(FillFingerprint {
        paper_trade_id: id,
        plan_id: row.plan_id.clone(),
        event_hash: String::new(),
        raw_trade_hash: digest(&raw_bytes(conn, id)?),
        audit_hash: original
            .terminal_audit_hash()
            .unwrap_or_default()
            .to_string(),
        fact_at: at,
        legacy_before_cutover: true,
        legacy_no_terminal: original.terminal_audit_hash().is_none(),
    })
}

pub(super) fn latest_rulings(
    rows: &[EventRow],
) -> Result<BTreeMap<i64, AdjudicationAction>, LedgerError> {
    let mut result = BTreeMap::new();
    for row in rows {
        if let Fact::AdjudicatedV1(fact) = decode(&row.payload)? {
            result.insert(fact.request.original.paper_trade_id, fact.request.action);
        }
    }
    Ok(result)
}
fn predecessor(rows: &[EventRow], id: i64) -> Result<Option<String>, LedgerError> {
    let mut result = None;
    for row in rows {
        if let Fact::AdjudicatedV1(fact) = decode(&row.payload)? {
            if fact.request.original.paper_trade_id == id {
                result = Some(row.event_hash.clone());
            }
        }
    }
    Ok(result)
}

fn prepare(
    conn: &mut SqliteConnection,
    request: &Adjudication,
    view: &PaperView,
    now: DateTime<Utc>,
) -> Result<AdjudicatedFact, LedgerError> {
    if request.expected_version != view.version || request.expected_head != view.event_hash {
        return Err(LedgerError::VersionChanged);
    }
    if request.decision_at > now
        || [
            &request.request_id,
            &request.reason,
            &request.evidence,
            &request.operator,
            &request.source,
        ]
        .iter()
        .any(|v| v.trim().is_empty())
    {
        return Err(LedgerError::InvalidInput("ruling requires explicit identity, reason, evidence, operator/source and nonfuture decision".into()));
    }
    if request.original != fingerprint(conn, &request.binding, request.original.paper_trade_id)? {
        return Err(LedgerError::IntegrityFailure(
            "ruling source fingerprint mismatch".into(),
        ));
    }
    validate_fact_time(request)?;
    let rows = events(conn, &request.binding.account_id)?;
    if predecessor(&rows, request.original.paper_trade_id)? != request.expected_predecessor {
        return Err(LedgerError::VersionChanged);
    }
    let projection = recompute(&rows, request, &view.projection)?;
    let historical_projection = if request.original.legacy_before_cutover {
        Some(effective::legacy_result(
            conn,
            &request.binding,
            &rows,
            Some(request),
        )?)
    } else {
        None
    };
    Ok(AdjudicatedFact {
        request: request.clone(),
        projection,
        historical_projection,
    })
}

pub(super) fn verify_ruling(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    seq: i64,
    previous: &str,
    fact: &AdjudicatedFact,
    catalog_already_verified: bool,
    audit_guard: Option<&mut V1AuditReplayGuard>,
    before: &Projection,
) -> Result<(), LedgerError> {
    if !catalog_already_verified {
        effective::verify_catalog(conn)?;
    }
    let request = &fact.request;
    validate_fact_time(request)?;
    if request.binding != *binding
        || request.expected_version != seq - 1
        || request.expected_head != previous
    {
        return Err(LedgerError::IntegrityFailure(
            "ruling binding/source/predecessor mismatch".into(),
        ));
    }
    let original = match audit_guard {
        Some(guard) => fingerprint_with_audit_guard(
            conn,
            binding,
            request.original.paper_trade_id,
            guard,
        )?,
        None => fingerprint(conn, binding, request.original.paper_trade_id)?,
    };
    if request.original != original {
        return Err(LedgerError::IntegrityFailure(
            "ruling binding/source/predecessor mismatch".into(),
        ));
    }
    let rows = events(conn, &binding.account_id)?
        .into_iter()
        .filter(|r| r.seq < seq)
        .collect::<Vec<_>>();
    if predecessor(&rows, request.original.paper_trade_id)? != request.expected_predecessor {
        return Err(LedgerError::IntegrityFailure(
            "ruling predecessor mismatch".into(),
        ));
    }
    // For unavailable projections we preserve the verified pre-ruling state; it
    // remains diagnostic only and cannot authorize execution.
    if recompute(&rows, request, before)? != fact.projection {
        return Err(LedgerError::IntegrityFailure(
            "ruling projection mismatch".into(),
        ));
    }
    let historical = if request.original.legacy_before_cutover {
        Some(effective::legacy_result(
            conn,
            binding,
            &rows,
            Some(request),
        )?)
    } else {
        None
    };
    if historical != fact.historical_projection {
        return Err(LedgerError::IntegrityFailure(
            "historical ruling projection mismatch".into(),
        ));
    }
    Ok(())
}

fn validate_fact_time(request: &Adjudication) -> Result<(), LedgerError> {
    if let AdjudicationAction::CorrectionDeclared { fact_at, .. } = &request.action {
        if *fact_at > request.decision_at {
            return Err(LedgerError::InvalidInput(
                "corrected fact cannot follow decision time".into(),
            ));
        }
        if !crate::calendar::verified_a_share_trading_day(day(*fact_at))
            .map_err(LedgerError::EvidenceUnavailable)?
        {
            return Err(LedgerError::InvalidInput(
                "corrected execution must be on a verified trading day".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn fee(price: Money, quantity: u32, side: &str) -> Result<Money, LedgerError> {
    let notional = price.mul(quantity)?;
    let commission = rate(notional, 3, 10000)?.max(Money(5_000_000));
    commission.add(if side == "sell" {
        rate(notional, 1, 1000)?
    } else {
        Money::ZERO
    })
}
fn rate(amount: Money, numerator: i64, denominator: i64) -> Result<Money, LedgerError> {
    i64::try_from(
        (i128::from(amount.0) * i128::from(numerator) + i128::from(denominator / 2))
            / i128::from(denominator),
    )
    .map(Money)
    .map_err(|_| LedgerError::Overflow)
}

fn recompute(
    rows: &[EventRow],
    request: &Adjudication,
    before: &Projection,
) -> Result<Projection, LedgerError> {
    if request.original.legacy_before_cutover {
        return Ok(before.clone());
    }
    let mut actions = latest_rulings(rows)?;
    actions.insert(request.original.paper_trade_id, request.action.clone());
    let result = recompute_available(rows, &actions);
    match result {
        Ok(state) => Ok(state),
        Err(LedgerError::EvidenceUnavailable(reason)) => {
            let mut state = before.clone();
            state.economic_unavailable = Some(format!(
                "ruling={} dependency unavailable: {reason}",
                request.request_id
            ));
            Ok(state)
        }
        Err(error) => Err(error),
    }
}

fn recompute_available(
    rows: &[EventRow],
    actions: &BTreeMap<i64, AdjudicationAction>,
) -> Result<Projection, LedgerError> {
    let Fact::Seeded { manifest, .. } = decode(
        &rows
            .first()
            .ok_or_else(|| LedgerError::IntegrityFailure("missing seed".into()))?
            .payload,
    )?
    else {
        return Err(LedgerError::IntegrityFailure("missing seed".into()));
    };
    let mut state = seed_projection(&manifest)?;
    let mut fills = Vec::new();
    let mut market = Vec::new();
    for row in rows {
        match decode::<Fact>(&row.payload)? {
            Fact::Order(order) if order.status == LedgerStatus::Filled => {
                let id = order
                    .paper_trade_id
                    .ok_or_else(|| LedgerError::IntegrityFailure("fill identity absent".into()))?;
                market.push((order.occurred_at, false, order.marks.clone()));
                let (price, quantity, at) = match actions.get(&id) {
                    Some(AdjudicationAction::Quarantine) => continue,
                    Some(AdjudicationAction::CorrectionDeclared {
                        price,
                        quantity,
                        fact_at,
                    }) => (*price, *quantity, *fact_at),
                    None => (order.requested_price, order.quantity, order.occurred_at),
                };
                if price <= Money::ZERO
                    || quantity == 0
                    || !quantity.is_multiple_of(100)
                    || at < manifest.cutover_at
                {
                    return Err(LedgerError::InvalidInput(
                        "correction price/quantity/time outside epoch".into(),
                    ));
                }
                fills.push((
                    at,
                    id,
                    row.command_id.clone(),
                    order.clone(),
                    price,
                    quantity,
                ));
            }
            Fact::Marked(batch) => market.push((
                batch.as_of,
                batch.closing,
                batch
                    .marks
                    .into_iter()
                    .map(|m| (m.code.clone(), m))
                    .collect(),
            )),
            _ => {}
        }
    }
    fills.sort_by_key(|f| (f.0, f.1));
    market.sort_by_key(|m| m.0);
    let mut fills = fills.into_iter().peekable();
    for (at, closing, marks) in market {
        while fills.peek().is_some_and(|f| f.0 <= at) {
            let (time, id, command, order, price, quantity) = fills.next().unwrap();
            apply_economics(&mut state, id, &command, &order, price, quantity, time)?;
        }
        state.marks.extend(marks);
        state.as_of = state.as_of.max(at);
        if closing {
            state.closes.insert(day(at), state.equity()?);
        }
    }
    for (time, id, command, order, price, quantity) in fills {
        apply_economics(&mut state, id, &command, &order, price, quantity, time)?;
    }
    state
        .marks
        .retain(|code, _| state.lots.iter().any(|lot| &lot.code == code));
    state.equity()?;
    Ok(state)
}

fn apply_economics(
    state: &mut Projection,
    id: i64,
    command: &str,
    order: &OrderFact,
    price: Money,
    quantity: u32,
    at: DateTime<Utc>,
) -> Result<(), LedgerError> {
    let notional = price.mul(quantity)?;
    let fee = fee(price, quantity, &order.direction)?;
    if order.direction == "buy" {
        state.cash = state.cash.sub(notional)?.sub(fee)?;
        let name = order
            .lot_changes
            .iter()
            .find_map(|c| {
                if c.before.is_none() {
                    c.after.as_ref().map(|l| l.name.clone())
                } else {
                    None
                }
            })
            .ok_or_else(|| LedgerError::IntegrityFailure("original buy lot absent".into()))?;
        state.lots.push(Lot {
            lot_id: format!("fill:{command}"),
            code: order.code.clone(),
            name,
            quantity,
            basis_price: price,
            buy_fee_remaining: fee,
            acquired_on: day(at),
            sellable_from: crate::calendar::verified_next_a_share_trading_day(day(at))
                .map_err(LedgerError::EvidenceUnavailable)?,
            reported_cost: None,
        });
    } else if order.direction == "sell" {
        let mut remaining = quantity;
        let mut basis = Money::ZERO;
        let mut buy_fee = Money::ZERO;
        for lot in &mut state.lots {
            if remaining == 0 || lot.code != order.code || lot.sellable_from > day(at) {
                continue;
            }
            let matched = remaining.min(lot.quantity);
            let allocated = if matched == lot.quantity {
                lot.buy_fee_remaining
            } else {
                Money(
                    i64::try_from(
                        i128::from(lot.buy_fee_remaining.0) * i128::from(matched)
                            / i128::from(lot.quantity),
                    )
                    .map_err(|_| LedgerError::Overflow)?,
                )
            };
            basis = basis.add(lot.basis_price.mul(matched)?)?;
            buy_fee = buy_fee.add(allocated)?;
            lot.buy_fee_remaining = lot.buy_fee_remaining.sub(allocated)?;
            lot.quantity -= matched;
            remaining -= matched;
        }
        if remaining > 0 {
            return Err(LedgerError::EvidenceUnavailable(format!(
                "fill={id} exceeds FIFO/T+1 inventory by {remaining}"
            )));
        }
        state.lots.retain(|lot| lot.quantity > 0);
        state.cash = state.cash.add(notional)?.sub(fee)?;
        state.realized_pnl = state
            .realized_pnl
            .add(notional.sub(basis)?.sub(buy_fee)?.sub(fee)?)?;
    } else {
        return Err(LedgerError::IntegrityFailure("unknown fill side".into()));
    }
    if state.cash < Money::ZERO {
        return Err(LedgerError::EvidenceUnavailable(format!(
            "fill={id} creates negative cash"
        )));
    }
    state.fees = state.fees.add(fee)?;
    state.as_of = state.as_of.max(at);
    Ok(())
}
