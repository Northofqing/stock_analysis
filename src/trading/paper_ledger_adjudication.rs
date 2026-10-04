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
                fw::historical(compare_original_audit_hash(&audit.bytes, &order, &mut FinancialWork::Historical))?;
                let terminal = diesel::sql_query("SELECT id,business_order_id,source,decision_basis,side,code,requested_price,execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at FROM order_audit WHERE id=?")
                    .bind::<BigInt,_>(order.audit.id).get_result::<crate::database::order_audit::CanonicalOrderAuditRow>(conn)?;
                let raw_values: Vec<serde_json::Value> = decode(&raw)?;
                let mut work = FinancialWork::Historical;
                fw::historical(compare_current_raw_fill(id, RawComparison::Historical {
                    values: &raw_values, work: &mut work
                }, &order, &terminal))?;
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
    let mut work = FinancialWork::Historical;
    let original = fw::historical(find_legacy_original(&source, id, &mut work))?;
    let prefix = fw::historical(begin_legacy_fingerprint(original, id, &mut work))?;
    let raw_trade_hash = digest(&raw_bytes(conn, id)?);
    fw::historical(finish_legacy_fingerprint(prefix, original, raw_trade_hash, &mut work))
}

pub(super) fn latest_rulings(rows: &[EventRow]) -> Result<BTreeMap<i64, AdjudicationAction>, LedgerError> {
    fw::historical(latest_rulings_with_work(rows, &mut FinancialWork::Historical))
}
fn latest_rulings_with_work(rows: &[EventRow], work: &mut FinancialWork<'_, '_>) -> fw::Result<BTreeMap<i64, AdjudicationAction>> {
    let mut result = BTreeMap::new();
    for row in rows {
        if let Fact::AdjudicatedV1(fact) = work.decode(row.payload.as_bytes())? {
            work.history_map_insert(&mut result, fact.request.original.paper_trade_id, fact.request.action)?;
        }
    }
    Ok(result)
}

fn predecessor(rows: &[EventRow], id: i64) -> Result<Option<String>, LedgerError> {
    fw::historical(predecessor_with_work(rows, id, &mut FinancialWork::Historical))
}
fn predecessor_with_work(rows: &[EventRow], id: i64, work: &mut FinancialWork<'_, '_>) -> fw::Result<Option<String>> {
    let mut result = None;
    for row in rows {
        if let Fact::AdjudicatedV1(fact) = work.decode(row.payload.as_bytes())? {
            if fact.request.original.paper_trade_id == id {
                result = Some(work.copy(&row.event_hash)?);
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
    let mut work = FinancialWork::Historical;
    fw::historical(check_ruling_header(request, binding, seq, previous, &mut work))?;
    let original = match audit_guard {
        Some(guard) => fingerprint_with_audit_guard(
            conn,
            binding,
            request.original.paper_trade_id,
            guard,
        )?,
        None => fingerprint(conn, binding, request.original.paper_trade_id)?,
    };
    fw::historical(compare_ruling_fingerprint(request, &original, &mut work))?;
    let rows = events(conn, &binding.account_id)?
        .into_iter()
        .filter(|r| r.seq < seq)
        .collect::<Vec<_>>();
    fw::historical(compare_ruling_projection(&rows, request, before, &fact.projection, &mut work))?;
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
    fw::historical(compare_historical_ruling(&historical, &fact.historical_projection, &mut work))?;
    Ok(())
}

fn validate_fact_time(request: &Adjudication) -> Result<(), LedgerError> {
    fw::historical(validate_fact_time_with_work(request, &mut FinancialWork::Historical))
}
fn validate_fact_time_with_work(request: &Adjudication, work: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
    if let AdjudicationAction::CorrectionDeclared { fact_at, .. } = &request.action {
        if *fact_at > request.decision_at {
            return Err(adjud_error(work, AdjudLiteral::FutureCorrection)?);
        }
        if !work.calendar_day(day(*fact_at))? {
            return Err(adjud_error(work, AdjudLiteral::CorrectionDay)?);
        }
    }
    Ok(())
}
fn check_ruling_header(request: &Adjudication, binding: &AccountBinding, seq: i64, previous: &str, work: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
    validate_fact_time_with_work(request, work)?;
    if request.binding != *binding || request.expected_version != seq - 1 || request.expected_head != previous {
        return Err(adjud_error(work, AdjudLiteral::RulingBinding)?);
    }
    Ok(())
}
fn compare_ruling_fingerprint(request: &Adjudication, original: &FillFingerprint, work: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
    if request.original != *original {
        return Err(adjud_error(work, AdjudLiteral::RulingBinding)?);
    }
    Ok(())
}
fn compare_ruling_projection(rows: &[EventRow], request: &Adjudication, before: &Projection, projection: &Projection, work: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
    if predecessor_with_work(rows, request.original.paper_trade_id, work)? != request.expected_predecessor {
        return Err(adjud_error(work, AdjudLiteral::RulingPredecessor)?);
    }
    if recompute_with_work(rows, request, before, work)? != *projection {
        return Err(adjud_error(work, AdjudLiteral::RulingProjection)?);
    }
    Ok(())
}
fn compare_historical_ruling(actual: &Option<(String, Option<String>)>, expected: &Option<(String, Option<String>)>, work: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
    if actual != expected {
        return Err(adjud_error(work, AdjudLiteral::HistoricalProjection)?);
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

fn recompute(rows: &[EventRow], request: &Adjudication, before: &Projection) -> Result<Projection, LedgerError> {
    fw::historical(recompute_with_work(rows, request, before, &mut FinancialWork::Historical))
}
fn recompute_with_work(
    rows: &[EventRow],
    request: &Adjudication,
    before: &Projection,
    work: &mut FinancialWork<'_, '_>,
) -> fw::Result<Projection> {
    if request.original.legacy_before_cutover {
        return work.copy(before);
    }
    let mut actions = latest_rulings_with_work(rows, work)?;
    let action = work.copy(&request.action)?;
    work.history_map_insert(&mut actions, request.original.paper_trade_id, action)?;
    let result = recompute_available_with_work(rows, &actions, work);
    match result {
        Ok(state) => Ok(state),
        Err(FinancialFailure::Financial(LedgerError::EvidenceUnavailable(reason))) => {
            let mut state = work.copy(before)?;
            state.economic_unavailable = Some(work.history_text(fw::HistoryText::Adjudication(AdjudText::Dependency {
                request: &request.request_id, reason: &reason
            }))?);
            Ok(state)
        }
        Err(error) => Err(error),
    }
}

fn recompute_available_with_work(
    rows: &[EventRow],
    actions: &BTreeMap<i64, AdjudicationAction>,
    work: &mut FinancialWork<'_, '_>,
) -> fw::Result<Projection> {
    let first = match rows.first() {
        Some(row) => row,
        None => return Err(adjud_error(work, AdjudLiteral::MissingSeed)?),
    };
    let Fact::Seeded {
        manifest,
        ..
    }
    = work.decode(first.payload.as_bytes())? else {
        return Err(adjud_error(work, AdjudLiteral::MissingSeed)?);
    };
    let mut state = seed_projection_with_work(&manifest, work)?;
    let mut fills = Vec::new();
    let mut market = Vec::new();
    for row in rows {
        match work.decode::<Fact>(row.payload.as_bytes())? {
            Fact::Order(order) if order.status == LedgerStatus::Filled => {
                let id = match order.paper_trade_id {
                    Some(id) => id,
                    None => return Err(adjud_error(work, AdjudLiteral::FillIdentityAbsent)?),
                };
                let marks = work.copy(&order.marks)?;
                work.history_push(&mut market, (order.occurred_at, false, marks))?;
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
                    return Err(adjud_error(work, AdjudLiteral::CorrectionOutsideEpoch)?);
                }
                let command = work.copy(&row.command_id)?;
                let order = work.copy(&order)?;
                work.history_push(&mut fills, (at, id, command, order, price, quantity))?;
            }
            Fact::Marked(batch) => {
                let marks = work.collect_marked_history_map(batch.marks)?;
                work.history_push(&mut market, (batch.as_of, batch.closing, marks))?;
            }
            _ => {}
        }
    }
    work.history_sort(fw::HistorySort::RecomputeFills(&mut fills))?;
    work.history_sort(fw::HistorySort::RecomputeMarkets(&mut market))?;
    let mut fills = fills.into_iter().peekable();
    for (at, closing, marks) in market {
        while fills.peek().is_some_and(|f| f.0 <= at) {
            let (time, id, command, order, price, quantity) = fills.next().unwrap();
            apply_economics_with_work(&mut state, id, &command, &order, price, quantity, time, work)?;
        }
        for (code, mark) in marks {
            work.insert(&mut state.marks, code, mark)?;
        }
        state.as_of = state.as_of.max(at);
        if closing {
            let equity = state.equity_with_work(work)?;
            work.insert(&mut state.closes, day(at), equity)?;
        }
    }
    for (time, id, command, order, price, quantity) in fills {
        apply_economics_with_work(&mut state, id, &command, &order, price, quantity, time, work)?;
    }
    state
        .marks
        .retain(|code, _| state.lots.iter().any(|lot| &lot.code == code));
    state.equity_with_work(work)?;
    Ok(state)
}

fn apply_economics_with_work(
    state: &mut Projection,
    id: i64,
    command: &str,
    order: &OrderFact,
    price: Money,
    quantity: u32,
    at: DateTime<Utc>,
    work: &mut FinancialWork<'_, '_>,
) -> fw::Result<()> {
    let notional = price.mul(quantity)?;
    let fee = fee(price, quantity, &order.direction)?;
    if order.direction == "buy" {
        state.cash = state.cash.sub(notional)?.sub(fee)?;
        let name = match order.lot_changes.iter().find_map(|change| if change.before.is_none() {
            change.after.as_ref().map(|lot| &lot.name)
        }
        else {
            None
        }) {
            Some(name) => work.copy(name)?,
            None => return Err(adjud_error(work, AdjudLiteral::BuyLotAbsent)?),
        };
        let lot = Lot {
            lot_id: work.history_text(fw::HistoryText::Adjudication(AdjudText::Lot(command)))?,
            code: work.copy(&order.code)?,
            name,
            quantity,
            basis_price: price,
            buy_fee_remaining: fee,
            acquired_on: day(at),
            sellable_from: work.calendar_next(day(at))?,
            reported_cost: None,
        };
        work.push(&mut state.lots, lot)?;
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
            return Err(LedgerError::EvidenceUnavailable(work.history_text(fw::HistoryText::Adjudication(AdjudText::Inventory {
                id, remaining
            }))?).into());
        }
        state.lots.retain(|lot| lot.quantity > 0);
        state.cash = state.cash.add(notional)?.sub(fee)?;
        state.realized_pnl = state
            .realized_pnl
            .add(notional.sub(basis)?.sub(buy_fee)?.sub(fee)?)?;
    } else {
        return Err(adjud_error(work, AdjudLiteral::UnknownSide)?);
    }
    if state.cash < Money::ZERO {
        return Err(LedgerError::EvidenceUnavailable(work.history_text(fw::HistoryText::Adjudication(AdjudText::NegativeCash(id)))?).into());
    }
    state.fees = state.fees.add(fee)?;
    state.as_of = state.as_of.max(at);
    Ok(())
}

pub(crate) type RecomputeFill = (DateTime<Utc>, i64, String, OrderFact, Money, u32);
pub(crate) type RecomputeMarket = (DateTime<Utc>, bool, BTreeMap<String, Mark>);
pub(crate) fn sort_recompute_fills_owner(rows: &mut [RecomputeFill]) {
    rows.sort_by_key(|f| (f.0, f.1));
}
pub(crate) fn sort_recompute_markets_owner(rows: &mut [RecomputeMarket]) {
    rows.sort_by_key(|m| m.0);
}
impl fw::history_sealed::Element for RecomputeFill {}
impl fw::HistoryElement for RecomputeFill {}
impl fw::history_sealed::Element for RecomputeMarket {}
impl fw::HistoryElement for RecomputeMarket {}
impl fw::history_sealed::TreeEntry for (i64, AdjudicationAction) {}
impl fw::HistoryTreeEntry for (i64, AdjudicationAction) {}
impl fw::history_sealed::TreeEntry for (i64, (AdjudicationAction, String)) {}
impl fw::HistoryTreeEntry for (i64, (AdjudicationAction, String)) {}
#[derive(Clone, Copy)]
pub(crate) enum AdjudLiteral {
    MissingSeed,
    FillIdentityAbsent,
    CorrectionOutsideEpoch,
    BuyLotAbsent,
    UnknownSide,
    RawTimestampMissing,
    RawTimestampAmbiguous,
    RawContradicts,
    FutureCorrection,
    CorrectionDay,
    RulingBinding,
    RulingPredecessor,
    RulingProjection,
    HistoricalProjection,
    AuditFingerprint,
    OutsideBoundFill
}
pub(crate) enum AdjudText<'a> {
    Literal(AdjudLiteral),
    Dependency {
        request: &'a str,
        reason: &'a str
    },
    DecisionPrefix(&'a str),
    Lot(&'a str), Inventory {
        id: i64,
        remaining: u32
    }, NegativeCash(i64),
}
impl AdjudText<'_> {
    pub(crate) fn write(&self, out: &mut fw::FinancialSink<'_>) -> Result<(), ()> {
        use std::fmt::Write;
        match self {
            Self::Literal(value) => out.bytes(match value {
                AdjudLiteral::MissingSeed => b"missing seed", AdjudLiteral::FillIdentityAbsent => b"fill identity absent", AdjudLiteral::CorrectionOutsideEpoch => b"correction price/quantity/time outside epoch", AdjudLiteral::BuyLotAbsent => b"original buy lot absent", AdjudLiteral::UnknownSide => b"unknown fill side",
                AdjudLiteral::RawTimestampMissing => b"raw fill timestamp missing",
                AdjudLiteral::RawTimestampAmbiguous => b"raw fill timestamp ambiguous",
                AdjudLiteral::RawContradicts => b"raw fill contradicts immutable event/terminal",
                AdjudLiteral::FutureCorrection => b"corrected fact cannot follow decision time",
                AdjudLiteral::CorrectionDay => b"corrected execution must be on a verified trading day",
                AdjudLiteral::RulingBinding => b"ruling binding/source/predecessor mismatch",
                AdjudLiteral::RulingPredecessor => b"ruling predecessor mismatch",
                AdjudLiteral::RulingProjection => b"ruling projection mismatch",
                AdjudLiteral::HistoricalProjection => b"historical ruling projection mismatch",
                AdjudLiteral::AuditFingerprint => b"original audit fingerprint mismatch",
                AdjudLiteral::OutsideBoundFill => b"original fill outside bound account/epoch/legacy prefix",
            }),
            Self::Dependency {
                request,
                reason
            } => write!(out, "ruling={request} dependency unavailable: {reason}").map_err(|_| ()),
            Self::DecisionPrefix(value) => write!(out, "{value} | PaperLedgerV1 account=").map_err(|_| ()),
            Self::Lot(command) => write!(out, "fill:{command}").map_err(|_| ()),
            Self::Inventory {
                id,
                remaining
            } => write!(out, "fill={id} exceeds FIFO/T+1 inventory by {remaining}").map_err(|_| ()),
            Self::NegativeCash(id) => write!(out, "fill={id} creates negative cash").map_err(|_| ()),
        }
    }
}
fn adjud_error(work: &mut FinancialWork<'_, '_>, value: AdjudLiteral) -> fw::Result<FinancialFailure> {
    let text = work.history_text(fw::HistoryText::Adjudication(AdjudText::Literal(value)))?;
    Ok(match value {
        AdjudLiteral::OutsideBoundFill | AdjudLiteral::CorrectionOutsideEpoch | AdjudLiteral::FutureCorrection | AdjudLiteral::CorrectionDay => LedgerError::InvalidInput(text),
        _ => LedgerError::IntegrityFailure(text),
    }.into())
}

enum RawComparison<'value, 'row, 'loan, 'pool> {
    Historical {
        values: &'value [serde_json::Value],
        work: &'row mut FinancialWork<'loan,
        'pool>
    },
    Paid(&'value mut crate::trading::paper_replay_codec_v1::RawScanLoan<'row, 'loan, 'pool>),
}
impl RawComparison<'_, '_, '_, '_> {
    fn len(&mut self) -> fw::Result<usize> {
        match self {
            Self::Historical {
                values,
                ..
            } => Ok(values.len()),
            Self::Paid(raw) => raw.len()
        }
    }
    fn text_equal(&mut self, index: usize, expected: &str) -> fw::Result<bool> {
        match self {
            Self::Historical {
                values,
                ..
            } => Ok(values.get(index).and_then(|v| v.as_str()) == Some(expected)),
            Self::Paid(raw) => raw.text_equal(index, expected)
        }
    }
    fn float_bits(&mut self, index: usize) -> fw::Result<Option<u64>> {
        match self {
            Self::Historical {
                values,
                ..
            } => Ok(values.get(index).and_then(|v| v.as_f64()).map(f64::to_bits)),
            Self::Paid(raw) => Ok(raw.number(index)?.map(|n| n.as_f64().to_bits()))
        }
    }
    fn unsigned(&mut self, index: usize) -> fw::Result<Option<u64>> {
        match self {
            Self::Historical {
                values,
                ..
            } => Ok(values.get(index).and_then(|v| v.as_u64())),
            Self::Paid(raw) => Ok(raw.number(index)?.and_then(|n| n.as_u64()))
        }
    }
    fn null(&mut self, index: usize) -> fw::Result<bool> {
        match self {
            Self::Historical {
                values,
                ..
            } => Ok(values.get(index).is_some_and(|v| v.is_null())),
            Self::Paid(raw) => raw.is_null(index)
        }
    }
    fn timestamp(&mut self, id: i64) -> fw::Result<DateTime<Utc>> {
        match self {
            Self::Historical {
                values,
                work
            } => parse_raw_timestamp_with_work(id, values.get(13).and_then(|v| v.as_str()), work),
            Self::Paid(raw) => raw.current_timestamp(id),
        }
    }
    fn decision_prefix(&mut self, decision: &str) -> fw::Result<bool> {
        match self {
            Self::Historical {
                values,
                work
            } => raw_decision_prefix_with_work(values.get(10).and_then(|v| v.as_str()), decision, work),
            Self::Paid(raw) => raw.current_decision_prefix(decision),
        }
    }
    fn contradiction(&mut self) -> fw::Result<FinancialFailure> {
        match self {
            Self::Historical {
                work,
                ..
            } => adjud_error(work, AdjudLiteral::RawContradicts),
            Self::Paid(raw) => raw.current_contradiction()
        }
    }
}
fn compare_current_raw_fill(id: i64, mut raw: RawComparison<'_, '_, '_, '_>, order: &OrderFact, terminal: &crate::database::order_audit::CanonicalOrderAuditRow) -> fw::Result<()> {
    let at = raw.timestamp(id)?;
    let exact = raw.len()? == 15
        && raw.text_equal(1, &order.plan_id)?
        && raw.text_equal(2, &order.code)?
        && raw.text_equal(4, &order.direction)?
        && raw.float_bits(5)? == Some(terminal.requested_price.to_bits())
        && raw.unsigned(6)? == Some(u64::from(order.quantity))
        && raw.text_equal(7, "Filled")?
        && raw.float_bits(8)? == Some(order.requested_price.cny().to_bits())
        && raw.null(9)?
        && raw.decision_prefix(&order.decision_basis)?
        && raw.text_equal(11, &order.account_mode)?
        && raw.text_equal(12, &order.data_mode)?
        && at.timestamp_millis() == order.occurred_at.timestamp_millis()
        && terminal.business_order_id == order.plan_id && terminal.source == "PaperTrade"
        && terminal.decision_basis == order.decision_basis && terminal.side == order.direction
        && terminal.code == order.code && terminal.quantity == i64::from(order.quantity)
        && terminal.execution_price.map(f64::to_bits) == Some(order.requested_price.cny().to_bits())
        && terminal.outcome == "Filled" && terminal.failure_reason.is_none();
    if !exact {
        return Err(raw.contradiction()?);
    }
    Ok(())
}
pub(crate) fn parse_raw_timestamp_with_work(id: i64, raw: Option<&str>, work: &mut FinancialWork<'_, '_>) -> fw::Result<DateTime<Utc>> {
    use chrono::TimeZone;
    let raw = match raw {
        Some(raw) => raw,
        None => return Err(adjud_error(work, AdjudLiteral::RawTimestampMissing)?)
    };
    let at = match crate::trading::paper_lot_ledger::parse_paper_fill_timestamp_body(id, raw, work) {
        Ok(at) => at,
        Err(FinancialFailure::History(text)) => return Err(LedgerError::IntegrityFailure(text).into()),
        Err(error) => return Err(error),
    };
    match chrono::FixedOffset::east_opt(8 * 3600).unwrap().from_local_datetime(&at).single() {
        Some(at) => Ok(at.with_timezone(&Utc)),
        None => Err(adjud_error(work, AdjudLiteral::RawTimestampAmbiguous)?),
    }
}
pub(crate) fn raw_decision_prefix_with_work(raw: Option<&str>, decision: &str, work: &mut FinancialWork<'_, '_>) -> fw::Result<bool> {
    match raw {
        Some(raw) if !raw.is_empty() => Ok(decision.starts_with(&work.history_text(fw::HistoryText::Adjudication(AdjudText::DecisionPrefix(raw)))?)),
        _ => Ok(false),
    }
}
pub(crate) fn raw_contradiction_with_work(work: &mut FinancialWork<'_, '_>) -> fw::Result<FinancialFailure> {
    adjud_error(work, AdjudLiteral::RawContradicts)
}

fn compare_original_audit_hash(
    hash: &str, order: &OrderFact, work: &mut FinancialWork<'_, '_>,
) -> fw::Result<()> {
    if hash != order.audit.record_hash {
        return Err(adjud_error(work, AdjudLiteral::AuditFingerprint)?);
    }
    Ok(())
}
fn find_legacy_original<'a>(
    source: &'a crate::database::attribution_epochs::VerifiedEpochFillSet,
    id: i64, work: &mut FinancialWork<'_, '_>,
) -> fw::Result<&'a crate::database::attribution_epochs::VerifiedEpochFill> {
    match source.fills().iter().find(|fill| fill.fill().id == id) {
        Some(fill) => Ok(fill),
        None => Err(adjud_error(work, AdjudLiteral::OutsideBoundFill)?),
    }
}
struct LegacyFingerprintPrefix {
    id: i64,
    plan: String,
    at: DateTime<Utc>,
}
fn begin_legacy_fingerprint(
    original: &crate::database::attribution_epochs::VerifiedEpochFill,
    id: i64, work: &mut FinancialWork<'_, '_>,
) -> fw::Result<LegacyFingerprintPrefix> {
    let row = original.fill();
    let at = work.ledger_timestamp(id, &row.occurred_at)?.and_utc();
    let plan = work.copy(&row.plan_id)?;
    Ok(LegacyFingerprintPrefix {
        id, plan, at
    })
}
fn finish_legacy_fingerprint(
    prefix: LegacyFingerprintPrefix,
    original: &crate::database::attribution_epochs::VerifiedEpochFill,
    raw_trade_hash: String, work: &mut FinancialWork<'_, '_>,
) -> fw::Result<FillFingerprint> {
    Ok(FillFingerprint {
        paper_trade_id: prefix.id,
        plan_id: prefix.plan,
        event_hash: String::new(),
        raw_trade_hash,
        audit_hash: work.copy_terminal_hash(original.terminal_audit_hash().unwrap_or_default())?,
        fact_at: prefix.at,
        legacy_before_cutover: true,
        legacy_no_terminal: original.terminal_audit_hash().is_none(),
    })
}
fn compare_paid_current_raw(
    id: i64, frame: &mut fw::RawRowFrame<'_, '_>, order: &OrderFact,
    terminal: &crate::database::order_audit::CanonicalOrderAuditRow,
) -> fw::Result<()> {
    let mut scan = frame.scan()?;
    compare_current_raw_fill(id, RawComparison::Paid(&mut scan), order, terminal)
}

#[cfg(test)]
pub(super) fn history_adjudication_fixture(binding: &AccountBinding, rows: &[EventRow], work: &mut FinancialWork<'_, '_>) {
    let at = crate::trading::paper_replay_history_v1_tests::at();
    let Fact::Seeded {
        manifest,
        ..
    }
    = decode(&rows[0].payload).unwrap() else {
        panic!("seed");
    };
    let before = seed_projection(&manifest).unwrap();
    let request = Adjudication {
        binding: binding.clone(), request_id: "fixed-ruling".into(), expected_version: 2,
        expected_head: rows[1].event_hash.clone(), expected_predecessor: None,
        original: FillFingerprint {
            paper_trade_id: 42,
            plan_id: "fixed-plan".into(),
            event_hash: "event".into(),
            raw_trade_hash: "raw".into(), audit_hash: "audit".into(), fact_at: at,
            legacy_before_cutover: false, legacy_no_terminal: false
            },
        action: AdjudicationAction::Quarantine, reason: "reviewed finite fixture".into(), evidence: "evidence".into(),
        operator: "fixture".into(), source: "history test".into(), decision_at: at,
    };
    let order = OrderFact {
        plan_id: "filled-plan".into(),
        intent_hash: "filled-intent".into(),
        code: "600001".into(),
        direction: "sell".into(),
        requested_price: Money::from_micros(10_000_000),
        price_intent: PriceIntent::FixedSignalPriceV1,
        quantity: 100,
        quote_price: Money::from_micros(10_000_000),
        quote_observed_at: at,
        account_mode: "Normal".into(),
        data_mode: "Full".into(),
        decision_basis: "fixture sell".into(),
        source_evidence: "fixture".into(),
        occurred_at: at,
        status: LedgerStatus::Filled,
        reason: None,
        cash_delta: Money::ZERO,
        commission: Money::ZERO,
        stamp: Money::ZERO,
        realized_delta: Money::ZERO,
        lot_changes: Vec::new(),
        marks: before.marks.clone(),
        paper_trade_id: Some(42),
        audit: AuditLink {
            id: 42,
            previous_hash: "previous".into(),
            record_hash: "record".into(),
            created_at: "time".into(),
        },
    };
    let mut filled_rows: Vec<_> = rows.iter().map(|row| {
        EventRow::from_bounded_sql_parts(row.seq, row.command_id.clone(), row.previous_hash.clone(),
            row.event_hash.clone(), row.payload.clone(), row.business_plan_id.clone(),
            row.intent_hash.clone(), row.is_terminal, row.paper_trade_id, row.order_audit_id)
    }).collect();
    filled_rows.push(EventRow::from_bounded_sql_parts(3, "filled-command".into(), "previous".into(),
        "event".into(), encode(&Fact::Order(order)).unwrap(), Some("filled-plan".into()),
        Some("filled-intent".into()), 1, Some(42), Some(42)));
    let rows = filled_rows.as_slice();
    let expected = recompute(rows, &request, &before).unwrap();
    let actual = recompute_with_work(rows, &request, &before, work).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.lots[0].quantity, 100, "quarantine removes the real filled sell");
    // Original event order places the order's mark after the same-time Marked
    // event; the shared stable comparator must preserve that last value.
    assert_eq!(actual.marks["600001"].price, Money::from_micros(10_000_000));
    let mut correction = request.clone();
    correction.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_micros(11_000_000),
        quantity: 100,
        fact_at: at,
    };
    let original_correction = recompute(rows, &correction, &before).unwrap();
    let corrected = recompute_with_work(rows, &correction, &before, work).unwrap();
    assert_eq!(corrected, original_correction);
    assert!(corrected.lots.is_empty());
    assert!(corrected.cash > before.cash);
    assert!(corrected.fees > Money::ZERO);
    let mut applied = work.copy(&before).unwrap();
    let used = work.history_used();
    let ruling = Fact::AdjudicatedV1(AdjudicatedFact {
        request: correction,
        projection: corrected.clone(),
        historical_projection: None,
    });
    execution::apply_fact_with_work(&mut applied, &ruling, work).unwrap();
    assert_eq!(applied, corrected, "adjudication applies its full projection copy");
    assert!(work.history_used() > used);
    let mut marks = manifest.marks.clone();
    marks.push(Mark {
        price: Money::from_micros(12_000_000), ..marks[0].clone()
    });
    let original: BTreeMap<_, _> = marks.clone().into_iter().map(|mark| (mark.code.clone(), mark)).collect();
    assert_eq!(work.collect_marked_history_map(marks).unwrap(), original);
    assert_eq!(original["600001"].price, Money::from_micros(12_000_000));
    let bytes = encode(&request).unwrap();
    let hash = work.history_hash(crate::trading::paper_replay_codec_v1::HistoryOutput::ExtraAdjudication(&request)).unwrap();
    assert_eq!(hash, digest(&bytes));
    let mut legacy = request.clone();
    legacy.original.legacy_before_cutover = true;
    let used = work.history_used();
    assert_eq!(recompute_with_work(rows, &legacy, &before, work).unwrap(), before);
    assert!(work.history_used() > used, "legacy ruling retains full grown projection copy");
}

#[cfg(test)]
pub(super) fn history_raw_fixture<'loan, 'pool>(mut work: FinancialWork<'loan, 'pool>) -> FinancialWork<'loan, 'pool> {
    use crate::database::order_audit::CanonicalOrderAuditRow;
    let at = crate::trading::paper_replay_history_v1_tests::at();
    let order = OrderFact {
        plan_id: "plan".into(), intent_hash: "intent".into(), code: "600001".into(), direction: "buy".into(),
        requested_price: Money::from_micros(10_000_000), price_intent: PriceIntent::FixedSignalPriceV1,
        quantity: 100, quote_price: Money::from_micros(10_000_000), quote_observed_at: at,
        account_mode: "Normal".into(), data_mode: "Full".into(),
        decision_basis: "reason | PaperLedgerV1 account=account".into(), source_evidence: "source".into(),
        occurred_at: at, status: LedgerStatus::Filled, reason: None,
        cash_delta: Money::ZERO, commission: Money::ZERO, stamp: Money::ZERO, realized_delta: Money::ZERO,
        lot_changes: Vec::new(), marks: BTreeMap::new(), paper_trade_id: Some(1),
        audit: AuditLink {
            id: 1,
            previous_hash: "previous".into(),
            record_hash: "record".into(),
            created_at: "time".into()
        },
    };
    let terminal = CanonicalOrderAuditRow {
        id: 1, business_order_id: order.plan_id.clone(), source: "PaperTrade".into(),
        decision_basis: order.decision_basis.clone(), side: order.direction.clone(), code: order.code.clone(),
        requested_price: 10.0, execution_price: Some(10.0), quantity: 100,
        quote_observed_at: None, outcome: "Filled".into(), failure_reason: None, created_at: "unused".into(),
    };
    for mutation in 0..10 {
        let mut fields = serde_json::json!([1,"plan","600001","unused","buy",10.0,100,"Filled",10.0,null,
            "reason","Normal","Full","2026-09-24 10:00:00","unused"]);
        match mutation {
            1 => fields[3] = serde_json::json!({
                "ignored":[1,true,null]
            }),
            2 => fields[5] = serde_json::json!(11.0),
            3 => fields[6] = serde_json::json!(100.0),
            4 => fields[13] = serde_json::json!("now"),
            5 => fields.as_array_mut().unwrap().push(serde_json::json!({
                "extra":[]
            })),
            6 => fields[10] = serde_json::json!("different"),
            7 => {
                fields[13] = serde_json::json!("now");
                fields.as_array_mut().unwrap().push(serde_json::json!(null));
            }
            8 | 9 => {
                fields[13] = serde_json::json!("now");
                fields[3] = serde_json::json!("RAW_UNUSED_SENTINEL");
            }
            _ => {}
        }
        let mut raw = serde_json::to_string(&fields).unwrap();
        if mutation == 8 {
            raw = raw.replace("\"RAW_UNUSED_SENTINEL\"", r#"{"ignored":[1e400]}"#);
        } else if mutation == 9 {
            raw = raw.replace("\"RAW_UNUSED_SENTINEL\"", r#"{"ignored":[true,]}"#);
        }
        let expected = match decode::<Vec<serde_json::Value>>(&raw) {
            Ok(original) => compare_current_raw_fill(1, RawComparison::Historical {
                values: &original, work: &mut FinancialWork::Historical,
            }, &order, &terminal),
            Err(error) => Err(error.into()),
        };
        let mut frame = match fw::RawRowFrame::fixture_copy(&raw, work) {
            Ok(frame) => frame, Err((error, _)) => panic!("{error:?}"),
        };
        let actual = compare_paid_current_raw(1, &mut frame, &order, &terminal);
        match (actual, expected) {
            (Ok(()), Ok(())) => assert!(mutation <= 1),
            (Err(FinancialFailure::Financial(actual)), Err(FinancialFailure::Financial(expected))) =>
                assert_eq!(actual.to_string(), expected.to_string()),
            pair => panic!("raw/current owner mismatch: {pair:?}"),
        }
        assert_eq!(frame.legacy_hash().unwrap(), digest(&raw));
        work = frame.finish();
    }
    work
}
