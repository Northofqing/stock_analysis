//! A single immutable read capability for all paper economic consumers.
use super::*;
use crate::performance::economic_position::{EconomicFillRow, FillCostLedger};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectiveFillScope {
    /// Only legal when this DB has no paper account. Never authorizes a trade.
    LegacyRaw,
    Epoch(AccountBinding),
    LegacyBeforeCutover(AccountBinding),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectiveHistory {
    /// None is permitted only for LegacyRaw, freezing its current raw prefix.
    AsKnown {
        ledger_version: Option<i64>,
    },
    RestatedLatest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveFillRequest {
    pub scope: EffectiveFillScope,
    pub history: EffectiveHistory,
    pub as_of: NaiveDate,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillAuthority {
    EpochTerminal {
        event_hash: String,
        audit_hash: String,
    },
    LegacyAudited {
        audit_hash: String,
    },
    LegacyNoTerminal,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillLineage {
    pub fill_id: i64,
    pub raw_hash: String,
    pub authority: FillAuthority,
    pub ruling_hash: Option<String>,
    pub quarantined: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveProjectionReceipt {
    pub request: EffectiveFillRequest,
    pub raw_high_water: i64,
    pub raw_source_hash: String,
    pub ledger_head: Option<(i64, String)>,
    /// Same-transaction financial inventory CAS proof; None for legacy history.
    pub inventory_fingerprint: Option<String>,
    /// Financial source identity excludes derived result events; ledger_head
    /// above remains the actual read/CAS provenance, not the result cache key.
    pub economic_head: Option<(i64, String)>,
    pub adjudication_head: Option<(i64, String)>,
    pub cutover_at: Option<DateTime<Utc>>,
    pub cutover_raw_high_water: Option<i64>,
    pub rule_version: String,
    pub projection_hash: String,
    pub catalog_generation: i64,
    pub catalog_objects_hash: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedEffectiveFillSet {
    pub(super) receipt: EffectiveProjectionReceipt,
    pub(super) rows: Vec<EconomicFillRow>,
    pub(super) lineage: Vec<FillLineage>,
    // Original OR corrected facts at/before as_of, including exclusions. The
    // complete lineage/receipt above still verifies every source and CAS head.
    period_lineage: Vec<FillLineage>,
    pub(super) unavailable: Option<String>,
    pub(super) seed_lots: Vec<Lot>,
}
impl VerifiedEffectiveFillSet {
    pub(super) fn snapshot_input_hash(&self) -> Result<String, LedgerError> {
        Ok(digest(&encode(&(
            "PaperSnapshotPeriodInputV1",
            &self.receipt.request.scope,
            self.receipt.request.as_of,
            self.receipt.cutover_at,
            self.receipt.cutover_raw_high_water,
            &self.receipt.rule_version,
            self.receipt.catalog_generation,
            &self.receipt.catalog_objects_hash,
            self.rows()?,
            &self.period_lineage,
            &self.seed_lots,
        ))?))
    }
    pub fn receipt(&self) -> &EffectiveProjectionReceipt {
        &self.receipt
    }
    pub fn lineage(&self) -> &[FillLineage] {
        &self.lineage
    }
    /// A terminal card is immutable raw evidence, not an automatic restatement.
    /// Rulings are disclosed through diagnostics, never by silently reissuing it.
    pub fn original_fill_card_allowed(
        &self,
        fill_id: i64,
        audit_hash: &str,
    ) -> Result<bool, LedgerError> {
        self.rows()?;
        let lineage = self
            .lineage
            .iter()
            .find(|item| item.fill_id == fill_id)
            .ok_or_else(|| {
                LedgerError::EvidenceUnavailable("P04 fill outside explicit effective scope".into())
            })?;
        let expected = match &lineage.authority {
            FillAuthority::EpochTerminal { audit_hash, .. }
            | FillAuthority::LegacyAudited { audit_hash } => audit_hash,
            FillAuthority::LegacyNoTerminal => {
                return Err(LedgerError::EvidenceUnavailable(
                    "P04 legacy fill has no original terminal authority".into(),
                ))
            }
        };
        if expected != audit_hash {
            return Err(LedgerError::IntegrityFailure(
                "P04 terminal does not match effective source lineage".into(),
            ));
        }
        Ok(lineage.ruling_hash.is_none() && !lineage.quarantined)
    }
    pub(crate) fn seed_carry(
        &self,
    ) -> Vec<crate::performance::attribution_epoch::LegacyCarryPosition> {
        let mut grouped = BTreeMap::<String, u64>::new();
        for lot in &self.seed_lots {
            *grouped.entry(lot.code.clone()).or_default() += u64::from(lot.quantity);
        }
        grouped
            .into_iter()
            .map(
                |(code, quantity)| crate::performance::attribution_epoch::LegacyCarryPosition {
                    code,
                    quantity,
                },
            )
            .collect()
    }
    pub fn rows(&self) -> Result<&[EconomicFillRow], LedgerError> {
        if let Some(reason) = &self.unavailable {
            return Err(LedgerError::EvidenceUnavailable(reason.clone()));
        }
        Ok(&self.rows)
    }
    pub fn costs(&self) -> Result<FillCostLedger, LedgerError> {
        let rows = self.rows()?;
        use crate::performance::economic_position::{CostBasisKind, FillCostEvidence};
        Ok(FillCostLedger {
            basis_id: format!("{FEE_MODEL}:{}", self.receipt.projection_hash),
            kind: CostBasisKind::Scenario,
            costs: rows
                .iter()
                .map(|r| {
                    Ok(FillCostEvidence {
                        fill_id: r.id,
                        adverse_cost: adjudication::fee(
                            Money::from_cny(r.fill_price.ok_or_else(|| {
                                LedgerError::EvidenceUnavailable("missing effective price".into())
                            })?)?,
                            u32::try_from(r.quantity).map_err(|_| LedgerError::Overflow)?,
                            &r.direction,
                        )?
                        .cny(),
                        evidence_id: format!(
                            "{FEE_MODEL}:{}:{}",
                            self.receipt.projection_hash, r.id
                        ),
                    })
                })
                .collect::<Result<_, LedgerError>>()?,
        })
    }
}
impl PaperLedger<'_> {
    pub fn verified_effective_fills(
        &self,
        request: &EffectiveFillRequest,
    ) -> Result<VerifiedEffectiveFillSet, LedgerError> {
        let mut conn = self
            .db
            .get_conn()
            .map_err(|e| LedgerError::Database(e.to_string()))?;
        conn.transaction(|conn| verified_on(conn, request))
    }
}

pub(super) fn verified_on(
    conn: &mut SqliteConnection,
    request: &EffectiveFillRequest,
) -> Result<VerifiedEffectiveFillSet, LedgerError> {
    let catalog = verify_catalog(conn)?;
    let EffectiveFillScope::Epoch(binding) = &request.scope else {
        return legacy_verified_on(conn, request, catalog);
    };
    let head = load(conn, binding)?;
    let view = match &request.history {
        EffectiveHistory::RestatedLatest => head,
        EffectiveHistory::AsKnown {
            ledger_version: Some(version),
        } if *version >= 1 && *version <= head.version => {
            replay_through(conn, binding, Some(*version))?
        }
        _ => {
            return Err(LedgerError::InvalidInput(
                "bound history requires a committed explicit version".into(),
            ))
        }
    };
    let events = events(conn, &binding.account_id)?
        .into_iter()
        .filter(|r| r.seq <= view.version)
        .collect::<Vec<_>>();
    let Fact::Seeded {
        manifest,
        legacy_high_water_id,
        ..
    } = decode(&events[0].payload)?
    else {
        return Err(LedgerError::IntegrityFailure(
            "effective source lacks seed".into(),
        ));
    };
    if request.as_of < day(manifest.cutover_at) {
        return Err(LedgerError::InvalidInput(
            "epoch projection predates its seed cutover".into(),
        ));
    }
    let mut rulings = BTreeMap::new();
    let mut adjudication_head = None;
    for event in &events {
        if let Fact::AdjudicatedV1(ruling) = decode(&event.payload)? {
            rulings.insert(
                ruling.request.original.paper_trade_id,
                (ruling.request.action, event.event_hash.clone()),
            );
            adjudication_head = Some((event.seq, event.event_hash.clone()));
        }
    }
    let mut rows = Vec::new();
    let mut lineage = Vec::new();
    let mut period_lineage = Vec::new();
    let mut raw_manifest = Vec::new();
    let mut high_water = 0;
    for event in &events {
        let Fact::Order(order) = decode(&event.payload)? else {
            continue;
        };
        let Some(id) = order.paper_trade_id else {
            continue;
        };
        let bytes = adjudication::raw_bytes(conn, id)?;
        high_water = high_water.max(id);
        raw_manifest.push((id, digest(&bytes)));
        if order.status != LedgerStatus::Filled {
            continue;
        }
        let source = adjudication::fingerprint(conn, binding, id)?;
        let mut row=diesel::sql_query("SELECT id,plan_id,code,name,direction,fill_price,quantity,CAST(ts AS TEXT) AS occurred_at,virtual_reason FROM paper_trades WHERE id=?")
            .bind::<BigInt,_>(id).get_result::<EconomicFillRow>(conn)?;
        let mut proof = FillLineage {
            fill_id: id,
            raw_hash: source.raw_trade_hash,
            authority: FillAuthority::EpochTerminal {
                event_hash: source.event_hash,
                audit_hash: source.audit_hash,
            },
            ruling_hash: None,
            quarantined: false,
        };
        if let Some((action, hash)) = rulings.get(&id) {
            proof.ruling_hash = Some(hash.clone());
            match action {
                AdjudicationAction::Quarantine => proof.quarantined = true,
                AdjudicationAction::CorrectionDeclared {
                    price,
                    quantity,
                    fact_at,
                } => {
                    row.fill_price = Some(price.cny());
                    row.quantity = i64::from(*quantity);
                    row.occurred_at = fact_at
                        .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                        .format("%Y-%m-%d %H:%M:%S%.9f")
                        .to_string();
                }
            }
        }
        if day(source.fact_at) <= request.as_of
            || crate::trading::paper_lot_ledger::parse_paper_fill_timestamp(
                row.id,
                &row.occurred_at,
            )
            .map_err(LedgerError::IntegrityFailure)?
            .date()
                <= request.as_of
        {
            period_lineage.push(proof.clone());
        }
        if !proof.quarantined {
            rows.push(row);
        }
        lineage.push(proof);
    }
    sort_rows(&mut rows)?;
    // The complete account projection above validates cash/FIFO/T+1 before a
    // historical date can hide any invalid dependent sell.
    filter_date(&mut rows, request.as_of)?;
    let unavailable = view.economic_unavailable.clone();
    let inventory_fingerprint = Some(view.inventory_fingerprint()?);
    let mut receipt = EffectiveProjectionReceipt {
        request: request.clone(),
        raw_high_water: high_water,
        raw_source_hash: digest(&encode(&raw_manifest)?),
        ledger_head: Some((view.version, view.event_hash)),
        inventory_fingerprint,
        economic_head: economic_head(&events)?,
        adjudication_head,
        cutover_at: Some(manifest.cutover_at),
        cutover_raw_high_water: Some(legacy_high_water_id),
        rule_version: "PaperEffectiveV1/micro-cny-half-up-v1/lot-rates-v1".into(),
        projection_hash: String::new(),
        catalog_generation: catalog.0,
        catalog_objects_hash: catalog.1,
    };
    let seed_lots = seed_projection(&manifest)?.lots;
    receipt.projection_hash = projection_hash(&receipt, &rows, &lineage, &unavailable, &seed_lots)?;
    Ok(VerifiedEffectiveFillSet {
        receipt,
        rows,
        lineage,
        period_lineage,
        unavailable,
        seed_lots,
    })
}

fn economic_head(events: &[EventRow]) -> Result<Option<(i64, String)>, LedgerError> {
    for row in events.iter().rev() {
        if !matches!(decode::<Fact>(&row.payload)?, Fact::DerivedSnapshotV1(_)) {
            return Ok(Some((row.seq, row.event_hash.clone())));
        }
    }
    Ok(None)
}
fn projection_hash(
    receipt: &EffectiveProjectionReceipt,
    rows: &[EconomicFillRow],
    lineage: &[FillLineage],
    unavailable: &Option<String>,
    seed_lots: &[Lot],
) -> Result<String, LedgerError> {
    // Read version/history are provenance. Economic identity is content-bound;
    // a trailing derived result cannot change its own input identity.
    let mut economic = receipt.clone();
    economic.ledger_head = economic.economic_head.clone();
    economic.request.history = EffectiveHistory::RestatedLatest;
    economic.projection_hash.clear();
    Ok(digest(&encode(&(
        "PaperEffectiveIdentityV1",
        economic,
        rows,
        lineage,
        unavailable,
        seed_lots,
    ))?))
}

#[derive(QueryableByName, Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct CatalogObject {
    #[diesel(sql_type=Text)]
    kind: String,
    #[diesel(sql_type=Text)]
    name: String,
    #[diesel(sql_type=Text)]
    owner: String,
    #[diesel(sql_type=Text)]
    sql: String,
}
#[derive(QueryableByName)]
struct Generation {
    #[diesel(sql_type=BigInt)]
    user_version: i64,
}
#[derive(QueryableByName)]
struct Application {
    #[diesel(sql_type=BigInt)]
    application_id: i64,
}

/// Local exact namespace proof, on the SAME SQLite transaction. This is not a
/// substitute for whole-application CatalogV2 activation authority.
pub(super) fn verify_catalog(conn: &mut SqliteConnection) -> Result<(i64, String), LedgerError> {
    if diesel::sql_query("SELECT COUNT(*) AS value FROM sqlite_temp_master WHERE name GLOB 'paper_ledger_*' OR tbl_name GLOB 'paper_ledger_*'").get_result::<IntegerRow>(conn)?.value!=0 {
        return Err(LedgerError::IntegrityFailure("TEMP paper namespace is not permitted".into()));
    }
    let generation = diesel::sql_query("PRAGMA user_version")
        .get_result::<Generation>(conn)?
        .user_version;
    let application = diesel::sql_query("PRAGMA application_id")
        .get_result::<Application>(conn)?
        .application_id;
    let mut objects=diesel::sql_query("SELECT type AS kind,name,tbl_name AS owner,sql FROM sqlite_master WHERE (name GLOB 'paper_ledger_*' OR tbl_name GLOB 'paper_ledger_*') AND sql IS NOT NULL ORDER BY type,name,tbl_name,sql").load::<CatalogObject>(conn)?;
    objects.sort();
    if objects.is_empty() {
        if !((generation == 0 && application == 0)
            || (generation == 1 && application == 1398035265))
        {
            return Err(LedgerError::IntegrityFailure(
                "missing paper namespace or unknown catalog generation".into(),
            ));
        }
    } else {
        let mut expected = crate::database::paper_ledger_schema_v1::STATEMENTS
            .iter()
            .map(|(kind, name, owner, sql)| CatalogObject {
                kind: (*kind).into(),
                name: (*name).into(),
                owner: (*owner).into(),
                sql: sql.replace("IF NOT EXISTS ", ""),
            })
            .collect::<Vec<_>>();
        expected.sort();
        if generation != 2 || application != 1398035265 || objects != expected {
            return Err(LedgerError::IntegrityFailure(
                "paper namespace missing/tampered/extra object or unknown catalog generation"
                    .into(),
            ));
        }
    }
    Ok((
        generation,
        digest(&encode(&(application, generation, objects))?),
    ))
}

pub(super) fn legacy_source(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
) -> Result<crate::database::attribution_epochs::VerifiedEpochFillSet, LedgerError> {
    let rows = events(conn, &binding.account_id)?;
    let Fact::Seeded {
        legacy_high_water_id,
        legacy_audit_high_water,
        ..
    } = decode(&rows.first().ok_or(LedgerError::NotSeeded)?.payload)?
    else {
        return Err(LedgerError::IntegrityFailure(
            "legacy prefix lacks seed".into(),
        ));
    };
    let audit_high_water =
        if legacy_audit_high_water == crate::database::order_audit::AUDIT_CHAIN_GENESIS {
            0
        } else {
            diesel::sql_query(
                "SELECT order_audit_id AS value FROM order_audit_chain WHERE record_hash=?",
            )
            .bind::<Text, _>(legacy_audit_high_water)
            .get_result::<IntegerRow>(conn)?
            .value
        };
    crate::database::attribution_epochs::load_verified_legacy_prefix(
        conn,
        legacy_high_water_id,
        audit_high_water,
    )
    .map_err(|e| LedgerError::IntegrityFailure(e.to_string()))
}

fn sort_rows(rows: &mut Vec<EconomicFillRow>) -> Result<(), LedgerError> {
    let mut ordered = rows
        .drain(..)
        .map(|r| {
            let at =
                crate::trading::paper_lot_ledger::parse_paper_fill_timestamp(r.id, &r.occurred_at)
                    .map_err(LedgerError::IntegrityFailure)?;
            Ok((at, r.id, r))
        })
        .collect::<Result<Vec<_>, LedgerError>>()?;
    ordered.sort_by_key(|r| (r.0, r.1));
    rows.extend(ordered.into_iter().map(|r| r.2));
    Ok(())
}
fn filter_date(rows: &mut Vec<EconomicFillRow>, as_of: NaiveDate) -> Result<(), LedgerError> {
    let mut selected = Vec::new();
    for row in rows.drain(..) {
        if crate::trading::paper_lot_ledger::parse_paper_fill_timestamp(row.id, &row.occurred_at)
            .map_err(LedgerError::IntegrityFailure)?
            .date()
            <= as_of
        {
            selected.push(row);
        }
    }
    *rows = selected;
    Ok(())
}

fn historical_rows(
    conn: &mut SqliteConnection,
    source: &crate::database::attribution_epochs::VerifiedEpochFillSet,
    events: &[EventRow],
    extra: Option<&Adjudication>,
    cutover: Option<DateTime<Utc>>,
) -> Result<(Vec<EconomicFillRow>, Vec<FillLineage>, Option<String>), LedgerError> {
    let mut actions = BTreeMap::new();
    for event in events {
        if let Fact::AdjudicatedV1(fact) = decode(&event.payload)? {
            if fact.request.original.legacy_before_cutover {
                actions.insert(
                    fact.request.original.paper_trade_id,
                    (fact.request.action, event.event_hash.clone()),
                );
            }
        }
    }
    if let Some(request) = extra {
        actions.insert(
            request.original.paper_trade_id,
            (request.action.clone(), digest(&encode(request)?)),
        );
    }
    let mut rows = Vec::new();
    let mut lineage = Vec::new();
    for fill in source.fills() {
        let mut row = fill.fill().clone();
        // Frozen legacy storage used SQLite CURRENT_TIMESTAMP (UTC). The new
        // economic seam has one explicit Shanghai-local fact-time contract.
        let utc =
            crate::trading::paper_lot_ledger::parse_paper_fill_timestamp(row.id, &row.occurred_at)
                .map_err(LedgerError::IntegrityFailure)?;
        row.occurred_at = utc
            .checked_add_signed(chrono::Duration::hours(8))
            .ok_or(LedgerError::Overflow)?
            .format("%Y-%m-%d %H:%M:%S%.9f")
            .to_string();
        let mut proof = FillLineage {
            fill_id: row.id,
            raw_hash: digest(&adjudication::raw_bytes(conn, row.id)?),
            authority: fill
                .terminal_audit_hash()
                .map(|hash| FillAuthority::LegacyAudited {
                    audit_hash: hash.into(),
                })
                .unwrap_or(FillAuthority::LegacyNoTerminal),
            ruling_hash: None,
            quarantined: false,
        };
        if let Some((action, hash)) = actions.get(&row.id) {
            proof.ruling_hash = Some(hash.clone());
            match action {
                AdjudicationAction::Quarantine => proof.quarantined = true,
                AdjudicationAction::CorrectionDeclared {
                    price,
                    quantity,
                    fact_at,
                } => {
                    if *price <= Money::ZERO
                        || *quantity == 0
                        || !quantity.is_multiple_of(100)
                        || cutover.is_some_and(|cutover| *fact_at >= cutover)
                    {
                        return Err(LedgerError::InvalidInput(
                            "historical correction outside legacy scope".into(),
                        ));
                    }
                    row.fill_price = Some(price.cny());
                    row.quantity = i64::from(*quantity);
                    row.occurred_at = fact_at
                        .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                        .format("%Y-%m-%d %H:%M:%S%.9f")
                        .to_string();
                }
            }
        }
        if !proof.quarantined {
            rows.push(row);
        }
        lineage.push(proof);
    }
    sort_rows(&mut rows)?;
    let fills = rows
        .iter()
        .map(|r| {
            Ok(crate::trading::paper_lot_ledger::PaperFill {
                id: r.id,
                code: r.code.clone(),
                name: r.name.clone(),
                direction: r.direction.clone(),
                fill_price: r.fill_price,
                quantity: r.quantity,
                occurred_at: crate::trading::paper_lot_ledger::parse_paper_fill_timestamp(
                    r.id,
                    &r.occurred_at,
                )
                .map_err(LedgerError::IntegrityFailure)?,
            })
        })
        .collect::<Result<Vec<_>, LedgerError>>()?;
    let unavailable =
        crate::trading::paper_lot_ledger::rebuild_paper_positions(&fills, NaiveDate::MAX).err();
    Ok((rows, lineage, unavailable))
}

pub(super) fn legacy_result(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    events: &[EventRow],
    extra: Option<&Adjudication>,
) -> Result<(String, Option<String>), LedgerError> {
    let source = legacy_source(conn, binding)?;
    let Fact::Seeded { manifest, .. } = decode(&events[0].payload)? else {
        return Err(LedgerError::NotSeeded);
    };
    let (rows, _, unavailable) =
        historical_rows(conn, &source, events, extra, Some(manifest.cutover_at))?;
    Ok((
        digest(&encode(&(
            "LegacyEconomicV1",
            source.all_status_paper_manifest_hash(),
            rows,
            &unavailable,
        ))?),
        unavailable,
    ))
}

fn legacy_verified_on(
    conn: &mut SqliteConnection,
    request: &EffectiveFillRequest,
    catalog: (i64, String),
) -> Result<VerifiedEffectiveFillSet, LedgerError> {
    use crate::database::attribution_epochs::{
        load_verified_epoch_fills_until, ResolvedAttributionEpoch,
    };
    let (source, event_rows, head, cutover) = match &request.scope {
        EffectiveFillScope::LegacyRaw => {
            if request.history
                != (EffectiveHistory::AsKnown {
                    ledger_version: None,
                })
            {
                return Err(LedgerError::InvalidInput(
                    "LegacyRaw permits only current raw as-known".into(),
                ));
            }
            if catalog.0 == 2
                && diesel::sql_query("SELECT COUNT(*) AS value FROM paper_ledger_account")
                    .get_result::<IntegerRow>(conn)?
                    .value
                    != 0
            {
                return Err(LedgerError::InvalidInput(
                    "seeded database requires explicit bound economic scope".into(),
                ));
            }
            (
                load_verified_epoch_fills_until(
                    conn,
                    &ResolvedAttributionEpoch::Legacy,
                    NaiveDate::MAX,
                )
                .map_err(|e| LedgerError::IntegrityFailure(e.to_string()))?,
                Vec::new(),
                None,
                None,
            )
        }
        EffectiveFillScope::LegacyBeforeCutover(binding) => {
            let current = load(conn, binding)?;
            let view = match request.history {
                EffectiveHistory::RestatedLatest => current,
                EffectiveHistory::AsKnown {
                    ledger_version: Some(version),
                } if version >= 1 && version <= current.version => {
                    replay_through(conn, binding, Some(version))?
                }
                _ => {
                    return Err(LedgerError::InvalidInput(
                        "bound legacy history requires explicit version".into(),
                    ))
                }
            };
            let event_rows = events(conn, &binding.account_id)?
                .into_iter()
                .filter(|e| e.seq <= view.version)
                .collect::<Vec<_>>();
            let Fact::Seeded { manifest, .. } = decode(&event_rows[0].payload)? else {
                return Err(LedgerError::NotSeeded);
            };
            (
                legacy_source(conn, binding)?,
                event_rows,
                Some((view.version, view.event_hash)),
                Some(manifest.cutover_at),
            )
        }
        _ => unreachable!("epoch handled separately"),
    };
    let (mut rows, lineage, unavailable) =
        historical_rows(conn, &source, &event_rows, None, cutover)?;
    filter_date(&mut rows, request.as_of)?;
    let mut period_ids = rows
        .iter()
        .map(|row| row.id)
        .collect::<std::collections::BTreeSet<_>>();
    for source_fill in source.fills() {
        let row = source_fill.fill();
        let original_date =
            crate::trading::paper_lot_ledger::parse_paper_fill_timestamp(row.id, &row.occurred_at)
                .map_err(LedgerError::IntegrityFailure)?
                .checked_add_signed(chrono::Duration::hours(8))
                .ok_or(LedgerError::Overflow)?
                .date();
        if original_date <= request.as_of {
            period_ids.insert(row.id);
        }
    }
    let period_lineage = lineage
        .iter()
        .filter(|proof| period_ids.contains(&proof.fill_id))
        .cloned()
        .collect();
    let adjudication_head =
        event_rows
            .iter()
            .rev()
            .find_map(|e| match decode::<Fact>(&e.payload) {
                Ok(Fact::AdjudicatedV1(_)) => Some((e.seq, e.event_hash.clone())),
                _ => None,
            });
    let mut receipt = EffectiveProjectionReceipt {
        request: request.clone(),
        raw_high_water: source.current_paper_trade_high_water(),
        raw_source_hash: source.all_status_paper_manifest_hash().into(),
        ledger_head: head,
        inventory_fingerprint: None,
        economic_head: economic_head(&event_rows)?,
        adjudication_head,
        cutover_at: cutover,
        cutover_raw_high_water: cutover.map(|_| source.current_paper_trade_high_water()),
        rule_version: "PaperEffectiveV1/LegacyNoCashAuthority/lot-rates-v1".into(),
        projection_hash: String::new(),
        catalog_generation: catalog.0,
        catalog_objects_hash: catalog.1,
    };
    receipt.projection_hash = projection_hash(&receipt, &rows, &lineage, &unavailable, &[])?;
    Ok(VerifiedEffectiveFillSet {
        receipt,
        rows,
        lineage,
        period_lineage,
        unavailable,
        seed_lots: Vec::new(),
    })
}
