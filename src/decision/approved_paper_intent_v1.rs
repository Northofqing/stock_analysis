//! The persisted intent is a closed observation. Only an actual namespace
//! issuer can construct the distinct, non-serializable approval capability.
//! Production approval/source contracts are not delivered and remain blocked.

use crate::data_gateway::{
    QualifiedFact, QualifiedListingStatus, QualifiedSuspensionStatus, QualifiedTradingFacts,
    SecurityBoard,
};
use crate::database::DatabaseConnectionAuthority;
use crate::market_domain::{AssetClass, Exchange};
use crate::trading::paper_book_v2_budget_v1::token;
use crate::trading::paper_book_v2_execution::ActualExecutionBinding;
use crate::trading::paper_book_v2_fill_model::{Side, WindowRecord, MODEL_VERSION};
use crate::trading::paper_ledger::LedgerError;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum TimeInForce {
    DaySession,
}

pub(crate) const INTENT_VERSION: &str = "approved-paper-parent-intent/v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IntentRecord {
    pub(crate) version: String,
    pub(crate) account_id: String,
    pub(crate) epoch_id: String,
    pub(crate) execution_manifest_hash: String,
    pub(crate) parent_id: String,
    pub(crate) investment_decision_id: String,
    pub(crate) family_id: String,
    pub(crate) chain_id: String,
    pub(crate) instrument_code: String,
    pub(crate) instrument_name: String,
    pub(crate) side: Side,
    pub(crate) quantity: u32,
    pub(crate) limit_micro_cny: i64,
    pub(crate) fee_price_cap_micro_cny: i64,
    pub(crate) session_date: NaiveDate,
    pub(crate) time_in_force: TimeInForce,
    pub(crate) approved_at: DateTime<Utc>,
    pub(crate) approval_reference: String,
    pub(crate) source_window: WindowRecord,
}
impl IntentRecord {
    pub(crate) fn validate(&self) -> Result<(), LedgerError> {
        self.source_window.validate()?;
        if self.version != INTENT_VERSION
            || [
                &self.account_id,
                &self.epoch_id,
                &self.parent_id,
                &self.investment_decision_id,
                &self.family_id,
                &self.chain_id,
                &self.instrument_code,
                &self.instrument_name,
                &self.approval_reference,
            ]
            .iter()
            .any(|v| !token(v))
            || self.execution_manifest_hash.len() != 64
            || !self
                .execution_manifest_hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.quantity == 0
            || self.quantity % 100 != 0
            || self.instrument_code != self.source_window.instrument_code
            || self.session_date != self.source_window.session_date
            || self.approved_at != self.source_window.observed_at
            || self.limit_micro_cny < self.source_window.lower_micro_cny
            || self.fee_price_cap_micro_cny < self.limit_micro_cny
            || self.fee_price_cap_micro_cny > self.source_window.upper_micro_cny
            || self.limit_micro_cny % self.source_window.tick_micro_cny != 0
            || self.fee_price_cap_micro_cny % self.source_window.tick_micro_cny != 0
        {
            return Err(LedgerError::InvalidInput(
                "closed parent intent binding invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct ApprovedPaperIntentV1 {
    record: IntentRecord,
    namespace: DatabaseConnectionAuthority,
}
impl ApprovedPaperIntentV1 {
    pub(crate) fn record(&self) -> &IntentRecord {
        &self.record
    }
    pub(crate) fn require_binding(
        &self,
        actual: &ActualExecutionBinding,
    ) -> Result<(), LedgerError> {
        actual.require_record_binding(&self.record, &self.namespace)
    }
}

#[derive(Debug)]
pub(crate) struct QualifiedPaperExecutionWindowV1 {
    record: WindowRecord,
    namespace: DatabaseConnectionAuthority,
}
impl QualifiedPaperExecutionWindowV1 {
    pub(crate) fn record(&self) -> &WindowRecord {
        &self.record
    }
    pub(crate) fn require_binding(
        &self,
        actual: &ActualExecutionBinding,
    ) -> Result<(), LedgerError> {
        actual.require_window_namespace(&self.namespace)
    }
}

/// No user amount, default risk constant, stored record or gateway observation
/// is promoted into an approval by this production boundary.
pub(crate) fn require_production_approval() -> Result<ApprovedPaperIntentV1, LedgerError> {
    Err(LedgerError::EvidenceUnavailable(
        "production paper intent and explicit budget approval unavailable".into(),
    ))
}

#[cfg(test)]
pub(crate) struct TestIntentRequest {
    pub(crate) parent_id: String,
    pub(crate) investment_decision_id: String,
    pub(crate) chain_id: String,
    pub(crate) name: String,
    pub(crate) side: Side,
    pub(crate) quantity: u32,
    pub(crate) limit_micro_cny: i64,
    pub(crate) fee_price_cap_micro_cny: i64,
}

/// Sole positive issuer in this working slice. The actual binding is obtained
/// after full Global CatalogV6 and full financial history validation; it is
/// not constructible from an account/path/hash or a caller declared bool.
#[cfg(test)]
pub(crate) fn issue_intent_for_isolated_test(
    actual: &ActualExecutionBinding,
    facts: &QualifiedTradingFacts,
    window: QualifiedPaperExecutionWindowV1,
    request: TestIntentRequest,
) -> Result<ApprovedPaperIntentV1, LedgerError> {
    actual.require_test_issuer()?;
    window.require_binding(actual)?;
    require_qualified_facts(facts, &window.record)?;
    actual.require_fee_segment(&window.record.fee_segment)?;
    let record = IntentRecord {
        version: INTENT_VERSION.into(),
        account_id: actual.account_id().into(),
        epoch_id: actual.epoch_id().into(),
        execution_manifest_hash: actual.manifest_hash().into(),
        parent_id: request.parent_id,
        investment_decision_id: request.investment_decision_id,
        family_id: actual.family_id().into(),
        chain_id: request.chain_id,
        instrument_code: window.record.instrument_code.clone(),
        instrument_name: request.name,
        side: request.side,
        quantity: request.quantity,
        limit_micro_cny: request.limit_micro_cny,
        fee_price_cap_micro_cny: request.fee_price_cap_micro_cny,
        session_date: window.record.session_date,
        time_in_force: TimeInForce::DaySession,
        approved_at: window.record.observed_at,
        approval_reference: "TEST_CODE_SYNTHETIC_APPROVAL_NOT_PRODUCTION".into(),
        source_window: window.record,
    };
    record.validate()?;
    Ok(ApprovedPaperIntentV1 {
        record,
        namespace: actual.database_authority().clone(),
    })
}

#[cfg(test)]
pub(crate) struct TestWindowObservation {
    pub(crate) observation_id: String,
    pub(crate) source_reference: String,
    pub(crate) source_at: DateTime<Utc>,
    pub(crate) observed_at: DateTime<Utc>,
    pub(crate) fresh_through: DateTime<Utc>,
    pub(crate) price_micro_cny: i64,
    pub(crate) modeled_available_quantity: u32,
}

#[cfg(test)]
pub(crate) fn issue_window_for_isolated_test(
    actual: &ActualExecutionBinding,
    facts: &QualifiedTradingFacts,
    observed: TestWindowObservation,
) -> Result<QualifiedPaperExecutionWindowV1, LedgerError> {
    actual.require_test_issuer()?;
    let regime = facts.price_regime().require().map_err(|_| unavailable())?;
    let record = WindowRecord {
        version: MODEL_VERSION.into(),
        observation_id: observed.observation_id,
        instrument_code: facts.request().instrument().code().into(),
        session_date: facts.request().effective_on(),
        source_at: observed.source_at,
        observed_at: observed.observed_at,
        fresh_through: observed.fresh_through,
        source_reference: observed.source_reference,
        facts_contract: facts.contract_version().into(),
        facts_batch_id: facts.evidence().ok_or_else(unavailable)?.batch_id.clone(),
        facts_source: facts.evidence().ok_or_else(unavailable)?.source.clone(),
        facts_source_at: facts
            .evidence()
            .ok_or_else(unavailable)?
            .source_at
            .clone()
            .ok_or_else(unavailable)?,
        facts_observed_at: facts
            .evidence()
            .ok_or_else(unavailable)?
            .observed_at
            .clone(),
        fee_segment: match regime.board() {
            SecurityBoard::Main => "ShanghaiMainA",
            SecurityBoard::Star => "ShanghaiStarA",
            _ => return Err(unavailable()),
        }
        .into(),
        listed: matches!(
            facts.lifecycle(),
            QualifiedFact::Available(QualifiedListingStatus::Listed)
        ),
        suspended: matches!(
            facts.suspension(),
            QualifiedFact::Available(QualifiedSuspensionStatus::Suspended { .. })
        ),
        tick_micro_cny: regime.tick_micros(),
        lower_micro_cny: regime.lower_price_micros(),
        upper_micro_cny: regime.upper_price_micros(),
        regime_version: regime.version().into(),
        price_micro_cny: observed.price_micro_cny,
        modeled_available_quantity: observed.modeled_available_quantity,
    };
    require_qualified_facts(facts, &record)?;
    record.validate()?;
    Ok(QualifiedPaperExecutionWindowV1 {
        record,
        namespace: actual.database_authority().clone(),
    })
}

fn unavailable() -> LedgerError {
    LedgerError::EvidenceUnavailable("qualified execution facts unavailable".into())
}
fn require_qualified_facts(
    facts: &QualifiedTradingFacts,
    window: &WindowRecord,
) -> Result<(), LedgerError> {
    let instrument = facts.request().instrument();
    let regime = facts.price_regime().require().map_err(|_| unavailable())?;
    if instrument.exchange() != Exchange::Shanghai
        || instrument.asset_class() != AssetClass::Equity
        || !matches!(regime.board(), SecurityBoard::Main | SecurityBoard::Star)
        || instrument.code() != window.instrument_code
        || facts.request().effective_on() != window.session_date
        || facts.lifecycle().require().map_err(|_| unavailable())?
            != &QualifiedListingStatus::Listed
        || facts.suspension().require().is_err()
        || facts.evidence().is_none()
        || facts.contract_version() != window.facts_contract
        || facts.evidence().is_some_and(|e| {
            e.batch_id != window.facts_batch_id
                || e.source != window.facts_source
                || e.source_at.as_deref() != Some(window.facts_source_at.as_str())
                || e.observed_at != window.facts_observed_at
        })
        || regime.tick_micros() != window.tick_micro_cny
        || regime.lower_price_micros() != window.lower_micro_cny
        || regime.upper_price_micros() != window.upper_micro_cny
        || regime.version() != window.regime_version
    {
        return Err(unavailable());
    }
    Ok(())
}
