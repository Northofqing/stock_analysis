//! Readonly delivery observations. These counts grant no delivery authority.

use super::*;

pub const DELIVERY_STATUS_STATES: [DecisionState; 14] = [
    DecisionState::Reserved,
    DecisionState::AttemptInFlight,
    DecisionState::AcceptedAuditPending,
    DecisionState::AcceptedTaskTransitionPending,
    DecisionState::Delivered,
    DecisionState::RejectedAuditPending,
    DecisionState::RejectedTaskTransitionPending,
    DecisionState::RejectedDurable,
    DecisionState::UncertainAuditPending,
    DecisionState::UncertainTaskTransitionPending,
    DecisionState::UncertainManualReview,
    DecisionState::ManualRejectedAuditPending,
    DecisionState::ManualRejectedTaskTransitionPending,
    DecisionState::ManualResolvedRejected,
];

#[derive(Clone, Debug, Serialize)]
pub struct DeliveryStateCount {
    pub state: DecisionState,
    pub count: u64,
}

/// A single actual SQLite read snapshot. No identities or caller clock escape.
#[derive(Clone, Debug, Serialize)]
pub struct DeliveryStatusSnapshot {
    pub observed_at: DateTime<Utc>,
    pub state_counts: [DeliveryStateCount; 14],
    pub total_decisions: u64,
    pub locally_pending_decisions: u64,
    pub deliverable_decisions: u64,
    pub non_progressable_foreign_attempts: u64,
    pub non_progressable_manual_reviews: u64,
}

pub(super) enum ReconcileClassification {
    Deliverable,
    LocallyPending,
    ForeignLiveAttempt,
    ManualReview,
    Terminal,
}

/// Keep the health projection and the original reconciliation summary on the
/// same state/lease rules. Expired foreign attempts are locally pending.
pub(super) fn classify_reconcile_state(
    state: DecisionState,
    retry_authorized: bool,
    owner: Option<&str>,
    lease: Option<&str>,
    local_owner: &str,
    now: DateTime<Utc>,
) -> Result<ReconcileClassification> {
    Ok(match state {
        DecisionState::Reserved => ReconcileClassification::Deliverable,
        DecisionState::RejectedDurable if retry_authorized => ReconcileClassification::Deliverable,
        DecisionState::AttemptInFlight => {
            let lease_live = lease
                .map(parse_timestamp)
                .transpose()?
                .is_some_and(|deadline| deadline > now);
            if lease_live && owner != Some(local_owner) {
                ReconcileClassification::ForeignLiveAttempt
            } else {
                ReconcileClassification::LocallyPending
            }
        }
        DecisionState::Delivered
        | DecisionState::RejectedDurable
        | DecisionState::ManualResolvedRejected => ReconcileClassification::Terminal,
        DecisionState::UncertainManualReview => ReconcileClassification::ManualReview,
        _ => ReconcileClassification::LocallyPending,
    })
}

fn increment(value: &mut u64) -> Result<()> {
    *value = value.checked_add(1).ok_or_else(|| {
        DurableDeliveryError::InvalidEnvelope("delivery status count overflow".to_owned())
    })?;
    Ok(())
}

impl DurableDeliveryCoordinator {
    /// Observe existing attested storage without recovery, audit, reservation,
    /// schema initialization, hydration or an external port. Empty is observed
    /// zero; callers must distinguish it from a runtime that does not exist.
    pub fn read_delivery_status(&self) -> Result<DeliveryStatusSnapshot> {
        self.with_connection(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            // Establish the SQLite snapshot before sampling the classification
            // clock, so all rows and the lease evaluation belong to this read.
            let _: i64 =
                transaction.query_row("SELECT COUNT(*) FROM delivery_decisions", [], |row| {
                    row.get(0)
                })?;
            let observed_at = Utc::now();
            let mut snapshot = DeliveryStatusSnapshot {
                observed_at,
                state_counts: DELIVERY_STATUS_STATES
                    .map(|state| DeliveryStateCount { state, count: 0 }),
                total_decisions: 0,
                locally_pending_decisions: 0,
                deliverable_decisions: 0,
                non_progressable_foreign_attempts: 0,
                non_progressable_manual_reviews: 0,
            };
            {
                let mut statement = transaction.prepare(
                    "SELECT d.state,d.retry_authorized,a.owner_instance_identity,a.lease_expires_at
                     FROM delivery_decisions d LEFT JOIN delivery_attempts a
                       ON a.attempt_identity=d.current_attempt_identity",
                )?;
                let rows = statement.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)? == 1,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })?;
                for row in rows {
                    let (raw_state, retry, owner, lease) = row?;
                    let state = DecisionState::parse(&raw_state)?;
                    increment(&mut snapshot.total_decisions)?;
                    increment(
                        &mut snapshot
                            .state_counts
                            .iter_mut()
                            .find(|entry| entry.state == state)
                            .expect("closed DecisionState is represented")
                            .count,
                    )?;
                    match classify_reconcile_state(
                        state,
                        retry,
                        owner.as_deref(),
                        lease.as_deref(),
                        &self.config.owner_instance_identity,
                        observed_at,
                    )? {
                        ReconcileClassification::Deliverable => {
                            increment(&mut snapshot.deliverable_decisions)?
                        }
                        ReconcileClassification::LocallyPending => {
                            increment(&mut snapshot.locally_pending_decisions)?
                        }
                        ReconcileClassification::ForeignLiveAttempt => {
                            increment(&mut snapshot.non_progressable_foreign_attempts)?
                        }
                        ReconcileClassification::ManualReview => {
                            increment(&mut snapshot.non_progressable_manual_reviews)?
                        }
                        ReconcileClassification::Terminal => {}
                    }
                }
            }
            transaction.commit()?;
            Ok(snapshot)
        })
    }
}
