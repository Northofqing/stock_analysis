//! Qualified security trading facts for one instrument and effective date.
//!
//! No code/name heuristic is accepted at this boundary. A future authority
//! adapter must provide explicit lifecycle, price-band and suspension coverage
//! for the requested instrument/date. Until that contract is delivered, the
//! production gateway returns field-level `ContractNotDelivered` states.

use crate::market_domain::InstrumentId;

use chrono::NaiveDate;
use thiserror::Error;

use super::{BatchEvidence, SecurityBoard};

pub const QUALIFIED_TRADING_FACTS_CONTRACT_V1: &str =
    "qualified_trading_facts_contract_unavailable_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingFactField {
    Lifecycle,
    PriceRegime,
    Suspension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingFactUnavailableReason {
    ContractNotDelivered,
    FactMissing,
    CoverageInsufficient,
    Stale,
    Conflict,
    IdentityMismatch,
}

impl TradingFactUnavailableReason {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::ContractNotDelivered => "trading_fact_contract_not_delivered",
            Self::FactMissing => "trading_fact_missing",
            Self::CoverageInsufficient => "trading_fact_coverage_insufficient",
            Self::Stale => "trading_fact_stale",
            Self::Conflict => "trading_fact_conflict",
            Self::IdentityMismatch => "trading_fact_identity_mismatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradingFactUnavailable {
    field: TradingFactField,
    reason: TradingFactUnavailableReason,
    message: String,
}

impl TradingFactUnavailable {
    fn new(
        field: TradingFactField,
        reason: TradingFactUnavailableReason,
        message: impl Into<String>,
    ) -> Self {
        Self {
            field,
            reason,
            message: message.into(),
        }
    }

    pub const fn field(&self) -> TradingFactField {
        self.field
    }

    pub const fn reason(&self) -> TradingFactUnavailableReason {
        self.reason
    }

    pub const fn reason_code(&self) -> &'static str {
        self.reason.reason_code()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedFact<T> {
    Available(T),
    Unavailable(TradingFactUnavailable),
}

impl<T> QualifiedFact<T> {
    pub fn require(&self) -> Result<&T, &TradingFactUnavailable> {
        match self {
            Self::Available(value) => Ok(value),
            Self::Unavailable(error) => Err(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedTradingFactsRequest {
    instrument: InstrumentId,
    effective_on: NaiveDate,
}

impl QualifiedTradingFactsRequest {
    pub fn new(instrument: InstrumentId, effective_on: NaiveDate) -> Self {
        Self {
            instrument,
            effective_on,
        }
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub const fn effective_on(&self) -> NaiveDate {
        self.effective_on
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualifiedListingStatus {
    PreListing,
    Listed,
    Delisted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityLifecycle {
    pub listed_on: NaiveDate,
    /// First date on which the instrument is no longer listed.
    pub delisted_on: Option<NaiveDate>,
    /// Last effective date for which the source proves the absence/presence of
    /// a delisting transition.
    pub covered_through: NaiveDate,
}

impl AuthorityLifecycle {
    fn status_on(&self, effective_on: NaiveDate) -> Result<QualifiedListingStatus, String> {
        if self.covered_through < self.listed_on {
            return Err("lifecycle coverage ends before listing date".to_owned());
        }
        if self
            .delisted_on
            .is_some_and(|delisted_on| delisted_on <= self.listed_on)
        {
            return Err("delisting date must be after listing date".to_owned());
        }
        if effective_on > self.covered_through {
            return Err(format!(
                "lifecycle coverage ends at {}, requested {effective_on}",
                self.covered_through
            ));
        }
        Ok(if effective_on < self.listed_on {
            QualifiedListingStatus::PreListing
        } else if self
            .delisted_on
            .is_some_and(|delisted_on| effective_on >= delisted_on)
        {
            QualifiedListingStatus::Delisted
        } else {
            QualifiedListingStatus::Listed
        })
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum QualifiedPriceError {
    #[error("price {price_micros} is outside [{lower_micros}, {upper_micros}]")]
    OutsideBand {
        price_micros: i64,
        lower_micros: i64,
        upper_micros: i64,
    },
    #[error("price {price_micros} is not on tick grid {tick_micros}")]
    OffTickGrid { price_micros: i64, tick_micros: i64 },
}

/// Exact authority-published daily band in micro-CNY. The authority adapter,
/// not this repository, owns exchange rounding and regime percentages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedPriceBand {
    board: SecurityBoard,
    is_st: bool,
    tick_micros: i64,
    lower_price_micros: i64,
    upper_price_micros: i64,
    effective_from: NaiveDate,
    effective_through: NaiveDate,
    version: String,
}

impl QualifiedPriceBand {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        board: SecurityBoard,
        is_st: bool,
        tick_micros: i64,
        lower_price_micros: i64,
        upper_price_micros: i64,
        effective_from: NaiveDate,
        effective_through: NaiveDate,
        version: impl Into<String>,
    ) -> Result<Self, QualifiedTradingFactsError> {
        let version = version.into();
        if tick_micros <= 0
            || lower_price_micros <= 0
            || upper_price_micros < lower_price_micros
            || lower_price_micros % tick_micros != 0
            || upper_price_micros % tick_micros != 0
            || effective_from > effective_through
            || version.trim().is_empty()
        {
            return Err(QualifiedTradingFactsError::InvalidPriceRegime);
        }
        Ok(Self {
            board,
            is_st,
            tick_micros,
            lower_price_micros,
            upper_price_micros,
            effective_from,
            effective_through,
            version,
        })
    }

    pub const fn board(&self) -> SecurityBoard {
        self.board
    }

    pub const fn is_st(&self) -> bool {
        self.is_st
    }

    pub const fn tick_micros(&self) -> i64 {
        self.tick_micros
    }

    pub const fn upper_price_micros(&self) -> i64 {
        self.upper_price_micros
    }

    pub const fn lower_price_micros(&self) -> i64 {
        self.lower_price_micros
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    fn covers(&self, effective_on: NaiveDate) -> bool {
        effective_on >= self.effective_from && effective_on <= self.effective_through
    }

    pub fn validate_price_micros(&self, price_micros: i64) -> Result<(), QualifiedPriceError> {
        if price_micros < self.lower_price_micros || price_micros > self.upper_price_micros {
            return Err(QualifiedPriceError::OutsideBand {
                price_micros,
                lower_micros: self.lower_price_micros,
                upper_micros: self.upper_price_micros,
            });
        }
        if price_micros % self.tick_micros != 0 {
            return Err(QualifiedPriceError::OffTickGrid {
                price_micros,
                tick_micros: self.tick_micros,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuspensionWindow {
    pub halted_from: NaiveDate,
    pub halted_through: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritySuspensionCoverage {
    pub covered_from: NaiveDate,
    pub covered_through: NaiveDate,
    pub windows: Vec<SuspensionWindow>,
}

impl AuthoritySuspensionCoverage {
    pub fn trading(covered_from: NaiveDate, covered_through: NaiveDate) -> Self {
        Self {
            covered_from,
            covered_through,
            windows: Vec::new(),
        }
    }

    pub fn with_windows(
        covered_from: NaiveDate,
        covered_through: NaiveDate,
        windows: Vec<SuspensionWindow>,
    ) -> Self {
        Self {
            covered_from,
            covered_through,
            windows,
        }
    }

    fn status_on(&self, effective_on: NaiveDate) -> Result<QualifiedSuspensionStatus, String> {
        if self.covered_from > self.covered_through {
            return Err("suspension coverage range is reversed".to_owned());
        }
        if effective_on < self.covered_from || effective_on > self.covered_through {
            return Err(format!(
                "suspension coverage {}..={} does not include {effective_on}",
                self.covered_from, self.covered_through
            ));
        }
        let mut previous_through = None;
        for window in &self.windows {
            if window.halted_from > window.halted_through
                || window.halted_from < self.covered_from
                || window.halted_through > self.covered_through
                || previous_through.is_some_and(|through| window.halted_from <= through)
            {
                return Err("suspension windows conflict with coverage or overlap".to_owned());
            }
            previous_through = Some(window.halted_through);
        }
        if let Some(window) = self.windows.iter().find(|window| {
            effective_on >= window.halted_from && effective_on <= window.halted_through
        }) {
            return Ok(QualifiedSuspensionStatus::Suspended {
                halted_from: window.halted_from,
                halted_through: window.halted_through,
            });
        }
        Ok(QualifiedSuspensionStatus::Trading)
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("qualified suspension evidence rejected reason_code={reason_code}: {message}")]
pub struct QualifiedSuspensionEvidenceError {
    reason_code: &'static str,
    message: String,
}

impl QualifiedSuspensionEvidenceError {
    fn new(reason_code: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason_code,
            message: message.into(),
        }
    }

    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

/// Exact authority coverage used to explain missing daily rows. Windows and
/// coverage are inclusive; a reopen date must be covered and evaluate to
/// `Trading`, never remain inside the final halt interval.
#[derive(Debug, Clone)]
pub struct QualifiedSuspensionEvidence {
    instrument: InstrumentId,
    coverage: AuthoritySuspensionCoverage,
    evidence: BatchEvidence,
    contract_version: String,
    authority_event_id: String,
    artifact_sha256: String,
}

impl QualifiedSuspensionEvidence {
    pub(crate) fn admit(
        instrument: InstrumentId,
        coverage: AuthoritySuspensionCoverage,
        evidence: BatchEvidence,
        contract_version: impl Into<String>,
        authority_event_id: impl Into<String>,
        artifact_sha256: impl Into<String>,
    ) -> Result<Self, QualifiedSuspensionEvidenceError> {
        let contract_version = contract_version.into();
        let authority_event_id = authority_event_id.into();
        let artifact_sha256 = artifact_sha256.into();
        let source_at = evidence.source_at.as_deref().unwrap_or_default();
        if contract_version.trim().is_empty()
            || authority_event_id.trim().is_empty()
            || artifact_sha256.len() != 64
            || !artifact_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || evidence.source.trim().is_empty()
            || evidence.batch_id.trim().is_empty()
            || source_at.trim().is_empty()
        {
            return Err(QualifiedSuspensionEvidenceError::new(
                "suspension_evidence_conflict",
                "suspension evidence contract/event/artifact/source/batch/source_at is incomplete",
            ));
        }
        let source_at = chrono::DateTime::parse_from_rfc3339(source_at).map_err(|error| {
            QualifiedSuspensionEvidenceError::new(
                "suspension_evidence_conflict",
                format!("suspension evidence source_at is invalid: {error}"),
            )
        })?;
        let observed_at =
            chrono::DateTime::parse_from_rfc3339(&evidence.observed_at).map_err(|error| {
                QualifiedSuspensionEvidenceError::new(
                    "suspension_evidence_conflict",
                    format!("suspension evidence observed_at is invalid: {error}"),
                )
            })?;
        if observed_at < source_at {
            return Err(QualifiedSuspensionEvidenceError::new(
                "suspension_evidence_conflict",
                "suspension evidence observed_at precedes source_at",
            ));
        }
        // Validate the complete ordered window set once, not only the first
        // requested date that happens to hit it.
        let mut previous_through = None;
        if coverage.covered_from > coverage.covered_through {
            return Err(QualifiedSuspensionEvidenceError::new(
                "suspension_evidence_conflict",
                "suspension coverage range is reversed",
            ));
        }
        for window in &coverage.windows {
            if window.halted_from > window.halted_through
                || window.halted_from < coverage.covered_from
                || window.halted_through > coverage.covered_through
                || previous_through.is_some_and(|through| window.halted_from <= through)
            {
                return Err(QualifiedSuspensionEvidenceError::new(
                    "suspension_evidence_conflict",
                    "suspension windows overlap or fall outside coverage",
                ));
            }
            previous_through = Some(window.halted_through);
        }
        Ok(Self {
            instrument,
            coverage,
            evidence,
            contract_version,
            authority_event_id,
            artifact_sha256,
        })
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub const fn evidence(&self) -> &BatchEvidence {
        &self.evidence
    }

    pub fn contract_version(&self) -> &str {
        &self.contract_version
    }

    pub fn authority_event_id(&self) -> &str {
        &self.authority_event_id
    }

    pub fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }

    pub fn explain_gap(
        &self,
        code: &str,
        missing_trading_dates: &[NaiveDate],
        reopen_on: NaiveDate,
    ) -> Result<(), QualifiedSuspensionEvidenceError> {
        if self.instrument.code() != code {
            return Err(QualifiedSuspensionEvidenceError::new(
                "suspension_evidence_identity_mismatch",
                format!(
                    "requested code {code:?} differs from evidence instrument {:?}",
                    self.instrument.code()
                ),
            ));
        }
        if missing_trading_dates.is_empty() {
            return Err(QualifiedSuspensionEvidenceError::new(
                "suspension_evidence_invalid_request",
                "gap explanation requires at least one missing trading date",
            ));
        }
        for date in missing_trading_dates {
            match self.coverage.status_on(*date) {
                Ok(QualifiedSuspensionStatus::Suspended { .. }) => {}
                Ok(QualifiedSuspensionStatus::Trading) => {
                    return Err(QualifiedSuspensionEvidenceError::new(
                        "suspension_evidence_partial_coverage",
                        format!("authority marks missing trading date {date} as trading"),
                    ));
                }
                Err(message) => {
                    return Err(QualifiedSuspensionEvidenceError::new(
                        "suspension_evidence_coverage_insufficient",
                        message,
                    ));
                }
            }
        }
        match self.coverage.status_on(reopen_on) {
            Ok(QualifiedSuspensionStatus::Trading) => Ok(()),
            Ok(QualifiedSuspensionStatus::Suspended { .. }) => {
                Err(QualifiedSuspensionEvidenceError::new(
                    "suspension_reopen_date_still_halted",
                    format!("reopen date {reopen_on} is still inside a halt interval"),
                ))
            }
            Err(message) => Err(QualifiedSuspensionEvidenceError::new(
                "suspension_evidence_coverage_insufficient",
                message,
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualifiedSuspensionStatus {
    Trading,
    Suspended {
        halted_from: NaiveDate,
        halted_through: NaiveDate,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct AuthorityTradingFactsRecord {
    pub instrument: InstrumentId,
    pub lifecycle: Option<AuthorityLifecycle>,
    pub price_regime: Option<QualifiedPriceBand>,
    pub suspension: Option<AuthoritySuspensionCoverage>,
    pub evidence: BatchEvidence,
    pub contract_version: String,
    pub fresh_through: NaiveDate,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum QualifiedTradingFactsError {
    #[error("qualified price regime is structurally invalid")]
    InvalidPriceRegime,
}

#[derive(Debug, Clone)]
pub struct QualifiedTradingFacts {
    request: QualifiedTradingFactsRequest,
    lifecycle: QualifiedFact<QualifiedListingStatus>,
    price_regime: QualifiedFact<QualifiedPriceBand>,
    suspension: QualifiedFact<QualifiedSuspensionStatus>,
    evidence: Option<BatchEvidence>,
    contract_version: String,
}

impl QualifiedTradingFacts {
    pub(crate) fn admit(
        request: QualifiedTradingFactsRequest,
        record: AuthorityTradingFactsRecord,
    ) -> Result<Self, QualifiedTradingFactsError> {
        if request.instrument != record.instrument {
            return Ok(Self::all_unavailable(
                request,
                TradingFactUnavailableReason::IdentityMismatch,
                "authority trading facts instrument does not match request",
                Some(record.evidence),
                record.contract_version,
            ));
        }
        let source_at = record.evidence.source_at.as_deref().unwrap_or_default();
        let evidence_times_valid = chrono::DateTime::parse_from_rfc3339(source_at)
            .ok()
            .zip(chrono::DateTime::parse_from_rfc3339(&record.evidence.observed_at).ok())
            .is_some_and(|(source_at, observed_at)| source_at <= observed_at);
        if record.contract_version.trim().is_empty()
            || record.evidence.source.trim().is_empty()
            || record.evidence.batch_id.trim().is_empty()
            || source_at.trim().is_empty()
            || !evidence_times_valid
        {
            return Ok(Self::all_unavailable(
                request,
                TradingFactUnavailableReason::Conflict,
                "authority trading facts contract/evidence is incomplete",
                Some(record.evidence),
                record.contract_version,
            ));
        }
        if request.effective_on > record.fresh_through {
            return Ok(Self::all_unavailable(
                request,
                TradingFactUnavailableReason::Stale,
                "authority trading facts freshness does not cover requested effective date",
                Some(record.evidence),
                record.contract_version,
            ));
        }
        let effective_on = request.effective_on;
        let lifecycle = match record.lifecycle {
            Some(lifecycle) => match lifecycle.status_on(effective_on) {
                Ok(status) => QualifiedFact::Available(status),
                Err(message) if effective_on > lifecycle.covered_through => {
                    QualifiedFact::Unavailable(TradingFactUnavailable::new(
                        TradingFactField::Lifecycle,
                        TradingFactUnavailableReason::CoverageInsufficient,
                        message,
                    ))
                }
                Err(message) => QualifiedFact::Unavailable(TradingFactUnavailable::new(
                    TradingFactField::Lifecycle,
                    TradingFactUnavailableReason::Conflict,
                    message,
                )),
            },
            None => QualifiedFact::Unavailable(TradingFactUnavailable::new(
                TradingFactField::Lifecycle,
                TradingFactUnavailableReason::FactMissing,
                "authority record omitted lifecycle fact",
            )),
        };
        let price_regime = match record.price_regime {
            Some(regime) if regime.covers(effective_on) => QualifiedFact::Available(regime),
            Some(regime) => QualifiedFact::Unavailable(TradingFactUnavailable::new(
                TradingFactField::PriceRegime,
                TradingFactUnavailableReason::CoverageInsufficient,
                format!(
                    "price regime {} does not cover {effective_on}",
                    regime.version()
                ),
            )),
            None => QualifiedFact::Unavailable(TradingFactUnavailable::new(
                TradingFactField::PriceRegime,
                TradingFactUnavailableReason::FactMissing,
                "authority record omitted price regime",
            )),
        };
        let suspension = match record.suspension {
            Some(coverage) => match coverage.status_on(effective_on) {
                Ok(status) => QualifiedFact::Available(status),
                Err(message)
                    if effective_on < coverage.covered_from
                        || effective_on > coverage.covered_through =>
                {
                    QualifiedFact::Unavailable(TradingFactUnavailable::new(
                        TradingFactField::Suspension,
                        TradingFactUnavailableReason::CoverageInsufficient,
                        message,
                    ))
                }
                Err(message) => QualifiedFact::Unavailable(TradingFactUnavailable::new(
                    TradingFactField::Suspension,
                    TradingFactUnavailableReason::Conflict,
                    message,
                )),
            },
            None => QualifiedFact::Unavailable(TradingFactUnavailable::new(
                TradingFactField::Suspension,
                TradingFactUnavailableReason::FactMissing,
                "authority record omitted suspension coverage",
            )),
        };
        Ok(Self {
            request,
            lifecycle,
            price_regime,
            suspension,
            evidence: Some(record.evidence),
            contract_version: record.contract_version,
        })
    }

    fn all_unavailable(
        request: QualifiedTradingFactsRequest,
        reason: TradingFactUnavailableReason,
        message: &str,
        evidence: Option<BatchEvidence>,
        contract_version: String,
    ) -> Self {
        Self {
            request,
            lifecycle: QualifiedFact::Unavailable(TradingFactUnavailable::new(
                TradingFactField::Lifecycle,
                reason,
                message,
            )),
            price_regime: QualifiedFact::Unavailable(TradingFactUnavailable::new(
                TradingFactField::PriceRegime,
                reason,
                message,
            )),
            suspension: QualifiedFact::Unavailable(TradingFactUnavailable::new(
                TradingFactField::Suspension,
                reason,
                message,
            )),
            evidence,
            contract_version,
        }
    }

    pub fn request(&self) -> &QualifiedTradingFactsRequest {
        &self.request
    }

    pub const fn lifecycle(&self) -> &QualifiedFact<QualifiedListingStatus> {
        &self.lifecycle
    }

    pub const fn price_regime(&self) -> &QualifiedFact<QualifiedPriceBand> {
        &self.price_regime
    }

    pub const fn suspension(&self) -> &QualifiedFact<QualifiedSuspensionStatus> {
        &self.suspension
    }

    pub const fn evidence(&self) -> Option<&BatchEvidence> {
        self.evidence.as_ref()
    }

    pub fn contract_version(&self) -> &str {
        &self.contract_version
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct QualifiedTradingFactsGateway;

impl QualifiedTradingFactsGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Production remains explicitly unavailable until an admitted authority
    /// contract supplies lifecycle, band/tick and suspension coverage.
    pub fn acquire(&self, request: QualifiedTradingFactsRequest) -> QualifiedTradingFacts {
        QualifiedTradingFacts::all_unavailable(
            request,
            TradingFactUnavailableReason::ContractNotDelivered,
            "qualified trading facts authority contract is not delivered",
            None,
            QUALIFIED_TRADING_FACTS_CONTRACT_V1.to_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::{BatchEvidence, SecurityBoard};
    use crate::market_domain::{AssetClass, Exchange, InstrumentId, ProviderId};
    use chrono::NaiveDate;

    fn task9_record(
        instrument: InstrumentId,
        covered_from: NaiveDate,
        covered_through: NaiveDate,
    ) -> AuthorityTradingFactsRecord {
        AuthorityTradingFactsRecord {
            instrument,
            lifecycle: Some(AuthorityLifecycle {
                listed_on: NaiveDate::from_ymd_opt(2020, 1, 2).unwrap(),
                delisted_on: Some(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()),
                covered_through,
            }),
            price_regime: Some(
                QualifiedPriceBand::new(
                    SecurityBoard::Main,
                    false,
                    10_000,
                    9_000_000,
                    11_000_000,
                    covered_from,
                    covered_through,
                    "TEST_CODE_EXPLICIT_REGIME_V1",
                )
                .unwrap(),
            ),
            suspension: Some(AuthoritySuspensionCoverage::trading(
                covered_from,
                covered_through,
            )),
            evidence: BatchEvidence {
                provider: ProviderId::Custom,
                source: "TEST_CODE_EXCHANGE_FACTS".to_owned(),
                source_at: Some("2026-09-25T00:00:00Z".to_owned()),
                observed_at: "2026-09-25T00:00:01Z".to_owned(),
                batch_id: "TEST_CODE_TRADING_FACTS".to_owned(),
            },
            contract_version: "TEST_CODE_TRADING_FACTS_V1".to_owned(),
            fresh_through: covered_through,
        }
    }

    #[test]
    fn task9_explicit_board_and_st_regimes_drive_price_band_without_name_or_prefix_inference() {
        let effective_on = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        let cases = [
            ("600001", SecurityBoard::Main, false, 11_000_000_i64),
            ("600002", SecurityBoard::Main, true, 10_500_000_i64),
            ("300001", SecurityBoard::ChiNext, false, 12_000_000_i64),
            ("688001", SecurityBoard::Star, false, 12_000_000_i64),
            ("920001", SecurityBoard::Beijing, false, 13_000_000_i64),
        ];

        for (code, board, is_st, upper_price_micros) in cases {
            let instrument = InstrumentId::new(
                if board == SecurityBoard::Beijing {
                    Exchange::Beijing
                } else if board == SecurityBoard::ChiNext {
                    Exchange::Shenzhen
                } else {
                    Exchange::Shanghai
                },
                code,
                AssetClass::Equity,
            )
            .unwrap();
            let request = QualifiedTradingFactsRequest::new(instrument.clone(), effective_on);
            let record = AuthorityTradingFactsRecord {
                instrument,
                lifecycle: Some(AuthorityLifecycle {
                    listed_on: NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
                    delisted_on: None,
                    covered_through: effective_on,
                }),
                price_regime: Some(
                    QualifiedPriceBand::new(
                        board,
                        is_st,
                        10_000,
                        7_000_000,
                        upper_price_micros,
                        effective_on,
                        effective_on,
                        "TEST_CODE_EXPLICIT_REGIME_V1",
                    )
                    .unwrap(),
                ),
                suspension: Some(AuthoritySuspensionCoverage::trading(
                    effective_on,
                    effective_on,
                )),
                evidence: BatchEvidence {
                    provider: ProviderId::Custom,
                    source: "TEST_CODE_EXCHANGE_FACTS".to_owned(),
                    source_at: Some("2026-09-25T00:00:00Z".to_owned()),
                    observed_at: "2026-09-25T00:00:01Z".to_owned(),
                    batch_id: format!("TEST_CODE_{code}_FACTS"),
                },
                contract_version: "TEST_CODE_TRADING_FACTS_V1".to_owned(),
                fresh_through: effective_on,
            };

            let facts = QualifiedTradingFacts::admit(request, record).unwrap();
            assert_eq!(
                facts.lifecycle().require().unwrap(),
                &QualifiedListingStatus::Listed
            );
            let regime = facts.price_regime().require().unwrap();
            assert_eq!(regime.board(), board);
            assert_eq!(regime.is_st(), is_st);
            assert!(regime.validate_price_micros(upper_price_micros).is_ok());
            assert!(regime
                .validate_price_micros(upper_price_micros - regime.tick_micros())
                .is_ok());
            assert!(matches!(
                regime.validate_price_micros(upper_price_micros - regime.tick_micros() / 2),
                Err(QualifiedPriceError::OffTickGrid { .. })
            ));
            assert!(regime
                .validate_price_micros(upper_price_micros + regime.tick_micros())
                .is_err());
        }
    }

    #[test]
    fn task9_lifecycle_effective_dates_and_field_level_unavailable_reasons_are_explicit() {
        let instrument =
            InstrumentId::new(Exchange::Shanghai, "600001", AssetClass::Equity).unwrap();
        let from = NaiveDate::from_ymd_opt(2019, 12, 1).unwrap();
        let through = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        for (date, expected) in [
            (
                NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
                QualifiedListingStatus::PreListing,
            ),
            (
                NaiveDate::from_ymd_opt(2020, 1, 2).unwrap(),
                QualifiedListingStatus::Listed,
            ),
            (
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                QualifiedListingStatus::Delisted,
            ),
        ] {
            let facts = QualifiedTradingFacts::admit(
                QualifiedTradingFactsRequest::new(instrument.clone(), date),
                task9_record(instrument.clone(), from, through),
            )
            .unwrap();
            assert_eq!(facts.lifecycle().require().unwrap(), &expected);
        }

        let unavailable =
            QualifiedTradingFactsGateway::new().acquire(QualifiedTradingFactsRequest::new(
                instrument.clone(),
                NaiveDate::from_ymd_opt(2026, 9, 25).unwrap(),
            ));
        for error in [
            unavailable.lifecycle().require().unwrap_err(),
            unavailable.price_regime().require().unwrap_err(),
            unavailable.suspension().require().unwrap_err(),
        ] {
            assert_eq!(
                error.reason(),
                TradingFactUnavailableReason::ContractNotDelivered
            );
        }

        let date = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        let mut missing = task9_record(instrument.clone(), from, through);
        missing.lifecycle = None;
        let facts = QualifiedTradingFacts::admit(
            QualifiedTradingFactsRequest::new(instrument.clone(), date),
            missing,
        )
        .unwrap();
        assert_eq!(
            facts.lifecycle().require().unwrap_err().reason(),
            TradingFactUnavailableReason::FactMissing
        );
        assert!(facts.price_regime().require().is_ok());

        let mut stale = task9_record(instrument.clone(), from, through);
        stale.fresh_through = date - chrono::Duration::days(1);
        let facts = QualifiedTradingFacts::admit(
            QualifiedTradingFactsRequest::new(instrument.clone(), date),
            stale,
        )
        .unwrap();
        assert_eq!(
            facts.price_regime().require().unwrap_err().reason(),
            TradingFactUnavailableReason::Stale
        );

        let mut insufficient = task9_record(instrument.clone(), from, through);
        insufficient.price_regime = Some(
            QualifiedPriceBand::new(
                SecurityBoard::Main,
                false,
                10_000,
                9_000_000,
                11_000_000,
                from,
                date - chrono::Duration::days(1),
                "TEST_CODE_OLD_REGIME_V1",
            )
            .unwrap(),
        );
        let facts = QualifiedTradingFacts::admit(
            QualifiedTradingFactsRequest::new(instrument.clone(), date),
            insufficient,
        )
        .unwrap();
        assert_eq!(
            facts.price_regime().require().unwrap_err().reason(),
            TradingFactUnavailableReason::CoverageInsufficient
        );

        let mut conflict = task9_record(instrument.clone(), from, through);
        conflict.contract_version.clear();
        let facts = QualifiedTradingFacts::admit(
            QualifiedTradingFactsRequest::new(instrument.clone(), date),
            conflict,
        )
        .unwrap();
        assert_eq!(
            facts.lifecycle().require().unwrap_err().reason(),
            TradingFactUnavailableReason::Conflict
        );

        let mut invalid_time = task9_record(instrument.clone(), from, through);
        invalid_time.evidence.observed_at = "not-rfc3339".to_owned();
        let facts = QualifiedTradingFacts::admit(
            QualifiedTradingFactsRequest::new(instrument.clone(), date),
            invalid_time,
        )
        .unwrap();
        assert_eq!(
            facts.lifecycle().require().unwrap_err().reason(),
            TradingFactUnavailableReason::Conflict
        );

        let mut overlapping = task9_record(instrument.clone(), from, through);
        overlapping.suspension = Some(AuthoritySuspensionCoverage::with_windows(
            from,
            through,
            vec![
                SuspensionWindow {
                    halted_from: date - chrono::Duration::days(2),
                    halted_through: date,
                },
                SuspensionWindow {
                    halted_from: date,
                    halted_through: date + chrono::Duration::days(1),
                },
            ],
        ));
        let facts = QualifiedTradingFacts::admit(
            QualifiedTradingFactsRequest::new(instrument.clone(), date),
            overlapping,
        )
        .unwrap();
        assert_eq!(
            facts.suspension().require().unwrap_err().reason(),
            TradingFactUnavailableReason::Conflict
        );

        let mut wrong_identity = task9_record(instrument.clone(), from, through);
        wrong_identity.instrument =
            InstrumentId::new(Exchange::Shanghai, "600002", AssetClass::Equity).unwrap();
        let facts = QualifiedTradingFacts::admit(
            QualifiedTradingFactsRequest::new(instrument, date),
            wrong_identity,
        )
        .unwrap();
        assert_eq!(
            facts.suspension().require().unwrap_err().reason(),
            TradingFactUnavailableReason::IdentityMismatch
        );
    }
}
