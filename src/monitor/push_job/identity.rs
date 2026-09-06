//! Stable W01 identity contracts.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use sha2::{Digest, Sha256};

use super::{PushJobError, Result};

const TEXT_RULE: &str = "must be 1..=512 UTF-8 bytes, trimmed, and contain no NUL";

#[derive(Clone, Debug, Eq, PartialEq)]
enum CanonicalValue {
    Null,
    String(String),
    Object(BTreeMap<&'static str, CanonicalValue>),
}

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

    fn from_bytes(bytes: [u8; 32]) -> Self {
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

fn namespace_value(namespace: &Namespace) -> CanonicalValue {
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

fn canonical_preimage(
    domain: &'static str,
    fields: &BTreeMap<&'static str, CanonicalValue>,
) -> Vec<u8> {
    debug_assert!(domain.is_ascii() && !domain.contains('\0'));
    let mut preimage = Vec::with_capacity(domain.len() + 1 + fields.len() * 32);
    preimage.extend_from_slice(domain.as_bytes());
    preimage.push(0);
    write_json_object(&mut preimage, fields);
    preimage
}

fn write_json_object(output: &mut Vec<u8>, fields: &BTreeMap<&'static str, CanonicalValue>) {
    output.push(b'{');
    for (index, (key, value)) in fields.iter().enumerate() {
        if index != 0 {
            output.push(b',');
        }
        write_json_string(output, key);
        output.push(b':');
        write_json_value(output, value);
    }
    output.push(b'}');
}

fn write_json_value(output: &mut Vec<u8>, value: &CanonicalValue) {
    match value {
        CanonicalValue::Null => output.extend_from_slice(b"null"),
        CanonicalValue::String(value) => write_json_string(output, value),
        CanonicalValue::Object(fields) => write_json_object(output, fields),
    }
}

fn write_json_string(output: &mut Vec<u8>, value: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    output.push(b'"');
    for character in value.chars() {
        match character {
            '"' => output.extend_from_slice(br#"\""#),
            '\\' => output.extend_from_slice(br"\\"),
            '\u{0008}' => output.extend_from_slice(br"\b"),
            '\t' => output.extend_from_slice(br"\t"),
            '\n' => output.extend_from_slice(br"\n"),
            '\u{000c}' => output.extend_from_slice(br"\f"),
            '\r' => output.extend_from_slice(br"\r"),
            control if control <= '\u{001f}' => {
                let byte = control as u8;
                output.extend_from_slice(b"\\u00");
                output.push(HEX[usize::from(byte >> 4)]);
                output.push(HEX[usize::from(byte & 0x0f)]);
            }
            other => {
                let mut encoded = [0; 4];
                output.extend_from_slice(other.encode_utf8(&mut encoded).as_bytes());
            }
        }
    }
    output.push(b'"');
}

fn canonical_digest(
    domain: &'static str,
    fields: &BTreeMap<&'static str, CanonicalValue>,
) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(canonical_preimage(domain, fields));
    Sha256Digest::from_bytes(hasher.finalize().into())
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
