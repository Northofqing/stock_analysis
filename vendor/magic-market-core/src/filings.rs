use crate::{
    CoreError, DataBatch, HttpsUrl, IsoDate, NonEmptyText, PositiveU32, ProviderId, SourceEvidence,
    SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use std::fmt;

/// SEC company identity with a normalized ten-digit CIK.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SecCompanyIdentity {
    cik: String,
    ticker: Option<String>,
}

impl SecCompanyIdentity {
    pub fn new<T>(cik: impl Into<String>, ticker: Option<T>) -> Result<Self, CoreError>
    where
        T: Into<String>,
    {
        let cik = cik.into();
        if cik.is_empty() || cik.len() > 10 || !cik.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(CoreError::InvalidValue {
                field: "sec_cik",
                value: cik,
                reason: "must contain 1 through 10 ASCII digits",
            });
        }
        let ticker = ticker
            .map(Into::into)
            .map(|value: String| {
                let value = value.to_ascii_uppercase();
                if value.is_empty()
                    || value.len() > 10
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
                {
                    return Err(CoreError::InvalidValue {
                        field: "sec_ticker",
                        value,
                        reason: "must contain 1 through 10 ASCII letters, digits, hyphens or dots",
                    });
                }
                Ok(value)
            })
            .transpose()?;
        Ok(Self {
            cik: format!("{cik:0>10}"),
            ticker,
        })
    }

    pub fn cik(&self) -> &str {
        &self.cik
    }

    pub fn ticker(&self) -> Option<&str> {
        self.ticker.as_deref()
    }
}

impl<'de> Deserialize<'de> for SecCompanyIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            cik: String,
            ticker: Option<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.cik, wire.ticker).map_err(de::Error::custom)
    }
}

/// Canonical SEC accession number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SecAccessionNumber(String);

impl SecAccessionNumber {
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let bytes = value.as_bytes();
        let valid = bytes.len() == 20
            && bytes[10] == b'-'
            && bytes[13] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(index, byte)| matches!(index, 10 | 13) || byte.is_ascii_digit());
        if !valid {
            return Err(CoreError::InvalidValue {
                field: "sec_accession_number",
                value,
                reason: "must match ##########-##-######",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn without_hyphens(&self) -> String {
        self.0
            .chars()
            .filter(|character| *character != '-')
            .collect()
    }
}

impl fmt::Display for SecAccessionNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SecAccessionNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// One path-safe SEC primary-document filename.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SecPrimaryDocument(String);

impl SecPrimaryDocument {
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into();
        let lower = value.to_ascii_lowercase();
        let has_allowed_extension = [".htm", ".html", ".txt", ".xml"]
            .iter()
            .any(|extension| lower.ends_with(extension));
        let has_only_safe_ascii = value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if value.is_empty()
            || value == "."
            || value == ".."
            || !has_only_safe_ascii
            || !has_allowed_extension
        {
            return Err(CoreError::InvalidValue {
                field: "sec_primary_document",
                value,
                reason: "must be one safe SEC document path segment",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecPrimaryDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SecPrimaryDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Bounded request for SEC filing metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompanyFilingRequest {
    companies: Vec<SecCompanyIdentity>,
    forms: Vec<NonEmptyText>,
    start: Option<IsoDate>,
    end: Option<IsoDate>,
    max_records: PositiveU32,
}

impl CompanyFilingRequest {
    pub fn new(
        companies: Vec<SecCompanyIdentity>,
        forms: Vec<NonEmptyText>,
        start: Option<IsoDate>,
        end: Option<IsoDate>,
        max_records: PositiveU32,
    ) -> Result<Self, CoreError> {
        if companies.is_empty() || companies.len() > 100 {
            return Err(CoreError::InvalidRequest(
                "company filing request accepts 1 through 100 companies".into(),
            ));
        }
        let mut company_ciks = HashSet::with_capacity(companies.len());
        if companies
            .iter()
            .any(|company| !company_ciks.insert(company.cik()))
        {
            return Err(CoreError::InvalidRequest(
                "company filing request contains duplicate companies".into(),
            ));
        }
        if forms.len() > 20 {
            return Err(CoreError::InvalidRequest(
                "company filing request accepts at most 20 forms".into(),
            ));
        }
        let mut seen_forms = HashSet::with_capacity(forms.len());
        if forms.iter().any(|form| !seen_forms.insert(form.clone())) {
            return Err(CoreError::InvalidRequest(
                "company filing request contains duplicate forms".into(),
            ));
        }
        match (&start, &end) {
            (Some(start), Some(end)) if start > end => {
                return Err(CoreError::InvalidRequest(
                    "company filing start must not exceed end".into(),
                ));
            }
            (Some(_), None) | (None, Some(_)) => {
                return Err(CoreError::InvalidRequest(
                    "company filing date range requires both start and end".into(),
                ));
            }
            _ => {}
        }
        if max_records.get() > 1_000 {
            return Err(CoreError::InvalidRequest(
                "company filing max_records must not exceed 1000".into(),
            ));
        }
        Ok(Self {
            companies,
            forms,
            start,
            end,
            max_records,
        })
    }

    pub fn companies(&self) -> &[SecCompanyIdentity] {
        &self.companies
    }
    pub fn forms(&self) -> &[NonEmptyText] {
        &self.forms
    }
    pub fn start(&self) -> Option<&IsoDate> {
        self.start.as_ref()
    }
    pub fn end(&self) -> Option<&IsoDate> {
        self.end.as_ref()
    }
    pub fn max_records(&self) -> PositiveU32 {
        self.max_records
    }
}

impl<'de> Deserialize<'de> for CompanyFilingRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            companies: Vec<SecCompanyIdentity>,
            forms: Vec<NonEmptyText>,
            start: Option<IsoDate>,
            end: Option<IsoDate>,
            max_records: PositiveU32,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.companies,
            wire.forms,
            wire.start,
            wire.end,
            wire.max_records,
        )
        .map_err(de::Error::custom)
    }
}

/// One checked SEC filing metadata record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompanyFiling {
    company: SecCompanyIdentity,
    company_name: NonEmptyText,
    form: NonEmptyText,
    filing_date: IsoDate,
    report_period: Option<IsoDate>,
    accession: SecAccessionNumber,
    primary_document: SecPrimaryDocument,
    filing_index_url: HttpsUrl,
    primary_document_url: HttpsUrl,
    accepted_at: Option<NonEmptyText>,
    evidence: SourceEvidence,
}

impl CompanyFiling {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        company: SecCompanyIdentity,
        company_name: impl Into<String>,
        form: impl Into<String>,
        filing_date: IsoDate,
        report_period: Option<IsoDate>,
        accession: SecAccessionNumber,
        primary_document: SecPrimaryDocument,
        filing_index_url: HttpsUrl,
        primary_document_url: HttpsUrl,
        accepted_at: Option<NonEmptyText>,
        evidence: SourceEvidence,
    ) -> Result<Self, CoreError> {
        if accepted_at.as_ref().map(NonEmptyText::as_str) != evidence.source_at() {
            return Err(CoreError::InvalidRequest(
                "company filing accepted_at must match source evidence".into(),
            ));
        }
        Ok(Self {
            company,
            company_name: NonEmptyText::new(company_name)?,
            form: NonEmptyText::new(form)?,
            filing_date,
            report_period,
            accession,
            primary_document,
            filing_index_url,
            primary_document_url,
            accepted_at,
            evidence,
        })
    }

    pub fn company(&self) -> &SecCompanyIdentity {
        &self.company
    }
    pub fn company_name(&self) -> &str {
        self.company_name.as_str()
    }
    pub fn form(&self) -> &str {
        self.form.as_str()
    }
    pub fn filing_date(&self) -> &IsoDate {
        &self.filing_date
    }
    pub fn report_period(&self) -> Option<&IsoDate> {
        self.report_period.as_ref()
    }
    pub fn accession(&self) -> &SecAccessionNumber {
        &self.accession
    }
    pub fn primary_document(&self) -> &SecPrimaryDocument {
        &self.primary_document
    }
    pub fn filing_index_url(&self) -> &HttpsUrl {
        &self.filing_index_url
    }
    pub fn primary_document_url(&self) -> &HttpsUrl {
        &self.primary_document_url
    }
    pub fn accepted_at(&self) -> Option<&str> {
        self.accepted_at.as_ref().map(NonEmptyText::as_str)
    }
    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

impl<'de> Deserialize<'de> for CompanyFiling {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            company: SecCompanyIdentity,
            company_name: String,
            form: String,
            filing_date: IsoDate,
            report_period: Option<IsoDate>,
            accession: SecAccessionNumber,
            primary_document: SecPrimaryDocument,
            filing_index_url: HttpsUrl,
            primary_document_url: HttpsUrl,
            accepted_at: Option<NonEmptyText>,
            evidence: SourceEvidence,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.company,
            wire.company_name,
            wire.form,
            wire.filing_date,
            wire.report_period,
            wire.accession,
            wire.primary_document,
            wire.filing_index_url,
            wire.primary_document_url,
            wire.accepted_at,
            wire.evidence,
        )
        .map_err(de::Error::custom)
    }
}

impl SourcedRecord for CompanyFiling {
    fn provider_id(&self) -> ProviderId {
        self.evidence.provider()
    }
    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
    fn evidence_source_at(&self) -> Option<&str> {
        self.evidence.source_at()
    }
    fn evidence_observed_at(&self) -> Option<&str> {
        Some(self.evidence.observed_at())
    }
}

pub trait CompanyFilingsProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn company_filings(
        &self,
        request: &CompanyFilingRequest,
    ) -> Result<DataBatch<CompanyFiling>, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FilingCapabilities {
    pub filing_metadata: bool,
    pub filing_documents: bool,
    pub xbrl_facts: bool,
}
