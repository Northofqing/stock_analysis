//! Stable W01 identity contracts.

use std::collections::BTreeMap;

use chrono::NaiveDate;

#[cfg(test)]
use super::canonical::canonical_preimage;
use super::canonical::{canonical_digest, CanonicalValue};
use super::{PushJobError, Result};

const TEXT_RULE: &str = "must be 1..=512 UTF-8 bytes, trimmed, and contain no NUL";

pub(super) fn validate_text(field: &'static str, value: String) -> Result<String> {
    let valid =
        !value.is_empty() && value.len() <= 512 && !value.contains('\0') && value.trim() == value;
    if !valid {
        return Err(PushJobError::InvalidText {
            field,
            reason: TEXT_RULE,
        });
    }
    Ok(value)
}

macro_rules! text_id {
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

text_id!(OccurrenceFamily, "occurrence_family");
text_id!(OccurrenceKey, "occurrence_key");
text_id!(AudienceId, "audience_id");
text_id!(CalendarId, "calendar_id");
text_id!(CompletionOwnerId, "completion_owner");
text_id!(ProducerId, "producer_id");
text_id!(RunId, "run_id");
text_id!(ScheduleOrTriggerId, "schedule_or_trigger_id");
text_id!(SourceContractId, "source_contract_id");
text_id!(SourceContractVersion, "source_contract_version");
text_id!(SubjectValue, "subject_id");
text_id!(UnitId, "unit_id");

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum Namespace {
    Production,
    Test { run_id: RunId },
}

impl Namespace {
    pub fn test(run_id: RunId) -> Self {
        Self::Test { run_id }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum SubjectId {
    Global,
    Entity(SubjectValue),
}

impl SubjectId {
    pub fn entity(value: String) -> Result<Self> {
        SubjectValue::try_new(value).map(Self::Entity)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn parse(field: &'static str, value: &str) -> Result<Self> {
        let valid = value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            return Err(PushJobError::InvalidSha256 { field });
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(super) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(hex::encode(bytes))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct UtcMicros(i64);

impl UtcMicros {
    pub fn try_new(value: i64) -> Result<Self> {
        if value < 0 {
            return Err(PushJobError::InvalidUtcMicros);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct BusinessDate(String);

impl BusinessDate {
    pub fn parse(value: &str) -> Result<Self> {
        let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map_err(|_| PushJobError::InvalidBusinessDate(value.to_owned()))?;
        if parsed.format("%Y-%m-%d").to_string() != value {
            return Err(PushJobError::InvalidBusinessDate(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct OccurrenceId(String);

impl OccurrenceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ScheduleOccurrenceId(String);

impl ScheduleOccurrenceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct IntentId(String);

impl IntentId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OccurrenceIdentityMaterial {
    business_date: BusinessDate,
    occurrence_family: OccurrenceFamily,
    occurrence_key: OccurrenceKey,
}

impl OccurrenceIdentityMaterial {
    pub fn new(
        business_date: BusinessDate,
        occurrence_family: OccurrenceFamily,
        occurrence_key: OccurrenceKey,
    ) -> Self {
        Self {
            business_date,
            occurrence_family,
            occurrence_key,
        }
    }

    pub fn business_date(&self) -> &BusinessDate {
        &self.business_date
    }

    pub fn occurrence_family(&self) -> &OccurrenceFamily {
        &self.occurrence_family
    }

    pub fn occurrence_key(&self) -> &OccurrenceKey {
        &self.occurrence_key
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleOccurrenceIdentityMaterial {
    namespace: Namespace,
    unit_id: UnitId,
    producer_id: ProducerId,
    schedule_or_trigger_id: ScheduleOrTriggerId,
    calendar_id: CalendarId,
    occurrence: OccurrenceIdentityMaterial,
    completion_owner: CompletionOwnerId,
    source_contract_id: SourceContractId,
}

impl ScheduleOccurrenceIdentityMaterial {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        namespace: Namespace,
        unit_id: UnitId,
        producer_id: ProducerId,
        schedule_or_trigger_id: ScheduleOrTriggerId,
        calendar_id: CalendarId,
        occurrence: OccurrenceIdentityMaterial,
        completion_owner: CompletionOwnerId,
        source_contract_id: SourceContractId,
    ) -> Self {
        Self {
            namespace,
            unit_id,
            producer_id,
            schedule_or_trigger_id,
            calendar_id,
            occurrence,
            completion_owner,
            source_contract_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntentIdentityMaterial {
    namespace: Namespace,
    unit_id: UnitId,
    completion_owner: CompletionOwnerId,
    source_contract_id: SourceContractId,
    occurrence: OccurrenceId,
    subject: SubjectId,
    audience: AudienceId,
}

impl IntentIdentityMaterial {
    pub fn new(
        namespace: Namespace,
        unit_id: UnitId,
        completion_owner: CompletionOwnerId,
        source_contract_id: SourceContractId,
        occurrence: OccurrenceId,
        subject: SubjectId,
        audience: AudienceId,
    ) -> Self {
        Self {
            namespace,
            unit_id,
            completion_owner,
            source_contract_id,
            occurrence,
            subject,
            audience,
        }
    }
}

pub(super) fn namespace_value(namespace: &Namespace) -> CanonicalValue {
    let (kind, run_id) = match namespace {
        Namespace::Production => ("Production", CanonicalValue::Null),
        Namespace::Test { run_id } => ("Test", CanonicalValue::String(run_id.as_str().to_owned())),
    };
    CanonicalValue::Object(BTreeMap::from([
        ("kind", CanonicalValue::String(kind.to_owned())),
        ("run_id", run_id),
    ]))
}

fn subject_value(subject: &SubjectId) -> CanonicalValue {
    let (kind, value) = match subject {
        SubjectId::Global => ("Global", CanonicalValue::Null),
        SubjectId::Entity(value) => ("Entity", CanonicalValue::String(value.as_str().to_owned())),
    };
    CanonicalValue::Object(BTreeMap::from([
        ("kind", CanonicalValue::String(kind.to_owned())),
        ("value", value),
    ]))
}

fn occurrence_fields(
    material: &OccurrenceIdentityMaterial,
) -> BTreeMap<&'static str, CanonicalValue> {
    BTreeMap::from([
        (
            "business_date",
            CanonicalValue::String(material.business_date.as_str().to_owned()),
        ),
        (
            "occurrence_family",
            CanonicalValue::String(material.occurrence_family.as_str().to_owned()),
        ),
        (
            "occurrence_key",
            CanonicalValue::String(material.occurrence_key.as_str().to_owned()),
        ),
    ])
}

pub fn derive_occurrence_id(material: &OccurrenceIdentityMaterial) -> OccurrenceId {
    let digest = canonical_digest("OccurrenceId/v1", &occurrence_fields(material));
    OccurrenceId(digest.as_str().to_owned())
}

#[cfg(test)]
pub(super) fn occurrence_preimage_fixture(material: &OccurrenceIdentityMaterial) -> Vec<u8> {
    canonical_preimage("OccurrenceId/v1", &occurrence_fields(material))
}

#[cfg(test)]
pub(super) fn canonical_string_preimage_fixture(value: &str) -> Vec<u8> {
    canonical_preimage(
        "Fixture/v1",
        &BTreeMap::from([("value", CanonicalValue::String(value.to_owned()))]),
    )
}

pub fn derive_schedule_occurrence_id(
    material: &ScheduleOccurrenceIdentityMaterial,
) -> ScheduleOccurrenceId {
    let occurrence = &material.occurrence;
    let fields = BTreeMap::from([
        (
            "business_date",
            CanonicalValue::String(occurrence.business_date.as_str().to_owned()),
        ),
        (
            "calendar_id",
            CanonicalValue::String(material.calendar_id.as_str().to_owned()),
        ),
        (
            "completion_owner",
            CanonicalValue::String(material.completion_owner.as_str().to_owned()),
        ),
        ("namespace", namespace_value(&material.namespace)),
        (
            "occurrence_family",
            CanonicalValue::String(occurrence.occurrence_family.as_str().to_owned()),
        ),
        (
            "occurrence_key",
            CanonicalValue::String(occurrence.occurrence_key.as_str().to_owned()),
        ),
        (
            "producer_id",
            CanonicalValue::String(material.producer_id.as_str().to_owned()),
        ),
        (
            "schedule_or_trigger_id",
            CanonicalValue::String(material.schedule_or_trigger_id.as_str().to_owned()),
        ),
        (
            "schema_version",
            CanonicalValue::String("ScheduleOccurrence/v1".to_owned()),
        ),
        (
            "source_contract_id",
            CanonicalValue::String(material.source_contract_id.as_str().to_owned()),
        ),
        (
            "unit_id",
            CanonicalValue::String(material.unit_id.as_str().to_owned()),
        ),
    ]);
    let digest = canonical_digest("ScheduleOccurrence/v1", &fields);
    ScheduleOccurrenceId(digest.as_str().to_owned())
}

pub fn derive_intent_id(material: &IntentIdentityMaterial) -> IntentId {
    let fields = BTreeMap::from([
        (
            "audience",
            CanonicalValue::String(material.audience.as_str().to_owned()),
        ),
        (
            "completion_owner",
            CanonicalValue::String(material.completion_owner.as_str().to_owned()),
        ),
        ("namespace", namespace_value(&material.namespace)),
        (
            "occurrence",
            CanonicalValue::String(material.occurrence.as_str().to_owned()),
        ),
        (
            "source_contract_id",
            CanonicalValue::String(material.source_contract_id.as_str().to_owned()),
        ),
        ("subject", subject_value(&material.subject)),
        (
            "unit_id",
            CanonicalValue::String(material.unit_id.as_str().to_owned()),
        ),
    ]);
    let digest = canonical_digest("PreparedPushIntent/v1", &fields);
    IntentId(digest.as_str().to_owned())
}
