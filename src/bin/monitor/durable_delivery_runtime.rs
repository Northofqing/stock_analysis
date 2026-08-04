//! BR-192 production adapter for counted delivery; BR-199 closes the R-08
//! public SourceOnly binding and envelope revalidation path.
//!
//! The library coordinator owns reservations, budget, cooldown, attempts,
//! fencing and immutable payloads.  This monitor adapter owns only runtime
//! composition: one typed authoritative sink, one exact-byte append port and
//! the producer-readiness barrier.

use chrono::{DateTime, NaiveDate, Utc};
use magic_market_core::{AssetClass, Exchange, InstrumentId};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use stock_analysis::durable_delivery::{
    AuthoritativeDeliveryRequest, AuthoritativeSink, AuthoritativeSinkPort,
    AuthoritativeSinkResult, CoordinatorConfig, DecisionState, DeliveryEnvelope, DeliverySubKind,
    DurableDeliveryCoordinator, ImmutableAppendPort, PushKind as DurablePushKind,
    ReviewTerminalReplayCompletionState, ReviewTerminalReplayInput, ScheduleHydration,
    ScheduleHydrationState, TaskBinding,
};
use stock_analysis::event::DurableDeliveryImmutableAppend;

use crate::notify::{DailyReportSubKind, PushKind, PushOutcome};

pub const BR194_REPLAY_AUTHORITY_MANIFEST_V1: &str = concat!(
    "database=data/durable_delivery.sqlite3\n",
    "durable_audit_dir=data/durable_delivery_audit/\n",
    "push_log_dir=data/push_log/\n",
    "delivery_audit_dir=data/event_audit/\n",
    "attempt_table=review_terminal_replay_attempts\n",
    "completion_table=review_terminal_replay_completions\n",
    "start_audit_kind=ReviewTerminalReplayStarted\n",
    "completion_audit_kind=ReviewTerminalReplayCompleted\n",
    "attempt_identity_domain=BR-194-terminal-replay-attempt-v1\n",
    "audit_identity_domain=delivery-critical-audit-v1\n",
    "audit_attempt_binding=NONE\n",
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalReplayClassification {
    ExistingTerminalHydrated,
    Failed(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalReplayEvidence {
    pub attempt_identity: String,
    pub decision_identity: String,
    pub replay_ordinal: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RuntimeNamespace {
    Production,
    Test { test_code: String },
}

impl RuntimeNamespace {
    pub(super) fn label(&self) -> String {
        match self {
            Self::Production => "production".to_owned(),
            Self::Test { test_code } => format!("test:{test_code}"),
        }
    }
}

struct RuntimeState {
    namespace: RuntimeNamespace,
    coordinator: Arc<DurableDeliveryCoordinator>,
    append: Arc<DurableDeliveryImmutableAppend>,
    sink: AuthoritativeSink,
    producer_ready: AtomicBool,
    schedule_hydrations: Mutex<Vec<ScheduleHydration>>,
    queued_schedule_hydration_ids: Mutex<std::collections::BTreeSet<String>>,
}

struct RuntimeCacheEntry<T> {
    namespace: RuntimeNamespace,
    state: Result<Arc<T>, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupReconcileEvidence {
    pub progress_count: usize,
    pub resumed_sink_calls: usize,
    pub non_progressable_foreign_attempts: Vec<String>,
    pub non_progressable_manual_reviews: Vec<String>,
    pub schedule_hydrations: Vec<ScheduleHydration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableDispatchEvidence {
    pub decision_identity: String,
    pub state: DecisionState,
    pub schedule_hydration: Option<ScheduleHydration>,
}

/// BR-192 typed cooldown scope supplied by a counted producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CountedDeliveryScope {
    Global,
    Ticket { instrument: InstrumentId },
}

/// BR-192 source ownership. Provider metadata cannot be attached to an
/// internally derived decision because the metadata lives only on this enum
/// variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CountedDeliveryOrigin {
    // Designed seam for later internally persisted evidence sources. It is
    // fully tested, but no production producer has migrated into it yet.
    #[cfg_attr(not(test), allow(dead_code))]
    InternalDurable,
    Provider {
        observed_at: Option<DateTime<Utc>>,
        as_of: Option<NaiveDate>,
        ordered_batch_ids: Vec<String>,
    },
}

/// Caller-supplied immutable evidence required for every counted delivery.
///
/// The fields are private so the evidence fingerprint can never drift away
/// from the exact canonical source bytes after construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountedDeliveryBinding {
    business_date: NaiveDate,
    schedule_occurrence_identity: String,
    source_binding_canonical: Vec<u8>,
    source_evidence_fingerprint: String,
    scope: CountedDeliveryScope,
    delivery_subject_hash: String,
    origin: CountedDeliveryOrigin,
    task_binding: Option<TaskBinding>,
    retry_authorized: bool,
}

impl CountedDeliveryBinding {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        business_date: NaiveDate,
        schedule_occurrence_identity: impl Into<String>,
        source_binding_canonical: Vec<u8>,
        scope: CountedDeliveryScope,
        delivery_subject_hash: impl Into<String>,
        origin: CountedDeliveryOrigin,
        task_binding: Option<TaskBinding>,
        retry_authorized: bool,
    ) -> Result<Self, String> {
        let schedule_occurrence_identity = schedule_occurrence_identity.into();
        let delivery_subject_hash = delivery_subject_hash.into();
        if schedule_occurrence_identity.trim().is_empty() {
            return Err("schedule_occurrence_identity must be non-empty".to_owned());
        }
        if source_binding_canonical.is_empty() {
            return Err("source_binding_canonical must be non-empty".to_owned());
        }
        if !is_sha256_hex(&delivery_subject_hash) {
            return Err("delivery_subject_hash must be lowercase SHA-256 hex".to_owned());
        }
        match &scope {
            CountedDeliveryScope::Global => {}
            CountedDeliveryScope::Ticket { .. } => {}
        }
        if let CountedDeliveryOrigin::Provider {
            ordered_batch_ids, ..
        } = &origin
        {
            if ordered_batch_ids
                .iter()
                .any(|batch_id| batch_id.trim().is_empty())
            {
                return Err("provider batch identities must not be empty".to_owned());
            }
        }
        let source_evidence_fingerprint = sha256_hex(&source_binding_canonical);
        Ok(Self {
            business_date,
            schedule_occurrence_identity,
            source_binding_canonical,
            source_evidence_fingerprint,
            scope,
            delivery_subject_hash,
            origin,
            task_binding,
            retry_authorized,
        })
    }

    pub fn business_date(&self) -> NaiveDate {
        self.business_date
    }

    pub fn schedule_occurrence_identity(&self) -> &str {
        &self.schedule_occurrence_identity
    }

    pub fn source_binding_canonical(&self) -> &[u8] {
        &self.source_binding_canonical
    }

    pub fn source_evidence_fingerprint(&self) -> &str {
        &self.source_evidence_fingerprint
    }

    pub fn scope(&self) -> &CountedDeliveryScope {
        &self.scope
    }

    pub fn governance_code(&self) -> Option<&str> {
        match &self.scope {
            CountedDeliveryScope::Global => None,
            CountedDeliveryScope::Ticket { instrument } => Some(instrument.code()),
        }
    }

    pub fn delivery_subject_hash(&self) -> &str {
        &self.delivery_subject_hash
    }

    pub fn origin(&self) -> &CountedDeliveryOrigin {
        &self.origin
    }

    pub fn task_binding(&self) -> Option<&TaskBinding> {
        self.task_binding.as_ref()
    }

    pub fn retry_authorized(&self) -> bool {
        self.retry_authorized
    }

    pub fn validate_r04_source_only(&self) -> Result<(), &'static str> {
        let expected_task_identity = crate::review_batch::review_task_identity(
            self.business_date,
            crate::review_batch::ReviewTask::R04,
        );
        if self.schedule_occurrence_identity != expected_task_identity
            || !is_sha256_hex(&self.delivery_subject_hash)
            || sha256_hex(&self.source_binding_canonical) != self.source_evidence_fingerprint
        {
            return Err("counted_source_only_binding_invalid");
        }
        let typed_transition_basis_canonical =
            super::push_templates::validate_review_lhb_source_binding_canonical_bytes(
                &self.source_binding_canonical,
            )?;
        let task_binding = self
            .task_binding
            .as_ref()
            .ok_or("counted_source_only_binding_invalid")?;
        if task_binding.task_identity != expected_task_identity
            || sha256_hex(&task_binding.transition_basis_canonical)
                != task_binding.transition_basis_sha256
        {
            return Err("counted_source_only_binding_invalid");
        }
        let canonical: serde_json::Value = serde_json::from_slice(&self.source_binding_canonical)
            .map_err(|_| "counted_source_only_binding_invalid")?;
        let object = canonical
            .as_object()
            .ok_or("counted_source_only_binding_invalid")?;
        let business_date = self.business_date.format("%Y-%m-%d").to_string();
        if !r04_object_has_exact_keys(
            object,
            &[
                "schema_version",
                "business_date",
                "template_id",
                "review_task_identity",
                "delivery_subject_identity",
                "evidence",
                "ordered_projection",
                "rendered_content_sha256",
                "task_transition_basis",
            ],
        ) || object
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
            || object
                .get("business_date")
                .and_then(serde_json::Value::as_str)
                != Some(business_date.as_str())
            || object
                .get("template_id")
                .and_then(serde_json::Value::as_str)
                != Some("review_lhb_v1")
            || object
                .get("review_task_identity")
                .and_then(serde_json::Value::as_str)
                != Some(expected_task_identity.as_str())
            || object
                .get("delivery_subject_identity")
                .and_then(serde_json::Value::as_str)
                != Some(self.delivery_subject_hash.as_str())
            || !object
                .get("rendered_content_sha256")
                .and_then(serde_json::Value::as_str)
                .is_some_and(is_sha256_hex)
        {
            return Err("counted_source_only_binding_invalid");
        }
        let transition_basis = object
            .get("task_transition_basis")
            .ok_or("counted_source_only_binding_invalid")?;
        let transition_object = transition_basis
            .as_object()
            .ok_or("counted_source_only_binding_invalid")?;
        if !r04_object_has_exact_keys(
            transition_object,
            &[
                "task_identity",
                "business_date",
                "task",
                "source",
                "source_time",
                "rule_ids",
                "snapshot_size",
                "batch_ids",
            ],
        ) || typed_transition_basis_canonical != task_binding.transition_basis_canonical
            || transition_basis
                .get("task_identity")
                .and_then(serde_json::Value::as_str)
                != Some(expected_task_identity.as_str())
            || transition_basis
                .get("business_date")
                .and_then(serde_json::Value::as_str)
                != Some(business_date.as_str())
            || transition_basis
                .get("task")
                .and_then(serde_json::Value::as_str)
                != Some("R-04")
        {
            return Err("counted_source_only_binding_invalid");
        }
        let evidence = object
            .get("evidence")
            .and_then(serde_json::Value::as_object)
            .ok_or("counted_source_only_binding_invalid")?;
        if !r04_object_has_exact_keys(
            evidence,
            &["provider", "source", "source_at", "observed_at", "batch_id"],
        ) || evidence.get("provider").and_then(serde_json::Value::as_str) != Some("Eastmoney")
            || evidence
                .get("source")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|source| source.trim().is_empty())
        {
            return Err("counted_source_only_binding_invalid");
        }
        let evidence_batch_id = evidence
            .get("batch_id")
            .and_then(serde_json::Value::as_str)
            .ok_or("counted_source_only_binding_invalid")?;
        if evidence_batch_id.trim().is_empty() {
            return Err("counted_source_only_binding_invalid");
        }
        let evidence_source_at = evidence
            .get("source_at")
            .and_then(serde_json::Value::as_str)
            .ok_or("counted_source_only_binding_invalid")?;
        let evidence_observed_at = evidence
            .get("observed_at")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| crate::push_templates::parse_r04_observed_at(value).ok())
            .ok_or("counted_source_only_binding_invalid")?;
        let projection = object
            .get("ordered_projection")
            .and_then(serde_json::Value::as_array)
            .filter(|rows| !rows.is_empty())
            .ok_or("counted_source_only_binding_invalid")?;
        if !r04_projection_is_exact(projection) {
            return Err("counted_source_only_binding_invalid");
        }
        let evidence_source = evidence
            .get("source")
            .and_then(serde_json::Value::as_str)
            .ok_or("counted_source_only_binding_invalid")?;
        let expected_rule_ids = ["BR-110", "BR-140", "BR-162", "BR-192", "BR-200"];
        let transition_rule_ids = transition_basis
            .get("rule_ids")
            .and_then(serde_json::Value::as_array)
            .ok_or("counted_source_only_binding_invalid")?;
        let transition_batch_ids = transition_basis
            .get("batch_ids")
            .and_then(serde_json::Value::as_array)
            .ok_or("counted_source_only_binding_invalid")?;
        if transition_basis
            .get("source")
            .and_then(serde_json::Value::as_str)
            != Some(evidence_source)
            || transition_basis
                .get("source_time")
                .and_then(serde_json::Value::as_str)
                != Some(business_date.as_str())
            || transition_basis
                .get("snapshot_size")
                .and_then(serde_json::Value::as_u64)
                != u64::try_from(projection.len()).ok()
            || transition_rule_ids.len() != expected_rule_ids.len()
            || !transition_rule_ids
                .iter()
                .zip(expected_rule_ids)
                .all(|(actual, expected)| actual.as_str() == Some(expected))
            || transition_batch_ids.len() != 1
            || transition_batch_ids[0].as_str() != Some(evidence_batch_id)
        {
            return Err("counted_source_only_binding_invalid");
        }
        match &self.origin {
            CountedDeliveryOrigin::Provider {
                observed_at: Some(observed_at),
                as_of: Some(as_of),
                ordered_batch_ids,
            } if *as_of == self.business_date
                && ordered_batch_ids.len() == 1
                && ordered_batch_ids[0] == evidence_batch_id
                && evidence_source_at == business_date
                && evidence_observed_at.with_timezone(&Utc) == *observed_at => {}
            _ => return Err("counted_source_only_binding_invalid"),
        }
        Ok(())
    }

    pub fn validate_r04_source_only_text(&self, text: &str) -> Result<(), &'static str> {
        self.validate_r04_source_only()?;
        let canonical: serde_json::Value = serde_json::from_slice(&self.source_binding_canonical)
            .map_err(|_| "counted_source_only_binding_invalid")?;
        let expected_hash = canonical
            .get("rendered_content_sha256")
            .and_then(serde_json::Value::as_str)
            .ok_or("counted_source_only_binding_invalid")?;
        if sha256_hex(text.as_bytes()) != expected_hash {
            return Err("counted_source_only_binding_invalid");
        }
        Ok(())
    }

    pub fn validate_r08_public_source_only(&self) -> Result<(), &'static str> {
        const INVALID: &str = "counted_r08_source_only_binding_invalid";
        let validated = super::push_templates::validate_r08_public_source_binding_canonical_bytes(
            &self.source_binding_canonical,
        )?;
        if self.business_date != validated.business_date
            || self.schedule_occurrence_identity != validated.task_identity
            || self.delivery_subject_hash != validated.delivery_subject_identity
            || !matches!(self.scope, CountedDeliveryScope::Global)
            || sha256_hex(&self.source_binding_canonical) != self.source_evidence_fingerprint
        {
            return Err(INVALID);
        }
        let task_binding = self.task_binding.as_ref().ok_or(INVALID)?;
        if task_binding.task_identity != validated.task_identity
            || task_binding.transition_basis_canonical != validated.transition_basis_canonical
            || sha256_hex(&task_binding.transition_basis_canonical)
                != task_binding.transition_basis_sha256
        {
            return Err(INVALID);
        }
        match &self.origin {
            CountedDeliveryOrigin::Provider {
                observed_at: Some(observed_at),
                as_of: Some(as_of),
                ordered_batch_ids,
            } if *as_of == validated.business_date
                && *observed_at == validated.max_observed_at
                && *ordered_batch_ids == validated.ordered_batch_ids => {}
            _ => return Err(INVALID),
        }
        Ok(())
    }

    pub fn validate_r08_public_source_only_text(&self, text: &str) -> Result<(), &'static str> {
        let validated = super::push_templates::validate_r08_public_source_binding_canonical_bytes(
            &self.source_binding_canonical,
        )?;
        self.validate_r08_public_source_only()?;
        if sha256_hex(text.as_bytes()) != validated.rendered_content_sha256 {
            return Err("counted_r08_source_only_binding_invalid");
        }
        Ok(())
    }
}

fn r04_object_has_exact_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    expected: &[&str],
) -> bool {
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

fn r04_optional_finite_number(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> bool {
    object
        .get(key)
        .is_some_and(|value| value.is_null() || r04_finite_number(value).is_some())
}

fn r04_finite_number(value: &serde_json::Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

fn r04_projection_is_exact(projection: &[serde_json::Value]) -> bool {
    for (expected_ordinal, row) in projection.iter().enumerate() {
        let Some(row) = row.as_object() else {
            return false;
        };
        if !r04_object_has_exact_keys(
            row,
            &[
                "source_order_ordinal",
                "exchange",
                "code",
                "ranking_net_amount_yuan",
                "disclosures",
            ],
        ) || row
            .get("source_order_ordinal")
            .and_then(serde_json::Value::as_u64)
            != u64::try_from(expected_ordinal).ok()
            || !matches!(
                row.get("exchange").and_then(serde_json::Value::as_str),
                Some("SH" | "SZ" | "BJ")
            )
            || row
                .get("code")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|code| code.trim().is_empty())
            || !row
                .get("ranking_net_amount_yuan")
                .and_then(r04_finite_number)
                .is_some_and(|amount| amount > 0.0)
        {
            return false;
        }
        let Some(disclosures) = row
            .get("disclosures")
            .and_then(serde_json::Value::as_array)
            .filter(|disclosures| !disclosures.is_empty())
        else {
            return false;
        };
        for disclosure in disclosures {
            let Some(disclosure) = disclosure.as_object() else {
                return false;
            };
            if !r04_object_has_exact_keys(
                disclosure,
                &[
                    "entry_id",
                    "trade_id",
                    "reason",
                    "buy_amount_yuan",
                    "sell_amount_yuan",
                    "net_amount_yuan",
                    "turnover_rate_pct",
                    "seats",
                ],
            ) || disclosure
                .get("entry_id")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|entry_id| entry_id.trim().is_empty())
                || disclosure
                    .get("trade_id")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|trade_id| trade_id.trim().is_empty())
                || !disclosure.get("reason").is_some_and(|reason| {
                    reason.is_null()
                        || reason
                            .as_str()
                            .is_some_and(|reason| !reason.trim().is_empty())
                })
                || !r04_optional_finite_number(disclosure, "buy_amount_yuan")
                || !r04_optional_finite_number(disclosure, "sell_amount_yuan")
                || !r04_optional_finite_number(disclosure, "net_amount_yuan")
                || !r04_optional_finite_number(disclosure, "turnover_rate_pct")
            {
                return false;
            }
            let Some(seats) = disclosure
                .get("seats")
                .and_then(serde_json::Value::as_array)
                .filter(|seats| seats.len() == 10)
            else {
                return false;
            };
            let mut buy_ranks = [false; 5];
            let mut sell_ranks = [false; 5];
            for seat in seats {
                let Some(seat) = seat.as_object() else {
                    return false;
                };
                if !r04_object_has_exact_keys(
                    seat,
                    &[
                        "side",
                        "rank",
                        "seat_name",
                        "amount_yuan",
                        "buy_amount_yuan",
                        "sell_amount_yuan",
                        "net_amount_yuan",
                    ],
                ) || seat
                    .get("seat_name")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|seat_name| seat_name.trim().is_empty())
                    || !seat
                        .get("amount_yuan")
                        .and_then(r04_finite_number)
                        .is_some_and(|amount| amount > 0.0)
                    || !r04_optional_finite_number(seat, "buy_amount_yuan")
                    || !r04_optional_finite_number(seat, "sell_amount_yuan")
                    || !r04_optional_finite_number(seat, "net_amount_yuan")
                {
                    return false;
                }
                let Some(rank) = seat.get("rank").and_then(serde_json::Value::as_u64) else {
                    return false;
                };
                let Ok(rank_index) = usize::try_from(rank.saturating_sub(1)) else {
                    return false;
                };
                let Some(ranks) = (match seat.get("side").and_then(serde_json::Value::as_str) {
                    Some("buy") => Some(&mut buy_ranks),
                    Some("sell") => Some(&mut sell_ranks),
                    _ => None,
                }) else {
                    return false;
                };
                let Some(seen) = ranks.get_mut(rank_index) else {
                    return false;
                };
                if *seen {
                    return false;
                }
                *seen = true;
            }
            if !buy_ranks.into_iter().all(std::convert::identity)
                || !sell_ranks.into_iter().all(std::convert::identity)
            {
                return false;
            }
        }
    }
    true
}

pub(super) struct MagiclawAuthoritativeSink {
    namespace: RuntimeNamespace,
    push_log_writer: Arc<crate::notify::PinnedPushLogWriter>,
    delivery_audit: Arc<stock_analysis::event::AuditDispatcher>,
}

impl MagiclawAuthoritativeSink {
    pub(super) fn bind(namespace: RuntimeNamespace) -> Result<Self, String> {
        let push_log_writer =
            crate::notify::eager_bind_push_log_capability(&namespace).map_err(|error| {
                format!(
                    "eagerly bind BR-192 push-log namespace={}: {error}",
                    namespace.label()
                )
            })?;
        let delivery_audit = bind_counted_delivery_audit(&namespace)?;
        Ok(Self {
            namespace,
            push_log_writer,
            delivery_audit,
        })
    }

    #[cfg(test)]
    pub(super) fn from_test_artifacts(
        namespace: RuntimeNamespace,
        push_log_writer: crate::notify::PinnedPushLogWriter,
        delivery_audit: stock_analysis::event::AuditDispatcher,
    ) -> Self {
        Self {
            namespace,
            push_log_writer: Arc::new(push_log_writer),
            delivery_audit: Arc::new(delivery_audit),
        }
    }

    fn reject_namespace(error: String) -> AuthoritativeSinkResult {
        let reason_code = if error == "production configuration rejected: V10_DRY_RUN_PUSH=1" {
            "production_dry_run_configuration_rejected"
        } else if error.starts_with("BR-192 authoritative sink namespace mismatch:") {
            "delivery_runtime_namespace_mismatch"
        } else {
            "delivery_runtime_mode_rejected"
        };
        AuthoritativeSinkResult::Rejected(stock_analysis::durable_delivery::TypedRejection {
            reason_code: reason_code.to_owned(),
            evidence: error.into_bytes(),
            retry_authorized: false,
            observed_at: Utc::now(),
        })
    }
}

impl AuthoritativeSinkPort for MagiclawAuthoritativeSink {
    fn sink_identity(&self) -> &str {
        "magiclaw-cli-typed-v1"
    }

    fn deliver(&self, request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        let current_namespace = match resolve_runtime_namespace() {
            Ok(namespace) => namespace,
            Err(error) => return Self::reject_namespace(error),
        };
        if current_namespace != self.namespace {
            return Self::reject_namespace(format!(
                "BR-192 authoritative sink namespace mismatch: bound={} current={}",
                self.namespace.label(),
                current_namespace.label()
            ));
        }
        crate::notify::deliver_authoritative_blocking(
            &self.namespace,
            &self.push_log_writer,
            &self.delivery_audit,
            request,
        )
    }
}

fn bind_counted_delivery_audit(
    namespace: &RuntimeNamespace,
) -> Result<Arc<stock_analysis::event::AuditDispatcher>, String> {
    let dispatcher = match namespace {
        RuntimeNamespace::Production => stock_analysis::event::bind_production_delivery_audit(),
        RuntimeNamespace::Test { test_code } => {
            stock_analysis::event::bind_test_delivery_audit(test_code)
        }
    }?;
    dispatcher.preflight().map_err(|error| {
        format!(
            "eagerly bind counted delivery audit namespace={}: {error}",
            namespace.label()
        )
    })?;
    Ok(dispatcher)
}

#[cfg(not(test))]
static RUNTIME: OnceLock<RuntimeCacheEntry<RuntimeState>> = OnceLock::new();

#[cfg(test)]
type TestRuntimeMap = Mutex<std::collections::BTreeMap<String, Result<Arc<RuntimeState>, String>>>;

#[cfg(test)]
static TEST_RUNTIMES: OnceLock<TestRuntimeMap> = OnceLock::new();

pub fn is_counted_kind(kind: PushKind) -> bool {
    durable_kind_and_sub_kind(kind).is_some()
}

/// Validate the physical runtime boundary before opening a coordinator,
/// reserving a counted decision or calling the authoritative sink.
///
/// `V10_DRY_RUN_PUSH` is a test-only transport switch. Accepting it in
/// production would let a synthetic TEST_CODE receipt become authoritative
/// production evidence, so production fails closed instead of silently
/// removing the switch and unexpectedly sending a real message.
pub fn validate_runtime_delivery_mode() -> Result<Option<String>, String> {
    match current_runtime_namespace()? {
        RuntimeNamespace::Production => Ok(None),
        RuntimeNamespace::Test { test_code } => Ok(Some(test_code)),
    }
}

pub(super) fn current_runtime_namespace() -> Result<RuntimeNamespace, String> {
    resolve_runtime_namespace()
}

fn resolve_runtime_namespace() -> Result<RuntimeNamespace, String> {
    let environment = stock_analysis::risk::env_guard::current_env();
    let dry_run_requested = std::env::var("V10_DRY_RUN_PUSH").ok().as_deref() == Some("1");
    match environment {
        stock_analysis::risk::env_guard::TradingEnv::Prod if dry_run_requested => {
            Err("production configuration rejected: V10_DRY_RUN_PUSH=1".to_owned())
        }
        stock_analysis::risk::env_guard::TradingEnv::Prod => Ok(RuntimeNamespace::Production),
        stock_analysis::risk::env_guard::TradingEnv::Test if !dry_run_requested => {
            Err("test durable delivery requires V10_DRY_RUN_PUSH=1".to_owned())
        }
        stock_analysis::risk::env_guard::TradingEnv::Test => {
            let test_code = resolve_test_code(None)?;
            Ok(RuntimeNamespace::Test { test_code })
        }
    }
}

pub async fn ensure_startup_reconciled() -> Result<StartupReconcileEvidence, String> {
    let state = runtime_state()?;
    if state.producer_ready.load(Ordering::Acquire) {
        return Ok(StartupReconcileEvidence {
            progress_count: 0,
            resumed_sink_calls: 0,
            non_progressable_foreign_attempts: Vec::new(),
            non_progressable_manual_reviews: Vec::new(),
            schedule_hydrations: pending_hydrations(state.as_ref())?,
        });
    }

    let reconcile_state = Arc::clone(&state);
    let evidence =
        tokio::task::spawn_blocking(move || reconcile_startup_blocking(reconcile_state.as_ref()))
            .await
            .map_err(|error| format!("BR-192 startup reconciliation join failed: {error}"))??;
    state.producer_ready.store(true, Ordering::Release);
    Ok(evidence)
}

/// Bind all namespace-sensitive counted and generic file capabilities before
/// any ordinary sink/router can deliver. This performs no provider or network
/// operation.
pub fn eager_bind_runtime_artifacts() -> Result<(), String> {
    runtime_state().map(|_| ())
}

pub fn pending_schedule_hydrations() -> Result<Vec<ScheduleHydration>, String> {
    let state = runtime_state()?;
    pending_hydrations(state.as_ref())
}

pub fn acknowledge_local_schedule_hydrations(
    transition_identities: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    if transition_identities.is_empty() {
        return Ok(());
    }
    let state = runtime_state()?;
    acknowledge_schedule_hydrations_blocking(state.as_ref(), transition_identities, Utc::now())
}

pub async fn deliver_counted_binding(
    binding: CountedDeliveryBinding,
    kind: PushKind,
    text: String,
    sub_kind: Option<DailyReportSubKind>,
) -> PushOutcome {
    if let Err(error) = ensure_startup_reconciled().await {
        return PushOutcome::Denied(format!("durable delivery admission frozen: {error}"));
    }
    let envelope = match envelope_from_binding(binding, kind, &text, sub_kind) {
        Ok(envelope) => envelope,
        Err(error) => {
            return PushOutcome::Denied(format!("durable delivery envelope rejected: {error}"));
        }
    };
    match deliver_envelope(envelope).await {
        Ok(evidence) => outcome_from_state(evidence.state),
        Err(error) => PushOutcome::SinkError(error),
    }
}

/// BR-196 presentation-gated durable envelope entry. The descriptor token is
/// consumed here and its notification kind must map to the envelope's exact
/// durable kind/sub-kind before any persistence or sink side effect.
pub async fn deliver_presented_envelope(
    token: crate::presentation_registry::ProductionPresentationToken,
    envelope: DeliveryEnvelope,
) -> Result<DurableDispatchEvidence, String> {
    let notification_kind = token.descriptor().push_kind;
    let Some((expected_kind, expected_sub_kind)) = durable_kind_and_sub_kind(notification_kind)
    else {
        return Err("BR-196 presentation kind has no durable delivery mapping".to_string());
    };
    if envelope.push_kind != expected_kind || envelope.sub_kind != expected_sub_kind {
        return Err("BR-196 presentation token/envelope kind mismatch".to_string());
    }
    deliver_envelope(envelope).await
}

async fn deliver_envelope(envelope: DeliveryEnvelope) -> Result<DurableDispatchEvidence, String> {
    ensure_startup_reconciled().await?;
    let state = runtime_state()?;
    tokio::task::spawn_blocking(move || deliver_envelope_blocking(state.as_ref(), envelope))
        .await
        .map_err(|error| format!("BR-192 counted delivery join failed: {error}"))?
}

/// Read the durable owner of an exact counted review occurrence.
///
/// BR-200 deliberately does not run startup reconciliation here: that phase
/// may resume a sink. Review producers may call this only after the normal
/// startup barrier has completed, preserving a strict read-only pre-provider
/// and pre-sink seam.
pub async fn inspect_review_task_occurrence(
    business_date: NaiveDate,
    push_kind: stock_analysis::durable_delivery::PushKind,
    task_identity: String,
) -> Result<Option<DurableDispatchEvidence>, String> {
    if !matches!(
        push_kind,
        stock_analysis::durable_delivery::PushKind::ReviewLhb
            | stock_analysis::durable_delivery::PushKind::EventCalendar
            | stock_analysis::durable_delivery::PushKind::ReviewProviderTopN
    ) {
        return Err(format!(
            "BR-200 unsupported review terminal preflight kind {push_kind}"
        ));
    }
    let state = runtime_state()?;
    if !state.producer_ready.load(Ordering::Acquire) {
        return Err(
            "BR-200 review terminal preflight requires completed startup reconciliation".to_owned(),
        );
    }
    let query_state = Arc::clone(&state);
    let date = business_date.format("%Y-%m-%d").to_string();
    let evidence = tokio::task::spawn_blocking(move || {
        query_state
            .coordinator
            .inspect_review_task_occurrence(
                &date,
                push_kind,
                DeliverySubKind::None,
                "GLOBAL",
                &task_identity,
            )
            .map_err(|error| format!("inspect BR-200 review occurrence: {error}"))
    })
    .await
    .map_err(|error| format!("BR-200 review terminal preflight join failed: {error}"))??;

    let Some(evidence) = evidence else {
        return Ok(None);
    };
    if let Some(hydration) = evidence.schedule_hydration.as_ref() {
        queue_hydrations(state.as_ref(), std::slice::from_ref(hydration))?;
    }
    Ok(Some(DurableDispatchEvidence {
        decision_identity: evidence.decision_identity,
        state: evidence.state,
        schedule_hydration: evidence.schedule_hydration,
    }))
}

pub fn replay_terminal_envelope(
    coordinator: &DurableDeliveryCoordinator,
    input: &ReviewTerminalReplayInput,
    now: DateTime<Utc>,
) -> Result<TerminalReplayClassification, String> {
    let prepared = coordinator
        .prepare(&input.envelope, 1, now)
        .map_err(|_| "terminal_replay_identity_invalid".to_owned())?;
    if prepared.decision_identity != input.decision_identity {
        return Ok(TerminalReplayClassification::Failed(
            "terminal_replay_identity_invalid",
        ));
    }
    match prepared.state {
        DecisionState::Delivered => match prepared.schedule_hydration {
            Some(hydration) if hydration.hydration_state == ScheduleHydrationState::Applied => {
                Ok(TerminalReplayClassification::ExistingTerminalHydrated)
            }
            _ => Ok(TerminalReplayClassification::Failed(
                "terminal_replay_hydration_not_applied",
            )),
        },
        DecisionState::RejectedDurable | DecisionState::ManualResolvedRejected => Ok(
            TerminalReplayClassification::Failed("terminal_replay_not_delivered"),
        ),
        _ => Ok(TerminalReplayClassification::Failed(
            "terminal_replay_would_require_sink",
        )),
    }
}

fn run_audited_terminal_replay_with<Classify>(
    coordinator: &DurableDeliveryCoordinator,
    append: &dyn ImmutableAppendPort,
    input: &ReviewTerminalReplayInput,
    classify: Classify,
) -> Result<TerminalReplayEvidence, String>
where
    Classify: FnOnce(
        &DurableDeliveryCoordinator,
        &ReviewTerminalReplayInput,
        DateTime<Utc>,
    ) -> Result<TerminalReplayClassification, String>,
{
    let attempt = coordinator
        .begin_review_terminal_replay(input, Utc::now())
        .map_err(|error| format!("terminal replay begin failed: {error}"))?;
    coordinator
        .append_review_terminal_replay_audit(
            &attempt.start_audit_identity,
            &attempt.decision_identity,
            "ReviewTerminalReplayStarted",
            append,
        )
        .map_err(|error| format!("terminal replay start audit append failed: {error}"))?;
    if !coordinator
        .review_terminal_replay_audit_appended(
            &attempt.start_audit_identity,
            &attempt.decision_identity,
            "ReviewTerminalReplayStarted",
        )
        .map_err(|error| format!("terminal replay start audit verification failed: {error}"))?
    {
        return Err("terminal_replay_evidence_unavailable".to_owned());
    }

    let (state, reason_code) = match classify(coordinator, input, Utc::now()) {
        Ok(TerminalReplayClassification::ExistingTerminalHydrated) => (
            ReviewTerminalReplayCompletionState::Passed,
            "existing_terminal_hydrated",
        ),
        Ok(TerminalReplayClassification::Failed(reason_code)) => {
            (ReviewTerminalReplayCompletionState::Failed, reason_code)
        }
        Err(error) => {
            log::error!(
                "[BR-194] terminal replay classification failed after Started audit: {error}"
            );
            (
                ReviewTerminalReplayCompletionState::Failed,
                "terminal_replay_evidence_unavailable",
            )
        }
    };
    let completion = match coordinator.finish_review_terminal_replay(
        &attempt,
        state,
        reason_code,
        0,
        0,
        0,
        0,
        Utc::now(),
    ) {
        Ok(completion) => completion,
        Err(error) if state == ReviewTerminalReplayCompletionState::Passed => coordinator
            .finish_review_terminal_replay(
                &attempt,
                ReviewTerminalReplayCompletionState::Failed,
                "terminal_replay_watermark_changed",
                0,
                0,
                0,
                0,
                Utc::now(),
            )
            .map_err(|fallback_error| {
                format!(
                    "terminal replay completion failed: {error}; failed evidence append also failed: {fallback_error}"
                )
            })?,
        Err(error) => return Err(format!("terminal replay completion failed: {error}")),
    };
    coordinator
        .append_review_terminal_replay_audit(
            &completion.completion_audit_identity,
            &completion.decision_identity,
            "ReviewTerminalReplayCompleted",
            append,
        )
        .map_err(|error| format!("terminal replay completion audit append failed: {error}"))?;
    if !coordinator
        .review_terminal_replay_audit_appended(
            &completion.completion_audit_identity,
            &completion.decision_identity,
            "ReviewTerminalReplayCompleted",
        )
        .map_err(|error| format!("terminal replay completion audit verification failed: {error}"))?
    {
        return Err("terminal_replay_evidence_unavailable".to_owned());
    }
    if completion.state != ReviewTerminalReplayCompletionState::Passed {
        return Err(completion.reason_code);
    }
    Ok(TerminalReplayEvidence {
        attempt_identity: attempt.attempt_identity,
        decision_identity: attempt.decision_identity,
        replay_ordinal: attempt.replay_ordinal,
    })
}

pub fn run_production_audited_terminal_replay(
    business_date: NaiveDate,
    task: crate::review_batch::ReviewTask,
) -> Result<TerminalReplayEvidence, String> {
    let _fixed_authority_manifest = BR194_REPLAY_AUTHORITY_MANIFEST_V1;
    if !matches!(
        task,
        crate::review_batch::ReviewTask::R04 | crate::review_batch::ReviewTask::R09
    ) {
        return Err("terminal_replay_identity_invalid".to_owned());
    }
    if stock_analysis::risk::env_guard::current_env()
        != stock_analysis::risk::env_guard::TradingEnv::Prod
    {
        return Err("terminal_replay_identity_invalid".to_owned());
    }
    let task_identity = crate::review_batch::review_task_identity(business_date, task);
    let coordinator =
        DurableDeliveryCoordinator::open(CoordinatorConfig::production(owner_instance_identity()))
            .map_err(|error| format!("terminal replay coordinator open failed: {error}"))?;
    let append = DurableDeliveryImmutableAppend::for_production()
        .map_err(|error| format!("terminal replay audit binding failed: {error}"))?;
    let input = coordinator
        .load_exact_terminal_replay_input(
            &business_date.format("%Y-%m-%d").to_string(),
            task.label(),
            &task_identity,
        )
        .map_err(|error| format!("terminal replay input rejected: {error}"))?;
    run_audited_terminal_replay_with(&coordinator, &append, &input, replay_terminal_envelope)
}

#[cfg(not(test))]
fn runtime_state() -> Result<Arc<RuntimeState>, String> {
    let namespace = resolve_runtime_namespace()?;
    let state = runtime_from_cache(&RUNTIME, namespace.clone(), build_runtime_state)?;
    ensure_runtime_state_namespace(state, &namespace)
}

#[cfg(test)]
fn runtime_state() -> Result<Arc<RuntimeState>, String> {
    let namespace = resolve_runtime_namespace()?;
    let test_code = match &namespace {
        RuntimeNamespace::Production => {
            return Err("BR-192 monitor unit tests cannot open the production runtime".to_owned());
        }
        RuntimeNamespace::Test { test_code } => test_code.clone(),
    };
    let mut runtimes = TEST_RUNTIMES
        .get_or_init(|| Mutex::new(std::collections::BTreeMap::new()))
        .lock()
        .map_err(|_| "BR-192 test runtime registry mutex poisoned".to_owned())?;
    if let Some(existing) = runtimes.get(&test_code) {
        let state = existing.as_ref().map(Arc::clone).map_err(Clone::clone)?;
        return ensure_runtime_state_namespace(state, &namespace);
    }
    let built = build_runtime_state(&namespace);
    runtimes.insert(test_code, built.clone());
    built
}

fn runtime_from_cache<T>(
    cache: &OnceLock<RuntimeCacheEntry<T>>,
    requested_namespace: RuntimeNamespace,
    build: impl FnOnce(&RuntimeNamespace) -> Result<Arc<T>, String>,
) -> Result<Arc<T>, String> {
    let entry = cache.get_or_init(|| RuntimeCacheEntry {
        namespace: requested_namespace.clone(),
        state: build(&requested_namespace),
    });
    if entry.namespace != requested_namespace {
        return Err(format!(
            "BR-192 durable runtime namespace mismatch: bound={} requested={}",
            entry.namespace.label(),
            requested_namespace.label()
        ));
    }
    entry.state.as_ref().map(Arc::clone).map_err(Clone::clone)
}

fn ensure_runtime_state_namespace(
    state: Arc<RuntimeState>,
    requested_namespace: &RuntimeNamespace,
) -> Result<Arc<RuntimeState>, String> {
    if &state.namespace != requested_namespace {
        return Err(format!(
            "BR-192 durable runtime state namespace mismatch: bound={} requested={}",
            state.namespace.label(),
            requested_namespace.label()
        ));
    }
    Ok(state)
}

fn build_runtime_state(namespace: &RuntimeNamespace) -> Result<Arc<RuntimeState>, String> {
    let owner_identity = owner_instance_identity();
    let (config, append) = match namespace {
        RuntimeNamespace::Production => (
            CoordinatorConfig::production(owner_identity),
            DurableDeliveryImmutableAppend::for_production().map_err(|error| {
                format!(
                    "bind durable delivery immutable append namespace={}: {error}",
                    namespace.label()
                )
            })?,
        ),
        RuntimeNamespace::Test { test_code } => {
            log::info!("[DurableDelivery][BR-192] test namespace bound code={test_code}");
            (
                CoordinatorConfig::test(
                    std::path::PathBuf::from("data/test")
                        .join(test_code)
                        .join("durable_delivery.sqlite3"),
                    test_code.clone(),
                    owner_identity,
                ),
                DurableDeliveryImmutableAppend::for_test_code(test_code).map_err(|error| {
                    format!(
                        "bind durable delivery immutable append namespace={}: {error}",
                        namespace.label()
                    )
                })?,
            )
        }
    };
    let sink = MagiclawAuthoritativeSink::bind(namespace.clone())?;
    let coordinator = DurableDeliveryCoordinator::open(config)
        .map_err(|error| format!("open durable delivery coordinator: {error}"))?;
    Ok(Arc::new(RuntimeState {
        namespace: namespace.clone(),
        coordinator: Arc::new(coordinator),
        append: Arc::new(append),
        sink: Arc::new(sink),
        producer_ready: AtomicBool::new(false),
        schedule_hydrations: Mutex::new(Vec::new()),
        queued_schedule_hydration_ids: Mutex::new(std::collections::BTreeSet::new()),
    }))
}

fn resolve_test_code(test_code_override: Option<&str>) -> Result<String, String> {
    let test_code = test_code_override
        .map(str::to_owned)
        .or_else(|| std::env::var("DURABLE_DELIVERY_TEST_CODE").ok())
        .ok_or_else(|| {
            "BR-192 test runtime requires explicit DURABLE_DELIVERY_TEST_CODE".to_owned()
        })?;
    validate_test_code(&test_code)?;
    Ok(test_code)
}

fn validate_test_code(test_code: &str) -> Result<(), String> {
    if !test_code.starts_with("TEST_CODE")
        || !test_code
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(
            "DURABLE_DELIVERY_TEST_CODE must start with TEST_CODE and be path-safe".to_owned(),
        );
    }
    Ok(())
}

fn reconcile_startup_blocking(state: &RuntimeState) -> Result<StartupReconcileEvidence, String> {
    let mut progress_count = 0usize;
    let mut resumed_sink_calls = 0usize;
    let mut hydrations = Vec::new();
    let mut foreign_attempts = std::collections::BTreeSet::new();
    let mut manual_reviews = std::collections::BTreeSet::new();
    let mut resumed_decisions = std::collections::BTreeSet::new();

    for _ in 0..100 {
        let summary = state
            .coordinator
            .reconcile_all_pending(state.append.as_ref(), Utc::now())
            .map_err(|error| format!("all-date local reconcile failed: {error}"))?;
        progress_count += summary.progress_count;
        hydrations.extend(summary.schedule_hydrations);
        if !summary.locally_pending_decisions.is_empty() {
            return Err(format!(
                "local pending decisions remain after reconcile: {:?}",
                summary.locally_pending_decisions
            ));
        }
        foreign_attempts.extend(summary.non_progressable_foreign_attempts);
        manual_reviews.extend(summary.non_progressable_manual_reviews);
        if summary.deliverable_decisions.is_empty() {
            let hydrations = unique_pending_hydrations(hydrations);
            queue_hydrations(state, &hydrations)?;
            return Ok(StartupReconcileEvidence {
                progress_count,
                resumed_sink_calls,
                non_progressable_foreign_attempts: foreign_attempts.into_iter().collect(),
                non_progressable_manual_reviews: manual_reviews.into_iter().collect(),
                schedule_hydrations: hydrations,
            });
        }

        for identity in summary.deliverable_decisions {
            if !resumed_decisions.insert(identity.clone()) {
                return Err(format!(
                    "stored decision {identity} remained deliverable after one startup resume; \
                     producers stay frozen to avoid a retry loop"
                ));
            }
            let outcome = state
                .coordinator
                .resume_deliverable(&identity, std::slice::from_ref(&state.sink), Utc::now())
                .map_err(|error| format!("resume stored decision {identity}: {error}"))?;
            resumed_sink_calls += outcome.sink_calls;
        }
    }
    Err("startup reconcile exceeded 100 fixed-point iterations".to_owned())
}

fn deliver_envelope_blocking(
    state: &RuntimeState,
    envelope: DeliveryEnvelope,
) -> Result<DurableDispatchEvidence, String> {
    let decision_identity = envelope.decision_identity.clone();
    state
        .coordinator
        .prepare(&envelope, 1, Utc::now())
        .map_err(|error| format!("prepare counted decision {decision_identity}: {error}"))?;
    let mut reconciled_hydrations = reconcile_current_decision(state, &decision_identity)?;

    if state
        .coordinator
        .decision_state(&decision_identity)
        .map_err(|error| format!("read prepared decision {decision_identity}: {error}"))?
        == DecisionState::Reserved
    {
        state
            .coordinator
            .resume_deliverable(
                &decision_identity,
                std::slice::from_ref(&state.sink),
                Utc::now(),
            )
            .map_err(|error| format!("deliver counted decision {decision_identity}: {error}"))?;
        reconciled_hydrations.extend(reconcile_current_decision(state, &decision_identity)?);
    }

    let final_state = state
        .coordinator
        .decision_state(&decision_identity)
        .map_err(|error| format!("read final decision {decision_identity}: {error}"))?;
    let reconciled_hydrations = unique_hydrations(reconciled_hydrations);
    let schedule_hydration = reconciled_hydrations
        .iter()
        .find(|hydration| hydration.decision_identity == decision_identity)
        .cloned();
    queue_hydrations(state, &reconciled_hydrations)?;
    observe_terminal(&envelope, final_state);
    Ok(DurableDispatchEvidence {
        decision_identity,
        state: final_state,
        schedule_hydration,
    })
}

fn reconcile_current_decision(
    state: &RuntimeState,
    decision_identity: &str,
) -> Result<Vec<ScheduleHydration>, String> {
    let mut hydrations = Vec::new();
    for _ in 0..20 {
        let summary = state
            .coordinator
            .reconcile_all_pending(state.append.as_ref(), Utc::now())
            .map_err(|error| format!("reconcile decision {decision_identity}: {error}"))?;
        hydrations.extend(summary.schedule_hydrations);
        if summary.progress_count == 0 {
            return Ok(hydrations);
        }
    }
    Err(format!(
        "decision {decision_identity} exceeded 20 local reconcile iterations"
    ))
}

fn envelope_from_binding(
    binding: CountedDeliveryBinding,
    kind: PushKind,
    text: &str,
    requested_sub_kind: Option<DailyReportSubKind>,
) -> Result<DeliveryEnvelope, String> {
    match kind {
        PushKind::ReviewLhb => binding
            .validate_r04_source_only_text(text)
            .map_err(str::to_owned)?,
        PushKind::EventCalendar => binding
            .validate_r08_public_source_only_text(text)
            .map_err(str::to_owned)?,
        _ => {}
    }
    let (push_kind, sub_kind) =
        durable_kind_and_sub_kind_with_override(kind, requested_sub_kind)
            .ok_or_else(|| format!("PushKind::{kind:?} is not in the counted catalog"))?;
    let scope_key = match binding.scope() {
        CountedDeliveryScope::Global => "GLOBAL".to_owned(),
        CountedDeliveryScope::Ticket { instrument } => canonical_ticket_scope(instrument),
    };
    let envelope = DeliveryEnvelope::new(
        binding.business_date().format("%Y-%m-%d").to_string(),
        push_kind,
        sub_kind,
        scope_key,
        binding.schedule_occurrence_identity().to_owned(),
        binding.source_evidence_fingerprint().to_owned(),
        binding.source_binding_canonical().to_vec(),
        binding.delivery_subject_hash().to_owned(),
        text.as_bytes().to_vec(),
        binding.retry_authorized(),
        binding.task_binding().cloned(),
    )
    .map_err(|error| error.to_string())?;
    match binding.origin() {
        CountedDeliveryOrigin::InternalDurable => Ok(envelope),
        CountedDeliveryOrigin::Provider {
            observed_at,
            as_of,
            ordered_batch_ids,
        } => envelope
            .with_provider_evidence(
                observed_at.as_ref().map(|value| value.to_rfc3339()),
                as_of
                    .as_ref()
                    .map(|value| value.format("%Y-%m-%d").to_string()),
                ordered_batch_ids.clone(),
            )
            .map_err(|error| error.to_string()),
    }
}

fn canonical_ticket_scope(instrument: &InstrumentId) -> String {
    let exchange = match instrument.exchange() {
        Exchange::Shanghai => "SHANGHAI",
        Exchange::Shenzhen => "SHENZHEN",
        Exchange::Beijing => "BEIJING",
    };
    let asset_class = match instrument.asset_class() {
        AssetClass::Equity => "EQUITY",
        AssetClass::Index => "INDEX",
        AssetClass::Fund => "FUND",
        AssetClass::Bond => "BOND",
        AssetClass::Option => "OPTION",
    };
    format!("{exchange}:{asset_class}:{}", instrument.code())
}

fn durable_kind_and_sub_kind(kind: PushKind) -> Option<(DurablePushKind, DeliverySubKind)> {
    durable_kind_and_sub_kind_with_override(kind, None)
}

fn durable_kind_and_sub_kind_with_override(
    kind: PushKind,
    requested_sub_kind: Option<DailyReportSubKind>,
) -> Option<(DurablePushKind, DeliverySubKind)> {
    use DurablePushKind as D;
    use PushKind as K;
    let mapped = match kind {
        K::HoldingPlan => (D::HoldingPlan, DeliverySubKind::None),
        K::HoldingEvent => (D::HoldingEvent, DeliverySubKind::None),
        K::T0Advice => (D::T0Advice, DeliverySubKind::None),
        K::CandidateTriggered => (D::CandidateTriggered, DeliverySubKind::None),
        K::CloseCall => (D::CloseCall, DeliverySubKind::None),
        K::ForbiddenOps => (D::ForbiddenOps, DeliverySubKind::None),
        K::PaperTrade => (D::PaperTrade, DeliverySubKind::None),
        K::ReviewMarket => (D::ReviewMarket, DeliverySubKind::None),
        K::ReviewLhb => (D::ReviewLhb, DeliverySubKind::None),
        K::ReviewSignal => (D::ReviewSignal, DeliverySubKind::None),
        K::ReviewFailure => (D::ReviewFailure, DeliverySubKind::None),
        K::TomorrowWatch => (D::TomorrowWatch, DeliverySubKind::None),
        K::EventCalendar => (D::EventCalendar, DeliverySubKind::None),
        K::ReviewProviderTopN => (D::ReviewProviderTopN, DeliverySubKind::None),
        K::FactorIC => (D::DailyReport, DeliverySubKind::FactorIC),
        K::SectorTier => (D::DailyReport, DeliverySubKind::SectorTier),
        K::CapitalVerify => (D::DailyReport, DeliverySubKind::CapitalVerify),
        K::DailyReport => (
            D::DailyReport,
            match requested_sub_kind {
                Some(DailyReportSubKind::FactorIC) => DeliverySubKind::FactorIC,
                Some(DailyReportSubKind::SectorTier) => DeliverySubKind::SectorTier,
                Some(DailyReportSubKind::CapitalVerify) => DeliverySubKind::CapitalVerify,
                None => DeliverySubKind::None,
            },
        ),
        _ => return None,
    };
    Some(mapped)
}

fn owner_instance_identity() -> String {
    let now = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    sha256_hex(format!("{}:{now}", std::process::id()).as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
}

fn pending_hydrations(state: &RuntimeState) -> Result<Vec<ScheduleHydration>, String> {
    let hydrations = state
        .schedule_hydrations
        .lock()
        .map_err(|_| "schedule hydration mutex poisoned".to_owned())?;
    Ok(hydrations.clone())
}

fn unique_hydrations(hydrations: Vec<ScheduleHydration>) -> Vec<ScheduleHydration> {
    let mut unique = std::collections::BTreeMap::new();
    for hydration in hydrations {
        unique.insert(hydration.transition_identity.clone(), hydration);
    }
    unique.into_values().collect()
}

fn unique_pending_hydrations(hydrations: Vec<ScheduleHydration>) -> Vec<ScheduleHydration> {
    unique_hydrations(hydrations)
        .into_iter()
        .filter(|hydration| hydration.hydration_state == ScheduleHydrationState::Pending)
        .collect()
}

fn queue_hydrations(state: &RuntimeState, hydrations: &[ScheduleHydration]) -> Result<(), String> {
    let mut queued_ids = state
        .queued_schedule_hydration_ids
        .lock()
        .map_err(|_| "schedule hydration identity mutex poisoned".to_owned())?;
    let mut queue = state
        .schedule_hydrations
        .lock()
        .map_err(|_| "schedule hydration mutex poisoned".to_owned())?;
    for hydration in hydrations {
        if hydration.hydration_state != ScheduleHydrationState::Pending {
            continue;
        }
        if queued_ids.insert(hydration.transition_identity.clone()) {
            queue.push(hydration.clone());
        }
    }
    Ok(())
}

fn acknowledge_schedule_hydrations_blocking(
    state: &RuntimeState,
    transition_identities: &std::collections::BTreeSet<String>,
    acknowledged_at: DateTime<Utc>,
) -> Result<(), String> {
    let requested = {
        let hydrations = state
            .schedule_hydrations
            .lock()
            .map_err(|_| "schedule hydration mutex poisoned".to_owned())?;
        let by_identity = hydrations
            .iter()
            .map(|hydration| (hydration.transition_identity.as_str(), hydration.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let missing = transition_identities
            .iter()
            .filter(|identity| !by_identity.contains_key(identity.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!(
                "BR-192 schedule hydration acknowledgement references unqueued transitions: {missing:?}"
            ));
        }
        transition_identities
            .iter()
            .map(|identity| {
                by_identity
                    .get(identity.as_str())
                    .expect("requested hydration was checked above")
                    .clone()
            })
            .collect::<Vec<_>>()
    };

    for hydration in &requested {
        state
            .coordinator
            .persist_schedule_hydration_applied(
                &hydration.transition_identity,
                &hydration.transition_sha256,
                state.append.as_ref(),
                acknowledged_at,
            )
            .map_err(|error| {
                format!(
                    "persist BR-192 schedule hydration {}: {error}",
                    hydration.transition_identity
                )
            })?;
    }

    let mut hydrations = state
        .schedule_hydrations
        .lock()
        .map_err(|_| "schedule hydration mutex poisoned".to_owned())?;
    hydrations.retain(|hydration| {
        !transition_identities.contains(hydration.transition_identity.as_str())
    });
    Ok(())
}

fn outcome_from_state(state: DecisionState) -> PushOutcome {
    match state {
        DecisionState::Delivered => PushOutcome::Pushed,
        DecisionState::RejectedDurable | DecisionState::ManualResolvedRejected => {
            PushOutcome::Denied(format!("durable delivery terminal state={state}"))
        }
        DecisionState::UncertainManualReview => PushOutcome::SinkError(
            "authoritative sink result is uncertain; manual review required".to_owned(),
        ),
        other => PushOutcome::SinkError(format!(
            "durable delivery is not terminal and will not be resent: state={other}"
        )),
    }
}

fn observe_terminal(envelope: &DeliveryEnvelope, state: DecisionState) {
    log::info!(
        "[DurableDelivery][BR-192] observer decision={} kind={} state={} bytes={}",
        envelope.decision_identity,
        envelope.push_kind,
        state,
        envelope.rendered_content.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    type R04JsonMutation = (&'static str, Box<dyn Fn(&mut serde_json::Value)>);

    struct ReplayAttemptAuditJoin {
        attempt_decision_identity: String,
        outbox_decision_identity: String,
        outbox_attempt_identity: Option<String>,
        audit_kind: String,
        start_canonical: Vec<u8>,
        audit_canonical: Vec<u8>,
        start_sha256: String,
        audit_sha256: String,
    }

    fn register_replay_sha256(connection: &rusqlite::Connection) {
        connection
            .create_scalar_function(
                "sha256_hex",
                1,
                rusqlite::functions::FunctionFlags::SQLITE_UTF8
                    | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
                |context| {
                    let canonical = context.get_raw(0).as_blob()?;
                    Ok(hex::encode(Sha256::digest(canonical)))
                },
            )
            .expect("register TEST_CODE replay sha256 authority");
    }

    struct TestNamespaceDir {
        root: std::path::PathBuf,
        retained: std::fs::File,
        device: u64,
        inode: u64,
    }

    impl TestNamespaceDir {
        fn new(test_code: &str) -> Self {
            use std::os::unix::fs::MetadataExt;
            assert!(
                test_code.starts_with("TEST_CODE")
                    && test_code
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
                "test namespace must be one TEST_CODE component"
            );
            let test_parent = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/test");
            std::fs::create_dir_all(&test_parent).expect("create TEST_CODE parent");
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("data/test")
                .join(test_code);
            std::fs::create_dir(&root).expect("create fresh exact TEST_CODE namespace");
            let retained = std::fs::File::open(&root).expect("retain TEST_CODE namespace inode");
            let metadata = retained
                .metadata()
                .expect("inspect retained TEST_CODE inode");
            Self {
                root,
                retained,
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }

        fn path(&self) -> &std::path::Path {
            &self.root
        }
    }

    impl Drop for TestNamespaceDir {
        fn drop(&mut self) {
            use std::os::unix::fs::MetadataExt;
            let retained = self
                .retained
                .metadata()
                .expect("inspect retained TEST_CODE inode before cleanup");
            let current = std::fs::symlink_metadata(&self.root)
                .expect("TEST_CODE namespace must still exist before cleanup");
            assert!(
                current.file_type().is_dir()
                    && current.dev() == self.device
                    && current.ino() == self.inode
                    && retained.dev() == self.device
                    && retained.ino() == self.inode,
                "refuse to remove a replaced TEST_CODE namespace"
            );
            std::fs::remove_dir_all(&self.root).expect("remove retained exact TEST_CODE namespace");
        }
    }

    struct AcceptingHydrationTestSink;

    struct RejectingReplayAppend;

    impl ImmutableAppendPort for RejectingReplayAppend {
        fn append_exact(
            &self,
            _record_kind: &str,
            _identity: &str,
            _canonical_bytes: &[u8],
            _sha256: &str,
        ) -> stock_analysis::durable_delivery::Result<String> {
            Err(
                stock_analysis::durable_delivery::DurableDeliveryError::PolicyMismatch(
                    "TEST_CODE_REPLAY_APPEND_FAULT".to_owned(),
                ),
            )
        }
    }

    struct RejectingCompletionReplayAppend {
        inner: DurableDeliveryImmutableAppend,
    }

    impl ImmutableAppendPort for RejectingCompletionReplayAppend {
        fn append_exact(
            &self,
            record_kind: &str,
            identity: &str,
            canonical_bytes: &[u8],
            sha256: &str,
        ) -> stock_analysis::durable_delivery::Result<String> {
            if record_kind == "ReviewTerminalReplayCompleted" {
                return Err(
                    stock_analysis::durable_delivery::DurableDeliveryError::PolicyMismatch(
                        "TEST_CODE_REPLAY_COMPLETION_APPEND_FAULT".to_owned(),
                    ),
                );
            }
            self.inner
                .append_exact(record_kind, identity, canonical_bytes, sha256)
        }
    }

    impl AuthoritativeSinkPort for AcceptingHydrationTestSink {
        fn sink_identity(&self) -> &str {
            "TEST_CODE_HYDRATION_AUTHORITATIVE_SINK"
        }

        fn deliver(&self, _request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
            AuthoritativeSinkResult::Accepted(stock_analysis::durable_delivery::TypedReceipt {
                channel: "TEST_CODE_CHANNEL".to_owned(),
                provider: "TEST_CODE_PROVIDER".to_owned(),
                message_id: "TEST_CODE_MESSAGE".to_owned(),
                platform_message_id: Some("TEST_CODE_PLATFORM_MESSAGE".to_owned()),
                accepted_at: Utc::now(),
                latency_ms: Some(1),
            })
        }
    }

    fn hydration_envelope(label: &str, business_date: &str) -> DeliveryEnvelope {
        DeliveryEnvelope::new(
            business_date,
            DurablePushKind::ReviewProviderTopN,
            DeliverySubKind::None,
            "GLOBAL",
            format!("TEST_CODE_OCCURRENCE_{label}"),
            format!("TEST_CODE_EVIDENCE_{label}"),
            format!("TEST_CODE_SOURCE_BINDING_{label}").into_bytes(),
            format!("TEST_CODE_SUBJECT_{label}"),
            format!("TEST_CODE_RENDERED_{label}").into_bytes(),
            true,
            Some(
                TaskBinding::new(
                    format!("TEST_CODE_TASK_{label}"),
                    format!("TEST_CODE_TRANSITION_BASIS_{label}").into_bytes(),
                )
                .expect("valid task binding"),
            ),
        )
        .expect("valid delivery envelope")
    }

    fn replay_envelope(
        business_date: NaiveDate,
        task: crate::review_batch::ReviewTask,
        label: &str,
    ) -> DeliveryEnvelope {
        let task_identity = crate::review_batch::review_task_identity(business_date, task);
        let transition_basis = serde_json::to_vec(&serde_json::json!({
            "task_identity": task_identity,
            "business_date": business_date.format("%Y-%m-%d").to_string(),
            "task": task.label(),
            "source": format!("TEST_CODE_SOURCE_{label}"),
            "rule_ids": ["BR-194"],
            "source_time": business_date.format("%Y-%m-%d").to_string(),
            "snapshot_size": 1,
            "request_hashes": [sha256_hex(label.as_bytes())],
            "batch_ids": [format!("TEST_CODE_BATCH_{label}")],
        }))
        .expect("serialize replay transition basis");
        DeliveryEnvelope::new(
            business_date.format("%Y-%m-%d").to_string(),
            match task {
                crate::review_batch::ReviewTask::R04 => DurablePushKind::ReviewLhb,
                crate::review_batch::ReviewTask::R09 => DurablePushKind::ReviewProviderTopN,
                _ => panic!("replay test supports only R-04/R-09"),
            },
            DeliverySubKind::None,
            "GLOBAL",
            format!("TEST_CODE_REPLAY_OCCURRENCE_{label}"),
            format!("TEST_CODE_REPLAY_EVIDENCE_{label}"),
            format!("TEST_CODE_REPLAY_SOURCE_{label}").into_bytes(),
            subject_hash(),
            format!("TEST_CODE_REPLAY_RENDERED_{label}").into_bytes(),
            true,
            Some(TaskBinding::new(task_identity, transition_basis).expect("valid task binding")),
        )
        .expect("valid replay envelope")
    }

    fn replay_state(test_code: &str) -> (TestNamespaceDir, Arc<RuntimeState>) {
        let namespace_dir = TestNamespaceDir::new(test_code);
        let namespace = RuntimeNamespace::Test {
            test_code: test_code.to_owned(),
        };
        let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            std::path::PathBuf::from("data/test")
                .join(test_code)
                .join("durable_delivery.sqlite3"),
            test_code,
            owner_instance_identity(),
        ))
        .expect("open replay test coordinator");
        let append = DurableDeliveryImmutableAppend::for_test_code(test_code)
            .expect("bind replay test append");
        let state = Arc::new(RuntimeState {
            namespace,
            coordinator: Arc::new(coordinator),
            append: Arc::new(append),
            sink: Arc::new(AcceptingHydrationTestSink),
            producer_ready: AtomicBool::new(false),
            schedule_hydrations: Mutex::new(Vec::new()),
            queued_schedule_hydration_ids: Mutex::new(std::collections::BTreeSet::new()),
        });
        (namespace_dir, state)
    }

    fn replay_input(
        state: &RuntimeState,
        business_date: NaiveDate,
        task: crate::review_batch::ReviewTask,
    ) -> ReviewTerminalReplayInput {
        let task_identity = crate::review_batch::review_task_identity(business_date, task);
        state
            .coordinator
            .load_exact_terminal_replay_input(
                &business_date.format("%Y-%m-%d").to_string(),
                task.label(),
                &task_identity,
            )
            .expect("load exact replay input")
    }

    fn deliver_and_hydrate_replay(state: &RuntimeState, envelope: &DeliveryEnvelope) {
        let hydration = deliver_hydration(state, envelope);
        state
            .coordinator
            .persist_schedule_hydration_applied(
                &hydration.transition_identity,
                &hydration.transition_sha256,
                state.append.as_ref(),
                Utc::now(),
            )
            .expect("apply replay schedule hydration");
    }

    fn r04_binding_from_canonical(
        business_date: NaiveDate,
        canonical: serde_json::Value,
    ) -> CountedDeliveryBinding {
        let (canonical_bytes, transition_basis) =
            crate::push_templates::canonical_review_lhb_source_binding_for_test(canonical);
        r04_binding_from_bytes_with_basis(business_date, canonical_bytes, transition_basis)
    }

    fn r04_binding_from_bytes(
        business_date: NaiveDate,
        canonical_bytes: Vec<u8>,
    ) -> CountedDeliveryBinding {
        let canonical: serde_json::Value =
            serde_json::from_slice(&canonical_bytes).expect("parse TEST_CODE R-04 binding");
        let transition_basis = serde_json::to_vec(&canonical["task_transition_basis"])
            .expect("serialize TEST_CODE transition basis");
        r04_binding_from_bytes_with_basis(business_date, canonical_bytes, transition_basis)
    }

    fn r04_binding_from_bytes_with_basis(
        business_date: NaiveDate,
        canonical_bytes: Vec<u8>,
        transition_basis: Vec<u8>,
    ) -> CountedDeliveryBinding {
        let canonical: serde_json::Value =
            serde_json::from_slice(&canonical_bytes).expect("parse TEST_CODE R-04 binding");
        let task_identity = crate::review_batch::review_task_identity(
            business_date,
            crate::review_batch::ReviewTask::R04,
        );
        CountedDeliveryBinding::new(
            business_date,
            task_identity.clone(),
            canonical_bytes,
            CountedDeliveryScope::Global,
            canonical["delivery_subject_identity"]
                .as_str()
                .expect("TEST_CODE delivery subject"),
            CountedDeliveryOrigin::Provider {
                observed_at: Some(
                    DateTime::parse_from_rfc3339(
                        canonical["evidence"]["observed_at"]
                            .as_str()
                            .expect("TEST_CODE observed_at"),
                    )
                    .expect("parse TEST_CODE observed_at")
                    .with_timezone(&Utc),
                ),
                as_of: Some(business_date),
                ordered_batch_ids: vec![canonical["evidence"]["batch_id"]
                    .as_str()
                    .expect("TEST_CODE batch")
                    .to_owned()],
            },
            Some(TaskBinding::new(task_identity, transition_basis).expect("valid task binding")),
            false,
        )
        .expect("valid TEST_CODE counted binding")
    }

    fn exact_r04_binding(text: &str) -> CountedDeliveryBinding {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 30).expect("valid TEST_CODE date");
        let business_date_text = business_date.format("%Y-%m-%d").to_string();
        let task_identity = crate::review_batch::review_task_identity(
            business_date,
            crate::review_batch::ReviewTask::R04,
        );
        let transition_basis = serde_json::json!({
            "task_identity": task_identity,
            "business_date": business_date_text,
            "task": "R-04",
            "source": "TEST_CODE_eastmoney_market_dragon_tiger",
            "source_time": business_date_text,
            "rule_ids": ["BR-110", "BR-140", "BR-162", "BR-192", "BR-200"],
            "snapshot_size": 1,
            "batch_ids": ["TEST_CODE_R04_BATCH"]
        });
        let seats = ["buy", "sell"]
            .into_iter()
            .flat_map(|side| {
                (1_u32..=5).map(move |rank| {
                    serde_json::json!({
                        "side": side,
                        "rank": rank,
                        "seat_name": format!("TEST_CODE_{side}_{rank}"),
                        "amount_yuan": 1_000_000.0 + f64::from(rank),
                        "buy_amount_yuan": if side == "buy" { Some(1_000_000.0) } else { None },
                        "sell_amount_yuan": if side == "sell" { Some(1_000_000.0) } else { None },
                        "net_amount_yuan": Some(if side == "buy" { 1_000_000.0 } else { -1_000_000.0 })
                    })
                })
            })
            .collect::<Vec<_>>();
        let canonical = serde_json::json!({
            "schema_version": 1,
            "business_date": business_date_text,
            "template_id": "review_lhb_v1",
            "review_task_identity": task_identity,
            "delivery_subject_identity": subject_hash(),
            "evidence": {
                "provider": "Eastmoney",
                "source": "TEST_CODE_eastmoney_market_dragon_tiger",
                "source_at": business_date_text,
                "observed_at": "2026-07-30T21:00:00+08:00",
                "batch_id": "TEST_CODE_R04_BATCH"
            },
            "ordered_projection": [{
                "source_order_ordinal": 0,
                "exchange": "SH",
                "code": "TEST_CODE_600396",
                "ranking_net_amount_yuan": 3_800_000.0,
                "disclosures": [{
                    "entry_id": "TEST_CODE_R04_ENTRY",
                    "trade_id": "TEST_CODE_R04_TRADE",
                    "reason": "TEST_CODE_REASON",
                    "buy_amount_yuan": 5_000_000.0,
                    "sell_amount_yuan": 1_200_000.0,
                    "net_amount_yuan": 3_800_000.0,
                    "turnover_rate_pct": 12.34,
                    "seats": seats
                }]
            }],
            "rendered_content_sha256": sha256_hex(text.as_bytes()),
            "task_transition_basis": transition_basis
        });
        r04_binding_from_canonical(business_date, canonical)
    }

    #[test]
    fn br194_r04_runtime_revalidates_exact_canonical_schema_and_rendered_text() {
        let text = "TEST_CODE_R04_EXACT_RENDERED_TEXT";
        let binding = exact_r04_binding(text);
        assert_eq!(binding.validate_r04_source_only(), Ok(()));
        assert_eq!(binding.validate_r04_source_only_text(text), Ok(()));
    }

    #[test]
    fn br194_r04_runtime_rejects_semantically_equal_noncanonical_bytes() {
        let exact = exact_r04_binding("TEST_CODE_R04_EXACT_RENDERED_TEXT");
        let value: serde_json::Value =
            serde_json::from_slice(exact.source_binding_canonical()).unwrap();
        let noncanonical = serde_json::to_vec_pretty(&value).unwrap();
        assert_ne!(noncanonical, exact.source_binding_canonical());
        let binding = r04_binding_from_bytes(exact.business_date(), noncanonical);
        assert_eq!(
            binding.validate_r04_source_only(),
            Err("counted_source_only_binding_invalid")
        );
    }

    #[test]
    fn br194_r04_runtime_rejects_schema_provider_projection_and_seat_mutations() {
        let text = "TEST_CODE_R04_EXACT_RENDERED_TEXT";
        let base = exact_r04_binding(text);
        let canonical: serde_json::Value =
            serde_json::from_slice(base.source_binding_canonical()).unwrap();
        let business_date = base.business_date();
        let mut mutations: Vec<R04JsonMutation> = vec![
            (
                "schema",
                Box::new(|value| value["schema_version"] = serde_json::json!(2)),
            ),
            (
                "provider",
                Box::new(|value| {
                    value["evidence"]["provider"] = serde_json::json!("TEST_CODE_BAD")
                }),
            ),
            (
                "source",
                Box::new(|value| value["evidence"]["source"] = serde_json::json!("")),
            ),
            (
                "ordinal",
                Box::new(|value| {
                    value["ordered_projection"][0]["source_order_ordinal"] = serde_json::json!(1)
                }),
            ),
            (
                "duplicate-seat",
                Box::new(|value| {
                    value["ordered_projection"][0]["disclosures"][0]["seats"][4]["rank"] =
                        serde_json::json!(4)
                }),
            ),
        ];
        for (label, mutation) in mutations.drain(..) {
            let mut changed = canonical.clone();
            mutation(&mut changed);
            let binding = r04_binding_from_canonical(business_date, changed);
            assert!(
                binding.validate_r04_source_only().is_err(),
                "mutation {label} must fail closed"
            );
        }
    }

    #[test]
    fn br194_r04_envelope_rejects_text_not_bound_by_canonical_hash() {
        let binding = exact_r04_binding("TEST_CODE_R04_BOUND_TEXT");
        assert!(envelope_from_binding(
            binding,
            PushKind::ReviewLhb,
            "TEST_CODE_R04_DIFFERENT_TEXT",
            None,
        )
        .unwrap_err()
        .contains("counted_source_only_binding_invalid"));
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_passes_with_equal_authority_watermarks() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (_namespace, state) = replay_state("TEST_CODE_BR194_REPLAY_EQUAL_WATERMARKS");
        let envelope =
            replay_envelope(business_date, crate::review_batch::ReviewTask::R09, "EQUAL");
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        let attempt = state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .expect("begin replay");
        state
            .coordinator
            .append_review_terminal_replay_audit(
                &attempt.start_audit_identity,
                &attempt.decision_identity,
                "ReviewTerminalReplayStarted",
                state.append.as_ref(),
            )
            .expect("append start audit");
        assert_eq!(
            replay_terminal_envelope(state.coordinator.as_ref(), &input, Utc::now())
                .expect("classify terminal envelope"),
            TerminalReplayClassification::ExistingTerminalHydrated
        );
        let completion = state
            .coordinator
            .finish_review_terminal_replay(
                &attempt,
                ReviewTerminalReplayCompletionState::Passed,
                "existing_terminal_hydrated",
                0,
                0,
                0,
                0,
                Utc::now(),
            )
            .expect("finish replay");
        assert_eq!(attempt.pre_sink_watermark, completion.post_sink_watermark);
        assert_eq!(
            attempt.pre_delivery_audit_watermark,
            completion.post_delivery_audit_watermark
        );
        assert_eq!(attempt.pre_sink_watermark.count, 1);
        assert_eq!(attempt.pre_delivery_audit_watermark.count, 1);
        state
            .coordinator
            .append_review_terminal_replay_audit(
                &completion.completion_audit_identity,
                &completion.decision_identity,
                "ReviewTerminalReplayCompleted",
                state.append.as_ref(),
            )
            .expect("append completion audit");
        assert!(state
            .coordinator
            .review_terminal_replay_audit_appended(
                &completion.completion_audit_identity,
                &completion.decision_identity,
                "ReviewTerminalReplayCompleted",
            )
            .expect("verify completion append"));
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_sink_eligibility_fails_before_sink() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (_namespace, state) = replay_state("TEST_CODE_BR194_REPLAY_NO_SINK");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "NO_SINK",
        );
        state
            .coordinator
            .prepare(&envelope, 1, Utc::now())
            .expect("reserve replay decision");
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        assert_eq!(
            replay_terminal_envelope(state.coordinator.as_ref(), &input, Utc::now())
                .expect("classify reserved decision"),
            TerminalReplayClassification::Failed("terminal_replay_would_require_sink")
        );
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_started_or_failed_cannot_verify() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (namespace, state) = replay_state("TEST_CODE_BR194_STARTED_FAILED");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "STARTED_FAILED",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        let dangling = state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .expect("begin dangling replay");
        assert_eq!(dangling.replay_ordinal, 1);
        state
            .coordinator
            .append_review_terminal_replay_audit(
                &dangling.start_audit_identity,
                &dangling.decision_identity,
                "ReviewTerminalReplayStarted",
                state.append.as_ref(),
            )
            .expect("append dangling start audit without a completion");

        let failed = run_audited_terminal_replay_with(
            state.coordinator.as_ref(),
            state.append.as_ref(),
            &input,
            |_, _, _| {
                Ok(TerminalReplayClassification::Failed(
                    "terminal_replay_not_delivered",
                ))
            },
        )
        .expect_err("Failed replay cannot verify");
        assert_eq!(failed, "terminal_replay_not_delivered");
        drop(state);

        let connection =
            rusqlite::Connection::open(namespace.path().join("durable_delivery.sqlite3"))
                .expect("open isolated replay database");
        let (attempts, failed_completions, passed_completions): (i64, i64, i64) = connection
            .query_row(
                "SELECT
                   (SELECT COUNT(*) FROM review_terminal_replay_attempts),
                   (SELECT COUNT(*) FROM review_terminal_replay_completions WHERE state='Failed'),
                   (SELECT COUNT(*) FROM review_terminal_replay_completions WHERE state='Passed')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (attempts, failed_completions, passed_completions),
            (2, 1, 0)
        );
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_classification_error_persists_failed_completion() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (namespace, state) = replay_state("TEST_CODE_BR194_CLASSIFY_ERROR");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "CLASSIFY_ERROR",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );

        let error = run_audited_terminal_replay_with(
            state.coordinator.as_ref(),
            state.append.as_ref(),
            &input,
            |_, _, _| Err("TEST_CODE classifier unavailable".to_owned()),
        )
        .expect_err("classification error must persist Failed terminal evidence");
        assert_eq!(error, "terminal_replay_evidence_unavailable");
        drop(state);

        let connection =
            rusqlite::Connection::open(namespace.path().join("durable_delivery.sqlite3"))
                .expect("open isolated replay database");
        let evidence: (i64, i64, i64) = connection
            .query_row(
                "SELECT
                   (SELECT COUNT(*) FROM review_terminal_replay_attempts),
                   (SELECT COUNT(*) FROM review_terminal_replay_completions
                     WHERE state='Failed'
                       AND reason_code='terminal_replay_evidence_unavailable'),
                   (SELECT COUNT(*) FROM immutable_audit_outbox
                     WHERE audit_kind='ReviewTerminalReplayCompleted'
                       AND append_state='Appended')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read failed terminal replay evidence");
        assert_eq!(evidence, (1, 1, 1));
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_rejects_out_of_contract_failed_reason() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (_namespace, state) = replay_state("TEST_CODE_BR194_REASON_VOCABULARY");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "REASON_VOCABULARY",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        let attempt = state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .expect("begin replay reason validation");

        let error = state
            .coordinator
            .finish_review_terminal_replay(
                &attempt,
                ReviewTerminalReplayCompletionState::Failed,
                "terminal_replay_classification_failed",
                0,
                0,
                0,
                0,
                Utc::now(),
            )
            .expect_err("seventh replay reason must fail closed");
        assert!(matches!(
            error,
            stock_analysis::durable_delivery::DurableDeliveryError::PolicyMismatch(reason)
                if reason == "terminal_replay_evidence_unavailable"
        ));
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_identity_and_audit_join_are_exact() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (namespace, state) = replay_state("TEST_CODE_BR194_EXACT_AUDIT_JOIN");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "EXACT_AUDIT_JOIN",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        let attempt = state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .unwrap();
        drop(state);

        let connection =
            rusqlite::Connection::open(namespace.path().join("durable_delivery.sqlite3"))
                .expect("open isolated replay database");
        let exact: ReplayAttemptAuditJoin = connection
            .query_row(
                "SELECT
                       a.decision_identity,o.decision_identity,o.attempt_identity,o.audit_kind,
                       a.start_canonical,o.audit_canonical,a.start_sha256,o.audit_sha256
                     FROM review_terminal_replay_attempts a
                     JOIN immutable_audit_outbox o
                       ON o.audit_identity=a.start_audit_identity
                     WHERE a.attempt_identity=?1",
                [&attempt.attempt_identity],
                |row| {
                    Ok(ReplayAttemptAuditJoin {
                        attempt_decision_identity: row.get(0)?,
                        outbox_decision_identity: row.get(1)?,
                        outbox_attempt_identity: row.get(2)?,
                        audit_kind: row.get(3)?,
                        start_canonical: row.get(4)?,
                        audit_canonical: row.get(5)?,
                        start_sha256: row.get(6)?,
                        audit_sha256: row.get(7)?,
                    })
                },
            )
            .unwrap();
        assert_eq!(
            exact.attempt_decision_identity,
            exact.outbox_decision_identity
        );
        assert_eq!(exact.outbox_attempt_identity, None);
        assert_eq!(exact.audit_kind, "ReviewTerminalReplayStarted");
        assert_eq!(exact.start_canonical, exact.audit_canonical);
        assert_eq!(exact.start_sha256, exact.audit_sha256);
        assert_eq!(attempt.decision_identity, exact.attempt_decision_identity);
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_trigger_recomputes_canonical_sha256() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (namespace, state) = replay_state("TEST_CODE_BR194_RECOMPUTE_SHA256");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "RECOMPUTE_SHA256",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .expect("seed exact replay authority");
        drop(state);

        let connection =
            rusqlite::Connection::open(namespace.path().join("durable_delivery.sqlite3"))
                .expect("open isolated replay database");
        register_replay_sha256(&connection);
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        let forged_hash = "0".repeat(64);
        connection
            .execute(
                "INSERT INTO immutable_audit_outbox(
                   audit_identity,decision_identity,attempt_identity,audit_kind,
                   predecessor_audit_identity,audit_canonical,audit_sha256,
                   append_state,immutable_audit_ref,created_at
                 )
                 SELECT
                   'TEST_CODE_FORGED_START_AUDIT',decision_identity,NULL,
                   'ReviewTerminalReplayStarted',NULL,X'7B7D',?1,
                   'Pending',NULL,started_at
                 FROM review_terminal_replay_attempts
                 LIMIT 1",
                [&forged_hash],
            )
            .expect("seed matching forged audit row");
        let error = connection
            .execute(
                "INSERT INTO review_terminal_replay_attempts(
                   attempt_identity,business_date,review_task,task_identity,
                   decision_identity,replay_ordinal,started_at,
                   pre_sink_count,pre_sink_set_sha256,
                   pre_delivery_audit_count,pre_delivery_audit_set_sha256,
                   provider_calls,start_canonical,start_sha256,start_audit_identity
                 )
                 SELECT
                   'TEST_CODE_FORGED_REPLAY_ATTEMPT',business_date,review_task,
                   task_identity,decision_identity,replay_ordinal+1,started_at,
                   pre_sink_count,pre_sink_set_sha256,
                   pre_delivery_audit_count,pre_delivery_audit_set_sha256,
                   0,X'7B7D',?1,'TEST_CODE_FORGED_START_AUDIT'
                 FROM review_terminal_replay_attempts
                 LIMIT 1",
                [&forged_hash],
            )
            .expect_err("matching forged hashes must not authorize replay evidence");
        match error {
            rusqlite::Error::SqliteFailure(sqlite, message) => {
                assert_eq!(
                    sqlite.extended_code,
                    rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER
                );
                assert!(message.unwrap_or_default().contains("start audit mismatch"));
            }
            other => panic!("unexpected forged-hash failure: {other}"),
        }
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_audit_uses_none_delivery_attempt_binding() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (namespace, state) = replay_state("TEST_CODE_BR194_NONE_ATTEMPT_BINDING");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "NONE_ATTEMPT_BINDING",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        let attempt = state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .unwrap();
        drop(state);
        let connection =
            rusqlite::Connection::open(namespace.path().join("durable_delivery.sqlite3"))
                .expect("open isolated replay database");
        let binding: Option<String> = connection
            .query_row(
                "SELECT attempt_identity FROM immutable_audit_outbox
                 WHERE audit_identity=?1",
                [&attempt.start_audit_identity],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(binding, None);
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_tables_reject_update_delete_and_second_completion() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (namespace, state) = replay_state("TEST_CODE_BR194_REPLAY_IMMUTABLE");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "IMMUTABLE",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        let attempt = state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .expect("begin replay");
        state
            .coordinator
            .finish_review_terminal_replay(
                &attempt,
                ReviewTerminalReplayCompletionState::Failed,
                "terminal_replay_evidence_unavailable",
                0,
                0,
                0,
                0,
                Utc::now(),
            )
            .expect("finish failed replay");
        drop(state);

        let connection =
            rusqlite::Connection::open(namespace.path().join("durable_delivery.sqlite3"))
                .expect("open isolated replay database");
        register_replay_sha256(&connection);
        connection
            .execute_batch("PRAGMA foreign_keys=ON;")
            .expect("enable isolated replay foreign keys");
        for statement in [
            "UPDATE review_terminal_replay_attempts SET started_at=started_at",
            "DELETE FROM review_terminal_replay_attempts",
            "UPDATE review_terminal_replay_completions SET completed_at=completed_at",
            "DELETE FROM review_terminal_replay_completions",
        ] {
            let error = connection
                .execute(statement, [])
                .expect_err("immutable replay mutation must fail");
            match error {
                rusqlite::Error::SqliteFailure(sqlite, _) => assert_eq!(
                    sqlite.extended_code,
                    rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER,
                    "wrong SQLite failure for {statement}"
                ),
                other => panic!("non-SQLite replay mutation failure for {statement}: {other}"),
            }
        }
        connection
            .execute(
                "INSERT INTO review_terminal_replay_completions
                 SELECT * FROM review_terminal_replay_completions",
                [],
            )
            .expect_err("second completion for one attempt must fail");
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_rejects_mismatched_completion_decision_and_audit() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (namespace, state) = replay_state("TEST_CODE_BR194_MISMATCHED_COMPLETION");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "MISMATCHED_COMPLETION",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .unwrap();
        drop(state);

        let connection =
            rusqlite::Connection::open(namespace.path().join("durable_delivery.sqlite3"))
                .expect("open isolated replay database");
        register_replay_sha256(&connection);
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        let error = connection
            .execute(
                "INSERT INTO review_terminal_replay_completions(
                   attempt_identity,decision_identity,state,completed_at,
                   post_sink_count,post_sink_set_sha256,post_delivery_audit_count,
                   post_delivery_audit_set_sha256,provider_calls,resume_calls,
                   sink_calls,delivery_audit_appends,reason_code,
                   completion_canonical,completion_sha256,completion_audit_identity
                 )
                 SELECT attempt_identity,decision_identity,'Failed',started_at,
                        pre_sink_count,pre_sink_set_sha256,pre_delivery_audit_count,
                        pre_delivery_audit_set_sha256,0,0,0,0,
                        'terminal_replay_evidence_unavailable',
                        start_canonical,start_sha256,start_audit_identity
                 FROM review_terminal_replay_attempts",
                [],
            )
            .expect_err("start audit cannot authorize a completion row");
        match error {
            rusqlite::Error::SqliteFailure(sqlite, message) => {
                assert_eq!(
                    sqlite.extended_code,
                    rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER
                );
                assert!(message
                    .unwrap_or_default()
                    .contains("completion audit mismatch"));
            }
            other => panic!("unexpected mismatched completion failure: {other}"),
        }
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_start_audit_ack_failure_blocks_classification() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (_namespace, state) = replay_state("TEST_CODE_BR194_START_APPEND_FAULT");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "START_APPEND_FAULT",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        let classify_calls = std::sync::atomic::AtomicUsize::new(0);
        let error = run_audited_terminal_replay_with(
            state.coordinator.as_ref(),
            &RejectingReplayAppend,
            &input,
            |coordinator, input, now| {
                classify_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                replay_terminal_envelope(coordinator, input, now)
            },
        )
        .expect_err("start append failure must fail closed");
        assert!(error.contains("start audit append failed"));
        assert_eq!(classify_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_completion_write_or_ack_failure_never_passes() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (namespace, state) = replay_state("TEST_CODE_BR194_COMPLETION_APPEND_FAULT");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "COMPLETION_APPEND_FAULT",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        let append = RejectingCompletionReplayAppend {
            inner: state.append.as_ref().clone(),
        };
        let error = run_audited_terminal_replay_with(
            state.coordinator.as_ref(),
            &append,
            &input,
            replay_terminal_envelope,
        )
        .expect_err("unacknowledged completion must never pass");
        assert!(error.contains("completion audit append failed"));
        drop(state);

        let connection =
            rusqlite::Connection::open(namespace.path().join("durable_delivery.sqlite3"))
                .expect("open isolated replay database");
        let evidence: (String, String) = connection
            .query_row(
                "SELECT c.state,o.append_state
                 FROM review_terminal_replay_completions c
                 JOIN immutable_audit_outbox o
                   ON o.audit_identity=c.completion_audit_identity",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(evidence, ("Passed".to_owned(), "Pending".to_owned()));
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_ordinals_advance_after_dangling_or_failed_attempts() {
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (namespace, state) = replay_state("TEST_CODE_BR194_REPLAY_ORDINALS");
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "ORDINALS",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        let dangling = state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .unwrap();
        let failed = state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .unwrap();
        state
            .coordinator
            .finish_review_terminal_replay(
                &failed,
                ReviewTerminalReplayCompletionState::Failed,
                "terminal_replay_evidence_unavailable",
                0,
                0,
                0,
                0,
                Utc::now(),
            )
            .unwrap();
        let next = state
            .coordinator
            .begin_review_terminal_replay(&input, Utc::now())
            .unwrap();
        assert_eq!(
            (
                dangling.replay_ordinal,
                failed.replay_ordinal,
                next.replay_ordinal
            ),
            (1, 2, 3)
        );
        drop(state);
        let connection =
            rusqlite::Connection::open(namespace.path().join("durable_delivery.sqlite3"))
                .expect("open isolated replay database");
        let retained: (i64, i64, i64) = connection
            .query_row(
                "SELECT COUNT(*),MIN(replay_ordinal),MAX(replay_ordinal)
                 FROM review_terminal_replay_attempts",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(retained, (3, 1, 3));
    }

    #[test]
    #[serial_test::serial(br194_replay_db)]
    fn br194_terminal_replay_cross_connection_contention_allocates_unique_ordinals() {
        let test_code = "TEST_CODE_BR194_REPLAY_CONTENTION";
        let business_date = NaiveDate::from_ymd_opt(2026, 7, 29).expect("valid date");
        let (_namespace, state) = replay_state(test_code);
        let envelope = replay_envelope(
            business_date,
            crate::review_batch::ReviewTask::R09,
            "CONTENTION",
        );
        deliver_and_hydrate_replay(state.as_ref(), &envelope);
        let input = replay_input(
            state.as_ref(),
            business_date,
            crate::review_batch::ReviewTask::R09,
        );
        let second = Arc::new(
            DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                std::path::PathBuf::from("data/test")
                    .join(test_code)
                    .join("durable_delivery.sqlite3"),
                test_code,
                "TEST_CODE_BR194_SECOND_CONNECTION",
            ))
            .expect("open second replay coordinator connection"),
        );
        let first = Arc::clone(&state.coordinator);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let spawn = |coordinator: Arc<DurableDeliveryCoordinator>,
                     input: ReviewTerminalReplayInput,
                     barrier: Arc<std::sync::Barrier>| {
            std::thread::spawn(move || {
                barrier.wait();
                coordinator
                    .begin_review_terminal_replay(&input, Utc::now())
                    .map(|attempt| attempt.replay_ordinal)
            })
        };
        let first_thread = spawn(first, input.clone(), barrier.clone());
        let second_thread = spawn(second, input, barrier.clone());
        barrier.wait();
        let mut ordinals = vec![
            first_thread.join().unwrap().unwrap(),
            second_thread.join().unwrap().unwrap(),
        ];
        ordinals.sort_unstable();
        assert_eq!(ordinals, [1, 2]);
    }

    fn deliver_hydration(state: &RuntimeState, envelope: &DeliveryEnvelope) -> ScheduleHydration {
        state
            .coordinator
            .prepare(envelope, 1, Utc::now())
            .expect("reserve counted decision");
        state
            .coordinator
            .reconcile_all_pending(state.append.as_ref(), Utc::now())
            .expect("append reservation audits");
        state
            .coordinator
            .resume_deliverable(
                &envelope.decision_identity,
                std::slice::from_ref(&state.sink),
                Utc::now(),
            )
            .expect("deliver counted decision");
        state
            .coordinator
            .reconcile_all_pending(state.append.as_ref(), Utc::now())
            .expect("append terminal transition")
            .schedule_hydrations
            .into_iter()
            .find(|hydration| hydration.decision_identity == envelope.decision_identity)
            .expect("pending schedule hydration")
    }

    fn subject_hash() -> String {
        "f8d53518ba6725c98450d031208450e7f8eb2dbdff2b9c71b21c14085e5d90ea".to_owned()
    }

    const FULL_CHAIN_CHILD_ENV: &str = "BR192_FULL_CHAIN_CHILD";
    const FULL_CHAIN_CHILD_ROLE_ENV: &str = "BR192_FULL_CHAIN_CHILD_ROLE";
    const FULL_CHAIN_BUSINESS_DATE_ENV: &str = "BR192_FULL_CHAIN_BUSINESS_DATE";
    const FULL_CHAIN_CHILD_TEST: &str =
        "durable_delivery_runtime::tests::TEST_CODE_br192_real_full_chain_child";

    fn full_chain_envelope(business_date: &str) -> DeliveryEnvelope {
        let source_binding = b"TEST_CODE_BR192_REAL_FULL_CHAIN_SOURCE_BINDING_V1".to_vec();
        DeliveryEnvelope::new(
            business_date,
            DurablePushKind::ReviewProviderTopN,
            DeliverySubKind::None,
            "GLOBAL",
            "TEST_CODE_BR192_REAL_FULL_CHAIN_OCCURRENCE",
            sha256_hex(&source_binding),
            source_binding,
            subject_hash(),
            b"TEST_CODE_BR192_REAL_FULL_CHAIN_RENDERED_CONTENT".to_vec(),
            true,
            None,
        )
        .expect("valid real full-chain TEST_CODE envelope")
    }

    fn domain_sha256(domain: &str, payload: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(domain.as_bytes());
        hasher.update([0]);
        hasher.update(payload);
        hex::encode(hasher.finalize())
    }

    fn collect_json_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        fn visit(path: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            if !path.exists() {
                return;
            }
            for entry in std::fs::read_dir(path).expect("read TEST_CODE artifact directory") {
                let path = entry.expect("read TEST_CODE artifact entry").path();
                if path.is_dir() {
                    visit(&path, files);
                } else if path.extension().and_then(std::ffi::OsStr::to_str) == Some("json") {
                    files.push(path);
                }
            }
        }
        let mut files = Vec::new();
        visit(root, &mut files);
        files.sort();
        files
    }

    #[derive(serde::Serialize)]
    struct FullChainAppendHashMaterial<'a> {
        hash_domain: &'a str,
        record_kind: &'a str,
        identity: &'a str,
        canonical_hex: &'a str,
        canonical_sha256: &'a str,
        previous_hash: Option<&'a str>,
    }

    fn verify_real_immutable_chain(path: &std::path::Path) -> Vec<serde_json::Value> {
        let content = std::fs::read_to_string(path).expect("read real immutable append chain");
        assert!(
            content.ends_with('\n'),
            "real immutable append chain must end at a complete record"
        );
        let mut previous_hash: Option<String> = None;
        let mut identities = std::collections::BTreeSet::new();
        let mut records = Vec::new();
        for line in content.lines() {
            let record: serde_json::Value =
                serde_json::from_str(line).expect("parse immutable append record");
            assert_eq!(
                record["hash_domain"],
                "stock_analysis.durable_delivery_immutable_append.v1"
            );
            assert_eq!(
                record["previous_hash"].as_str(),
                previous_hash.as_deref(),
                "immutable append predecessor must be exact"
            );
            let record_kind = record["record_kind"].as_str().unwrap();
            let identity = record["identity"].as_str().unwrap();
            let canonical_hex = record["canonical_hex"].as_str().unwrap();
            let canonical_sha256 = record["canonical_sha256"].as_str().unwrap();
            assert!(
                identities.insert(identity.to_owned()),
                "immutable append identities must be unique"
            );
            let canonical = hex::decode(canonical_hex).expect("decode immutable canonical bytes");
            assert_eq!(sha256_hex(&canonical), canonical_sha256);
            let material = FullChainAppendHashMaterial {
                hash_domain: "stock_analysis.durable_delivery_immutable_append.v1",
                record_kind,
                identity,
                canonical_hex,
                canonical_sha256,
                previous_hash: previous_hash.as_deref(),
            };
            let expected_hash =
                sha256_hex(&serde_json::to_vec(&material).expect("serialize append hash material"));
            assert_eq!(record["record_hash"].as_str(), Some(expected_hash.as_str()));
            previous_hash = Some(expected_hash);
            records.push(record);
        }
        assert!(!records.is_empty(), "real immutable append chain is empty");
        records
    }

    fn assert_exact_immutable_join<'a>(
        records: &'a [serde_json::Value],
        identity: &str,
        expected_kind: &str,
        expected_canonical: &[u8],
        expected_sha256: &str,
        expected_ref: &str,
    ) -> &'a serde_json::Value {
        let matching = records
            .iter()
            .filter(|record| record["identity"].as_str() == Some(identity))
            .collect::<Vec<_>>();
        assert_eq!(
            matching.len(),
            1,
            "SQLite audit identity must join exactly one immutable record: {identity}"
        );
        let record = matching[0];
        assert_eq!(
            record["record_kind"].as_str(),
            Some(expected_kind),
            "SQLite audit identity joined the wrong immutable record kind"
        );
        let canonical = hex::decode(
            record["canonical_hex"]
                .as_str()
                .expect("immutable canonical_hex"),
        )
        .expect("decode exact immutable canonical bytes");
        assert_eq!(
            canonical, expected_canonical,
            "SQLite canonical bytes must equal immutable canonical bytes"
        );
        assert_eq!(
            record["canonical_sha256"].as_str(),
            Some(expected_sha256),
            "SQLite canonical SHA-256 must equal immutable canonical SHA-256"
        );
        let record_hash = record["record_hash"]
            .as_str()
            .expect("immutable record_hash");
        assert_eq!(
            expected_ref,
            format!("durable-delivery:{record_hash}"),
            "SQLite immutable reference must bind the exact immutable record hash"
        );
        record
    }

    #[test]
    #[ignore = "isolated child invoked by the BR-192 real full-chain parent"]
    #[allow(non_snake_case)]
    fn TEST_CODE_br192_real_full_chain_child() {
        if std::env::var_os(FULL_CHAIN_CHILD_ENV).is_none() {
            return;
        }
        let test_code =
            std::env::var("DURABLE_DELIVERY_TEST_CODE").expect("full-chain child TEST_CODE");
        let role = std::env::var(FULL_CHAIN_CHILD_ROLE_ENV).expect("full-chain child process role");
        let business_date =
            std::env::var(FULL_CHAIN_BUSINESS_DATE_ENV).expect("full-chain business date");
        let state = build_runtime_state(&RuntimeNamespace::Test {
            test_code: test_code.clone(),
        })
        .expect("bind real full-chain child runtime");
        let outcome =
            deliver_envelope_blocking(state.as_ref(), full_chain_envelope(&business_date))
                .expect("run real full-chain child delivery");
        assert_eq!(role, "competitor");
        assert_eq!(
            outcome.state,
            DecisionState::AttemptInFlight,
            "competing process must observe the live foreign lease without a second sink call"
        );
    }

    #[test]
    fn br192_real_two_process_crash_recovery_joins_all_file_chains() {
        use std::io::Write;

        let test_code = format!(
            "TEST_CODE_BR192_REAL_FULL_CHAIN_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        );
        let namespace = TestNamespaceDir::new(&test_code);
        let business_date = Utc::now().date_naive().format("%Y-%m-%d").to_string();
        let executable = std::env::current_exe().expect("current monitor test binary");
        let mut crash_child = Command::new(&executable)
            .args(["--ignored", "--exact", FULL_CHAIN_CHILD_TEST, "--nocapture"])
            .env(FULL_CHAIN_CHILD_ENV, "1")
            .env(FULL_CHAIN_CHILD_ROLE_ENV, "crash-owner")
            .env(FULL_CHAIN_BUSINESS_DATE_ENV, &business_date)
            .env("STOCK_ENV_MODE", "test")
            .env("V10_DRY_RUN_PUSH", "1")
            .env("DURABLE_DELIVERY_TEST_CODE", &test_code)
            .env("BR192_FULL_CHAIN_CRASH_AFTER_ACCEPTED", "1")
            .env_remove("PUSH_LOG_DIR")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn accepted-then-crash owner process");

        let ready_path = namespace.path().join("br192_remote_accepted.ready");
        for _ in 0..3_000 {
            if ready_path.is_file() {
                break;
            }
            assert!(
                crash_child.try_wait().unwrap().is_none(),
                "accepted-crash child exited before publishing its durable ready marker"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            ready_path.is_file(),
            "accepted-crash child did not reach the post-receipt crash window"
        );
        let accepted_marker: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&ready_path).expect("read durable accepted marker"),
        )
        .expect("parse accepted marker");

        let before_competitor = collect_json_files(&namespace.path().join("push_log"));
        assert_eq!(
            before_competitor.len(),
            2,
            "accepted owner must have one pending and one committed artifact"
        );
        let competitor = Command::new(&executable)
            .args(["--ignored", "--exact", FULL_CHAIN_CHILD_TEST, "--nocapture"])
            .env(FULL_CHAIN_CHILD_ENV, "1")
            .env(FULL_CHAIN_CHILD_ROLE_ENV, "competitor")
            .env(FULL_CHAIN_BUSINESS_DATE_ENV, &business_date)
            .env("STOCK_ENV_MODE", "test")
            .env("V10_DRY_RUN_PUSH", "1")
            .env("DURABLE_DELIVERY_TEST_CODE", &test_code)
            .env_remove("BR192_FULL_CHAIN_CRASH_AFTER_ACCEPTED")
            .env_remove("PUSH_LOG_DIR")
            .output()
            .expect("run competing full-chain process");
        assert!(
            competitor.status.success(),
            "competing process failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&competitor.stdout),
            String::from_utf8_lossy(&competitor.stderr)
        );
        assert_eq!(
            collect_json_files(&namespace.path().join("push_log")),
            before_competitor,
            "foreign-lease competitor must not call the authoritative sink"
        );

        let release_path = namespace.path().join("br192_remote_accepted.release");
        let mut release = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&release_path)
            .expect("create accepted-crash release marker");
        release
            .write_all(b"TEST_CODE_RELEASE")
            .expect("write accepted-crash release marker");
        release
            .sync_all()
            .expect("fsync accepted-crash release marker");
        std::fs::File::open(namespace.path())
            .and_then(|directory| directory.sync_all())
            .expect("fsync TEST_CODE namespace after crash release");
        let crash_output = crash_child
            .wait_with_output()
            .expect("collect accepted-crash child");
        assert_eq!(
            crash_output.status.code(),
            Some(86),
            "owner must crash only after durable remote acceptance\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&crash_output.stdout),
            String::from_utf8_lossy(&crash_output.stderr)
        );

        let artifacts = collect_json_files(&namespace.path().join("push_log"));
        let pending_path = artifacts
            .iter()
            .find(|path| {
                path.file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|name| name.ends_with("_audit_pending.json"))
            })
            .expect("unique pending artifact");
        let committed_path = artifacts
            .iter()
            .find(|path| {
                path.file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|name| name.ends_with("_committed.json"))
            })
            .expect("unique committed artifact");
        let pending_bytes = std::fs::read(pending_path).expect("read pending artifact");
        let committed_bytes = std::fs::read(committed_path).expect("read committed artifact");
        let pending: serde_json::Value =
            serde_json::from_slice(&pending_bytes).expect("parse pending artifact");
        let committed: serde_json::Value =
            serde_json::from_slice(&committed_bytes).expect("parse committed artifact");
        assert_eq!(pending["schema"], "stock_analysis.counted_push_log.v1");
        assert_eq!(pending["state"], "AuditPending");
        assert_eq!(committed["schema"], "stock_analysis.counted_push_log.v1");
        assert_eq!(committed["state"], "Committed");
        assert_eq!(accepted_marker, pending["sink_result"]);

        let envelope = full_chain_envelope(&business_date);
        let decision_identity = envelope.decision_identity.clone();
        let attempt_identity = pending["attempt_identity"].as_str().unwrap().to_owned();
        let expected_decision_hash = domain_sha256(
            "stock_analysis.counted_decision_identity.v1",
            decision_identity.as_bytes(),
        );
        let expected_attempt_hash = domain_sha256(
            "stock_analysis.counted_attempt_identity.v1",
            attempt_identity.as_bytes(),
        );
        assert_eq!(
            pending["decision_identity"].as_str(),
            Some(decision_identity.as_str())
        );
        assert_eq!(
            pending["decision_identity_hash"].as_str(),
            Some(expected_decision_hash.as_str())
        );
        assert_eq!(
            pending["attempt_identity_hash"].as_str(),
            Some(expected_attempt_hash.as_str())
        );
        assert_eq!(
            domain_sha256(
                "stock_analysis.counted_push_log_artifact.v1",
                &pending_bytes
            ),
            committed["pending_artifact_sha256"]
        );
        assert_eq!(
            committed["decision_identity_hash"],
            pending["decision_identity_hash"]
        );
        assert_eq!(
            committed["attempt_identity_hash"],
            pending["attempt_identity_hash"]
        );
        let sink_result_bytes =
            serde_json::to_vec(&pending["sink_result"]).expect("serialize exact sink result");
        assert_eq!(
            domain_sha256("stock_analysis.counted_sink_result.v1", &sink_result_bytes),
            pending["sink_result_sha256"]
        );
        let receipt: stock_analysis::durable_delivery::TypedReceipt =
            serde_json::from_value(pending["sink_result"]["receipt"].clone())
                .expect("parse exact accepted receipt");
        let receipt_bytes = serde_json::to_vec(&receipt).expect("serialize exact accepted receipt");
        assert_eq!(
            domain_sha256("stock_analysis.counted_receipt.v1", &receipt_bytes),
            pending["receipt_sha256"]
        );
        assert_eq!(receipt.channel, "TEST_CODE_DRY_RUN");
        assert_eq!(receipt.provider, "TEST_CODE_MAGICLAW_DRY_RUN");

        let event_audit_root = namespace.path().join("event_audit");
        let event_files = std::fs::read_dir(&event_audit_root)
            .expect("read real schema-v3 event audit")
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().and_then(std::ffi::OsStr::to_str) == Some("jsonl"))
            .collect::<Vec<_>>();
        assert_eq!(event_files.len(), 1);
        let event_records = std::fs::read_to_string(&event_files[0])
            .expect("read real schema-v3 event audit")
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        let event_value = event_records
            .iter()
            .map(|record| &record["envelope"])
            .find(|event| event["id"] == committed["delivery_audit_event_id"])
            .expect("exact committed schema-v3 event");
        let event: stock_analysis::event::EventEnvelope =
            serde_json::from_value(event_value.clone()).expect("parse exact schema-v3 event");
        assert_eq!(event.payload["audit_schema_version"], 3);
        assert_eq!(
            event.payload["decision_identity_hash"],
            pending["decision_identity_hash"]
        );
        assert_eq!(
            event.payload["attempt_identity_hash"],
            pending["attempt_identity_hash"]
        );
        assert_eq!(
            event.payload["artifact_sha256"],
            committed["pending_artifact_sha256"]
        );
        assert_eq!(
            event.payload["sink_result_sha256"],
            pending["sink_result_sha256"]
        );
        assert_eq!(event.payload["receipt_sha256"], pending["receipt_sha256"]);
        assert_eq!(
            event.payload["counted_join_hash"].as_str(),
            Some(event.id.as_str())
        );
        assert_eq!(
            committed["counted_join_hash"].as_str(),
            Some(event.id.as_str())
        );
        stock_analysis::event::AuditDispatcher::for_test_code(&test_code)
            .expect("rebind schema-v3 event audit")
            .verify_exact_counted_event(&event)
            .expect("verify exact schema-v3 event and hash chain");

        let database_path = std::path::PathBuf::from("data/test")
            .join(&test_code)
            .join("durable_delivery.sqlite3");
        let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            database_path,
            &test_code,
            sha256_hex(b"TEST_CODE_BR192_FULL_CHAIN_PARENT_OWNER"),
        ))
        .expect("open same real TEST_CODE SQLite coordinator");
        let append = DurableDeliveryImmutableAppend::for_test_code(&test_code)
            .expect("rebind same real immutable append authority");
        let recovery_at = Utc::now() + chrono::Duration::seconds(180);
        coordinator
            .reconcile_all_pending(&append, recovery_at)
            .expect("classify expired crashed attempt without resend");
        assert_eq!(
            coordinator.decision_state(&decision_identity).unwrap(),
            DecisionState::UncertainManualReview
        );
        let manual = stock_analysis::durable_delivery::ManualResolutionCommand {
            decision_identity: decision_identity.clone(),
            disposition: stock_analysis::durable_delivery::ManualDisposition::Accepted {
                receipt: Some(receipt.clone()),
            },
            operator_identity: "TEST_CODE_BR192_AUTHORIZED_OPERATOR".to_owned(),
            reason: "TEST_CODE exact retained remote receipt after accepted-process crash"
                .to_owned(),
            external_evidence: pending_bytes.clone(),
            resolved_at: recovery_at + chrono::Duration::seconds(1),
        };
        assert_eq!(
            coordinator
                .resolve_uncertain(&manual, &append)
                .expect("authorize exact retained accepted receipt"),
            DecisionState::AcceptedAuditPending
        );
        coordinator
            .reconcile_all_pending(&append, manual.resolved_at)
            .expect("append manual acceptance and terminal coordinator join");
        assert_eq!(
            coordinator.decision_state(&decision_identity).unwrap(),
            DecisionState::Delivered
        );
        coordinator
            .verify_manual_accepted_delivery(&decision_identity)
            .expect("verify terminal coordinator join");

        let database =
            rusqlite::Connection::open(namespace.path().join("durable_delivery.sqlite3"))
                .expect("open real TEST_CODE SQLite read model");
        let (attempts, sink_results, delivered_transitions, pending_outbox): (i64, i64, i64, i64) = (
            database
                .query_row(
                    "SELECT COUNT(*) FROM delivery_attempts WHERE decision_identity=?1",
                    [&decision_identity],
                    |row| row.get(0),
                )
                .unwrap(),
            database
                .query_row(
                    "SELECT COUNT(*) FROM sink_results WHERE decision_identity=?1",
                    [&decision_identity],
                    |row| row.get(0),
                )
                .unwrap(),
            database
                .query_row(
                    "SELECT COUNT(*) FROM delivery_state_events
                     WHERE decision_identity=?1 AND to_state='Delivered'",
                    [&decision_identity],
                    |row| row.get(0),
                )
                .unwrap(),
            database
                .query_row(
                    "SELECT COUNT(*) FROM immutable_audit_outbox
                     WHERE decision_identity=?1 AND append_state!='Appended'",
                    [&decision_identity],
                    |row| row.get(0),
                )
                .unwrap(),
        );
        let (
            manual_receipt,
            accepted_audit_identity,
            accepted_audit_canonical,
            accepted_audit_sha256,
            accepted_audit_ref,
        ): (Vec<u8>, String, Vec<u8>, String, String) = database
            .query_row(
                "SELECT receipt_canonical,accepted_audit_identity,
                        frozen_delivery_audit_canonical,
                        frozen_delivery_audit_sha256,accepted_audit_ref
                 FROM manual_resolutions
                 WHERE decision_identity=?1 AND disposition='Accepted'",
                [&decision_identity],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("query exact manual accepted audit binding");
        let (
            delivered_audit_identity,
            delivered_audit_canonical,
            delivered_audit_sha256,
            delivered_audit_ref,
        ): (String, Vec<u8>, String, String) = database
            .query_row(
                "SELECT o.audit_identity,o.audit_canonical,o.audit_sha256,
                        o.immutable_audit_ref
                 FROM delivery_state_events e
                 JOIN immutable_audit_outbox o ON o.audit_identity=e.audit_identity
                 WHERE e.decision_identity=?1 AND e.to_state='Delivered'
                   AND o.append_state='Appended'",
                [&decision_identity],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("query exact Delivered immutable audit binding");
        assert_eq!(attempts, 1, "two processes must share one attempt");
        assert_eq!(
            sink_results, 0,
            "crashed process must not fabricate a persisted coordinator receipt"
        );
        assert_eq!(delivered_transitions, 1);
        assert_eq!(manual_receipt, receipt_bytes);
        assert_eq!(pending_outbox, 0);

        let immutable_records = verify_real_immutable_chain(
            &namespace
                .path()
                .join("durable_delivery_audit/durable_delivery_v1.jsonl"),
        );
        assert_exact_immutable_join(
            &immutable_records,
            &accepted_audit_identity,
            "DeliveryAcceptedAudit",
            &accepted_audit_canonical,
            &accepted_audit_sha256,
            &accepted_audit_ref,
        );
        let delivered_record = assert_exact_immutable_join(
            &immutable_records,
            &delivered_audit_identity,
            "DecisionStateChanged",
            &delivered_audit_canonical,
            &delivered_audit_sha256,
            &delivered_audit_ref,
        );
        let delivered_canonical = String::from_utf8(
            hex::decode(
                delivered_record["canonical_hex"]
                    .as_str()
                    .expect("Delivered canonical_hex"),
            )
            .expect("decode Delivered canonical bytes"),
        )
        .expect("Delivered canonical bytes are UTF-8 JSON");
        assert!(
            delivered_canonical.contains(&decision_identity)
                && delivered_canonical.contains("\"to_state\":\"Delivered\""),
            "the exact SQLite-joined immutable record must be the terminal Delivered transition"
        );
    }

    const FOREIGN_CWD_CHILD_ENV: &str = "BR192_RUNTIME_FOREIGN_CWD_CHILD";
    const FOREIGN_CWD_CHILD_TEST_CODE_ENV: &str = "BR192_RUNTIME_FOREIGN_CWD_TEST_CODE";
    const FOREIGN_CWD_CHILD_PATH_ENV: &str = "BR192_RUNTIME_FOREIGN_CWD_PATH";
    const FOREIGN_CWD_CHILD_TEST: &str =
        "durable_delivery_runtime::tests::TEST_CODE_br192_runtime_foreign_cwd_child";

    fn snapshot_tree(root: &std::path::Path) -> Option<Vec<(std::path::PathBuf, u64, u64, u64)>> {
        use std::os::unix::fs::MetadataExt;

        fn visit(
            base: &std::path::Path,
            path: &std::path::Path,
            rows: &mut Vec<(std::path::PathBuf, u64, u64, u64)>,
        ) {
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::symlink_metadata(path).expect("snapshot production object");
            rows.push((
                path.strip_prefix(base)
                    .expect("snapshot path beneath production root")
                    .to_path_buf(),
                metadata.dev(),
                metadata.ino(),
                metadata.len(),
            ));
            if metadata.file_type().is_dir() {
                let mut children = std::fs::read_dir(path)
                    .expect("read production snapshot directory")
                    .map(|entry| entry.expect("read production snapshot entry").path())
                    .collect::<Vec<_>>();
                children.sort();
                for child in children {
                    visit(base, &child, rows);
                }
            }
        }

        let metadata = match std::fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => panic!("snapshot production root {}: {error}", root.display()),
        };
        let mut rows = vec![(
            std::path::PathBuf::new(),
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
        )];
        if metadata.file_type().is_dir() {
            let mut children = std::fs::read_dir(root)
                .expect("read production snapshot root")
                .map(|entry| entry.expect("read production snapshot entry").path())
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                visit(root, &child, &mut rows);
            }
        }
        Some(rows)
    }

    #[test]
    #[ignore = "isolated child invoked by the foreign-CWD parent regression"]
    #[allow(non_snake_case)]
    fn TEST_CODE_br192_runtime_foreign_cwd_child() {
        if std::env::var_os(FOREIGN_CWD_CHILD_ENV).is_none() {
            return;
        }
        let test_code =
            std::env::var(FOREIGN_CWD_CHILD_TEST_CODE_ENV).expect("foreign-CWD child TEST_CODE");
        let foreign_cwd = std::path::PathBuf::from(
            std::env::var_os(FOREIGN_CWD_CHILD_PATH_ENV).expect("foreign-CWD child path"),
        );
        assert_eq!(
            std::env::current_dir().expect("foreign child cwd"),
            foreign_cwd
        );
        let state = build_runtime_state(&RuntimeNamespace::Test {
            test_code: test_code.clone(),
        })
        .expect("bind runtime from a foreign CWD");
        assert_eq!(
            state.namespace,
            RuntimeNamespace::Test {
                test_code: test_code.clone(),
            }
        );
        assert!(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("data/test")
                .join(&test_code)
                .join("durable_delivery.sqlite3")
                .is_file(),
            "coordinator must anchor the relative test path under the manifest root"
        );
        assert!(
            !foreign_cwd.join("data").exists(),
            "relative caller path must not create a second foreign-CWD authority"
        );
    }

    #[test]
    fn br192_runtime_foreign_cwd_anchors_sqlite_and_does_not_touch_production() {
        let test_code = format!(
            "TEST_CODE_RUNTIME_FOREIGN_CWD_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        );
        let namespace = TestNamespaceDir::new(&test_code);
        let foreign_cwd = namespace.path().join("foreign_cwd");
        std::fs::create_dir(&foreign_cwd).expect("create unrelated foreign CWD");
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let production_roots = [
            manifest.join("data/durable_delivery.sqlite3"),
            manifest.join("data/durable_delivery_audit"),
            manifest.join("data/push_log"),
            manifest.join("data/event_audit"),
        ];
        let before = production_roots
            .iter()
            .map(|root| snapshot_tree(root))
            .collect::<Vec<_>>();

        let output = Command::new(std::env::current_exe().expect("current monitor test binary"))
            .args([
                "--ignored",
                "--exact",
                FOREIGN_CWD_CHILD_TEST,
                "--nocapture",
            ])
            .current_dir(&foreign_cwd)
            .env(FOREIGN_CWD_CHILD_ENV, "1")
            .env(FOREIGN_CWD_CHILD_TEST_CODE_ENV, &test_code)
            .env(FOREIGN_CWD_CHILD_PATH_ENV, &foreign_cwd)
            .env("STOCK_ENV_MODE", "test")
            .env("V10_DRY_RUN_PUSH", "1")
            .env("DURABLE_DELIVERY_TEST_CODE", &test_code)
            .env_remove("PUSH_LOG_DIR")
            .output()
            .expect("run foreign-CWD runtime child");
        assert!(
            output.status.success(),
            "foreign-CWD runtime child failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(namespace.path().join("durable_delivery.sqlite3").is_file());
        assert!(!foreign_cwd.join("data").exists());
        let after = production_roots
            .iter()
            .map(|root| snapshot_tree(root))
            .collect::<Vec<_>>();
        assert_eq!(
            before, after,
            "TEST_CODE foreign-CWD runtime must not create, remove, or replace production artifacts"
        );
    }

    #[test]
    fn br192_main_eagerly_binds_runtime_artifacts_exactly_once_before_sink_init() {
        let source = include_str!("main.rs");
        let eager_call = "durable_delivery_runtime::eager_bind_runtime_artifacts()";
        assert_eq!(source.matches(eager_call).count(), 1);
        let preflight = source
            .find("let audit_preflight =")
            .expect("BR-144 preflight remains in startup");
        let eager = source.find(eager_call).expect("BR-192 eager bind call");
        let sink_init = source
            .find("let _sink_count = l6_sink::sink_count();")
            .expect("sink initialization remains in startup");
        assert!(
            preflight < eager && eager < sink_init,
            "eager capability bind must occur after BR-144 preflight and before sink initialization"
        );
    }

    #[test]
    fn br192_internal_binding_uses_exact_canonical_source_hash() {
        let binding = CountedDeliveryBinding::new(
            NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
            "TEST_CODE_SCHEDULE_OCCURRENCE",
            b"source-binding".to_vec(),
            CountedDeliveryScope::Ticket {
                instrument: InstrumentId::new(
                    Exchange::Shanghai,
                    "TEST_CODE_600000",
                    AssetClass::Equity,
                )
                .unwrap(),
            },
            subject_hash(),
            CountedDeliveryOrigin::InternalDurable,
            None,
            true,
        )
        .unwrap();

        assert_eq!(
            binding.source_evidence_fingerprint(),
            "b557d393e87ce7d6160a323bba5978ea2347f264308eef8b03c12d38bac2d2cd"
        );
        let envelope =
            envelope_from_binding(binding, PushKind::HoldingPlan, "counted body", None).unwrap();
        assert_eq!(envelope.scope_key, "SHANGHAI:EQUITY:TEST_CODE_600000");
        assert_eq!(
            envelope.schedule_occurrence_identity,
            "TEST_CODE_SCHEDULE_OCCURRENCE"
        );
        assert_eq!(envelope.source_binding_canonical, b"source-binding");
        assert_eq!(
            envelope.source_evidence_fingerprint,
            "b557d393e87ce7d6160a323bba5978ea2347f264308eef8b03c12d38bac2d2cd"
        );
        assert!(envelope.provider_observed_at.is_none());
        assert!(envelope.provider_as_of.is_none());
        assert!(envelope.original_batch_ids.is_empty());
        assert!(envelope.retry_authorized);
    }

    #[test]
    fn br192_provider_binding_preserves_optional_metadata_and_batch_order() {
        let binding = CountedDeliveryBinding::new(
            NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
            "TEST_CODE_PROVIDER_OCCURRENCE",
            b"source-binding".to_vec(),
            CountedDeliveryScope::Global,
            subject_hash(),
            CountedDeliveryOrigin::Provider {
                observed_at: Some(
                    DateTime::parse_from_rfc3339("2026-07-30T15:36:00+08:00")
                        .unwrap()
                        .with_timezone(&Utc),
                ),
                as_of: None,
                ordered_batch_ids: vec![
                    "TEST_CODE_BATCH_VOLUME".to_owned(),
                    "TEST_CODE_BATCH_FLOW".to_owned(),
                ],
            },
            None,
            false,
        )
        .unwrap();

        let envelope =
            envelope_from_binding(binding, PushKind::ReviewMarket, "counted body", None).unwrap();
        assert_eq!(envelope.scope_key, "GLOBAL");
        assert_eq!(
            envelope.provider_observed_at.as_deref(),
            Some("2026-07-30T07:36:00+00:00")
        );
        assert!(envelope.provider_as_of.is_none());
        assert_eq!(
            envelope.original_batch_ids,
            vec![
                "TEST_CODE_BATCH_VOLUME".to_owned(),
                "TEST_CODE_BATCH_FLOW".to_owned()
            ]
        );
    }

    #[test]
    fn br192_binding_rejects_non_hash_subject_and_empty_provider_batch_id() {
        let invalid_subject = CountedDeliveryBinding::new(
            NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
            "TEST_CODE_OCCURRENCE",
            b"source-binding".to_vec(),
            CountedDeliveryScope::Global,
            "not-a-hash",
            CountedDeliveryOrigin::InternalDurable,
            None,
            false,
        )
        .unwrap_err();
        assert_eq!(
            invalid_subject,
            "delivery_subject_hash must be lowercase SHA-256 hex"
        );

        let empty_batch = CountedDeliveryBinding::new(
            NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
            "TEST_CODE_OCCURRENCE",
            b"source-binding".to_vec(),
            CountedDeliveryScope::Global,
            subject_hash(),
            CountedDeliveryOrigin::Provider {
                observed_at: None,
                as_of: None,
                ordered_batch_ids: vec![String::new()],
            },
            None,
            false,
        )
        .unwrap_err();
        assert_eq!(empty_batch, "provider batch identities must not be empty");
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_each_test_guard_owns_one_stable_physical_runtime_namespace() {
        let (first_code, first_state, first_append_root) = {
            let _guard = crate::TestEnvGuard::dry_run_non_quiet();
            let test_code =
                std::env::var("DURABLE_DELIVERY_TEST_CODE").expect("first TEST_CODE namespace");
            let state = runtime_state().expect("first isolated runtime");
            let same_state = runtime_state().expect("same guard reuses runtime");
            assert!(Arc::ptr_eq(&state, &same_state));
            let append_root = state.append.base_dir().to_path_buf();
            let test_namespace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("data/test")
                .join(&test_code);
            assert_eq!(
                append_root,
                test_namespace_root.join("durable_delivery_audit")
            );
            assert!(test_namespace_root
                .join("durable_delivery.sqlite3")
                .is_file());
            (test_code, state, append_root)
        };

        let (second_code, second_state, second_append_root) = {
            let _guard = crate::TestEnvGuard::dry_run_non_quiet();
            let test_code =
                std::env::var("DURABLE_DELIVERY_TEST_CODE").expect("second TEST_CODE namespace");
            let state = runtime_state().expect("second isolated runtime");
            (
                test_code,
                Arc::clone(&state),
                state.append.base_dir().to_path_buf(),
            )
        };

        assert_ne!(first_code, second_code);
        assert_ne!(first_append_root, second_append_root);
        assert!(!Arc::ptr_eq(&first_state, &second_state));
    }

    #[test]
    fn br192_test_code_cannot_escape_its_physical_namespace() {
        let error =
            validate_test_code("TEST_CODE/../../prod").expect_err("path traversal rejected");
        assert!(error.contains("path-safe"));
    }

    #[test]
    fn br192_cached_process_runtime_rejects_namespace_switch() {
        let cache = OnceLock::<RuntimeCacheEntry<u8>>::new();
        let build_calls = std::cell::Cell::new(0_usize);
        let production = runtime_from_cache(&cache, RuntimeNamespace::Production, |_| {
            build_calls.set(build_calls.get() + 1);
            Ok(Arc::new(7_u8))
        })
        .expect("initialize production runtime");
        assert_eq!(*production, 7);
        let same_production = runtime_from_cache(&cache, RuntimeNamespace::Production, |_| {
            build_calls.set(build_calls.get() + 1);
            Ok(Arc::new(8_u8))
        })
        .expect("reuse production runtime");
        assert!(Arc::ptr_eq(&production, &same_production));
        assert_eq!(build_calls.get(), 1);

        let error = runtime_from_cache(
            &cache,
            RuntimeNamespace::Test {
                test_code: "TEST_CODE_NAMESPACE_SWITCH".to_owned(),
            },
            |_| {
                build_calls.set(build_calls.get() + 1);
                Ok(Arc::new(9_u8))
            },
        )
        .expect_err("cached production runtime must reject a test namespace");

        assert_eq!(
            error,
            "BR-192 durable runtime namespace mismatch: bound=production requested=test:TEST_CODE_NAMESPACE_SWITCH"
        );
        assert_eq!(
            build_calls.get(),
            1,
            "namespace mismatch must not invoke a replacement builder"
        );
    }

    fn authoritative_test_request(label: &str) -> AuthoritativeDeliveryRequest {
        let rendered_content = format!("TEST_CODE_BOUND_SINK_{label}").into_bytes();
        AuthoritativeDeliveryRequest {
            decision_identity: sha256_hex(format!("decision:{label}").as_bytes()),
            attempt_identity: sha256_hex(format!("attempt:{label}").as_bytes()),
            fence_token: 1,
            push_kind: DurablePushKind::HoldingEvent,
            stable_template_id: DurablePushKind::HoldingEvent
                .stable_template_id()
                .to_owned(),
            rendered_content_sha256: sha256_hex(&rendered_content),
            rendered_content,
        }
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_production_bound_sink_rejects_later_test_environment() {
        let _env_guard = crate::TestEnvGuard::capture(&[
            "STOCK_ENV_MODE",
            "V10_DRY_RUN_PUSH",
            "DURABLE_DELIVERY_TEST_CODE",
            "PUSH_LOG_DIR",
        ]);
        std::env::set_var("STOCK_ENV_MODE", "prod");
        std::env::remove_var("V10_DRY_RUN_PUSH");
        std::env::remove_var("DURABLE_DELIVERY_TEST_CODE");
        std::env::remove_var("PUSH_LOG_DIR");
        let bound_fixture_code = format!(
            "TEST_CODE_BOUND_PRODUCTION_WRITER_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        );
        let bound_fixture = TestNamespaceDir::new(&bound_fixture_code);
        let production_bound_sink = MagiclawAuthoritativeSink::from_test_artifacts(
            RuntimeNamespace::Production,
            crate::notify::PinnedPushLogWriter::for_test_anchor(
                "production",
                bound_fixture.path(),
                std::path::Path::new("push_log"),
            )
            .expect("bind production-semantic test push-log"),
            stock_analysis::event::AuditDispatcher::for_test_code(&bound_fixture_code)
                .expect("bind production-semantic TEST_CODE event audit"),
        );

        let test_code = format!(
            "TEST_CODE_BOUND_PRODUCTION_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        );
        let namespace_dir = TestNamespaceDir::new(&test_code);
        std::env::set_var("STOCK_ENV_MODE", "test");
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("DURABLE_DELIVERY_TEST_CODE", &test_code);
        let push_log_root = namespace_dir.path().join("push_log");
        std::env::set_var("PUSH_LOG_DIR", &push_log_root);

        let result =
            production_bound_sink.deliver(&authoritative_test_request("PRODUCTION_TO_TEST"));

        assert!(
            matches!(result, AuthoritativeSinkResult::Rejected(_)),
            "a production-bound sink must fail closed after an environment switch; \
             it must not synthesize a TEST_CODE receipt: {result:?}"
        );
        assert!(
            !push_log_root.exists(),
            "namespace mismatch must be rejected before push-log persistence"
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn br192_test_bound_sink_rejects_a_different_current_test_code() {
        let _env_guard = crate::TestEnvGuard::capture(&[
            "STOCK_ENV_MODE",
            "V10_DRY_RUN_PUSH",
            "DURABLE_DELIVERY_TEST_CODE",
            "PUSH_LOG_DIR",
        ]);
        let bound_test_code = format!(
            "TEST_CODE_BOUND_A_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        );
        std::env::set_var("STOCK_ENV_MODE", "test");
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("DURABLE_DELIVERY_TEST_CODE", &bound_test_code);
        let test_bound_sink = MagiclawAuthoritativeSink::bind(RuntimeNamespace::Test {
            test_code: bound_test_code,
        })
        .expect("bind TEST_CODE push-log writer");

        let current_test_code = format!(
            "TEST_CODE_BOUND_B_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        );
        let namespace_dir = TestNamespaceDir::new(&current_test_code);
        std::env::set_var("DURABLE_DELIVERY_TEST_CODE", &current_test_code);
        let push_log_root = namespace_dir.path().join("push_log");
        std::env::set_var("PUSH_LOG_DIR", &push_log_root);

        let result = test_bound_sink.deliver(&authoritative_test_request("TEST_A_TO_TEST_B"));

        assert!(
            matches!(result, AuthoritativeSinkResult::Rejected(_)),
            "a test-bound sink must fail closed when the current TEST_CODE differs; \
             it must not accept under the replacement namespace: {result:?}"
        );
        assert!(
            !push_log_root.exists(),
            "test namespace mismatch must be rejected before push-log persistence"
        );
    }

    #[test]
    fn br192_runtime_never_requeues_an_applied_schedule_hydration_after_restart() {
        let test_code = format!(
            "TEST_CODE_HYDRATION_QUEUE_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        );
        let _namespace_dir = TestNamespaceDir::new(&test_code);
        let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            std::path::PathBuf::from("data/test")
                .join(&test_code)
                .join("durable_delivery.sqlite3"),
            &test_code,
            format!("owner-{test_code}-0123456789abcdef"),
        ))
        .expect("open test coordinator");
        let state = RuntimeState {
            namespace: RuntimeNamespace::Test {
                test_code: test_code.clone(),
            },
            coordinator: Arc::new(coordinator),
            append: Arc::new(
                DurableDeliveryImmutableAppend::for_test_code(&test_code)
                    .expect("bind exact TEST_CODE immutable append"),
            ),
            sink: Arc::new(
                MagiclawAuthoritativeSink::bind(RuntimeNamespace::Test {
                    test_code: test_code.clone(),
                })
                .expect("bind TEST_CODE push-log writer"),
            ),
            producer_ready: AtomicBool::new(false),
            schedule_hydrations: Mutex::new(Vec::new()),
            queued_schedule_hydration_ids: Mutex::new(std::collections::BTreeSet::new()),
        };
        let pending = ScheduleHydration {
            decision_identity: "TEST_CODE_DECISION_PENDING".to_owned(),
            task_identity: "TEST_CODE_TASK_PENDING".to_owned(),
            transition_identity: "TEST_CODE_TRANSITION_PENDING".to_owned(),
            transition_canonical: b"TEST_CODE_TRANSITION_CANONICAL_PENDING".to_vec(),
            transition_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
            transition_basis_canonical: b"TEST_CODE_TRANSITION_BASIS_PENDING".to_vec(),
            transition_basis_sha256:
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
            immutable_audit_ref: "TEST_CODE_IMMUTABLE_REF_PENDING".to_owned(),
            hydration_state: ScheduleHydrationState::Pending,
        };
        let mut applied = pending.clone();
        applied.task_identity = "TEST_CODE_TASK_APPLIED".to_owned();
        applied.transition_identity = "TEST_CODE_TRANSITION_APPLIED".to_owned();
        applied.hydration_state = ScheduleHydrationState::Applied;

        queue_hydrations(&state, &[applied.clone(), pending.clone()])
            .expect("queue pending hydration only");
        assert_eq!(
            pending_hydrations(&state).expect("read queued hydrations"),
            vec![pending]
        );

        let restarted_queue = unique_pending_hydrations(vec![applied]);
        assert!(
            restarted_queue.is_empty(),
            "an Applied database projection must not be hydrated into scheduler memory again"
        );
    }

    #[test]
    fn br192_runtime_acknowledges_only_consumed_date_and_restart_retains_foreign_date() {
        let test_code = format!(
            "TEST_CODE_HYDRATION_DATES_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        );
        let _namespace_dir = TestNamespaceDir::new(&test_code);
        let database_path = std::path::PathBuf::from("data/test")
            .join(&test_code)
            .join("durable_delivery.sqlite3");
        let state = RuntimeState {
            namespace: RuntimeNamespace::Test {
                test_code: test_code.clone(),
            },
            coordinator: Arc::new(
                DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                    &database_path,
                    &test_code,
                    format!("owner-first-{test_code}-0123456789abcdef"),
                ))
                .expect("open first test coordinator"),
            ),
            append: Arc::new(
                DurableDeliveryImmutableAppend::for_test_code(&test_code)
                    .expect("bind first exact TEST_CODE immutable append"),
            ),
            sink: Arc::new(AcceptingHydrationTestSink),
            producer_ready: AtomicBool::new(false),
            schedule_hydrations: Mutex::new(Vec::new()),
            queued_schedule_hydration_ids: Mutex::new(std::collections::BTreeSet::new()),
        };
        let consumed =
            deliver_hydration(&state, &hydration_envelope("CONSUMED_DATE", "2026-07-29"));
        let foreign = deliver_hydration(&state, &hydration_envelope("FOREIGN_DATE", "2026-07-30"));
        queue_hydrations(&state, &[consumed.clone(), foreign.clone()])
            .expect("queue both pending business dates");

        acknowledge_schedule_hydrations_blocking(
            &state,
            &std::collections::BTreeSet::from([consumed.transition_identity.clone()]),
            Utc::now(),
        )
        .expect("durably acknowledge only the consumed business date");
        assert_eq!(
            pending_hydrations(&state).expect("read retained foreign hydration"),
            vec![foreign.clone()]
        );

        let restarted = RuntimeState {
            namespace: RuntimeNamespace::Test {
                test_code: test_code.clone(),
            },
            coordinator: Arc::new(
                DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                    &database_path,
                    &test_code,
                    format!("owner-restart-{test_code}-0123456789abcdef"),
                ))
                .expect("open restarted test coordinator"),
            ),
            append: Arc::new(
                DurableDeliveryImmutableAppend::for_test_code(&test_code)
                    .expect("bind restarted exact TEST_CODE immutable append"),
            ),
            sink: Arc::new(AcceptingHydrationTestSink),
            producer_ready: AtomicBool::new(false),
            schedule_hydrations: Mutex::new(Vec::new()),
            queued_schedule_hydration_ids: Mutex::new(std::collections::BTreeSet::new()),
        };
        let restart_summary = restarted
            .coordinator
            .reconcile_all_pending(restarted.append.as_ref(), Utc::now())
            .expect("restart reconciliation");
        let restart_pending = unique_pending_hydrations(restart_summary.schedule_hydrations);
        assert_eq!(restart_pending, vec![foreign.clone()]);
        queue_hydrations(&restarted, &restart_pending).expect("rehydrate foreign date only");
        assert_eq!(
            pending_hydrations(&restarted).expect("read restarted queue"),
            vec![foreign]
        );
    }
}
