//! Versioned restatement projection. Raw BR-251 terminal evidence remains raw.
use super::*;
use crate::trading::paper_ledger::{
    EffectiveProjectionReceipt, FillLineage, VerifiedEffectiveFillSet,
};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EffectiveCycleAttribution {
    pub cycle_open_fill_id: i64,
    pub code: String,
    pub economic_entry_at: DateTime<FixedOffset>,
    pub economic_exit_at: DateTime<FixedOffset>,
    pub source_fill_ids: Vec<i64>,
    pub entry_composition: Vec<EntryFamilyComposition>,
    pub gross_pnl: f64,
    pub scenario_net_pnl: f64,
    pub scenario_net_return: f64,
    pub benchmark_return: MetricAvailability<f64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct EffectiveAttributionReport {
    rule_version: String,
    from: NaiveDate,
    to: NaiveDate,
    projection: EffectiveProjectionReceipt,
    lineage: Vec<FillLineage>,
    cycles: Vec<EffectiveCycleAttribution>,
    open_cycles: usize,
    opening_inventory: Option<crate::trading::paper_ledger::OpeningInventorySample>,
    fee_kind: CostBasisKind,
    result_hash: String,
}
impl EffectiveAttributionReport {
    pub fn cycles(&self) -> &[EffectiveCycleAttribution] {
        &self.cycles
    }
    pub fn projection(&self) -> &EffectiveProjectionReceipt {
        &self.projection
    }
    pub fn result_hash(&self) -> &str {
        &self.result_hash
    }
    pub fn render_summary(&self) -> String {
        let daily = self
            .cycles
            .iter()
            .filter(|cycle| cycle.economic_exit_at.date_naive() == self.to)
            .collect::<Vec<_>>();
        let daily_net = daily
            .iter()
            .map(|cycle| cycle.scenario_net_pnl)
            .sum::<f64>();
        let window_net = self
            .cycles
            .iter()
            .map(|cycle| cycle.scenario_net_pnl)
            .sum::<f64>();
        let opening = self
            .opening_inventory
            .as_ref()
            .map_or(0, |sample| sample.remaining_opening_lots.len());
        format!("🧾 模拟账本归因（{}）\n日内完整策略周期 {}，Scenario 净盈亏 {daily_net:+.2} 元\n窗口 {}～{}：{} 个周期，Scenario 净盈亏 {window_net:+.2} 元\n未闭合策略周期 {}；期初库存剩余 {} 个批次（不计策略胜率/连续止损）\n本报告仅含已实现结果，不含未实现估值；费用为模型假设，不是实扣凭据。\n有效投影 {}",self.to,daily.len(),self.from,self.to,self.cycles.len(),self.open_cycles,opening,self.projection.projection_hash)
    }
    pub fn render_markdown(&self) -> Result<String, String> {
        Ok(format!(
            "{}\n\n```json\n{}\n```\n",
            self.render_summary(),
            String::from_utf8(self.canonical_bytes()?).map_err(|e| e.to_string())?
        ))
    }
    /// The result binds the financial frontier, not an unrelated derived-event
    /// suffix. The complete observed read head remains available in projection().
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, String> {
        let mut payload = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let projection = payload
            .get_mut("projection")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("missing report projection")?;
        projection.remove("ledger_head");
        projection.insert(
            "head_contract".into(),
            serde_json::json!("FinancialSourceFrontierV1"),
        );
        serde_json::to_vec(&payload).map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct PreparedEffectiveAttributionReport {
    report: EffectiveAttributionReport,
    append: AttributionReportAppend,
}
impl PreparedEffectiveAttributionReport {
    pub fn report(&self) -> &EffectiveAttributionReport {
        &self.report
    }
}

/// Online daily/window owner. A single read capability feeds both views.
pub fn commit_effective_window(
    database: &DatabaseManager,
    binding: crate::trading::paper_ledger::AccountBinding,
    date: NaiveDate,
    days: u32,
    invoked_at: DateTime<FixedOffset>,
) -> Result<(PreparedEffectiveAttributionReport, AttributionReportReceipt), ReplayError> {
    use crate::trading::paper_ledger::{
        EffectiveFillRequest, EffectiveFillScope, EffectiveHistory,
    };
    if days == 0 {
        return Err(effective_integrity(
            "effective window must contain at least one day".into(),
        ));
    }
    let resolved = crate::database::attribution_epochs::AttributionEpochStore::new(database)
        .load_selector(&AttributionEpochSelector::Active)
        .map_err(map_epoch_store_error)?;
    let ResolvedAttributionEpoch::Epoch(epoch) = resolved else {
        return Err(ReplayError::new(
            ReplayErrorClass::Unavailable,
            ReplayStage::Epoch,
            "effective_online_requires_active_attribution_epoch",
            false,
        ));
    };
    let from = date
        .checked_sub_signed(chrono::Duration::days(i64::from(days) - 1))
        .ok_or_else(|| effective_integrity("effective window date overflow".into()))?
        .max(epoch.effective_trading_date);
    EffectiveAttributionRunner::new(database).commit(
        ReplayRequest {
            mode: ReplayMode::Range {
                from,
                to: date,
                invoked_at,
            },
            epoch: AttributionEpochSelector::Active,
            benchmark_day_manifests: Vec::new(),
        },
        EffectiveFillRequest {
            scope: EffectiveFillScope::Epoch(binding),
            history: EffectiveHistory::RestatedLatest,
            as_of: date,
        },
    )
}

/// Uses the existing append-only report owner without a second source loader.
pub struct EffectiveAttributionRunner<'a> {
    database: &'a DatabaseManager,
    benchmark_instrument: String,
    minute_semantics: MinuteLabelSemantics,
}
impl<'a> EffectiveAttributionRunner<'a> {
    pub fn new(database: &'a DatabaseManager) -> Self {
        Self {
            database,
            benchmark_instrument: HS300_CANONICAL.into(),
            minute_semantics: MinuteLabelSemantics::Unverified,
        }
    }
}
impl AttributionReplayRunner<'_> {
    pub fn preview_effective(
        &self,
        request: ReplayRequest,
        paper: crate::trading::paper_ledger::EffectiveFillRequest,
    ) -> Result<PreparedEffectiveAttributionReport, ReplayError> {
        EffectiveAttributionRunner {
            database: self.database,
            benchmark_instrument: self.benchmark_instrument.clone(),
            minute_semantics: self.minute_semantics.clone(),
        }
        .preview(request, paper)
    }
    pub fn commit_effective(
        &self,
        request: ReplayRequest,
        paper: crate::trading::paper_ledger::EffectiveFillRequest,
    ) -> Result<(PreparedEffectiveAttributionReport, AttributionReportReceipt), ReplayError> {
        EffectiveAttributionRunner {
            database: self.database,
            benchmark_instrument: self.benchmark_instrument.clone(),
            minute_semantics: self.minute_semantics.clone(),
        }
        .commit(request, paper)
    }
}
impl EffectiveAttributionRunner<'_> {
    /// Explicit historical scope. Preview does not append or mutate old reports.
    pub fn preview(
        &self,
        request: ReplayRequest,
        paper: crate::trading::paper_ledger::EffectiveFillRequest,
    ) -> Result<PreparedEffectiveAttributionReport, ReplayError> {
        use crate::trading::paper_ledger::{verified_effective_fills_on, EffectiveFillScope};
        let admitted = admit_replay_request(request)?;
        let calendar = resolve_admitted_calendar(&admitted).map_err(map_calendar_error)?;
        if paper.as_of != calendar.target_to() {
            return Err(ReplayError::new(
                ReplayErrorClass::FailedIntegrity,
                ReplayStage::Request,
                "paper_as_of_range_mismatch",
                false,
            ));
        }
        let (set, epoch) = self
            .database
            .attribution_read_transaction(|conn| {
                let set = verified_effective_fills_on(conn, &paper)
                    .map_err(|e| EpochReplaySnapshotFailure::Replay(paper_error(e)))?;
                set.rows()
                    .map_err(|e| EpochReplaySnapshotFailure::Replay(paper_error(e)))?;
                let resolved = load_selector_with_connection(conn, &admitted.epoch)
                    .map_err(EpochReplaySnapshotFailure::Epoch)?;
                let epoch = match resolved {
                    ResolvedAttributionEpoch::Legacy => AttributionReportEpochBinding::Legacy,
                    ResolvedAttributionEpoch::Epoch(receipt) => {
                        let source = set.receipt();
                        let cutover_day = source.cutover_at.map(|at| {
                            at.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap())
                                .date_naive()
                        });
                        if !matches!(source.request.scope, EffectiveFillScope::Epoch(_))
                            || source.cutover_raw_high_water != Some(receipt.paper_trade_high_water)
                            || cutover_day != Some(receipt.effective_trading_date)
                            || calendar.target_from() < receipt.effective_trading_date
                            || canonical_legacy_carry_manifest_hash(&set.seed_carry())
                                != receipt.legacy_carry_manifest_hash
                        {
                            return Err(EpochReplaySnapshotFailure::Replay(ReplayError::new(
                                ReplayErrorClass::Unavailable,
                                ReplayStage::Epoch,
                                "paper_attribution_cutover_not_aligned",
                                false,
                            )));
                        }
                        AttributionReportEpochBinding::Epoch {
                            epoch_id: receipt.epoch_id,
                            epoch_receipt_hash: receipt.receipt_hash,
                            effective_date: receipt.effective_trading_date,
                            legacy_carry_manifest_hash: receipt.legacy_carry_manifest_hash,
                            exclusion_manifest_hash: effective_hash(
                                b"PaperEffectiveExclusionsV1",
                                &serde_json::to_vec(set.lineage()).map_err(|e| {
                                    EpochReplaySnapshotFailure::Replay(effective_integrity(
                                        e.to_string(),
                                    ))
                                })?,
                            ),
                        }
                    }
                };
                Ok((set, epoch))
            })
            .map_err(map_epoch_replay_transaction_error)
            .map_err(|error| match error {
                EpochReplaySnapshotFailure::Epoch(error) => map_epoch_store_error(error),
                EpochReplaySnapshotFailure::Replay(error) => error,
                EpochReplaySnapshotFailure::Load(error) => map_attribution_load_failure(&error),
            })?;
        let mut required = calendar
            .required_trading_dates()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        for row in set.rows().map_err(paper_error)? {
            let date = parse_paper_fill_timestamp(row.id, &row.occurred_at)
                .map_err(effective_integrity)?
                .date();
            if !verified_a_share_trading_day(date).map_err(effective_integrity)? {
                return Err(effective_integrity(
                    "effective fill is not on a verified trading day".into(),
                ));
            }
            required.insert(date);
        }
        // A realized-PnL restatement can be useful without benchmark evidence.
        // It explicitly freezes Unavailable metrics; supplied manifests must
        // still pass the existing exact benchmark reader, never a loose fallback.
        let (bars, benchmark_hash) = if admitted.benchmark_day_manifests.is_empty() {
            (
                Vec::new(),
                effective_hash(b"PaperEffectiveBenchmarkUnavailableV1", b"not_requested"),
            )
        } else {
            let mut summary = FailureEvidenceSummary::new(&admitted);
            let (bars, hash, _) = load_runner_benchmarks_for_dates(
                self.database,
                &self.benchmark_instrument,
                &required,
                &admitted.benchmark_day_manifests,
                &mut summary,
            )?;
            (bars, hash)
        };
        let report = compute_effective_attribution(
            &set,
            calendar.target_from(),
            &bars,
            &self.minute_semantics,
        )
        .map_err(effective_integrity)?;
        let result_payload = serde_json::json!({"schema":"PaperEffectiveAttributionReportV1","attribution_epoch":epoch,"result":serde_json::from_slice::<serde_json::Value>(&report.canonical_bytes().map_err(effective_integrity)?).map_err(|e|effective_integrity(e.to_string()))?});
        let source_identity = serde_json::to_vec(&(
            set.receipt().projection_hash.clone(),
            &set.receipt().request,
            &epoch,
        ))
        .map_err(|e| effective_integrity(e.to_string()))?;
        let append = AttributionReportAppend {
            invocation: AttributionInvocation {
                target_from: calendar.target_from(),
                target_to: calendar.target_to(),
                rule_version: "PaperEffectiveAttributionV1".into(),
                ..admitted.provisional_invocation
            },
            epoch,
            trade_hash: effective_hash(b"PaperEffectiveAttributionSourceV1", &source_identity),
            fee: AttributionEvidenceHash::Available(effective_hash(
                b"PaperEffectiveScenarioFeesV1",
                set.costs().map_err(paper_error)?.basis_id.as_bytes(),
            )),
            stock_close_hash: effective_hash(
                b"PaperEffectiveNoStockMarkV1",
                b"realized_cycles_only",
            ),
            benchmark_manifest_hash: benchmark_hash,
            calendar_authority_hash: calendar.authority_hash().into(),
            regime: AttributionEvidenceHash::Unavailable("market_regime_unavailable".into()),
            result_payload,
        };
        Ok(PreparedEffectiveAttributionReport { report, append })
    }
    pub fn commit(
        &self,
        request: ReplayRequest,
        paper: crate::trading::paper_ledger::EffectiveFillRequest,
    ) -> Result<(PreparedEffectiveAttributionReport, AttributionReportReceipt), ReplayError> {
        let prepared = self.preview(request, paper)?;
        let receipt = AttributionReportStore::new(self.database)
            .commit_report(prepared.append.clone())
            .map_err(map_store_error)?;
        Ok((prepared, receipt))
    }
}
fn effective_hash(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update([0]);
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}
fn effective_integrity(detail: String) -> ReplayError {
    ReplayError::new(
        ReplayErrorClass::FailedIntegrity,
        ReplayStage::Compute,
        "effective_attribution_integrity",
        false,
    )
    .with_typed_failure("effective_attribution", detail.as_bytes(), None, None)
}
fn paper_error(error: crate::trading::paper_ledger::LedgerError) -> ReplayError {
    use crate::trading::paper_ledger::LedgerError;
    let (class, code) = match error {
        LedgerError::IntegrityFailure(_)
        | LedgerError::IdentityConflict
        | LedgerError::InvalidInput(_) => (
            ReplayErrorClass::FailedIntegrity,
            "effective_paper_integrity",
        ),
        _ => (ReplayErrorClass::Unavailable, "effective_paper_unavailable"),
    };
    ReplayError::new(class, ReplayStage::TradeEvidence, code, false).with_typed_failure(
        code,
        error.to_string().as_bytes(),
        None,
        None,
    )
}
pub fn compute_effective_attribution(
    set: &VerifiedEffectiveFillSet,
    from: NaiveDate,
    bars: &[BenchmarkBar],
    semantics: &MinuteLabelSemantics,
) -> Result<EffectiveAttributionReport, String> {
    let to = set.receipt().request.as_of;
    if from > to {
        return Err("effective attribution range reversed".into());
    }
    let economic = crate::performance::economic_position::report_from_effective(set)?;
    let offset = FixedOffset::east_opt(8 * 3600).unwrap();
    let mut cycles = Vec::new();
    for cycle in economic.closed_positions {
        if cycle.closed_at.date() < from || cycle.closed_at.date() > to {
            continue;
        }
        let entry = offset
            .from_local_datetime(&cycle.opened_at)
            .single()
            .ok_or("invalid economic entry time")?;
        let exit = offset
            .from_local_datetime(&cycle.closed_at)
            .single()
            .ok_or("invalid economic exit time")?;
        let NetMetrics::Available {
            kind: CostBasisKind::Scenario,
            net_pnl,
            return_on_buy_notional,
            ..
        } = cycle.net
        else {
            return Err("effective attribution requires same-projection Scenario fees, never invented Observed authority".into());
        };
        let benchmark_return =
            align_cycle_benchmark(entry, exit, bars, semantics).map_err(|e| e.to_string())?;
        cycles.push(EffectiveCycleAttribution {
            cycle_open_fill_id: cycle.cycle_open_fill_id,
            code: cycle.code,
            economic_entry_at: entry,
            economic_exit_at: exit,
            source_fill_ids: cycle.source_fill_ids,
            entry_composition: cycle.entry_composition,
            gross_pnl: cycle.gross_pnl,
            scenario_net_pnl: net_pnl,
            scenario_net_return: return_on_buy_notional,
            benchmark_return,
        });
    }
    cycles.sort_by_key(|cycle| (cycle.economic_exit_at, cycle.cycle_open_fill_id));
    let mut result = EffectiveAttributionReport {
        rule_version: "PaperEffectiveAttributionV1".into(),
        from,
        to,
        projection: set.receipt().clone(),
        lineage: set.lineage().to_vec(),
        cycles,
        open_cycles: economic.open_positions.len(),
        opening_inventory: economic.opening_inventory,
        fee_kind: CostBasisKind::Scenario,
        result_hash: String::new(),
    };
    let bytes = result.canonical_bytes()?;
    let mut hasher = Sha256::new();
    hasher.update(b"PaperEffectiveAttributionV1\0");
    hasher.update(bytes);
    result.result_hash = hex::encode(hasher.finalize());
    Ok(result)
}
