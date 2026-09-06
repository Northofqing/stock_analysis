//! W04 source-reference values. Prepared facts and the capture state are added in the next slice.

use std::collections::BTreeMap;

use super::canonical::CanonicalValue;
use super::identity::validate_text;
use super::{Result, Sha256Digest, SourceContractId};

macro_rules! fact_text {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn try_new(value: String) -> Result<Self> {
                validate_text($field, value).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

fact_text!(SourceRefId, "source_ref_id");
fact_text!(SourceProvider, "source_provider");
fact_text!(ExternalId, "external_id");

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SourceRef {
    source_ref_id: SourceRefId,
    provider: SourceProvider,
    external_id: ExternalId,
    source_contract_id: SourceContractId,
    content_sha256: Sha256Digest,
}

impl SourceRef {
    pub fn new(
        source_ref_id: SourceRefId,
        provider: SourceProvider,
        external_id: ExternalId,
        source_contract_id: SourceContractId,
        content_sha256: Sha256Digest,
    ) -> Self {
        Self {
            source_ref_id,
            provider,
            external_id,
            source_contract_id,
            content_sha256,
        }
    }

    pub fn source_ref_id(&self) -> &SourceRefId {
        &self.source_ref_id
    }

    pub fn provider(&self) -> &SourceProvider {
        &self.provider
    }

    pub fn external_id(&self) -> &ExternalId {
        &self.external_id
    }

    pub fn source_contract_id(&self) -> &SourceContractId {
        &self.source_contract_id
    }

    pub fn content_sha256(&self) -> &Sha256Digest {
        &self.content_sha256
    }
}

pub(super) fn source_ref_value(source_ref: &SourceRef) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "content_sha256",
            CanonicalValue::String(source_ref.content_sha256.as_str().to_owned()),
        ),
        (
            "external_id",
            CanonicalValue::String(source_ref.external_id.as_str().to_owned()),
        ),
        (
            "provider",
            CanonicalValue::String(source_ref.provider.as_str().to_owned()),
        ),
        (
            "source_contract_id",
            CanonicalValue::String(source_ref.source_contract_id.as_str().to_owned()),
        ),
        (
            "source_ref_id",
            CanonicalValue::String(source_ref.source_ref_id.as_str().to_owned()),
        ),
    ]))
}
