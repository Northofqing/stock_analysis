//! W17 comparison evidence for two adapters over one captured input.
//!
//! Only the capabilities supplied here are denied and counted. Rust callbacks can still access
//! global I/O; adapting those calls remains a production migration gate. A match grants no
//! activation, authentication, completion, or physical-owner authority.

use std::{cell::Cell, collections::BTreeSet, fmt, time::Duration};

use super::{
    AttemptId, CompletionDirective, JobDecision, JobDecisionView, PreparedFactsSnapshot,
    ReasonCode, RunContext, SemanticProjection, Sha256Digest, UnitId, UtcMicros,
};

/// The closed set of effects covered by this execution interface.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ShadowEffect {
    ProviderSecondCall,
    LlmRecompute,
    BusinessDbWrite,
    DurableDbWrite,
    CursorAdvance,
    CandidateWatchlistOutcome,
    PaperOrderFill,
    TransportSend,
}

impl ShadowEffect {
    pub const ALL: [Self; 8] = [
        Self::ProviderSecondCall,
        Self::LlmRecompute,
        Self::BusinessDbWrite,
        Self::DurableDbWrite,
        Self::CursorAdvance,
        Self::CandidateWatchlistOutcome,
        Self::PaperOrderFill,
        Self::TransportSend,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderSecondCall => "provider_second_call",
            Self::LlmRecompute => "llm_recompute",
            Self::BusinessDbWrite => "business_db_write",
            Self::DurableDbWrite => "durable_db_write",
            Self::CursorAdvance => "cursor_advance",
            Self::CandidateWatchlistOutcome => "candidate_watchlist_outcome",
            Self::PaperOrderFill => "paper_order_fill",
            Self::TransportSend => "transport_send",
        }
    }
}

/// An immutable snapshot. There is no public constructor or counter reset.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ShadowEffectCounts([u64; 8]);

impl ShadowEffectCounts {
    pub fn get(&self, effect: ShadowEffect) -> u64 {
        self.0[effect as usize]
    }

    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|count| *count == 0)
    }
}

impl fmt::Debug for ShadowEffectCounts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut counts = f.debug_map();
        for effect in ShadowEffect::ALL {
            counts.entry(&effect.as_str(), &self.get(effect));
        }
        counts.finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("shadow effect denied: {effect:?}")]
pub struct ShadowEffectDenied {
    effect: ShadowEffect,
}

impl ShadowEffectDenied {
    pub fn effect(self) -> ShadowEffect {
        self.effect
    }
}

/// Execution-local, deny-only capabilities. No operation implementation can be installed.
#[derive(Debug)]
pub struct ShadowDeniedEffects {
    counts: Cell<ShadowEffectCounts>,
}

impl ShadowDeniedEffects {
    fn new() -> Self {
        Self {
            counts: Cell::new(ShadowEffectCounts([0; 8])),
        }
    }

    /// Count before returning denial, even if an adapter ignores the error. Saturation can never
    /// turn a nonzero attempt count back into zero. This method never executes an action.
    pub fn request(&self, effect: ShadowEffect) -> Result<(), ShadowEffectDenied> {
        let mut counts = self.counts.get();
        counts.0[effect as usize] = counts.get(effect).saturating_add(1);
        self.counts.set(counts);
        Err(ShadowEffectDenied { effect })
    }
}

/// These are the only excluded fields. Business time belongs to RunContext or JobDecision.
#[derive(Default)]
pub struct ShadowDiagnostics {
    pub attempt_id: Option<AttemptId>,
    pub latency: Option<Duration>,
    pub diagnostic_timestamp: Option<UtcMicros>,
}

impl fmt::Debug for ShadowDiagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShadowDiagnostics").finish_non_exhaustive()
    }
}

/// Actual typed outputs, together with the input instances observed by the adapter.
pub struct ShadowObservation<'a> {
    context: &'a RunContext,
    facts: &'a PreparedFactsSnapshot,
    decision: JobDecision,
    projection: Option<SemanticProjection>,
    reason: ReasonCode,
    completion: CompletionDirective,
    diagnostics: ShadowDiagnostics,
}

impl<'a> ShadowObservation<'a> {
    pub fn new(
        context: &'a RunContext,
        facts: &'a PreparedFactsSnapshot,
        decision: JobDecision,
        projection: Option<SemanticProjection>,
        reason: ReasonCode,
        completion: CompletionDirective,
        diagnostics: ShadowDiagnostics,
    ) -> Self {
        Self {
            context,
            facts,
            decision,
            projection,
            reason,
            completion,
            diagnostics,
        }
    }

    pub fn diagnostics(&self) -> &ShadowDiagnostics {
        &self.diagnostics
    }
}

impl fmt::Debug for ShadowObservation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShadowObservation").finish_non_exhaustive()
    }
}

/// Adapters must discard arbitrary error strings at this boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("shadow callback failed")]
pub struct ShadowCallbackFailure;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ShadowPath {
    Old,
    New,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShadowPathStatus {
    NotExecuted,
    Completed,
    CallbackFailed,
    InvalidObservation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ShadowInvalidBinding {
    ContextInstance,
    FactsInstance,
    ReadyContext,
    ReadyFacts,
    ReadyUnit,
    ReadyOccurrence,
    ReadyProjectionMissing,
    ReadyProjectionMismatch,
    NoDataEvidence,
    DecisionReason,
}

/// Closed, deterministically ordered differences containing no payload or callback strings.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ShadowDifference {
    ContextFactsBinding,
    CallbackFailed(ShadowPath),
    InvalidObservation {
        path: ShadowPath,
        binding: ShadowInvalidBinding,
    },
    JobDecision,
    SemanticProjection,
    RenderedSha256,
    RenderedBytes,
    ReasonCode,
    CompletionSchedule,
    CompletionCursor,
    CompletionRetry,
    CompletionManual,
}

/// Read-only evidence for this invocation. Only execute_shadow can construct it.
#[derive(Debug)]
pub struct ShadowReport {
    unit_id: UnitId,
    run_context_sha256: Sha256Digest,
    prepared_facts_sha256: Sha256Digest,
    statuses: [ShadowPathStatus; 2],
    counts: [ShadowEffectCounts; 2],
    differences: Vec<ShadowDifference>,
}

impl ShadowReport {
    pub fn unit_id(&self) -> &UnitId {
        &self.unit_id
    }

    pub fn run_context_sha256(&self) -> &Sha256Digest {
        &self.run_context_sha256
    }

    pub fn prepared_facts_sha256(&self) -> &Sha256Digest {
        &self.prepared_facts_sha256
    }

    pub fn status(&self, path: ShadowPath) -> ShadowPathStatus {
        self.statuses[path as usize]
    }

    pub fn counts(&self, path: ShadowPath) -> &ShadowEffectCounts {
        &self.counts[path as usize]
    }

    pub fn differences(&self) -> &[ShadowDifference] {
        &self.differences
    }

    pub fn reasons(&self) -> Vec<ReasonCode> {
        let mut reasons = Vec::new();
        if !self.differences.is_empty() {
            reasons.push(ReasonCode::ShadowSemanticDiff);
        }
        if self.counts.iter().any(|counts| !counts.is_zero()) {
            reasons.push(ReasonCode::ShadowSideEffectAttempted);
        }
        reasons
    }

    /// Match means only equal covered semantics and zero attempts on the supplied capabilities.
    pub fn is_match(&self) -> bool {
        self.statuses == [ShadowPathStatus::Completed; 2]
            && self.differences.is_empty()
            && self.counts.iter().all(ShadowEffectCounts::is_zero)
    }
}

/// Executes both synchronous adapters against the same references. Initial context/facts
/// mismatch executes neither adapter. Callback failure does not skip the other adapter.
pub fn execute_shadow<'a, Old, New>(
    context: &'a RunContext,
    facts: &'a PreparedFactsSnapshot,
    old: Old,
    new: New,
) -> ShadowReport
where
    Old: FnOnce(
        &'a RunContext,
        &'a PreparedFactsSnapshot,
        &ShadowDeniedEffects,
    ) -> Result<ShadowObservation<'a>, ShadowCallbackFailure>,
    New: FnOnce(
        &'a RunContext,
        &'a PreparedFactsSnapshot,
        &ShadowDeniedEffects,
    ) -> Result<ShadowObservation<'a>, ShadowCallbackFailure>,
{
    let mut report = ShadowReport {
        unit_id: context.unit_id().clone(),
        run_context_sha256: context.canonical_sha256(),
        prepared_facts_sha256: facts.facts().canonical_sha256(),
        statuses: [ShadowPathStatus::NotExecuted; 2],
        counts: [ShadowEffectCounts([0; 8]); 2],
        differences: Vec::new(),
    };
    if facts.facts().run_context_sha256() != &report.run_context_sha256 {
        report
            .differences
            .push(ShadowDifference::ContextFactsBinding);
        return report;
    }

    let old_effects = ShadowDeniedEffects::new();
    let new_effects = ShadowDeniedEffects::new();
    let old_result = old(context, facts, &old_effects);
    let new_result = new(context, facts, &new_effects);
    report.counts = [old_effects.counts.get(), new_effects.counts.get()];
    let mut differences = BTreeSet::new();
    for (path, result) in [
        (ShadowPath::Old, &old_result),
        (ShadowPath::New, &new_result),
    ] {
        report.statuses[path as usize] = match result {
            Err(ShadowCallbackFailure) => {
                differences.insert(ShadowDifference::CallbackFailed(path));
                ShadowPathStatus::CallbackFailed
            }
            Ok(observation) => {
                let invalid = validate_observation(context, facts, &report, observation);
                if invalid.is_empty() {
                    ShadowPathStatus::Completed
                } else {
                    for binding in invalid {
                        differences.insert(ShadowDifference::InvalidObservation { path, binding });
                    }
                    ShadowPathStatus::InvalidObservation
                }
            }
        };
    }
    // Preserve output differences even when a path supplied an invalid binding.
    if let (Ok(old), Ok(new)) = (&old_result, &new_result) {
        compare_observations(old, new, &mut differences);
    }
    report.differences = differences.into_iter().collect();
    report
}

fn validate_observation(
    context: &RunContext,
    facts: &PreparedFactsSnapshot,
    report: &ShadowReport,
    observation: &ShadowObservation<'_>,
) -> Vec<ShadowInvalidBinding> {
    use ShadowInvalidBinding as Invalid;
    let mut invalid = Vec::new();
    if !std::ptr::eq(context, observation.context) {
        invalid.push(Invalid::ContextInstance);
    }
    if !facts.shares_instance_with(observation.facts) {
        invalid.push(Invalid::FactsInstance);
    }
    match observation.decision.view() {
        JobDecisionView::Ready(push) => {
            if push.run_context_sha256() != &report.run_context_sha256 {
                invalid.push(Invalid::ReadyContext);
            }
            if push.prepared_facts_sha256() != &report.prepared_facts_sha256 {
                invalid.push(Invalid::ReadyFacts);
            }
            if push.unit_id() != context.unit_id() {
                invalid.push(Invalid::ReadyUnit);
            }
            if push.occurrence() != context.occurrence() {
                invalid.push(Invalid::ReadyOccurrence);
            }
            match &observation.projection {
                None => invalid.push(Invalid::ReadyProjectionMissing),
                Some(projection) if projection.sha256() != push.semantic_projection_sha256() => {
                    invalid.push(Invalid::ReadyProjectionMismatch);
                }
                Some(_) => {}
            }
        }
        JobDecisionView::NoData {
            reason,
            evidence_sha256,
        } => {
            if !facts.facts().verified_empty() || evidence_sha256 != &report.prepared_facts_sha256 {
                invalid.push(Invalid::NoDataEvidence);
            }
            if observation.reason != reason {
                invalid.push(Invalid::DecisionReason);
            }
        }
        JobDecisionView::Disabled { reason }
        | JobDecisionView::BlockedOnInput { reason, .. }
        | JobDecisionView::Suppressed { reason, .. }
        | JobDecisionView::RetryableFailure { reason, .. }
        | JobDecisionView::PermanentFailure { reason } => {
            if observation.reason != reason {
                invalid.push(Invalid::DecisionReason);
            }
        }
    }
    invalid
}

fn compare_observations(
    old: &ShadowObservation<'_>,
    new: &ShadowObservation<'_>,
    differences: &mut BTreeSet<ShadowDifference>,
) {
    use ShadowDifference as Diff;
    for (different, item) in [
        (old.decision != new.decision, Diff::JobDecision),
        (old.projection != new.projection, Diff::SemanticProjection),
        (old.reason != new.reason, Diff::ReasonCode),
        (
            old.completion.schedule() != new.completion.schedule(),
            Diff::CompletionSchedule,
        ),
        (
            old.completion.cursor() != new.completion.cursor(),
            Diff::CompletionCursor,
        ),
        (
            old.completion.retry() != new.completion.retry(),
            Diff::CompletionRetry,
        ),
        (
            old.completion.manual() != new.completion.manual(),
            Diff::CompletionManual,
        ),
    ] {
        if different {
            differences.insert(item);
        }
    }
    if let (JobDecisionView::Ready(old), JobDecisionView::Ready(new)) =
        (old.decision.view(), new.decision.view())
    {
        if old.rendered_sha256() != new.rendered_sha256() {
            differences.insert(Diff::RenderedSha256);
        }
        if old.replay_rendered_bytes() != new.replay_rendered_bytes() {
            differences.insert(Diff::RenderedBytes);
        }
    }
}
