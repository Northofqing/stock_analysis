use sha2::{Digest, Sha256};
use stock_analysis::database::user_position_snapshot::UserPositionSnapshot;
use stock_analysis::durable_delivery::{
    HoldingPlanOccurrenceObservation, HoldingPlanOwnedOccurrence,
};
use stock_analysis::market_domain::InstrumentId;

use super::{durable_delivery_runtime, market_data, push_templates, PreparedHoldingPlan};

/// A producer candidate, never sending or completion authority.
pub(super) enum HoldingPlanCandidate {
    Original {
        instrument: InstrumentId,
        owned: HoldingPlanOwnedOccurrence,
    },
    Fresh(PreparedHoldingPlan),
}

impl HoldingPlanCandidate {
    pub(super) fn instrument(&self) -> Result<&InstrumentId, String> {
        match self {
            Self::Original { instrument, .. } => Ok(instrument),
            Self::Fresh(prepared) => match prepared.binding.scope() {
                durable_delivery_runtime::CountedDeliveryScope::Ticket { instrument } => {
                    Ok(instrument)
                }
                _ => Err("holding_plan_ticket_scope_required".into()),
            },
        }
    }
    pub(super) fn business_date(&self) -> Result<chrono::NaiveDate, String> {
        match self {
            Self::Original { owned, .. } => {
                chrono::NaiveDate::parse_from_str(&owned.envelope().business_date, "%Y-%m-%d")
                    .map_err(|_| "holding_plan_original_date_invalid".into())
            }
            Self::Fresh(prepared) => Ok(prepared.binding.business_date()),
        }
    }
    pub(super) fn occurrence(&self) -> &str {
        match self {
            Self::Original { owned, .. } => &owned.envelope().schedule_occurrence_identity,
            Self::Fresh(prepared) => prepared.binding.schedule_occurrence_identity(),
        }
    }
}

pub(super) struct HoldingPlanPreparation {
    pub(super) candidates: Vec<HoldingPlanCandidate>,
    pub(super) failures: Vec<String>,
}

pub(super) struct HoldingPlanTickReport {
    pub(super) physically_accepted: usize,
    pub(super) failures: Vec<String>,
}

/// This is an inspection deadline, not a completed/delivered flag.
pub(super) struct HoldingPlanInspectionSchedule {
    date: chrono::NaiveDate,
    next: std::time::Instant,
}
impl HoldingPlanInspectionSchedule {
    pub(super) fn new(date: chrono::NaiveDate, now: std::time::Instant) -> Self {
        Self {
            date,
            next: now + std::time::Duration::from_secs(1800),
        }
    }
    pub(super) fn begin_if_due(
        &mut self,
        date: chrono::NaiveDate,
        now: std::time::Instant,
    ) -> bool {
        if self.date != date {
            *self = Self::new(date, now);
            return false;
        }
        if now < self.next {
            return false;
        }
        self.next = now + std::time::Duration::from_secs(1800);
        true
    }
}

fn instrument_for_code(code: &str) -> Result<InstrumentId, String> {
    let identity =
        stock_analysis::data_gateway::instrument_identity::resolve_production_equity(code, None)
            .map_err(|_| "holding_plan_instrument_invalid".to_owned())?;
    identity
        .require_a_share()
        .map_err(|_| "holding_plan_instrument_invalid".to_owned())?;
    Ok(identity.instrument().clone())
}

fn shanghai_capture(
    observed_at: chrono::DateTime<chrono::FixedOffset>,
) -> chrono::DateTime<chrono::FixedOffset> {
    // Same actual instant; never replace the captured time with a second now.
    observed_at
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 60 * 60).expect("valid Shanghai offset"))
        .fixed_offset()
}

pub(super) fn prepare_tick_with(
    banner: Option<&push_templates::BannerCtx>,
    load_snapshot: impl FnOnce() -> Result<Option<UserPositionSnapshot>, String>,
    mut inspect_owner: impl FnMut(
        chrono::NaiveDate,
        &InstrumentId,
    ) -> Result<HoldingPlanOccurrenceObservation, String>,
    load_legacy: impl FnOnce(chrono::NaiveDate) -> Result<std::collections::HashSet<String>, String>,
    fetch_quotes: impl FnOnce(&[String]) -> Result<market_data::TopStockBatch, String>,
    observed_at: chrono::DateTime<chrono::FixedOffset>,
) -> Result<HoldingPlanPreparation, String> {
    let observed_at = shanghai_capture(observed_at);
    let snapshot = load_snapshot()
        .map_err(|e| format!("持仓快照读取失败: {e}"))?
        .ok_or_else(|| "无用户确认持仓快照 (BR-226)".to_owned())?;
    let mut result = HoldingPlanPreparation {
        candidates: Vec::new(),
        failures: Vec::new(),
    };
    if snapshot.confirm_empty || snapshot.items.is_empty() {
        return Ok(result);
    }
    let date = observed_at.date_naive();
    let mut missing = Vec::new();
    for item in &snapshot.items {
        let instrument = match instrument_for_code(&item.code) {
            Ok(instrument) => instrument,
            Err(reason) => {
                result.failures.push(format!("code={} {reason}", item.code));
                continue;
            }
        };
        match inspect_owner(date, &instrument) {
            Ok(HoldingPlanOccurrenceObservation::Owned(owned)) => result
                .candidates
                .push(HoldingPlanCandidate::Original { instrument, owned }),
            Ok(HoldingPlanOccurrenceObservation::Missing) => missing.push(item.code.clone()),
            Err(e) => result
                .failures
                .push(format!("code={} 原 owner 读取拒绝: {e}", item.code)),
        }
    }
    // All business DB handles are dropped by this detached legacy reader
    // before the runtime counted/coordinator lock or any provider operation.
    let legacy = if missing.is_empty() {
        Ok(std::collections::HashSet::new())
    } else {
        load_legacy(date)
    };
    let mut fresh = Vec::new();
    for code in missing {
        match &legacy {
            Ok(markers) if markers.contains(&code) => result
                .failures
                .push(format!("code={code} holding_plan_legacy_unknown")),
            Ok(_) => fresh.push(code),
            Err(e) => result.failures.push(format!("code={code} 新候选阻断: {e}")),
        }
    }
    if !fresh.is_empty() {
        match banner {
            None => result
                .failures
                .push("holding_plan_fresh_banner_unavailable".into()),
            Some(banner) => {
                match prepare_fresh_subset(banner, &snapshot, fresh, fetch_quotes, observed_at) {
                    Ok(prepared) => result
                        .candidates
                        .extend(prepared.into_iter().map(HoldingPlanCandidate::Fresh)),
                    Err(e) => result.failures.push(e),
                }
            }
        }
    }
    Ok(result)
}

/// Legacy markers are evidence of an old workflow, never physical completion.
/// This reader does not create/heal/update the legacy table.
pub(super) fn read_legacy_markers(
    date: chrono::NaiveDate,
) -> Result<std::collections::HashSet<String>, String> {
    let mut connection = stock_analysis::database::DatabaseManager::get()
        .get_conn()
        .map_err(|_| "holding_plan_legacy_read_unavailable".to_owned())?;
    read_legacy_markers_with_connection(&mut connection, date)
}

struct LegacyReadFailure(String);
impl From<diesel::result::Error> for LegacyReadFailure {
    fn from(_: diesel::result::Error) -> Self {
        Self("holding_plan_legacy_transaction_failed".into())
    }
}

fn read_legacy_markers_with_connection(
    connection: &mut diesel::SqliteConnection,
    date: chrono::NaiveDate,
) -> Result<std::collections::HashSet<String>, String> {
    read_legacy_markers_transaction(connection, date, || {})
}

fn read_legacy_markers_transaction(
    connection: &mut diesel::SqliteConnection,
    date: chrono::NaiveDate,
    metadata_observed: impl FnOnce(),
) -> Result<std::collections::HashSet<String>, String> {
    use diesel::Connection;
    // SQLite's normal BEGIN is Deferred. No writer SQL is issued; all main
    // metadata, columns and rows belong to this one actual read snapshot.
    connection
        .transaction::<_, LegacyReadFailure, _>(|connection| {
            read_legacy_markers_snapshot(connection, date, metadata_observed)
                .map_err(LegacyReadFailure)
        })
        .map_err(|error| error.0)
}

fn read_legacy_markers_snapshot(
    connection: &mut diesel::SqliteConnection,
    date: chrono::NaiveDate,
    metadata_observed: impl FnOnce(),
) -> Result<std::collections::HashSet<String>, String> {
    use diesel::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Object {
        #[diesel(sql_type = diesel::sql_types::Text)]
        kind: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        definition: String,
    }
    #[derive(diesel::QueryableByName)]
    struct Column {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        cid: i32,
        #[diesel(sql_type = diesel::sql_types::Text)]
        name: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        declared_type: String,
        #[diesel(sql_type = diesel::sql_types::Integer)]
        required: i32,
        #[diesel(sql_type = diesel::sql_types::Integer)]
        pk: i32,
    }
    #[derive(diesel::QueryableByName)]
    struct Marker {
        #[diesel(sql_type = diesel::sql_types::Text)]
        plan_date: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        code: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        pushed_at: String,
    }
    let objects: Vec<Object> = diesel::sql_query(
        "SELECT type AS kind, sql AS definition FROM main.sqlite_master WHERE name = 'holding_plan_daily' COLLATE NOCASE UNION ALL SELECT type AS kind, sql AS definition FROM temp.sqlite_master WHERE name = 'holding_plan_daily' COLLATE NOCASE"
    ).load(connection).map_err(|_| "holding_plan_legacy_schema_read_failed".to_owned())?;
    metadata_observed();
    if objects.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    if objects.len() != 1 || objects[0].kind != "table" {
        return Err("holding_plan_legacy_schema_unknown".into());
    }
    let normalized = objects[0]
        .definition
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized != "createtableholding_plan_daily(plan_datetextnotnull,codetextnotnull,pushed_attextnotnull,primarykey(plan_date,code))" {
        return Err("holding_plan_legacy_schema_unknown".into());
    }
    let columns: Vec<Column> = diesel::sql_query(
        "SELECT cid, name, type AS declared_type, [notnull] AS required, pk FROM pragma_table_info('holding_plan_daily', 'main') ORDER BY cid"
    ).load(connection).map_err(|_| "holding_plan_legacy_schema_read_failed".to_owned())?;
    let expected = [("plan_date", 1), ("code", 2), ("pushed_at", 0)];
    if columns.len() != expected.len()
        || columns
            .iter()
            .zip(expected)
            .enumerate()
            .any(|(index, (column, (name, pk)))| {
                column.cid != index as i32
                    || column.name != name
                    || !column.declared_type.eq_ignore_ascii_case("TEXT")
                    || column.required != 1
                    || column.pk != pk
            })
    {
        return Err("holding_plan_legacy_schema_unknown".into());
    }
    let rows: Vec<Marker> = diesel::sql_query(
        "SELECT plan_date, code, pushed_at FROM main.holding_plan_daily WHERE plan_date = ?",
    )
    .bind::<diesel::sql_types::Text, _>(date.format("%Y-%m-%d").to_string())
    .load(connection)
    .map_err(|_| "holding_plan_legacy_row_read_failed".to_owned())?;
    let mut markers = std::collections::HashSet::new();
    for row in rows {
        if row.plan_date != date.format("%Y-%m-%d").to_string()
            || row.code.len() != 6
            || !row.code.bytes().all(|b| b.is_ascii_digit())
            || chrono::DateTime::parse_from_rfc3339(&row.pushed_at).is_err()
            || !markers.insert(row.code)
        {
            return Err("holding_plan_legacy_row_unknown".into());
        }
    }
    Ok(markers)
}

pub(super) async fn dispatch_tick(
    banner: Option<push_templates::BannerCtx>,
) -> HoldingPlanTickReport {
    let preparation = match durable_delivery_runtime::prepare_holding_plan_tick(banner).await {
        Ok(value) => value,
        Err(e) => {
            return HoldingPlanTickReport {
                physically_accepted: 0,
                failures: vec![e],
            }
        }
    };
    let mut report = HoldingPlanTickReport {
        physically_accepted: 0,
        failures: preparation.failures,
    };
    for candidate in preparation.candidates {
        match dispatch_candidate(candidate).await {
            Ok(()) => report.physically_accepted += 1,
            Err(e) => report.failures.push(e),
        }
    }
    report
}

async fn dispatch_candidate(candidate: HoldingPlanCandidate) -> Result<(), String> {
    let candidate =
        match durable_delivery_runtime::reconcile_holding_plan_candidate(candidate).await? {
            durable_delivery_runtime::HoldingPlanLocalProgress::PhysicalAccepted => return Ok(()),
            durable_delivery_runtime::HoldingPlanLocalProgress::NotSendable(reason) => {
                return Err(reason)
            }
            durable_delivery_runtime::HoldingPlanLocalProgress::Candidate(candidate) => candidate,
        };
    let outcome = super::notify::push_holding_plan_candidate(candidate).await?;
    match outcome {
        durable_delivery_runtime::HoldingPlanDispatchResult::PhysicalAccepted => Ok(()),
        durable_delivery_runtime::HoldingPlanDispatchResult::NotCompleted(reason) => Err(reason),
        durable_delivery_runtime::HoldingPlanDispatchResult::AlreadyOwned(candidate) => {
            // The first critical section has ended. Governance is now applied
            // to the actual original winner; there is no second quote/prepare.
            let candidate = match durable_delivery_runtime::reconcile_holding_plan_candidate(
                candidate,
            )
            .await?
            {
                durable_delivery_runtime::HoldingPlanLocalProgress::PhysicalAccepted => {
                    return Ok(())
                }
                durable_delivery_runtime::HoldingPlanLocalProgress::NotSendable(reason) => {
                    return Err(reason)
                }
                durable_delivery_runtime::HoldingPlanLocalProgress::Candidate(candidate) => {
                    candidate
                }
            };
            match super::notify::push_holding_plan_candidate(candidate).await? {
                durable_delivery_runtime::HoldingPlanDispatchResult::PhysicalAccepted => Ok(()),
                durable_delivery_runtime::HoldingPlanDispatchResult::NotCompleted(reason) => {
                    Err(reason)
                }
                durable_delivery_runtime::HoldingPlanDispatchResult::AlreadyOwned(_) => {
                    Err("holding_plan_owner_changed_again".into())
                }
            }
        }
    }
}

#[cfg(test)]
pub(super) fn prepare_holding_plan_messages_with(
    banner: &push_templates::BannerCtx,
    load_snapshot: impl FnOnce() -> Result<Option<UserPositionSnapshot>, String>,
    fetch_quote_batch: impl FnOnce(&[String]) -> Result<market_data::TopStockBatch, String>,
    capture_now: impl FnOnce() -> chrono::DateTime<chrono::FixedOffset>,
) -> Result<Vec<PreparedHoldingPlan>, String> {
    let observed_at = shanghai_capture(capture_now());
    let snapshot = load_snapshot()
        .map_err(|error| format!("持仓快照读取失败: {error}"))?
        .ok_or_else(|| "无用户确认持仓快照 (BR-226)".to_string())?;
    let requested = snapshot
        .items
        .iter()
        .map(|item| item.code.clone())
        .collect::<Vec<_>>();
    prepare_fresh_subset(banner, &snapshot, requested, fetch_quote_batch, observed_at)
}

fn prepare_fresh_subset(
    banner: &push_templates::BannerCtx,
    snapshot: &UserPositionSnapshot,
    requested_codes: Vec<String>,
    fetch_quote_batch: impl FnOnce(&[String]) -> Result<market_data::TopStockBatch, String>,
    observed_at: chrono::DateTime<chrono::FixedOffset>,
) -> Result<Vec<PreparedHoldingPlan>, String> {
    if snapshot.confirm_empty || requested_codes.is_empty() {
        return Ok(Vec::new());
    }
    let instruments = requested_codes
        .iter()
        .map(|code| instrument_for_code(code).map(|instrument| (code.clone(), instrument)))
        .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
    let quote_batch = fetch_quote_batch(&requested_codes)
        .map_err(|error| format!("持仓行情批次拒绝: {error}"))?;
    if quote_batch.coverage != stock_analysis::data_gateway::QuoteCoverageDisposition::Complete
        || quote_batch.requested != requested_codes
        || !quote_batch.rejected.is_empty()
        || !quote_batch.missing.is_empty()
    {
        return Err(format!(
            "持仓行情批次拒绝: coverage={:?} requested={:?} rejected={} missing={:?}",
            quote_batch.coverage,
            quote_batch.requested,
            quote_batch.rejected.len(),
            quote_batch.missing
        ));
    }
    let quote_map: std::collections::HashMap<String, &stock_analysis::market_data::TopStock> =
        quote_batch
            .stocks
            .iter()
            .map(|quote| (quote.code.clone(), quote))
            .collect();

    let business_date = observed_at.date_naive();
    let hhmm = observed_at.format("%H:%M").to_string();
    let mut out = Vec::new();
    for item in snapshot
        .items
        .iter()
        .filter(|item| requested_codes.contains(&item.code))
    {
        let Some(quote) = quote_map.get(&item.code) else {
            log::warn!("[T-03] code={} 行情缺失, 跳过该票 (其余照常)", item.code);
            continue;
        };
        if item.cost_price <= 0.0 {
            log::warn!("[T-03] code={} 成本价非法, 跳过", item.code);
            continue;
        }
        let pnl_pct = (quote.price / item.cost_price - 1.0) * 100.0;
        let stop = item.cost_price * 0.92;
        let intent = if quote.price <= stop {
            push_templates::Intent::StopLossReview
        } else if pnl_pct > 5.0 {
            push_templates::Intent::Reduce
        } else if pnl_pct < -3.0 {
            push_templates::Intent::Add
        } else {
            push_templates::Intent::Hold
        };
        let reason = match intent {
            push_templates::Intent::StopLossReview => {
                format!("浮亏 {pnl_pct:.1}% 已触及成本止损参考线，暂停加仓并核查风险")
            }
            push_templates::Intent::Reduce => {
                format!("浮盈 {pnl_pct:.1}% 触发减仓观察 (>+5%)")
            }
            push_templates::Intent::Add => format!("浮亏 {pnl_pct:.1}% 触发加仓观察 (<-3%)"),
            push_templates::Intent::Hold => format!("浮盈 {pnl_pct:.1}%, 持有观望区间"),
            _ => unreachable!("T-03 只产出 StopLossReview/Reduce/Add/Hold"),
        };
        let reasons = vec![reason];
        let text = push_templates::render_holding_plan(
            banner,
            push_templates::HoldingPlanParams {
                name: &item.name,
                code: &item.code,
                hhmm: &hhmm,
                intent,
                price: quote.price,
                cost: item.cost_price,
                avail: u32::try_from(item.quantity).unwrap_or(u32::MAX),
                reduce_zone: (intent != push_templates::Intent::StopLossReview)
                    .then_some((item.cost_price * 1.02, item.cost_price * 1.05)),
                support: item.cost_price * 0.95,
                pressure: item.cost_price * 1.10,
                stop,
                invalidations: &[],
                reasons: &reasons,
            },
        );
        let canonical = serde_json::json!({
            "schema_version": "HOLDING_PLAN_SOURCE_BINDING_V1",
            "code": item.code,
            "name": item.name,
            "intent": intent.label(),
            "price": quote.price,
            "cost": item.cost_price,
            "quantity": item.quantity,
            "pnl_pct": pnl_pct,
            "observed_at": observed_at.to_rfc3339(),
            "snapshot": {
                "snapshot_row_id": snapshot.snapshot_row_id,
                "snapshot_id": snapshot.snapshot_id,
                "effective_at": snapshot.effective_at.to_rfc3339(),
                "confirmed_at": snapshot.confirmed_at.to_rfc3339(),
                "source": snapshot.source,
                "confirm_empty": snapshot.confirm_empty,
                "evidence_sha256": snapshot.evidence_sha256,
            },
            "quote_batch": {
                "provider": quote_batch.evidence.provider,
                "source": quote_batch.evidence.source,
                "source_at": quote_batch.evidence.source_at,
                "observed_at": quote_batch.evidence.observed_at,
                "batch_id": quote_batch.evidence.batch_id,
                "coverage": format!("{:?}", quote_batch.coverage),
                "requested": quote_batch.requested,
                "rejected": quote_batch.rejected.iter().map(|row| serde_json::json!({
                    "code": row.code,
                    "reason_code": row.reason_code,
                    "message": row.message,
                })).collect::<Vec<_>>(),
                "missing": quote_batch.missing,
            },
            "requested_codes": requested_codes,
        });
        let canonical_bytes = canonical.to_string().into_bytes();
        let subject_hash = hex::encode(Sha256::digest(&canonical_bytes));
        let instrument = instruments
            .get(&item.code)
            .cloned()
            .ok_or_else(|| "holding_plan_instrument_invalid".to_owned())?;
        let binding = durable_delivery_runtime::CountedDeliveryBinding::new(
            business_date,
            format!("holding-plan:{business_date}:{}", item.code),
            canonical_bytes,
            durable_delivery_runtime::CountedDeliveryScope::Ticket { instrument },
            subject_hash,
            durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
            None,
            true,
        )
        .map_err(|error| format!("counted binding 构造失败 code={}: {error}", item.code))?;
        out.push(PreparedHoldingPlan {
            code: item.code.clone(),
            text,
            binding,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use stock_analysis::data_gateway::BatchEvidence;
    use stock_analysis::market_data::TopStock;
    use stock_analysis::market_domain::ProviderId;
    use stock_analysis::portfolio::user_position_snapshot::UserPositionItemInput;

    use super::*;

    fn position(code: &str, name: &str, quantity: u64, cost_price: f64) -> UserPositionItemInput {
        UserPositionItemInput {
            code: code.to_owned(),
            name: name.to_owned(),
            quantity,
            cost_price,
        }
    }

    fn snapshot_with(snapshot_id: &str, items: Vec<UserPositionItemInput>) -> UserPositionSnapshot {
        UserPositionSnapshot {
            snapshot_row_id: 41,
            snapshot_id: snapshot_id.to_owned(),
            effective_at: chrono::DateTime::parse_from_rfc3339("2026-09-09T15:00:00+08:00")
                .expect("fixture effective_at"),
            confirmed_at: chrono::DateTime::parse_from_rfc3339("2026-09-09T15:05:00+08:00")
                .expect("fixture confirmed_at"),
            source: "TEST_CODE_USER_CONFIRMED".to_owned(),
            confirm_empty: false,
            evidence_sha256: "a".repeat(64),
            items,
        }
    }

    fn snapshot() -> UserPositionSnapshot {
        snapshot_with(
            "TEST_CODE_SNAPSHOT_A",
            vec![position("600000", "浦发银行", 300, 8.0)],
        )
    }

    fn quote(code: &str, name: &str, price: f64) -> TopStock {
        TopStock {
            code: code.to_owned(),
            name: name.to_owned(),
            price,
            change_pct: 1.0,
            volume_ratio: None,
            main_net_yi: None,
        }
    }

    fn quote_batch_with(
        stocks: Vec<TopStock>,
        batch_id: &str,
        source_at: Option<&str>,
    ) -> market_data::TopStockBatch {
        let requested = stocks.iter().map(|stock| stock.code.clone()).collect();
        market_data::TopStockBatch {
            stocks,
            evidence: BatchEvidence {
                provider: ProviderId::Tencent,
                source: "TEST_CODE_QUOTE_SOURCE".to_owned(),
                source_at: source_at.map(str::to_owned),
                observed_at: "2026-09-10T09:30:01+08:00".to_owned(),
                batch_id: batch_id.to_owned(),
            },
            coverage: stock_analysis::data_gateway::QuoteCoverageDisposition::Complete,
            requested,
            rejected: Vec::new(),
            missing: Vec::new(),
        }
    }

    fn quote_batch() -> market_data::TopStockBatch {
        quote_batch_with(
            vec![quote("600000", "浦发银行", 8.5)],
            "TEST_CODE_QUOTE_BATCH_A",
            Some("2026-09-10T09:29:58+08:00"),
        )
    }

    fn now() -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339("2026-09-10T09:30:02+08:00")
            .expect("fixture observed_at")
    }

    fn canonical(prepared: &PreparedHoldingPlan) -> serde_json::Value {
        serde_json::from_slice(prepared.binding.source_binding_canonical())
            .expect("canonical source binding")
    }

    #[test]
    fn holding_plan_source_binding_retains_snapshot_and_quote_batch_provenance() {
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot())),
            |_| Ok(quote_batch()),
            now,
        )
        .expect("prepare holding plan");

        let canonical = canonical(&prepared[0]);
        assert_eq!(
            canonical["schema_version"],
            "HOLDING_PLAN_SOURCE_BINDING_V1"
        );
        assert_eq!(canonical["code"], "600000");
        assert_eq!(canonical["name"], "浦发银行");
        assert_eq!(canonical["intent"], "逢高减仓");
        assert_eq!(canonical["price"], 8.5);
        assert_eq!(canonical["cost"], 8.0);
        assert_eq!(canonical["quantity"], 300);
        assert_eq!(canonical["pnl_pct"], 6.25);
        assert_eq!(canonical["observed_at"], "2026-09-10T09:30:02+08:00");
        assert_eq!(canonical["snapshot"]["snapshot_row_id"], 41);
        assert_eq!(canonical["snapshot"]["snapshot_id"], "TEST_CODE_SNAPSHOT_A");
        assert_eq!(
            canonical["snapshot"]["effective_at"],
            "2026-09-09T15:00:00+08:00"
        );
        assert_eq!(
            canonical["snapshot"]["confirmed_at"],
            "2026-09-09T15:05:00+08:00"
        );
        assert_eq!(canonical["snapshot"]["source"], "TEST_CODE_USER_CONFIRMED");
        assert_eq!(canonical["snapshot"]["confirm_empty"], false);
        assert_eq!(canonical["snapshot"]["evidence_sha256"], "a".repeat(64));
        assert_eq!(canonical["quote_batch"]["provider"], "Tencent");
        assert_eq!(canonical["quote_batch"]["source"], "TEST_CODE_QUOTE_SOURCE");
        assert_eq!(
            canonical["quote_batch"]["source_at"],
            "2026-09-10T09:29:58+08:00"
        );
        assert_eq!(
            canonical["quote_batch"]["observed_at"],
            "2026-09-10T09:30:01+08:00"
        );
        assert_eq!(
            canonical["quote_batch"]["batch_id"],
            "TEST_CODE_QUOTE_BATCH_A"
        );
        assert_eq!(canonical["requested_codes"], serde_json::json!(["600000"]));
    }

    #[test]
    fn missing_or_failed_snapshot_never_requests_quotes() {
        for (snapshot_result, expected_error) in [
            (Ok(None), "无用户确认持仓快照 (BR-226)"),
            (
                Err("TEST_CODE_DB_DOWN".to_owned()),
                "持仓快照读取失败: TEST_CODE_DB_DOWN",
            ),
        ] {
            let quote_called = Cell::new(false);
            let error = prepare_holding_plan_messages_with(
                &push_templates::BannerCtx::test_default(),
                || snapshot_result,
                |_| {
                    quote_called.set(true);
                    Ok(quote_batch())
                },
                now,
            )
            .err()
            .expect("snapshot failure must stop preparation");
            assert_eq!(error, expected_error);
            assert!(!quote_called.get());
        }
    }

    #[test]
    fn confirmed_empty_or_itemless_snapshot_is_silent_without_quotes() {
        let mut confirmed_empty = snapshot();
        confirmed_empty.confirm_empty = true;
        let itemless = snapshot_with("TEST_CODE_ITEMLESS", Vec::new());

        for snapshot in [confirmed_empty, itemless] {
            let quote_called = Cell::new(false);
            let prepared = prepare_holding_plan_messages_with(
                &push_templates::BannerCtx::test_default(),
                || Ok(Some(snapshot)),
                |_| {
                    quote_called.set(true);
                    Ok(quote_batch())
                },
                now,
            )
            .expect("empty snapshot is a successful no-op");
            assert!(prepared.is_empty());
            assert!(!quote_called.get());
        }
    }

    #[test]
    fn quote_batch_failure_is_propagated() {
        let error = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot())),
            |_| Err("TEST_CODE_QUOTES_DOWN".to_owned()),
            now,
        )
        .err()
        .expect("quote failure must stop preparation");

        assert_eq!(error, "持仓行情批次拒绝: TEST_CODE_QUOTES_DOWN");
    }

    #[test]
    fn task9_partial_quote_coverage_cannot_prepare_any_holding_decision() {
        let snapshot = snapshot_with(
            "TEST_CODE_PARTIAL",
            vec![
                position("600000", "浦发银行", 300, 8.0),
                position("000001", "平安银行", 200, 10.0),
            ],
        );
        let error = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot)),
            |_| {
                let mut batch = quote_batch();
                batch.coverage = stock_analysis::data_gateway::QuoteCoverageDisposition::Partial;
                batch.requested = vec!["600000".to_owned(), "000001".to_owned()];
                batch.missing = vec!["000001".to_owned()];
                Ok(batch)
            },
            now,
        )
        .err()
        .expect("partial coverage must fail the atomic holding-plan join");
        assert!(error.contains("coverage=Partial"));
        assert!(error.contains("000001"));
    }

    #[test]
    fn nonpositive_cost_is_skipped_without_hiding_other_positions() {
        let snapshot = snapshot_with(
            "TEST_CODE_INVALID_COST",
            vec![
                position("600000", "非法成本", 300, 0.0),
                position("000001", "有效持仓", 200, 10.0),
            ],
        );
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot)),
            |_| {
                Ok(quote_batch_with(
                    vec![
                        quote("600000", "非法成本", 8.5),
                        quote("000001", "有效持仓", 10.0),
                    ],
                    "TEST_CODE_INVALID_COST_BATCH",
                    Some("2026-09-10T09:29:58+08:00"),
                ))
            },
            now,
        )
        .expect("invalid item is isolated");

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].code, "000001");
    }

    #[test]
    fn reduce_and_add_positions_use_snapshot_cost_quantity_and_batch_price() {
        let snapshot = snapshot_with(
            "TEST_CODE_REDUCE_ADD",
            vec![
                position("600000", "减仓票", 300, 8.0),
                position("000001", "加仓票", 200, 10.0),
            ],
        );
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot)),
            |_| {
                Ok(quote_batch_with(
                    vec![
                        quote("600000", "行情名称不绑定", 8.5),
                        quote("000001", "行情名称不绑定", 9.5),
                    ],
                    "TEST_CODE_REDUCE_ADD_BATCH",
                    Some("2026-09-10T09:29:58+08:00"),
                ))
            },
            now,
        )
        .expect("prepare reduce and add proposals");

        assert_eq!(prepared.len(), 2);
        assert!(prepared[0]
            .text
            .contains("持仓建议 减仓票(600000)（09:30）"));
        assert!(prepared[0]
            .text
            .contains("动作倾向: 逢高减仓 | 现价8.50 成本8.00 可用300股"));
        assert!(prepared[1]
            .text
            .contains("持仓建议 加仓票(000001)（09:30）"));
        assert!(prepared[1]
            .text
            .contains("动作倾向: 加仓 | 现价9.50 成本10.00 可用200股"));
        assert_eq!(canonical(&prepared[0])["intent"], "逢高减仓");
        assert_eq!(canonical(&prepared[1])["intent"], "加仓");
    }

    #[test]
    fn loss_at_or_below_stop_suspends_adding_and_calls_for_review() {
        for price in [9.19, 9.20] {
            let prepared = prepare_holding_plan_messages_with(
                &push_templates::BannerCtx::test_default(),
                || {
                    Ok(Some(snapshot_with(
                        "TEST_CODE_AT_OR_BELOW_STOP",
                        vec![position("000001", "止损核查票", 200, 10.0)],
                    )))
                },
                |_| {
                    Ok(quote_batch_with(
                        vec![quote("000001", "止损核查票", price)],
                        "TEST_CODE_AT_OR_BELOW_STOP_BATCH",
                        Some("2026-09-10T09:29:58+08:00"),
                    ))
                },
                now,
            )
            .expect("prepare at-or-below-stop holding plan");

            assert_eq!(prepared.len(), 1, "price={price}");
            assert_eq!(canonical(&prepared[0])["intent"], "止损核查");
            assert!(prepared[0].text.contains("动作倾向: 止损核查"));
            assert!(prepared[0].text.contains("成本参考价位"));
            assert!(prepared[0].text.contains("暂停加仓并核查风险"));
            assert!(!prepared[0].text.contains("减仓观察区"));
        }
    }

    #[test]
    fn hold_position_uses_cost_reference_levels() {
        let snapshot = snapshot_with(
            "TEST_CODE_HOLD",
            vec![position("600000", "持有票", u64::from(u32::MAX) + 1, 10.0)],
        );
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot)),
            |_| {
                Ok(quote_batch_with(
                    vec![quote("600000", "行情名称不绑定", 10.0)],
                    "TEST_CODE_HOLD_BATCH",
                    Some("2026-09-10T09:29:58+08:00"),
                ))
            },
            now,
        )
        .expect("prepare hold proposal");

        assert_eq!(canonical(&prepared[0])["intent"], "持有观望");
        assert!(prepared[0].text.contains("动作倾向: 持有观望"));
        assert!(prepared[0]
            .text
            .contains("现价10.00 成本10.00 可用4294967295股"));
        assert!(prepared[0]
            .text
            .contains("成本参考价位: 下沿9.50 | 上沿11.00 | 止损线9.20"));
    }

    #[test]
    fn one_captured_local_time_drives_every_proposal_date_text_and_binding() {
        let clock_calls = Cell::new(0);
        let snapshot = snapshot_with(
            "TEST_CODE_ONE_CLOCK",
            vec![
                position("600000", "甲", 300, 8.0),
                position("000001", "乙", 200, 10.0),
            ],
        );
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot)),
            |_| {
                Ok(quote_batch_with(
                    vec![quote("600000", "甲", 8.5), quote("000001", "乙", 10.0)],
                    "TEST_CODE_ONE_CLOCK_BATCH",
                    Some("2026-09-10T09:29:58+08:00"),
                ))
            },
            || {
                clock_calls.set(clock_calls.get() + 1);
                now()
            },
        )
        .expect("prepare one clock round");

        assert_eq!(clock_calls.get(), 1);
        assert_eq!(prepared.len(), 2);
        for message in prepared {
            assert_eq!(message.binding.business_date().to_string(), "2026-09-10");
            assert!(message.text.contains("（09:30）"));
            assert_eq!(
                canonical(&message)["observed_at"],
                "2026-09-10T09:30:02+08:00"
            );
        }
    }

    #[test]
    fn absent_quote_source_at_remains_null() {
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot())),
            |_| {
                Ok(quote_batch_with(
                    vec![quote("600000", "浦发银行", 8.5)],
                    "TEST_CODE_NO_SOURCE_AT",
                    None,
                ))
            },
            now,
        )
        .expect("prepare quote without provider source time");

        let canonical = canonical(&prepared[0]);
        let quote_batch = canonical["quote_batch"]
            .as_object()
            .expect("quote_batch object");
        assert!(quote_batch.contains_key("source_at"));
        assert!(quote_batch["source_at"].is_null());
    }

    #[test]
    fn snapshot_and_quote_batch_identity_changes_are_visible_without_changing_text() {
        let prepare = |snapshot_id: &str, batch_id: &str| {
            prepare_holding_plan_messages_with(
                &push_templates::BannerCtx::test_default(),
                || {
                    Ok(Some(snapshot_with(
                        snapshot_id,
                        vec![position("600000", "浦发银行", 300, 8.0)],
                    )))
                },
                |_| {
                    Ok(quote_batch_with(
                        vec![quote("600000", "浦发银行", 8.5)],
                        batch_id,
                        Some("2026-09-10T09:29:58+08:00"),
                    ))
                },
                now,
            )
            .expect("prepare identity variant")
            .remove(0)
        };
        let base = prepare("TEST_CODE_SNAPSHOT_A", "TEST_CODE_QUOTE_BATCH_A");
        let changed_snapshot = prepare("TEST_CODE_SNAPSHOT_B", "TEST_CODE_QUOTE_BATCH_A");
        let changed_quote = prepare("TEST_CODE_SNAPSHOT_A", "TEST_CODE_QUOTE_BATCH_B");

        assert_eq!(base.text, changed_snapshot.text);
        assert_eq!(base.text, changed_quote.text);
        assert_ne!(
            base.binding.source_binding_canonical(),
            changed_snapshot.binding.source_binding_canonical()
        );
        assert_ne!(
            base.binding.source_evidence_fingerprint(),
            changed_snapshot.binding.source_evidence_fingerprint()
        );
        assert_ne!(
            base.binding.source_binding_canonical(),
            changed_quote.binding.source_binding_canonical()
        );
        assert_ne!(
            base.binding.source_evidence_fingerprint(),
            changed_quote.binding.source_evidence_fingerprint()
        );
    }

    #[test]
    fn snapshot_selected_before_external_latest_changes_remains_the_join_source() {
        let latest = RefCell::new(snapshot_with(
            "TEST_CODE_SNAPSHOT_A",
            vec![position("600000", "快照A名称", 300, 8.0)],
        ));
        let requested = RefCell::new(Vec::<String>::new());
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || {
                let selected = latest.borrow().clone();
                *latest.borrow_mut() = snapshot_with(
                    "TEST_CODE_SNAPSHOT_B",
                    vec![position("000001", "快照B名称", 200, 10.0)],
                );
                Ok(Some(selected))
            },
            |codes| {
                *requested.borrow_mut() = codes.to_vec();
                Ok(quote_batch_with(
                    vec![quote("600000", "行情名称不绑定", 8.5)],
                    "TEST_CODE_SNAPSHOT_RACE_BATCH",
                    Some("2026-09-10T09:29:58+08:00"),
                ))
            },
            now,
        )
        .expect("prepare selected snapshot");

        assert_eq!(requested.borrow().clone(), vec!["600000".to_owned()]);
        assert_eq!(prepared.len(), 1);
        assert!(prepared[0].text.contains("快照A名称(600000)"));
        let canonical = canonical(&prepared[0]);
        assert_eq!(canonical["snapshot"]["snapshot_id"], "TEST_CODE_SNAPSHOT_A");
        assert_eq!(canonical["requested_codes"], serde_json::json!(["600000"]));
        assert_eq!(canonical["code"], "600000");
    }
    #[test]
    fn t03_exact_owner_bin_legacy_read_is_select_only_and_unknown_schema_blocks_fresh() {
        use diesel::connection::SimpleConnection;
        use diesel::{Connection, RunQueryDsl};
        let mut connection = diesel::SqliteConnection::establish(":memory:").unwrap();
        let date = now().date_naive();
        assert!(read_legacy_markers_with_connection(&mut connection, date)
            .unwrap()
            .is_empty());
        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type=diesel::sql_types::BigInt)]
            total: i64,
        }
        let tables = || {
            diesel::sql_query(
                "SELECT count(*) AS total FROM sqlite_master WHERE name='holding_plan_daily'",
            )
        };
        assert_eq!(
            tables().get_result::<Count>(&mut connection).unwrap().total,
            0
        );
        connection.batch_execute("CREATE TABLE holding_plan_daily(plan_date TEXT NOT NULL,code TEXT NOT NULL,pushed_at TEXT NOT NULL,PRIMARY KEY(plan_date,code)); INSERT INTO holding_plan_daily VALUES('2026-09-10','600000','2026-09-10T09:30:02+08:00');").unwrap();
        for _ in 0..3 {
            assert_eq!(
                read_legacy_markers_with_connection(&mut connection, date).unwrap(),
                std::collections::HashSet::from(["600000".to_owned()])
            );
        }
        assert_eq!(
            diesel::sql_query("SELECT count(*) AS total FROM holding_plan_daily")
                .get_result::<Count>(&mut connection)
                .unwrap()
                .total,
            1
        );
        connection
            .batch_execute("ALTER TABLE holding_plan_daily ADD COLUMN unknown TEXT;")
            .unwrap();
        assert_eq!(
            read_legacy_markers_with_connection(&mut connection, date).unwrap_err(),
            "holding_plan_legacy_schema_unknown"
        );
        assert_eq!(
            diesel::sql_query("SELECT count(*) AS total FROM holding_plan_daily")
                .get_result::<Count>(&mut connection)
                .unwrap()
                .total,
            1
        );
    }

    #[test]
    fn t03_exact_owner_bin_malformed_legacy_row_and_temp_shadow_are_not_absence() {
        use diesel::connection::SimpleConnection;
        use diesel::Connection;
        let mut connection = diesel::SqliteConnection::establish(":memory:").unwrap();
        connection.batch_execute("CREATE TABLE holding_plan_daily(plan_date TEXT NOT NULL,code TEXT NOT NULL,pushed_at TEXT NOT NULL,PRIMARY KEY(plan_date,code)); INSERT INTO holding_plan_daily VALUES('2026-09-10','600000','TEST_CODE_BAD_TIME');").unwrap();
        assert_eq!(
            read_legacy_markers_with_connection(&mut connection, now().date_naive()).unwrap_err(),
            "holding_plan_legacy_row_unknown"
        );
        connection.batch_execute("CREATE TEMP TABLE holding_plan_daily(plan_date TEXT NOT NULL,code TEXT NOT NULL,pushed_at TEXT NOT NULL,PRIMARY KEY(plan_date,code));").unwrap();
        assert_eq!(
            read_legacy_markers_with_connection(&mut connection, now().date_naive()).unwrap_err(),
            "holding_plan_legacy_schema_unknown"
        );
    }

    #[test]
    fn t03_exact_owner_bin_inspection_deadline_advances_on_failures_and_is_date_bound() {
        let start = std::time::Instant::now();
        let date = now().date_naive();
        let mut schedule = HoldingPlanInspectionSchedule::new(date, start);
        assert!(!schedule.begin_if_due(date, start + std::time::Duration::from_secs(1799)));
        for failed_tick in 1..=3 {
            let due = start + std::time::Duration::from_secs(failed_tick * 1800);
            assert!(schedule.begin_if_due(date, due));
            assert!(!schedule.begin_if_due(date, due + std::time::Duration::from_secs(1)));
        }
        let next_date = date.succ_opt().unwrap();
        let next = start + std::time::Duration::from_secs(6000);
        assert!(!schedule.begin_if_due(next_date, next));
        assert!(!schedule.begin_if_due(next_date, next + std::time::Duration::from_secs(1799)));
        assert!(schedule.begin_if_due(next_date, next + std::time::Duration::from_secs(1800)));
    }

    #[test]
    fn t03_exact_owner_bin_real_callers_use_shared_owner_route_and_keep_governance_order() {
        let main = include_str!("main.rs");
        let manual = include_str!("manual_push.rs");
        assert!(main.contains("holding_plan::dispatch_tick(current_banner_for("));
        assert!(manual.contains("crate::holding_plan::dispatch_tick(Some(banner.clone()))"));
        assert!(!main.contains("T03_RETRY_CAPS"));
        assert!(!main.contains("fn holding_plan_daily_record"));
        assert!(!main.contains("prepare_holding_plan_messages("));
        assert!(!manual.contains("push_counted_with_binding("));
        let notify = include_str!("notify.rs");
        let route = &notify[notify.find("fn preflight_holding_plan_candidate(").unwrap()
            ..notify
                .find("pub(crate) async fn push_p01_origin_with_binding(")
                .unwrap()];
        assert!(
            route.find("launch_gate_check(").unwrap()
                < route.find("v14_gate_counted_binding(").unwrap()
        );
        assert!(
            route.find("acquire_token(").unwrap()
                < route
                    .find("preflight_holding_plan_candidate(token,")
                    .unwrap()
        );
        assert!(
            route
                .find("preflight_holding_plan_candidate(token,")
                .unwrap()
                < route
                    .find("deliver_holding_plan_candidate(governed)")
                    .unwrap()
        );
    }
    #[test]
    fn t03_exact_owner_bin_fresh_capture_keeps_same_instant_and_shanghai_date_text() {
        let capture = chrono::DateTime::parse_from_rfc3339("2026-09-09T17:30:02-07:00").unwrap();
        let mut capture_calls = 0;
        let prepared = prepare_holding_plan_messages_with(
            &push_templates::BannerCtx::test_default(),
            || Ok(Some(snapshot())),
            |_| Ok(quote_batch()),
            || {
                capture_calls += 1;
                capture
            },
        )
        .unwrap();
        assert_eq!(capture_calls, 1);
        assert_eq!(
            prepared[0].binding.business_date(),
            chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap()
        );
        assert!(prepared[0].text.contains("（08:30）"));
        let source = canonical(&prepared[0]);
        let saved =
            chrono::DateTime::parse_from_rfc3339(source["observed_at"].as_str().unwrap()).unwrap();
        assert_eq!(saved, capture);
        assert_eq!(saved.offset().local_minus_utc(), 8 * 3600);
    }

    #[test]
    fn t03_exact_owner_bin_legacy_multi_read_is_one_actual_wal_snapshot() {
        use diesel::connection::SimpleConnection;
        use diesel::{Connection, RunQueryDsl};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("TEST_CODE_T03_LEGACY.sqlite3");
        let mut reader = diesel::SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        reader.batch_execute("PRAGMA journal_mode=WAL; CREATE TABLE holding_plan_daily(plan_date TEXT NOT NULL,code TEXT NOT NULL,pushed_at TEXT NOT NULL,PRIMARY KEY(plan_date,code));").unwrap();
        let mut writer = diesel::SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        let mut hook_calls = 0;
        let markers=read_legacy_markers_transaction(&mut reader,now().date_naive(),|| {
            hook_calls+=1;
            writer.batch_execute("ALTER TABLE holding_plan_daily ADD COLUMN unknown TEXT; INSERT INTO holding_plan_daily(plan_date,code,pushed_at) VALUES('2026-09-10','600000','2026-09-10T09:30:02+08:00');").unwrap();
        }).unwrap();
        assert_eq!(hook_calls, 1);
        assert!(
            markers.is_empty(),
            "old metadata/columns/rows must share original snapshot"
        );
        assert_eq!(
            read_legacy_markers_with_connection(&mut reader, now().date_naive()).unwrap_err(),
            "holding_plan_legacy_schema_unknown"
        );
        #[derive(diesel::QueryableByName)]
        struct Count {
            #[diesel(sql_type=diesel::sql_types::BigInt)]
            total: i64,
        }
        let reader_changes = diesel::sql_query("SELECT total_changes() AS total")
            .get_result::<Count>(&mut reader)
            .unwrap()
            .total;
        assert_eq!(
            reader_changes, 0,
            "snapshot reader performs no business writes"
        );
        assert_eq!(
            diesel::sql_query("SELECT count(*) AS total FROM main.holding_plan_daily")
                .get_result::<Count>(&mut writer)
                .unwrap()
                .total,
            1
        );
        assert_eq!(
            diesel::sql_query(
                "SELECT count(*) AS total FROM pragma_table_info('holding_plan_daily','main')"
            )
            .get_result::<Count>(&mut writer)
            .unwrap()
            .total,
            4
        );
        // The failed snapshot releases its transaction; the other connection
        // can still complete a subsequent independent write.
        writer.batch_execute("INSERT INTO holding_plan_daily(plan_date,code,pushed_at) VALUES('2026-09-10','000001','2026-09-10T09:31:02+08:00');").unwrap();
        assert_eq!(
            diesel::sql_query("SELECT count(*) AS total FROM main.holding_plan_daily")
                .get_result::<Count>(&mut writer)
                .unwrap()
                .total,
            2
        );
        writer
            .batch_execute("DROP TABLE holding_plan_daily;")
            .unwrap();
        assert!(
            read_legacy_markers_with_connection(&mut reader, now().date_naive())
                .unwrap()
                .is_empty(),
            "failed read must release its old snapshot before another inspection"
        );
    }
}
