use crate::{
    DataBatch, FiniteNumber, HttpsUrl, InstrumentId, IsoDate, Money, NonEmptyText, PositiveU32,
    Price, SourceEvidence, SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::collections::HashSet;

/// Scope used by research-report queries and records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportScope {
    Instrument(InstrumentId),
    Industry(NonEmptyText),
}

/// One fiscal-period estimate retained exactly as supplied.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EarningsEstimate {
    fiscal_year: PositiveU32,
    eps: Option<FiniteNumber>,
    eps_min: Option<FiniteNumber>,
    eps_max: Option<FiniteNumber>,
    contributor_count: Option<PositiveU32>,
    revenue: Option<Money>,
    profit: Option<Money>,
}

impl EarningsEstimate {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fiscal_year: PositiveU32,
        eps: Option<FiniteNumber>,
        eps_min: Option<FiniteNumber>,
        eps_max: Option<FiniteNumber>,
        contributor_count: Option<PositiveU32>,
        revenue: Option<Money>,
        profit: Option<Money>,
    ) -> Result<Self, crate::CoreError> {
        if let (Some(minimum), Some(maximum)) = (eps_min, eps_max) {
            if minimum.get() > maximum.get() {
                return Err(crate::CoreError::InvalidRequest(
                    "earnings EPS minimum must not exceed maximum".into(),
                ));
            }
        }
        Ok(Self {
            fiscal_year,
            eps,
            eps_min,
            eps_max,
            contributor_count,
            revenue,
            profit,
        })
    }

    pub fn fiscal_year(&self) -> PositiveU32 {
        self.fiscal_year
    }

    pub fn eps(&self) -> Option<FiniteNumber> {
        self.eps
    }

    pub fn eps_min(&self) -> Option<FiniteNumber> {
        self.eps_min
    }

    pub fn eps_max(&self) -> Option<FiniteNumber> {
        self.eps_max
    }

    pub fn contributor_count(&self) -> Option<PositiveU32> {
        self.contributor_count
    }

    pub fn revenue(&self) -> Option<Money> {
        self.revenue
    }

    pub fn profit(&self) -> Option<Money> {
        self.profit
    }
}

#[derive(Deserialize)]
struct EarningsEstimateWire {
    fiscal_year: PositiveU32,
    eps: Option<FiniteNumber>,
    eps_min: Option<FiniteNumber>,
    eps_max: Option<FiniteNumber>,
    contributor_count: Option<PositiveU32>,
    revenue: Option<Money>,
    profit: Option<Money>,
}

impl<'de> Deserialize<'de> for EarningsEstimate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = EarningsEstimateWire::deserialize(deserializer)?;
        Self::new(
            wire.fiscal_year,
            wire.eps,
            wire.eps_min,
            wire.eps_max,
            wire.contributor_count,
            wire.revenue,
            wire.profit,
        )
        .map_err(de::Error::custom)
    }
}

/// One published stock or industry research report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResearchReport {
    pub report_id: NonEmptyText,
    pub scope: ReportScope,
    pub title: NonEmptyText,
    pub organization: NonEmptyText,
    #[serde(default)]
    pub organization_id: Option<NonEmptyText>,
    pub author: Option<NonEmptyText>,
    pub rating: Option<NonEmptyText>,
    pub industry_code: Option<NonEmptyText>,
    pub industry_name: Option<NonEmptyText>,
    pub published_at: NonEmptyText,
    pub canonical_url: HttpsUrl,
    pub pdf_url: Option<HttpsUrl>,
    pub estimates: Vec<EarningsEstimate>,
    /// Eastmoney source upper-bound field, proven by live report evidence.
    #[serde(default)]
    pub source_indv_aim_price_t: Option<Price>,
    /// Eastmoney source lower-bound field, proven by live report evidence.
    #[serde(default)]
    pub source_indv_aim_price_l: Option<Price>,
    pub evidence: SourceEvidence,
}

impl SourcedRecord for ResearchReport {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

/// Provider consensus for one instrument and observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsensusSnapshot {
    pub instrument: InstrumentId,
    /// Source-proven security name paired with `instrument`.
    pub name: NonEmptyText,
    pub estimates: Vec<EarningsEstimate>,
    pub contributor_count: Option<PositiveU32>,
    pub evidence: SourceEvidence,
}

impl SourcedRecord for ConsensusSnapshot {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

/// Complete observation range requested for one instrument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetPriceRequest {
    instrument: InstrumentId,
    from: IsoDate,
    through: IsoDate,
}

impl TargetPriceRequest {
    pub fn new(
        instrument: InstrumentId,
        from: IsoDate,
        through: IsoDate,
    ) -> Result<Self, crate::CoreError> {
        if from > through {
            return Err(crate::CoreError::InvalidRequest(
                "target-price request start must not exceed end".into(),
            ));
        }
        Ok(Self {
            instrument,
            from,
            through,
        })
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn from(&self) -> &IsoDate {
        &self.from
    }
    pub fn through(&self) -> &IsoDate {
        &self.through
    }
}

#[derive(Deserialize)]
struct TargetPriceRequestWire {
    instrument: InstrumentId,
    from: IsoDate,
    through: IsoDate,
}

impl<'de> Deserialize<'de> for TargetPriceRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TargetPriceRequestWire::deserialize(deserializer)?;
        Self::new(wire.instrument, wire.from, wire.through).map_err(de::Error::custom)
    }
}

/// One report-level target-price observation. Eastmoney `L` is the lower bound
/// and `T` is the upper bound; both exact source fields remain available.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TargetPriceObservation {
    instrument: InstrumentId,
    instrument_name: NonEmptyText,
    report_id: NonEmptyText,
    institution_id: NonEmptyText,
    institution_name: NonEmptyText,
    published_on: IsoDate,
    source_indv_aim_price_t: Price,
    source_indv_aim_price_l: Price,
    normalized_low: Price,
    normalized_high: Price,
    evidence: SourceEvidence,
}

impl TargetPriceObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        instrument_name: NonEmptyText,
        report_id: NonEmptyText,
        institution_id: NonEmptyText,
        institution_name: NonEmptyText,
        published_on: IsoDate,
        source_indv_aim_price_t: Price,
        source_indv_aim_price_l: Price,
        evidence: SourceEvidence,
    ) -> Result<Self, crate::CoreError> {
        validate_target_evidence_date(&published_on, &evidence, "target-price observation")?;
        if source_indv_aim_price_l.get() > source_indv_aim_price_t.get() {
            return Err(crate::CoreError::InvalidRequest(
                "source indvAimPriceL lower bound must not exceed indvAimPriceT upper bound".into(),
            ));
        }
        let normalized_low = source_indv_aim_price_l;
        let normalized_high = source_indv_aim_price_t;
        Ok(Self {
            instrument,
            instrument_name,
            report_id,
            institution_id,
            institution_name,
            published_on,
            source_indv_aim_price_t,
            source_indv_aim_price_l,
            normalized_low,
            normalized_high,
            evidence,
        })
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn instrument_name(&self) -> &NonEmptyText {
        &self.instrument_name
    }
    pub fn report_id(&self) -> &NonEmptyText {
        &self.report_id
    }
    pub fn institution_id(&self) -> &NonEmptyText {
        &self.institution_id
    }
    pub fn institution_name(&self) -> &NonEmptyText {
        &self.institution_name
    }
    pub fn published_on(&self) -> &IsoDate {
        &self.published_on
    }
    pub fn source_indv_aim_price_t(&self) -> Price {
        self.source_indv_aim_price_t
    }
    pub fn source_indv_aim_price_l(&self) -> Price {
        self.source_indv_aim_price_l
    }
    pub fn normalized_low(&self) -> Price {
        self.normalized_low
    }
    pub fn normalized_high(&self) -> Price {
        self.normalized_high
    }
    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

#[derive(Deserialize)]
struct TargetPriceObservationWire {
    instrument: InstrumentId,
    instrument_name: NonEmptyText,
    report_id: NonEmptyText,
    institution_id: NonEmptyText,
    institution_name: NonEmptyText,
    published_on: IsoDate,
    source_indv_aim_price_t: Price,
    source_indv_aim_price_l: Price,
    normalized_low: Price,
    normalized_high: Price,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for TargetPriceObservation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TargetPriceObservationWire::deserialize(deserializer)?;
        let value = Self::new(
            wire.instrument,
            wire.instrument_name,
            wire.report_id,
            wire.institution_id,
            wire.institution_name,
            wire.published_on,
            wire.source_indv_aim_price_t,
            wire.source_indv_aim_price_l,
            wire.evidence,
        )
        .map_err(de::Error::custom)?;
        if value.normalized_low != wire.normalized_low
            || value.normalized_high != wire.normalized_high
        {
            return Err(de::Error::custom(
                "target-price normalized range contradicts source T/L fields",
            ));
        }
        Ok(value)
    }
}

impl SourcedRecord for TargetPriceObservation {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

/// Complete, evidence-preserving aggregation over report target prices.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TargetPriceConsensus {
    instrument: InstrumentId,
    instrument_name: NonEmptyText,
    requested_from: IsoDate,
    requested_through: IsoDate,
    observation_start: IsoDate,
    observation_end: IsoDate,
    sample_count: PositiveU32,
    contributor_count: PositiveU32,
    low: Price,
    mean: Price,
    high: Price,
    observations: Vec<TargetPriceObservation>,
    input_evidence: Vec<SourceEvidence>,
    evidence: SourceEvidence,
}

impl TargetPriceConsensus {
    pub fn new(
        request: &TargetPriceRequest,
        mut observations: Vec<TargetPriceObservation>,
        evidence: SourceEvidence,
    ) -> Result<Self, crate::CoreError> {
        if observations.is_empty() {
            return Err(crate::CoreError::InvalidRequest(
                "target-price aggregation requires at least one complete observation".into(),
            ));
        }
        observations.sort_by(|left, right| {
            left.published_on
                .cmp(&right.published_on)
                .then_with(|| left.report_id.as_str().cmp(right.report_id.as_str()))
        });
        let mut report_ids = HashSet::with_capacity(observations.len());
        let mut report_institutions = HashSet::with_capacity(observations.len());
        let mut institutions = HashSet::with_capacity(observations.len());
        let instrument_name = observations
            .first()
            .map(|observation| observation.instrument_name.clone())
            .ok_or_else(|| {
                crate::CoreError::InvalidRequest("target-price instrument name absent".into())
            })?;
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        let mut sum = 0.0_f64;
        for observation in &observations {
            if observation.instrument() != request.instrument() {
                return Err(crate::CoreError::InvalidRequest(
                    "target-price observation instrument does not match request".into(),
                ));
            }
            if observation.instrument_name() != &instrument_name {
                return Err(crate::CoreError::InvalidRequest(
                    "target-price observations contain conflicting instrument names".into(),
                ));
            }
            if observation.published_on() < request.from()
                || observation.published_on() > request.through()
            {
                return Err(crate::CoreError::InvalidRequest(
                    "target-price observation is outside requested range".into(),
                ));
            }
            if observation.evidence.provider() != evidence.provider()
                || observation.evidence.batch_id() != evidence.batch_id()
            {
                return Err(crate::CoreError::InvalidRequest(
                    "target-price observation evidence does not match aggregate batch".into(),
                ));
            }
            if !report_ids.insert(observation.report_id.as_str().to_owned())
                || !report_institutions.insert((
                    observation.report_id.as_str().to_owned(),
                    observation.institution_id.as_str().to_owned(),
                ))
            {
                return Err(crate::CoreError::InvalidRequest(
                    "target-price aggregation contains duplicate institution/report input".into(),
                ));
            }
            institutions.insert(observation.institution_id.as_str().to_owned());
            low = low.min(observation.normalized_low.get());
            high = high.max(observation.normalized_high.get());
            sum += (observation.normalized_low.get() + observation.normalized_high.get()) / 2.0;
        }
        let observation_start = observations
            .first()
            .map(|value| value.published_on.clone())
            .ok_or_else(|| crate::CoreError::InvalidRequest("target-price start absent".into()))?;
        let observation_end = observations
            .last()
            .map(|value| value.published_on.clone())
            .ok_or_else(|| crate::CoreError::InvalidRequest("target-price end absent".into()))?;
        validate_target_evidence_date(&observation_end, &evidence, "target-price aggregate")?;
        let sample_count = PositiveU32::new(u32::try_from(observations.len()).map_err(|_| {
            crate::CoreError::InvalidRequest("target-price sample count overflow".into())
        })?)?;
        let contributor_count =
            PositiveU32::new(u32::try_from(institutions.len()).map_err(|_| {
                crate::CoreError::InvalidRequest("target-price contributor count overflow".into())
            })?)?;
        let mean = Price::new(sum / f64::from(sample_count.get()))?;
        let input_evidence = observations
            .iter()
            .map(|observation| observation.evidence.clone())
            .collect();
        Ok(Self {
            instrument: request.instrument().clone(),
            instrument_name,
            requested_from: request.from().clone(),
            requested_through: request.through().clone(),
            observation_start,
            observation_end,
            sample_count,
            contributor_count,
            low: Price::new(low)?,
            mean,
            high: Price::new(high)?,
            observations,
            input_evidence,
            evidence,
        })
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn instrument_name(&self) -> &NonEmptyText {
        &self.instrument_name
    }
    pub fn requested_from(&self) -> &IsoDate {
        &self.requested_from
    }
    pub fn requested_through(&self) -> &IsoDate {
        &self.requested_through
    }
    pub fn observation_start(&self) -> &IsoDate {
        &self.observation_start
    }
    pub fn observation_end(&self) -> &IsoDate {
        &self.observation_end
    }
    pub fn sample_count(&self) -> PositiveU32 {
        self.sample_count
    }
    pub fn contributor_count(&self) -> PositiveU32 {
        self.contributor_count
    }
    pub fn low(&self) -> Price {
        self.low
    }
    /// Arithmetic mean of every report's `(L + T) / 2` midpoint.
    ///
    /// This is a project-derived aggregate, not a provider-published consensus
    /// target and not a weighted analyst estimate.
    pub fn mean(&self) -> Price {
        self.mean
    }
    pub fn high(&self) -> Price {
        self.high
    }
    pub fn observations(&self) -> &[TargetPriceObservation] {
        &self.observations
    }
    pub fn input_evidence(&self) -> &[SourceEvidence] {
        &self.input_evidence
    }
    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

#[derive(Deserialize)]
struct TargetPriceConsensusWire {
    instrument: InstrumentId,
    instrument_name: NonEmptyText,
    requested_from: IsoDate,
    requested_through: IsoDate,
    observation_start: IsoDate,
    observation_end: IsoDate,
    sample_count: PositiveU32,
    contributor_count: PositiveU32,
    low: Price,
    mean: Price,
    high: Price,
    observations: Vec<TargetPriceObservation>,
    input_evidence: Vec<SourceEvidence>,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for TargetPriceConsensus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TargetPriceConsensusWire::deserialize(deserializer)?;
        let request = TargetPriceRequest::new(
            wire.instrument.clone(),
            wire.requested_from,
            wire.requested_through,
        )
        .map_err(de::Error::custom)?;
        let value =
            Self::new(&request, wire.observations, wire.evidence).map_err(de::Error::custom)?;
        if value.observation_start != wire.observation_start
            || value.instrument_name != wire.instrument_name
            || value.observation_end != wire.observation_end
            || value.sample_count != wire.sample_count
            || value.contributor_count != wire.contributor_count
            || value.low != wire.low
            || value.mean != wire.mean
            || value.high != wire.high
            || value.input_evidence != wire.input_evidence
        {
            return Err(de::Error::custom(
                "target-price aggregate fields contradict input observations",
            ));
        }
        Ok(value)
    }
}

impl SourcedRecord for TargetPriceConsensus {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

fn validate_target_evidence_date(
    date: &IsoDate,
    evidence: &SourceEvidence,
    context: &str,
) -> Result<(), crate::CoreError> {
    let source_at = evidence.source_at().ok_or_else(|| {
        crate::CoreError::InvalidRequest(format!("{context} evidence must include source_at"))
    })?;
    if source_at.get(..10) != Some(date.as_str())
        || !matches!(source_at.as_bytes().get(10), None | Some(b' ') | Some(b'T'))
    {
        return Err(crate::CoreError::InvalidRequest(format!(
            "{context} evidence date does not match the observation date"
        )));
    }
    Ok(())
}

/// Searchable content family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SemanticChannel {
    Report,
    News,
    Announcement,
    General,
}

/// Normalized semantic-search document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSearchDocument {
    pub document_id: NonEmptyText,
    pub channel: SemanticChannel,
    pub title: NonEmptyText,
    pub excerpt: Option<NonEmptyText>,
    pub canonical_url: HttpsUrl,
    pub published_at: Option<NonEmptyText>,
    pub evidence: SourceEvidence,
}

impl SourcedRecord for SemanticSearchDocument {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

/// Bounded report page request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResearchRequest {
    scope: ReportScope,
    page: PositiveU32,
    page_size: PositiveU32,
}

impl ResearchRequest {
    pub fn new(
        scope: ReportScope,
        page: PositiveU32,
        page_size: PositiveU32,
    ) -> Result<Self, crate::CoreError> {
        if page_size.get() > 100 {
            return Err(crate::CoreError::InvalidRequest(
                "research page_size must be at most 100".into(),
            ));
        }
        Ok(Self {
            scope,
            page,
            page_size,
        })
    }

    pub fn scope(&self) -> &ReportScope {
        &self.scope
    }

    pub fn page(&self) -> PositiveU32 {
        self.page
    }

    pub fn page_size(&self) -> PositiveU32 {
        self.page_size
    }
}

#[derive(Deserialize)]
struct ResearchRequestWire {
    scope: ReportScope,
    page: PositiveU32,
    page_size: PositiveU32,
}

impl<'de> Deserialize<'de> for ResearchRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ResearchRequestWire::deserialize(deserializer)?;
        Self::new(wire.scope, wire.page, wire.page_size).map_err(de::Error::custom)
    }
}

/// Bounded semantic query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticSearchRequest {
    query: NonEmptyText,
    channel: SemanticChannel,
    limit: PositiveU32,
}

impl SemanticSearchRequest {
    pub fn new(
        query: impl Into<String>,
        channel: SemanticChannel,
        limit: PositiveU32,
    ) -> Result<Self, crate::CoreError> {
        if limit.get() > 100 {
            return Err(crate::CoreError::InvalidRequest(
                "semantic search limit must be at most 100".into(),
            ));
        }
        Ok(Self {
            query: NonEmptyText::new(query)?,
            channel,
            limit,
        })
    }

    pub fn query(&self) -> &NonEmptyText {
        &self.query
    }

    pub fn channel(&self) -> SemanticChannel {
        self.channel
    }

    pub fn limit(&self) -> PositiveU32 {
        self.limit
    }
}

#[derive(Deserialize)]
struct SemanticSearchRequestWire {
    query: String,
    channel: SemanticChannel,
    limit: PositiveU32,
}

impl<'de> Deserialize<'de> for SemanticSearchRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SemanticSearchRequestWire::deserialize(deserializer)?;
        Self::new(wire.query, wire.channel, wire.limit).map_err(de::Error::custom)
    }
}

/// Research-domain capability declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResearchCapabilities {
    pub reports: bool,
    pub consensus: bool,
    pub target_price_consensus: bool,
    pub semantic_search: bool,
    pub pdf_download: bool,
    pub document_body: bool,
}

/// Identity pair for one source report document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchDocumentRequest {
    pub report_id: NonEmptyText,
    pub pdf_url: HttpsUrl,
}

/// Original bounded report body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResearchDocument {
    pub report_id: NonEmptyText,
    pub pdf_url: HttpsUrl,
    pub content_type: NonEmptyText,
    pub body: Vec<u8>,
    pub evidence: SourceEvidence,
}

impl ResearchDocument {
    pub fn new(
        report_id: NonEmptyText,
        pdf_url: HttpsUrl,
        body: Vec<u8>,
        evidence: SourceEvidence,
    ) -> Result<Self, crate::CoreError> {
        if !body.starts_with(b"%PDF-") {
            return Err(crate::CoreError::InvalidRequest(
                "research document body must start with a PDF header".into(),
            ));
        }
        if body.len() > 32 * 1024 * 1024 {
            return Err(crate::CoreError::InvalidRequest(
                "research document body must be at most 32 MiB".into(),
            ));
        }
        let complete_body = body.strip_suffix_pdf_whitespace().ok_or_else(|| {
            crate::CoreError::InvalidRequest(
                "research document body must end with a PDF EOF marker".into(),
            )
        })?;
        let before_eof = complete_body.strip_suffix(b"%%EOF").ok_or_else(|| {
            crate::CoreError::InvalidRequest(
                "research document body must end with a PDF EOF marker".into(),
            )
        })?;
        if !before_eof
            .windows(b"startxref".len())
            .any(|window| window == b"startxref")
        {
            return Err(crate::CoreError::InvalidRequest(
                "research document body must contain startxref before its EOF marker".into(),
            ));
        }
        Ok(Self {
            report_id,
            pdf_url,
            content_type: NonEmptyText::new("application/pdf")?,
            body,
            evidence,
        })
    }
}

trait PdfBodyExt {
    fn strip_suffix_pdf_whitespace(&self) -> Option<&[u8]>;
}

impl PdfBodyExt for [u8] {
    fn strip_suffix_pdf_whitespace(&self) -> Option<&[u8]> {
        let end = self
            .iter()
            .rposition(|byte| !matches!(byte, 0x00 | b'\t' | b'\n' | 0x0c | b'\r' | b' '))?
            + 1;
        Some(&self[..end])
    }
}

#[derive(Deserialize)]
struct ResearchDocumentWire {
    report_id: NonEmptyText,
    pdf_url: HttpsUrl,
    content_type: NonEmptyText,
    body: Vec<u8>,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for ResearchDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ResearchDocumentWire::deserialize(deserializer)?;
        if wire.content_type.as_str() != "application/pdf" {
            return Err(de::Error::custom(
                "research document content_type must be application/pdf",
            ));
        }
        Self::new(wire.report_id, wire.pdf_url, wire.body, wire.evidence).map_err(de::Error::custom)
    }
}

impl SourcedRecord for ResearchDocument {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

pub trait ResearchReports {
    type Error: std::error::Error + Send + Sync + 'static;

    fn research_reports(
        &self,
        request: &ResearchRequest,
    ) -> Result<DataBatch<ResearchReport>, Self::Error>;
}

pub trait ResearchDocuments {
    type Error: std::error::Error + Send + Sync + 'static;

    fn research_document(
        &self,
        request: &ResearchDocumentRequest,
    ) -> Result<DataBatch<ResearchDocument>, Self::Error>;
}

pub trait ConsensusData {
    type Error: std::error::Error + Send + Sync + 'static;

    fn consensus(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<ConsensusSnapshot>, Self::Error>;
}

pub trait TargetPriceData {
    type Error: std::error::Error + Send + Sync + 'static;

    fn target_price_consensus(
        &self,
        request: &TargetPriceRequest,
    ) -> Result<DataBatch<TargetPriceConsensus>, Self::Error>;
}

pub trait SemanticSearch {
    type Error: std::error::Error + Send + Sync + 'static;

    fn semantic_search(
        &self,
        request: &SemanticSearchRequest,
    ) -> Result<DataBatch<SemanticSearchDocument>, Self::Error>;
}
