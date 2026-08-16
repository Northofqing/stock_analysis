//! BR-173 canonical identity boundary for six-digit China-listed equities.

use std::{error::Error, fmt};

use crate::magic_compat::ProviderId;
use magic_market_core::{AssetClass, Exchange, InstrumentId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EquityShareClass {
    A,
    B,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EquitySegment {
    ShanghaiMainA,
    ShanghaiStarA,
    ShanghaiB,
    ShenzhenMainA,
    ShenzhenChiNextA,
    ShenzhenB,
    BeijingA,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderExchangeEvidence {
    provider: ProviderId,
    capability: String,
    exchange: Exchange,
    batch_id: String,
    item_id: Option<String>,
    observed_at: String,
}

impl ProviderExchangeEvidence {
    pub fn new(
        provider: ProviderId,
        capability: impl Into<String>,
        exchange: Exchange,
        batch_id: impl Into<String>,
        item_id: Option<String>,
        observed_at: impl Into<String>,
    ) -> Result<Self, EquityIdentityError> {
        if provider == ProviderId::Custom {
            return Err(EquityIdentityError::InvalidProviderEvidence {
                field: ProviderEvidenceField::Provider,
                issue: ProviderEvidenceIssue::UnverifiableCustomProvider,
            });
        }
        let capability = capability.into();
        validate_provider_text(ProviderEvidenceField::Capability, &capability)?;
        let batch_id = batch_id.into();
        validate_provider_text(ProviderEvidenceField::BatchId, &batch_id)?;
        if let Some(item_id) = item_id.as_deref() {
            validate_provider_text(ProviderEvidenceField::ItemId, item_id)?;
        }
        let observed_at = observed_at.into();
        validate_provider_text(ProviderEvidenceField::ObservedAt, &observed_at)?;
        chrono::DateTime::parse_from_rfc3339(&observed_at).map_err(|_| {
            EquityIdentityError::InvalidProviderEvidence {
                field: ProviderEvidenceField::ObservedAt,
                issue: ProviderEvidenceIssue::InvalidRfc3339,
            }
        })?;
        Ok(Self {
            provider,
            capability,
            exchange,
            batch_id,
            item_id,
            observed_at,
        })
    }

    pub fn provider(&self) -> ProviderId {
        self.provider
    }

    pub fn capability(&self) -> &str {
        &self.capability
    }

    pub fn exchange(&self) -> Exchange {
        self.exchange
    }

    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }

    pub fn item_id(&self) -> Option<&str> {
        self.item_id.as_deref()
    }

    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderEvidenceField {
    Provider,
    Capability,
    BatchId,
    ItemId,
    ObservedAt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderEvidenceIssue {
    UnverifiableCustomProvider,
    Empty,
    SurroundingWhitespace,
    ControlCharacter,
    InvalidRfc3339,
}

fn validate_provider_text(
    field: ProviderEvidenceField,
    value: &str,
) -> Result<(), EquityIdentityError> {
    let issue = if value.is_empty() {
        Some(ProviderEvidenceIssue::Empty)
    } else if value.trim() != value {
        Some(ProviderEvidenceIssue::SurroundingWhitespace)
    } else if value.chars().any(char::is_control) {
        Some(ProviderEvidenceIssue::ControlCharacter)
    } else {
        None
    };
    if let Some(issue) = issue {
        return Err(EquityIdentityError::InvalidProviderEvidence { field, issue });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityResolutionSource {
    CanonicalSegment,
    ProviderVerified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalEquityIdentity {
    instrument: InstrumentId,
    canonical_code: String,
    share_class: EquityShareClass,
    segment: EquitySegment,
    provider_evidence: Option<ProviderExchangeEvidence>,
    resolution_source: IdentityResolutionSource,
}

impl CanonicalEquityIdentity {
    pub fn storage_code(&self) -> &str {
        self.instrument.code()
    }

    pub fn canonical_code(&self) -> &str {
        &self.canonical_code
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn exchange(&self) -> Exchange {
        self.instrument.exchange()
    }

    pub fn share_class(&self) -> EquityShareClass {
        self.share_class
    }

    pub fn segment(&self) -> EquitySegment {
        self.segment
    }

    pub fn provider_evidence(&self) -> Option<&ProviderExchangeEvidence> {
        self.provider_evidence.as_ref()
    }

    pub fn resolution_source(&self) -> IdentityResolutionSource {
        self.resolution_source
    }

    pub fn require_a_share(&self) -> Result<&Self, EquityIdentityError> {
        if self.share_class == EquityShareClass::A {
            return Ok(self);
        }
        Err(EquityIdentityError::UnsupportedShareClass {
            code: self.canonical_code.clone(),
            share_class: self.share_class,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EquityIdentityError {
    InvalidEquityCode {
        code: String,
    },
    TestIdentityInProduction {
        code: String,
    },
    RealIdentityInTest {
        code: String,
    },
    HistoricalAliasRequired {
        code: String,
    },
    UnsupportedEquityPrefix {
        code: String,
    },
    UnsupportedShareClass {
        code: String,
        share_class: EquityShareClass,
    },
    ProviderMarketConflict {
        code: String,
        canonical_exchange: Exchange,
        provider: ProviderId,
        provider_exchange: Exchange,
    },
    InvalidProviderEvidence {
        field: ProviderEvidenceField,
        issue: ProviderEvidenceIssue,
    },
    CoreInvariant {
        message: String,
    },
}

impl EquityIdentityError {
    pub fn invalid_provider_field(&self) -> Option<ProviderEvidenceField> {
        match self {
            Self::InvalidProviderEvidence { field, .. } => Some(*field),
            _ => None,
        }
    }
}

impl fmt::Display for EquityIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEquityCode { code } => {
                write!(
                    formatter,
                    "equity code must be exactly six ASCII digits: {code:?}"
                )
            }
            Self::TestIdentityInProduction { code } => {
                write!(
                    formatter,
                    "production resolver rejects test identity: {code}"
                )
            }
            Self::RealIdentityInTest { code } => {
                write!(
                    formatter,
                    "test resolver requires TEST_CODE_ namespace: {code}"
                )
            }
            Self::HistoricalAliasRequired { code } => write!(
                formatter,
                "historical code requires an exact official old-to-92xxxx mapping: {code}"
            ),
            Self::UnsupportedEquityPrefix { code } => {
                write!(formatter, "unsupported current equity prefix: {code}")
            }
            Self::UnsupportedShareClass { code, share_class } => write!(
                formatter,
                "equity {code} has unsupported share class {share_class:?}"
            ),
            Self::ProviderMarketConflict {
                code,
                canonical_exchange,
                provider,
                provider_exchange,
            } => write!(
                formatter,
                "provider {provider:?} exchange {provider_exchange:?} conflicts with canonical \
                 exchange {canonical_exchange:?} for {code}"
            ),
            Self::InvalidProviderEvidence { field, issue } => {
                write!(
                    formatter,
                    "invalid provider evidence field {field:?}: {issue:?}"
                )
            }
            Self::CoreInvariant { message } => {
                write!(
                    formatter,
                    "validated equity identity rejected by core: {message}"
                )
            }
        }
    }
}

impl Error for EquityIdentityError {}

pub fn resolve_production_equity(
    code: &str,
    evidence: Option<ProviderExchangeEvidence>,
) -> Result<CanonicalEquityIdentity, EquityIdentityError> {
    if code.starts_with("TEST_CODE_") {
        return Err(EquityIdentityError::TestIdentityInProduction {
            code: code.to_owned(),
        });
    }
    validate_six_digit_code(code, code)?;
    resolve_validated_equity(code, code, evidence)
}

#[cfg(test)]
pub fn resolve_test_equity(
    code: &str,
    evidence: Option<ProviderExchangeEvidence>,
) -> Result<CanonicalEquityIdentity, EquityIdentityError> {
    let Some(canonical_code) = code.strip_prefix("TEST_CODE_") else {
        return Err(EquityIdentityError::RealIdentityInTest {
            code: code.to_owned(),
        });
    };
    validate_six_digit_code(canonical_code, code)?;
    resolve_validated_equity(code, canonical_code, evidence)
}

fn validate_six_digit_code(code: &str, reported_code: &str) -> Result<(), EquityIdentityError> {
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(EquityIdentityError::InvalidEquityCode {
            code: reported_code.to_owned(),
        });
    }
    Ok(())
}

fn resolve_validated_equity(
    storage_code: &str,
    canonical_code: &str,
    evidence: Option<ProviderExchangeEvidence>,
) -> Result<CanonicalEquityIdentity, EquityIdentityError> {
    let (exchange, share_class, segment) = classify_current_equity(canonical_code)?;
    if let Some(provider_evidence) = evidence.as_ref() {
        if provider_evidence.exchange != exchange {
            return Err(EquityIdentityError::ProviderMarketConflict {
                code: canonical_code.to_owned(),
                canonical_exchange: exchange,
                provider: provider_evidence.provider,
                provider_exchange: provider_evidence.exchange,
            });
        }
    }
    let instrument =
        InstrumentId::new(exchange, storage_code, AssetClass::Equity).map_err(|error| {
            EquityIdentityError::CoreInvariant {
                message: error.to_string(),
            }
        })?;
    let resolution_source = if evidence.is_some() {
        IdentityResolutionSource::ProviderVerified
    } else {
        IdentityResolutionSource::CanonicalSegment
    };
    Ok(CanonicalEquityIdentity {
        instrument,
        canonical_code: canonical_code.to_owned(),
        share_class,
        segment,
        provider_evidence: evidence,
        resolution_source,
    })
}

fn classify_current_equity(
    code: &str,
) -> Result<(Exchange, EquityShareClass, EquitySegment), EquityIdentityError> {
    if ["43", "83", "87", "88"]
        .iter()
        .any(|prefix| code.starts_with(prefix))
    {
        return Err(EquityIdentityError::HistoricalAliasRequired {
            code: code.to_owned(),
        });
    }
    if code.starts_with("92") {
        return Ok((
            Exchange::Beijing,
            EquityShareClass::A,
            EquitySegment::BeijingA,
        ));
    }

    let Some(prefix) = code.get(..3) else {
        return Err(EquityIdentityError::UnsupportedEquityPrefix {
            code: code.to_owned(),
        });
    };
    let identity = match prefix {
        "600" | "601" | "603" | "605" => (
            Exchange::Shanghai,
            EquityShareClass::A,
            EquitySegment::ShanghaiMainA,
        ),
        "688" | "689" => (
            Exchange::Shanghai,
            EquityShareClass::A,
            EquitySegment::ShanghaiStarA,
        ),
        "900" => (
            Exchange::Shanghai,
            EquityShareClass::B,
            EquitySegment::ShanghaiB,
        ),
        "000" | "001" | "002" | "003" => (
            Exchange::Shenzhen,
            EquityShareClass::A,
            EquitySegment::ShenzhenMainA,
        ),
        "300" | "301" => (
            Exchange::Shenzhen,
            EquityShareClass::A,
            EquitySegment::ShenzhenChiNextA,
        ),
        "200" => (
            Exchange::Shenzhen,
            EquityShareClass::B,
            EquitySegment::ShenzhenB,
        ),
        _ => {
            return Err(EquityIdentityError::UnsupportedEquityPrefix {
                code: code.to_owned(),
            })
        }
    };
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use magic_market_core::{Exchange, ProviderId};

    use super::{resolve_production_equity, EquitySegment, EquityShareClass};

    #[test]
    fn production_resolves_shanghai_main_a_share() {
        let identity = resolve_production_equity("600001", None).expect("valid Shanghai A share");

        assert_eq!(identity.storage_code(), "600001");
        assert_eq!(identity.canonical_code(), "600001");
        assert_eq!(identity.exchange(), Exchange::Shanghai);
        assert_eq!(identity.share_class(), EquityShareClass::A);
        assert_eq!(identity.segment(), EquitySegment::ShanghaiMainA);
    }

    #[test]
    fn production_maps_every_registered_current_equity_segment() {
        let cases = [
            (
                "601001",
                Exchange::Shanghai,
                EquityShareClass::A,
                EquitySegment::ShanghaiMainA,
            ),
            (
                "603001",
                Exchange::Shanghai,
                EquityShareClass::A,
                EquitySegment::ShanghaiMainA,
            ),
            (
                "605001",
                Exchange::Shanghai,
                EquityShareClass::A,
                EquitySegment::ShanghaiMainA,
            ),
            (
                "688001",
                Exchange::Shanghai,
                EquityShareClass::A,
                EquitySegment::ShanghaiStarA,
            ),
            (
                "689001",
                Exchange::Shanghai,
                EquityShareClass::A,
                EquitySegment::ShanghaiStarA,
            ),
            (
                "900901",
                Exchange::Shanghai,
                EquityShareClass::B,
                EquitySegment::ShanghaiB,
            ),
            (
                "000001",
                Exchange::Shenzhen,
                EquityShareClass::A,
                EquitySegment::ShenzhenMainA,
            ),
            (
                "001001",
                Exchange::Shenzhen,
                EquityShareClass::A,
                EquitySegment::ShenzhenMainA,
            ),
            (
                "002001",
                Exchange::Shenzhen,
                EquityShareClass::A,
                EquitySegment::ShenzhenMainA,
            ),
            (
                "003001",
                Exchange::Shenzhen,
                EquityShareClass::A,
                EquitySegment::ShenzhenMainA,
            ),
            (
                "300001",
                Exchange::Shenzhen,
                EquityShareClass::A,
                EquitySegment::ShenzhenChiNextA,
            ),
            (
                "301001",
                Exchange::Shenzhen,
                EquityShareClass::A,
                EquitySegment::ShenzhenChiNextA,
            ),
            (
                "200001",
                Exchange::Shenzhen,
                EquityShareClass::B,
                EquitySegment::ShenzhenB,
            ),
            (
                "920001",
                Exchange::Beijing,
                EquityShareClass::A,
                EquitySegment::BeijingA,
            ),
            (
                "929999",
                Exchange::Beijing,
                EquityShareClass::A,
                EquitySegment::BeijingA,
            ),
        ];

        for (code, exchange, share_class, segment) in cases {
            let identity =
                resolve_production_equity(code, None).expect("registered current equity segment");
            assert_eq!(identity.exchange(), exchange, "{code}");
            assert_eq!(identity.share_class(), share_class, "{code}");
            assert_eq!(identity.segment(), segment, "{code}");
        }
    }

    #[test]
    fn production_rejects_non_exact_six_ascii_digit_codes() {
        for code in ["", " ", "60001", "600001 ", "６００００１", "60000A"] {
            let error = resolve_production_equity(code, None).expect_err("invalid equity code");
            assert_eq!(
                error,
                super::EquityIdentityError::InvalidEquityCode {
                    code: code.to_owned()
                },
                "{code:?}"
            );
        }
    }

    #[test]
    fn production_requires_official_mapping_for_historical_alias_ranges() {
        for code in ["430001", "830001", "870001", "880001"] {
            let error =
                resolve_production_equity(code, None).expect_err("historical alias is not current");
            assert_eq!(
                error,
                super::EquityIdentityError::HistoricalAliasRequired {
                    code: code.to_owned()
                },
                "{code}"
            );
        }

        let beijing_provider_evidence = super::ProviderExchangeEvidence::new(
            ProviderId::Tdx,
            "realtime_quote",
            Exchange::Beijing,
            "batch-old-code-1",
            Some("item-830001".to_owned()),
            "2026-07-27T09:30:00+08:00",
        )
        .expect("valid exchange evidence cannot establish current identity");
        assert_eq!(
            resolve_production_equity("830001", Some(beijing_provider_evidence))
                .expect_err("provider exchange cannot promote historical alias"),
            super::EquityIdentityError::HistoricalAliasRequired {
                code: "830001".to_owned()
            }
        );

        assert_eq!(
            resolve_production_equity("700001", None).expect_err("unregistered current prefix"),
            super::EquityIdentityError::UnsupportedEquityPrefix {
                code: "700001".to_owned()
            }
        );
    }

    #[test]
    fn a_share_guard_rejects_b_shares_without_relabeling_identity() {
        let b_share = resolve_production_equity("900901", None).expect("valid Shanghai B share");
        assert_eq!(
            b_share
                .require_a_share()
                .expect_err("B share is not A share"),
            super::EquityIdentityError::UnsupportedShareClass {
                code: "900901".to_owned(),
                share_class: EquityShareClass::B,
            }
        );
        assert_eq!(b_share.exchange(), Exchange::Shanghai);
        assert_eq!(
            b_share.instrument().asset_class(),
            magic_market_core::AssetClass::Equity
        );

        let a_share = resolve_production_equity("921001", None).expect("valid Beijing A share");
        assert_eq!(
            a_share
                .require_a_share()
                .expect("A-share consumer accepts A"),
            &a_share
        );
    }

    #[test]
    fn verified_provider_exchange_is_retained_and_cannot_rewrite_canonical_exchange() {
        let evidence = super::ProviderExchangeEvidence::new(
            ProviderId::Tdx,
            "realtime_quote",
            Exchange::Shanghai,
            "batch-600001-1",
            Some("item-600001".to_owned()),
            "2026-07-27T09:30:00+08:00",
        )
        .expect("valid provider exchange evidence");
        let identity = resolve_production_equity("600001", Some(evidence.clone()))
            .expect("matching provider exchange");
        assert_eq!(identity.provider_evidence(), Some(&evidence));
        assert_eq!(evidence.provider(), ProviderId::Tdx);
        assert_eq!(evidence.capability(), "realtime_quote");
        assert_eq!(evidence.exchange(), Exchange::Shanghai);
        assert_eq!(evidence.batch_id(), "batch-600001-1");
        assert_eq!(evidence.item_id(), Some("item-600001"));
        assert_eq!(evidence.observed_at(), "2026-07-27T09:30:00+08:00");
        assert_eq!(
            identity.resolution_source(),
            super::IdentityResolutionSource::ProviderVerified
        );

        let conflict = super::ProviderExchangeEvidence::new(
            ProviderId::Tdx,
            "realtime_quote",
            Exchange::Beijing,
            "batch-600001-2",
            Some("item-600001".to_owned()),
            "2026-07-27T09:30:01+08:00",
        )
        .expect("valid but conflicting provider evidence");
        assert_eq!(
            resolve_production_equity("600001", Some(conflict))
                .expect_err("provider exchange must match canonical exchange"),
            super::EquityIdentityError::ProviderMarketConflict {
                code: "600001".to_owned(),
                canonical_exchange: Exchange::Shanghai,
                provider: ProviderId::Tdx,
                provider_exchange: Exchange::Beijing,
            }
        );
    }

    #[test]
    fn provider_exchange_evidence_rejects_missing_or_malformed_fields() {
        let cases = [
            (
                "",
                "batch-1",
                Some("item-1".to_owned()),
                "2026-07-27T09:30:00+08:00",
                super::ProviderEvidenceField::Capability,
            ),
            (
                "quote",
                "",
                Some("item-1".to_owned()),
                "2026-07-27T09:30:00+08:00",
                super::ProviderEvidenceField::BatchId,
            ),
            (
                "quote",
                "batch-1",
                Some(String::new()),
                "2026-07-27T09:30:00+08:00",
                super::ProviderEvidenceField::ItemId,
            ),
            (
                "quote",
                "batch-1",
                Some("item-1".to_owned()),
                "2026-07-27 09:30:00",
                super::ProviderEvidenceField::ObservedAt,
            ),
        ];

        for (capability, batch_id, item_id, observed_at, field) in cases {
            let error = super::ProviderExchangeEvidence::new(
                ProviderId::Tdx,
                capability,
                Exchange::Shanghai,
                batch_id,
                item_id,
                observed_at,
            )
            .expect_err("malformed provider evidence");
            assert_eq!(error.invalid_provider_field(), Some(field));
        }

        for (value, field) in [
            (" quote", super::ProviderEvidenceField::Capability),
            ("batch\n1", super::ProviderEvidenceField::BatchId),
        ] {
            let (capability, batch_id) = if field == super::ProviderEvidenceField::Capability {
                (value, "batch-1")
            } else {
                ("quote", value)
            };
            let error = super::ProviderExchangeEvidence::new(
                ProviderId::Tdx,
                capability,
                Exchange::Shanghai,
                batch_id,
                Some("item-1".to_owned()),
                "2026-07-27T09:30:00+08:00",
            )
            .expect_err("non-canonical provider evidence text");
            assert_eq!(error.invalid_provider_field(), Some(field));
        }
    }

    #[test]
    fn provider_exchange_evidence_rejects_unverifiable_custom_provider_identity() {
        let error = super::ProviderExchangeEvidence::new(
            ProviderId::Custom,
            "quote",
            Exchange::Shanghai,
            "batch-1",
            Some("item-1".to_owned()),
            "2026-07-27T09:30:00+08:00",
        )
        .expect_err("unit Custom provider carries no verifiable provider identity");
        assert_eq!(
            error,
            super::EquityIdentityError::InvalidProviderEvidence {
                field: super::ProviderEvidenceField::Provider,
                issue: super::ProviderEvidenceIssue::UnverifiableCustomProvider,
            }
        );
    }

    #[test]
    fn production_and_test_resolvers_enforce_opposite_symbol_namespaces() {
        assert_eq!(
            resolve_production_equity("TEST_CODE_600001", None)
                .expect_err("production rejects test identity"),
            super::EquityIdentityError::TestIdentityInProduction {
                code: "TEST_CODE_600001".to_owned()
            }
        );
        assert_eq!(
            super::resolve_test_equity("600001", None)
                .expect_err("test resolver rejects bare real identity"),
            super::EquityIdentityError::RealIdentityInTest {
                code: "600001".to_owned()
            }
        );

        let test_identity = super::resolve_test_equity("TEST_CODE_929999", None)
            .expect("namespaced current Beijing identity");
        assert_eq!(test_identity.storage_code(), "TEST_CODE_929999");
        assert_eq!(test_identity.instrument().code(), "TEST_CODE_929999");
        assert_eq!(test_identity.canonical_code(), "929999");
        assert_eq!(test_identity.exchange(), Exchange::Beijing);
        assert_eq!(
            test_identity.resolution_source(),
            super::IdentityResolutionSource::CanonicalSegment
        );
    }
}
