//! Recoverable observed P05 Unit façade. Observation is not QualifiedFacts,
//! coverage or a physical receipt. Main supplies its existing qualified batch
//! and renderer after its original admission checks; this module adds no source
//! qualification. A restored Unit does not invoke a provider or rebuild bytes.
use crate::durable_delivery::{
    AuthoritativeSink, DurableDeliveryCoordinator, DurableDeliveryError, Result,
};
use crate::durable_delivery::{P05ObservedDraftInput, P05ObservedSourceBytes};
use crate::opportunity::candidate_panel::CandidateEntry;
use chrono::{DateTime, FixedOffset};
use std::sync::Arc;

pub use crate::durable_delivery::P05InvalidationRenderFacts;
/// Raw input witnesses from the existing batch loader, never qualification.
#[derive(Debug)]
pub struct P05ObservedBatchSourceBytes {
    pub quote_evidence: Option<Vec<u8>>,
    pub statistics_evidence: Option<Vec<u8>>,
    pub p5_file_witnesses: Vec<u8>,
    pub p5_candidate_refs: Vec<u8>,
    pub chain_query: Vec<u8>,
    pub chain_candidate_refs: Vec<u8>,
}
/// Full immutable intent is committed before this opaque handle is returned.
/// A handle cannot be deserialized, claim completion or select a child set.
pub struct P05AuctionUnit {
    counted: Arc<DurableDeliveryCoordinator>,
    date: String,
    intent_identity: String,
    child_count: usize,
}
/// Revision-bound read snapshot with original outcomes; later mutation can
/// invalidate its completion pointer. Finalize rechecks the actual current
/// revision/receipts by CAS. It grants no source/sink permission or future-current
/// proof, and no bool promotes nonacceptance.
#[derive(Debug)]
pub struct P05UnitObservation(crate::durable_delivery::P05UnitReceiptObservation);
impl P05UnitObservation {
    pub fn unit_identity(&self) -> &str {
        self.0.draft_identity()
    }
    pub fn mutation_revision(&self) -> i64 {
        self.0.mutation_revision()
    }
    pub fn completion_identity(&self) -> Option<&str> {
        self.0.completion_identity()
    }
    pub fn child_observations(&self) -> &[crate::durable_delivery::P05ChildReceiptObservation] {
        self.0.children()
    }
}
/// Observation of a real pre-09:20 prospective initialization, not authority.
#[derive(Debug)]
pub struct P05ProspectiveObservation {
    origin_identity: String,
}
impl P05ProspectiveObservation {
    pub fn origin_identity(&self) -> &str {
        &self.origin_identity
    }
}
impl P05AuctionUnit {
    pub fn initialize_prospective_family(
        counted: &DurableDeliveryCoordinator,
    ) -> Result<P05ProspectiveObservation> {
        let identity = counted
            .initialize_prospective_p05_family(counted.require_p05_production_singleton()?)?;
        Ok(P05ProspectiveObservation {
            origin_identity: identity,
        })
    }

    /// Restore first, before collecting a fresh provider batch. Unknown Started
    /// with no exact actual freeze is an error; it never reopens the worker.
    pub async fn restore(
        counted: Arc<DurableDeliveryCoordinator>,
        business_date: &str,
    ) -> Result<Option<Self>> {
        let Some(draft) = counted.read_p05_unit_draft(business_date)? else {
            return Ok(None);
        };
        let intent = if let Some(intent) = counted.read_p05_unit_intent(business_date)? {
            intent
        } else {
            counted.require_p05_production_singleton()?;
            counted.finish_p05_unit_preparation_global(&draft).await?
        };
        Ok(Some(Self {
            counted,
            date: business_date.into(),
            intent_identity: intent.identity().into(),
            child_count: intent.children().len(),
        }))
    }
    /// Inputs are ordinary observations, never a JSON/hash authority factory.
    /// The fixed date and actual production clock must share 09:20..09:25.
    pub async fn observe_batch(
        counted: Arc<DurableDeliveryCoordinator>,
        captured_shanghai: DateTime<FixedOffset>,
        entries: &[CandidateEntry],
        source: P05ObservedBatchSourceBytes,
        auction_rendered: Vec<u8>,
        board_rendered: Vec<u8>,
        renderer: &mut impl FnMut(&P05InvalidationRenderFacts) -> Result<Vec<u8>>,
    ) -> Result<Self> {
        counted.require_p05_production_singleton()?;
        let input = P05ObservedDraftInput::from_observed(
            captured_shanghai,
            entries,
            P05ObservedSourceBytes {
                quote_evidence: source.quote_evidence,
                statistics_evidence: source.statistics_evidence,
                p5_file_witnesses: source.p5_file_witnesses,
                p5_candidate_refs: source.p5_candidate_refs,
                chain_query: source.chain_query,
                chain_candidate_refs: source.chain_candidate_refs,
            },
            auction_rendered,
            board_rendered,
        )?;
        let draft = counted.store_p05_observed_draft_with_renderer(&input, renderer)?;
        let intent = counted.finish_p05_unit_preparation_global(&draft).await?;
        Ok(Self {
            date: draft.business_date().into(),
            intent_identity: intent.identity().into(),
            child_count: intent.children().len(),
            counted,
        })
    }
    pub fn require_counted_owner(&self, counted: &Arc<DurableDeliveryCoordinator>) -> Result<()> {
        require_same_counted(&self.counted, counted)
    }
    pub fn inspect_child(&self, index: usize) -> Result<P05ChildDispatchView> {
        self.check_intent()?;
        Ok(P05ChildDispatchView {
            counted: self.counted.clone(),
            inner: self
                .counted
                .inspect_p05_unit_child_global(&self.date, index)?,
        })
    }
    pub fn inspect_owned_child(
        counted: Arc<DurableDeliveryCoordinator>,
        decision_identity: &str,
    ) -> Result<Option<P05ChildDispatchView>> {
        let Some(inner) = counted.inspect_p05_owned_child_global(decision_identity)? else {
            return Ok(None);
        };
        Ok(Some(P05ChildDispatchView { counted, inner }))
    }
    pub fn inspect_unfinished(
        counted: &DurableDeliveryCoordinator,
    ) -> Result<Vec<P05StoredUnitSnapshot>> {
        counted.inspect_p05_unfinished_units()
    }
    pub fn prepare_child(
        &self,
        view: &P05ChildDispatchView,
        sink_count: usize,
    ) -> Result<P05PreparedChild> {
        self.check_child_view(view)?;
        view.prepare_child(sink_count)
    }
    pub fn resume_prepared_child(
        &self,
        prepared: &P05PreparedChild,
        sinks: &[AuthoritativeSink],
    ) -> Result<crate::durable_delivery::ResumeOutcome> {
        prepared.require_counted_owner(&self.counted)?;
        if prepared.inner.intent_identity() != self.intent_identity
            || prepared.inner.business_date() != self.date
        {
            return Err(DurableDeliveryError::PolicyMismatch(
                "P05 prepared child belongs to another Unit".into(),
            ));
        }
        prepared.resume(sinks)
    }
    fn check_child_view(&self, view: &P05ChildDispatchView) -> Result<()> {
        view.require_counted_owner(&self.counted)?;
        if view.inner.intent_identity() != self.intent_identity
            || view.inner.business_date() != self.date
        {
            return Err(DurableDeliveryError::PolicyMismatch(
                "P05 child view belongs to another Unit".into(),
            ));
        }
        Ok(())
    }
    pub fn intent_identity(&self) -> &str {
        &self.intent_identity
    }
    pub fn child_count(&self) -> usize {
        self.child_count
    }
    pub fn business_date(&self) -> &str {
        &self.date
    }
    /// Child indices follow immutable Auction/T08 sorted removals/Board order.
    /// Original state/retry policy controls each attempt, including Accepted
    /// no-resend. Reconciliation/hydration use the existing coordinator ports.
    pub fn dispatch_child(
        &self,
        index: usize,
        sinks: &[AuthoritativeSink],
        append: &dyn crate::durable_delivery::ImmutableAppendPort,
    ) -> Result<crate::durable_delivery::ResumeOutcome> {
        let view = self.inspect_child(index)?;
        self.counted
            .dispatch_p05_child_inspection(&view.inner, sinks, append)
    }
    pub fn observe(&self) -> Result<P05UnitObservation> {
        self.check_intent()?;
        Ok(P05UnitObservation(
            self.counted.observe_p05_unit_receipts(&self.date)?,
        ))
    }
    /// CAS against a read-only revision; all child receipt joins are recomputed
    /// in the finalization transaction. Caller evidence is never adopted.
    pub fn finalize(&self, observed: &P05UnitObservation) -> Result<P05UnitObservation> {
        self.check_intent()?;
        if !self.counted.p05_has_first_completion(&self.date)? {
            self.counted.validate_p05_unit_prediction_on(
                self.counted.require_p05_production_singleton()?,
                &self.date,
            )?;
        }
        let current = self.counted.observe_p05_unit_receipts(&self.date)?;
        if current.draft_identity() != observed.0.draft_identity() {
            return Err(DurableDeliveryError::PolicyMismatch(
                "P05 finalizer observation belongs to another Unit".into(),
            ));
        }
        Ok(P05UnitObservation(
            self.counted
                .finalize_p05_unit_observed(&self.date, &observed.0)?,
        ))
    }
    fn check_intent(&self) -> Result<()> {
        let intent = self
            .counted
            .read_p05_unit_intent(&self.date)?
            .ok_or_else(|| {
                DurableDeliveryError::PolicyMismatch("P05 original complete intent absent".into())
            })?;
        if intent.identity() != self.intent_identity || intent.children().len() != self.child_count
        {
            return Err(DurableDeliveryError::PolicyMismatch(
                "P05 fixed intent differs from handle".into(),
            ));
        }
        Ok(())
    }
}

pub use crate::durable_delivery::{P05StoredUnitPhase, P05StoredUnitSnapshot};
fn require_same_counted(
    original: &Arc<DurableDeliveryCoordinator>,
    actual: &Arc<DurableDeliveryCoordinator>,
) -> Result<()> {
    if !Arc::ptr_eq(original, actual) {
        return Err(DurableDeliveryError::PolicyMismatch(
            "P05 view/Unit is not bound to the actual counted runtime instance".into(),
        ));
    }
    Ok(())
}
/// Readonly original immutable envelope with a released actual-reader result.
/// A failed read cannot authorize a fresh opening; passive inspection itself
/// does not reserve budget or send, and never qualifies ordinary input bytes.
pub struct P05ChildDispatchView {
    counted: Arc<DurableDeliveryCoordinator>,
    inner: crate::durable_delivery::P05ChildInspection,
}
impl P05ChildDispatchView {
    pub fn envelope(&self) -> &crate::durable_delivery::DeliveryEnvelope {
        self.inner.envelope()
    }
    pub fn business_date(&self) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(self.inner.business_date(), "%Y-%m-%d")
            .expect("strict readonly date")
    }
    pub fn unit_identity(&self) -> &str {
        self.inner.unit_identity()
    }
    pub fn intent_identity(&self) -> &str {
        self.inner.intent_identity()
    }
    pub fn child_identity(&self) -> &str {
        self.inner.child_identity()
    }
    pub fn ordinal(&self) -> usize {
        self.inner.ordinal()
    }
    pub fn governance_code(&self) -> Option<&str> {
        self.inner.governance_code()
    }
    pub fn rendered_text(&self) -> &str {
        std::str::from_utf8(&self.envelope().rendered_content)
            .expect("strict readonly UTF8 envelope")
    }
    pub fn require_counted_owner(&self, counted: &Arc<DurableDeliveryCoordinator>) -> Result<()> {
        require_same_counted(&self.counted, counted)
    }
    /// Sole contextual prepare, no reconciliation or resume.
    pub fn prepare_child(&self, sink_count: usize) -> Result<P05PreparedChild> {
        let outcome = self
            .counted
            .prepare_p05_child_inspection(&self.inner, sink_count)?;
        Ok(P05PreparedChild {
            counted: self.counted.clone(),
            inner: self.inner.clone(),
            outcome,
        })
    }
}
/// Existing real contextual owner. No JSON/DB/clock factory and no completion
/// boolean. The bin's original advance body owns reconciliation/hydration.
pub struct P05PreparedChild {
    counted: Arc<DurableDeliveryCoordinator>,
    inner: crate::durable_delivery::P05ChildInspection,
    outcome: crate::durable_delivery::PrepareOutcome,
}
impl P05PreparedChild {
    pub fn envelope(&self) -> &crate::durable_delivery::DeliveryEnvelope {
        self.inner.envelope()
    }
    pub fn preparation(&self) -> &crate::durable_delivery::PrepareOutcome {
        &self.outcome
    }
    pub fn require_counted_owner(&self, counted: &Arc<DurableDeliveryCoordinator>) -> Result<()> {
        require_same_counted(&self.counted, counted)
    }
    /// Only original resume. A new physical opening rechecks the actual reader;
    /// Accepted/noop and raw/audit recovery need no operational availability.
    pub fn resume(
        &self,
        sinks: &[AuthoritativeSink],
    ) -> Result<crate::durable_delivery::ResumeOutcome> {
        self.counted.resume_p05_child_inspection(&self.inner, sinks)
    }
}
