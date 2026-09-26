//! Immutable derived results; never a second financial ledger.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRevision {
    pub target_date: NaiveDate,
    pub algorithm: String,
    pub projection: EffectiveProjectionReceipt,
    pub metrics: crate::performance::snapshot::PerformanceSnapshot,
    pub opening_exclusions: Vec<OpeningInventoryExit>,
    pub account_realized_pnl: Money,
    pub result_hash: String,
}
impl PaperLedger<'_> {
    /// Explicitly bound writer. Ordinary unactivated/legacy reads cannot mint
    /// an authoritative revision. There is no write to the old date table.
    pub fn settle_snapshot(
        &self,
        request: &EffectiveFillRequest,
    ) -> Result<SnapshotRevision, LedgerError> {
        let binding = binding(request)?;
        let mut conn = self
            .db
            .get_conn()
            .map_err(|e| LedgerError::Database(e.to_string()))?;
        conn.immediate_transaction(|conn| {
            let effective = effective::verified_on(conn, request)?;
            effective.rows()?;
            let command = command(&effective);
            if let Some(original) = find(conn, binding, &command)? {
                return Ok(original);
            }
            let view = load(conn, binding)?;
            let sample = effective.opening_inventory_sample()?;
            let pnls = sample
                .exit_pnls
                .iter()
                .filter(|exit| exit.date == request.as_of)
                .filter_map(|exit| exit.strategy_net_pnl.map(Money::cny))
                .collect::<Vec<_>>();
            let account_realized_pnl = sample
                .exit_pnls
                .iter()
                .filter(|exit| exit.date == request.as_of)
                .try_fold(Money::ZERO, |total, exit| total.add(exit.account_net_pnl))?;
            let metrics = crate::performance::snapshot::metrics_from_pnls(
                request.as_of,
                &pnls,
                i32::try_from(view.version.checked_add(1).ok_or(LedgerError::Overflow)?)
                    .map_err(|_| LedgerError::Overflow)?,
                (self.clock)().format("%Y-%m-%d %H:%M:%S").to_string(),
            )
            .map_err(LedgerError::EvidenceUnavailable)?;
            let mut revision = SnapshotRevision {
                target_date: request.as_of,
                algorithm: ALGORITHM.into(),
                projection: effective.receipt().clone(),
                metrics,
                opening_exclusions: sample.excluded_exits,
                account_realized_pnl,
                result_hash: String::new(),
            };
            revision.result_hash = result_hash(&revision)?;
            append(
                conn,
                binding,
                &command,
                &view,
                Fact::DerivedSnapshotV1(revision.clone()),
            )?;
            Ok(revision)
        })
    }
    pub fn current_snapshot(
        &self,
        request: &EffectiveFillRequest,
    ) -> Result<Option<SnapshotRevision>, LedgerError> {
        let binding = binding(request)?;
        let mut conn = self
            .db
            .get_conn()
            .map_err(|e| LedgerError::Database(e.to_string()))?;
        conn.transaction(|conn| {
            let effective = effective::verified_on(conn, request)?;
            effective.rows()?;
            find(conn, binding, &command(&effective))
        })
    }
}

const ALGORITHM: &str = "PaperSnapshotNetFifoV1";
fn binding(request: &EffectiveFillRequest) -> Result<&AccountBinding, LedgerError> {
    match &request.scope {
        EffectiveFillScope::Epoch(binding) | EffectiveFillScope::LegacyBeforeCutover(binding) => {
            Ok(binding)
        }
        EffectiveFillScope::LegacyRaw => Err(LedgerError::NotSeeded),
    }
}
fn command(effective: &VerifiedEffectiveFillSet) -> String {
    format!(
        "derived-snapshot:{ALGORITHM}:{}:{}",
        effective.receipt().request.as_of,
        effective.receipt().projection_hash
    )
}
fn find(
    conn: &mut SqliteConnection,
    binding: &AccountBinding,
    command: &str,
) -> Result<Option<SnapshotRevision>, LedgerError> {
    for row in events(conn, &binding.account_id)? {
        if row.command_id == command {
            let Fact::DerivedSnapshotV1(revision) = decode(&row.payload)? else {
                return Err(LedgerError::IdentityConflict);
            };
            validate(&revision)?;
            return Ok(Some(revision));
        }
    }
    Ok(None)
}
fn result_hash(revision: &SnapshotRevision) -> Result<String, LedgerError> {
    Ok(digest(&encode(&(
        ALGORITHM,
        revision.target_date,
        &revision.projection,
        &revision.metrics,
        &revision.opening_exclusions,
        revision.account_realized_pnl,
    ))?))
}
pub(super) fn validate(revision: &SnapshotRevision) -> Result<(), LedgerError> {
    if revision.algorithm != ALGORITHM
        || revision.target_date != revision.projection.request.as_of
        || revision.metrics.date != revision.target_date.to_string()
        || result_hash(revision)? != revision.result_hash
    {
        return Err(LedgerError::IntegrityFailure(
            "unknown/invalid derived snapshot payload".into(),
        ));
    }
    Ok(())
}
