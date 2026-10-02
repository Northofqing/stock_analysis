//! Recoverable observed P05 Unit façade. Observation is not QualifiedFacts,
//! coverage or a physical receipt. Main supplies its existing qualified batch
//! and renderer after its original admission checks; this module adds no source
//! qualification. A restored Unit does not invoke a provider or rebuild bytes.
use crate::database::DatabaseManager;
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
    prediction: &'static DatabaseManager,
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
fn actual_prediction() -> Result<&'static DatabaseManager> {
    DatabaseManager::try_get().ok_or_else(|| {
        DurableDeliveryError::PolicyMismatch("P05 actual operational singleton unavailable".into())
    })
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
        let identity = counted.initialize_prospective_p05_family(actual_prediction()?)?;
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
        let prediction = actual_prediction()?;
        let Some(draft) = counted.read_p05_unit_draft(business_date)? else {
            return Ok(None);
        };
        let intent = counted.finish_p05_unit_preparation_global(&draft).await?;
        Ok(Some(Self {
            counted,
            prediction,
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
        let prediction = actual_prediction()?;
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
            prediction,
        })
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
        self.check_intent()?;
        self.counted
            .dispatch_p05_unit_child_on(self.prediction, &self.date, index, sinks, append)
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
        self.counted
            .validate_p05_unit_prediction_on(self.prediction, &self.date)?;
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
