use crate::{
    DataBatch, FiniteNumber, InstrumentId, IsoDate, NonEmptyText, Quantity, SourceEvidence,
    SourcedRecord,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileFact {
    pub key: NonEmptyText,
    pub source_label: NonEmptyText,
    pub value: NonEmptyText,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityProfile {
    pub instrument: InstrumentId,
    pub name: NonEmptyText,
    pub industry: Option<NonEmptyText>,
    pub listed_on: Option<IsoDate>,
    pub total_shares: Option<Quantity>,
    pub floating_shares: Option<Quantity>,
    pub facts: Vec<ProfileFact>,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatementKind {
    Balance,
    Income,
    CashFlow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinancialLine {
    pub key: NonEmptyText,
    pub source_label: NonEmptyText,
    pub value: Option<FiniteNumber>,
    pub unit: Option<NonEmptyText>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinancialStatement {
    pub instrument: InstrumentId,
    pub kind: StatementKind,
    pub report_period: IsoDate,
    pub announced_on: Option<IsoDate>,
    pub currency: Option<NonEmptyText>,
    pub lines: Vec<FinancialLine>,
    pub evidence: SourceEvidence,
}

impl SourcedRecord for SecurityProfile {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

impl SourcedRecord for FinancialStatement {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CompanyCapabilities {
    pub security_profile: bool,
    pub balance_sheet: bool,
    pub income_statement: bool,
    pub cash_flow_statement: bool,
}

pub trait SecurityProfiles {
    type Error: std::error::Error + Send + Sync + 'static;
    fn security_profiles(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<SecurityProfile>, Self::Error>;
}

pub trait FinancialStatements {
    type Error: std::error::Error + Send + Sync + 'static;
    fn financial_statements(
        &self,
        instruments: &[InstrumentId],
        kind: StatementKind,
    ) -> Result<DataBatch<FinancialStatement>, Self::Error>;
}
