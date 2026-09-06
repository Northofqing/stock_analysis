//! W06 exact-byte, typed machine catalog registry. Activation and production wiring start later.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::identity::validate_text;
use super::{
    CompletionOwnerId, GitSha40, MonitorKind, OccurrenceFamily, PhaseEpic, ProducerId,
    Sha256Digest, UnitId,
};

const BUNDLED_CATALOG_BYTES: &[u8] =
    include_bytes!("../../../docs/push-system/push-capability-catalog.v1.json");
const BUNDLED_CATALOG_SHA256: &str =
    "0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3";
const EXPECTED_KIND_COUNT: usize = 65;
const EXPECTED_PRODUCER_COUNT: usize = 102;
const EXPECTED_UNIT_COUNT: usize = 52;
const EXPECTED_ENUM_EXTERNAL_PRODUCER_COUNT: usize = 10;
const EXPECTED_ENUM_EXTERNAL_PRODUCER_IDS: [&str; 10] = [
    "chain-post-close-timer",
    "chain-preopen-timer",
    "cli-chain",
    "cli-replay-force",
    "cli-single-default",
    "cli-single-lhb",
    "cli-single-schedule",
    "cli-summary-default",
    "cli-summary-lhb",
    "cli-summary-schedule",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MachineCatalogStatus {
    Provisional,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum CatalogStatus {
    Active,
    Inactive,
    Starved,
    OptIn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogEntity {
    Header,
    Kind,
    Producer,
    Unit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogRelation {
    KindProducer,
    ProducerUnit,
    UnitProducer,
    CompletionOwner,
    OccurrenceFamilies,
    PhaseEpics,
    DuplicateCompletionOwner,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MachineCatalogError {
    #[error("machine catalog bytes do not match expected SHA-256")]
    DigestMismatch {
        expected: Sha256Digest,
        actual: Sha256Digest,
    },
    #[error("invalid machine catalog JSON at line {line}, column {column}")]
    InvalidJson { line: usize, column: usize },
    #[error("unsupported machine catalog schema version {actual}")]
    UnsupportedSchemaVersion { actual: u32 },
    #[error("unsupported machine catalog status")]
    UnsupportedCatalogStatus,
    #[error("machine catalog {entity:?} count mismatch: expected {expected}, got {actual}")]
    CountMismatch {
        entity: CatalogEntity,
        expected: usize,
        actual: usize,
    },
    #[error(
        "machine catalog status count mismatch for {status:?}: expected {expected}, got {actual}"
    )]
    StatusCountMismatch {
        status: CatalogStatus,
        expected: usize,
        actual: usize,
    },
    #[error(
        "machine catalog enum-external producer count mismatch: expected {expected}, got {actual}"
    )]
    EnumExternalCountMismatch { expected: usize, actual: usize },
    #[error("machine catalog enum-external producer identity set mismatch")]
    EnumExternalIdentityMismatch,
    #[error("invalid machine catalog {field} for {entity:?}")]
    InvalidValue {
        entity: CatalogEntity,
        field: &'static str,
    },
    #[error("duplicate machine catalog {entity:?} id {id}")]
    DuplicateId { entity: CatalogEntity, id: String },
    #[error("producer {producer_id} has {actual} monitor kinds; expected zero or one")]
    ProducerKindCardinality { producer_id: String, actual: usize },
    #[error("empty {field} registration for {entity:?} {id}")]
    EmptyMembers {
        entity: CatalogEntity,
        field: &'static str,
        id: String,
    },
    #[error("duplicate member in {field} registration for {entity:?} {id}")]
    DuplicateMember {
        entity: CatalogEntity,
        field: &'static str,
        id: String,
    },
    #[error("machine catalog relationship mismatch {relation:?} at {id}")]
    RelationshipMismatch {
        relation: CatalogRelation,
        id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogKindRegistration {
    kind: MonitorKind,
    primary_phase: PhaseEpic,
    status: CatalogStatus,
    producer_ids: Vec<ProducerId>,
}

impl CatalogKindRegistration {
    pub fn kind(&self) -> MonitorKind {
        self.kind
    }

    pub fn primary_phase(&self) -> PhaseEpic {
        self.primary_phase
    }

    pub fn status(&self) -> CatalogStatus {
        self.status
    }

    pub fn producer_ids(&self) -> &[ProducerId] {
        &self.producer_ids
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogProducerRegistration {
    id: ProducerId,
    monitor_kind: Option<MonitorKind>,
    phase_epics: Vec<PhaseEpic>,
    occurrence_family: OccurrenceFamily,
    completion_owner: CompletionOwnerId,
    unit_id: UnitId,
}

impl CatalogProducerRegistration {
    pub fn id(&self) -> &ProducerId {
        &self.id
    }

    pub fn monitor_kind(&self) -> Option<MonitorKind> {
        self.monitor_kind
    }

    pub fn phase_epics(&self) -> &[PhaseEpic] {
        &self.phase_epics
    }

    pub fn occurrence_family(&self) -> &OccurrenceFamily {
        &self.occurrence_family
    }

    pub fn completion_owner(&self) -> &CompletionOwnerId {
        &self.completion_owner
    }

    pub fn unit_id(&self) -> &UnitId {
        &self.unit_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogUnitRegistration {
    id: UnitId,
    completion_owner: CompletionOwnerId,
    producer_ids: Vec<ProducerId>,
    occurrence_families: Vec<OccurrenceFamily>,
    phase_epics: Vec<PhaseEpic>,
}

impl CatalogUnitRegistration {
    pub fn id(&self) -> &UnitId {
        &self.id
    }

    pub fn completion_owner(&self) -> &CompletionOwnerId {
        &self.completion_owner
    }

    pub fn producer_ids(&self) -> &[ProducerId] {
        &self.producer_ids
    }

    pub fn occurrence_families(&self) -> &[OccurrenceFamily] {
        &self.occurrence_families
    }

    pub fn phase_epics(&self) -> &[PhaseEpic] {
        &self.phase_epics
    }
}

#[derive(Debug)]
pub struct MachineCatalog {
    schema_version: u32,
    status: MachineCatalogStatus,
    baseline_commit: GitSha40,
    enum_evidence_id: String,
    catalog_sha256: Sha256Digest,
    kinds: Vec<CatalogKindRegistration>,
    producers: Vec<CatalogProducerRegistration>,
    units: Vec<CatalogUnitRegistration>,
    kind_index: BTreeMap<MonitorKind, usize>,
    producer_index: BTreeMap<ProducerId, usize>,
    unit_index: BTreeMap<UnitId, usize>,
}

impl MachineCatalog {
    pub fn bundled() -> Result<Self, MachineCatalogError> {
        let expected =
            Sha256Digest::parse("catalog_sha256", BUNDLED_CATALOG_SHA256).map_err(|_| {
                MachineCatalogError::InvalidValue {
                    entity: CatalogEntity::Header,
                    field: "bundled_catalog_sha256",
                }
            })?;
        Self::parse_v1_exact(BUNDLED_CATALOG_BYTES, &expected)
    }

    pub(super) fn parse_v1_exact(
        bytes: &[u8],
        expected_sha256: &Sha256Digest,
    ) -> Result<Self, MachineCatalogError> {
        let actual_sha256 = digest(bytes);
        if &actual_sha256 != expected_sha256 {
            return Err(MachineCatalogError::DigestMismatch {
                expected: expected_sha256.clone(),
                actual: actual_sha256,
            });
        }

        let raw: RawMachineCatalog =
            serde_json::from_slice(bytes).map_err(|error| MachineCatalogError::InvalidJson {
                line: error.line(),
                column: error.column(),
            })?;
        if raw.schema_version != 1 {
            return Err(MachineCatalogError::UnsupportedSchemaVersion {
                actual: raw.schema_version,
            });
        }
        if raw.status != "PROVISIONAL" {
            return Err(MachineCatalogError::UnsupportedCatalogStatus);
        }
        validate_count(CatalogEntity::Kind, EXPECTED_KIND_COUNT, raw.kinds.len())?;
        validate_count(
            CatalogEntity::Producer,
            EXPECTED_PRODUCER_COUNT,
            raw.producers.len(),
        )?;
        validate_count(
            CatalogEntity::Unit,
            EXPECTED_UNIT_COUNT,
            raw.migration_units.len(),
        )?;

        let baseline_commit = GitSha40::parse(&raw.baseline_commit).map_err(|_| {
            MachineCatalogError::InvalidValue {
                entity: CatalogEntity::Header,
                field: "baseline_commit",
            }
        })?;
        let enum_evidence_id =
            validate_text("enum_evidence_id", raw.enum_evidence_id).map_err(|_| {
                MachineCatalogError::InvalidValue {
                    entity: CatalogEntity::Header,
                    field: "enum_evidence_id",
                }
            })?;

        let kinds = raw
            .kinds
            .into_iter()
            .map(parse_kind)
            .collect::<Result<Vec<_>, _>>()?;
        let producers = raw
            .producers
            .into_iter()
            .map(parse_producer)
            .collect::<Result<Vec<_>, _>>()?;
        let units = raw
            .migration_units
            .into_iter()
            .map(parse_unit)
            .collect::<Result<Vec<_>, _>>()?;
        let kind_index = unique_index(
            CatalogEntity::Kind,
            kinds
                .iter()
                .enumerate()
                .map(|(index, entry)| (entry.kind, entry.kind.as_str().to_owned(), index)),
        )?;
        let producer_index = unique_index(
            CatalogEntity::Producer,
            producers
                .iter()
                .enumerate()
                .map(|(index, entry)| (entry.id.clone(), entry.id.as_str().to_owned(), index)),
        )?;
        let unit_index = unique_index(
            CatalogEntity::Unit,
            units
                .iter()
                .enumerate()
                .map(|(index, entry)| (entry.id.clone(), entry.id.as_str().to_owned(), index)),
        )?;

        let catalog = Self {
            schema_version: raw.schema_version,
            status: MachineCatalogStatus::Provisional,
            baseline_commit,
            enum_evidence_id,
            catalog_sha256: actual_sha256,
            kinds,
            producers,
            units,
            kind_index,
            producer_index,
            unit_index,
        };
        catalog.validate_closure()?;
        Ok(catalog)
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn status(&self) -> MachineCatalogStatus {
        self.status
    }

    pub fn baseline_commit(&self) -> &GitSha40 {
        &self.baseline_commit
    }

    pub fn enum_evidence_id(&self) -> &str {
        &self.enum_evidence_id
    }

    pub fn catalog_sha256(&self) -> &Sha256Digest {
        &self.catalog_sha256
    }

    pub fn kinds(&self) -> &[CatalogKindRegistration] {
        &self.kinds
    }

    pub fn producers(&self) -> &[CatalogProducerRegistration] {
        &self.producers
    }

    pub fn units(&self) -> &[CatalogUnitRegistration] {
        &self.units
    }

    pub fn kind(&self, kind: MonitorKind) -> Option<&CatalogKindRegistration> {
        self.kind_index.get(&kind).map(|index| &self.kinds[*index])
    }

    pub fn producer(&self, id: &ProducerId) -> Option<&CatalogProducerRegistration> {
        self.producer_index
            .get(id)
            .map(|index| &self.producers[*index])
    }

    pub fn unit(&self, id: &UnitId) -> Option<&CatalogUnitRegistration> {
        self.unit_index.get(id).map(|index| &self.units[*index])
    }

    pub fn enum_external_producers(&self) -> impl Iterator<Item = &CatalogProducerRegistration> {
        self.producers
            .iter()
            .filter(|producer| producer.monitor_kind.is_none())
    }

    pub fn producers_for_kind(&self, kind: MonitorKind) -> Vec<&CatalogProducerRegistration> {
        self.kind(kind)
            .into_iter()
            .flat_map(|entry| entry.producer_ids.iter())
            .filter_map(|id| self.producer(id))
            .collect()
    }

    pub fn producers_for_unit(&self, id: &UnitId) -> Vec<&CatalogProducerRegistration> {
        self.unit(id)
            .into_iter()
            .flat_map(|entry| entry.producer_ids.iter())
            .filter_map(|producer_id| self.producer(producer_id))
            .collect()
    }

    pub fn unit_for_producer(&self, id: &ProducerId) -> Option<&CatalogUnitRegistration> {
        self.producer(id)
            .and_then(|producer| self.unit(&producer.unit_id))
    }

    fn validate_closure(&self) -> Result<(), MachineCatalogError> {
        self.validate_status_counts()?;

        let external_count = self
            .producers
            .iter()
            .filter(|producer| producer.monitor_kind.is_none())
            .count();
        if external_count != EXPECTED_ENUM_EXTERNAL_PRODUCER_COUNT {
            return Err(MachineCatalogError::EnumExternalCountMismatch {
                expected: EXPECTED_ENUM_EXTERNAL_PRODUCER_COUNT,
                actual: external_count,
            });
        }
        let actual_external_ids = self
            .producers
            .iter()
            .filter(|producer| producer.monitor_kind.is_none())
            .map(|producer| producer.id.as_str())
            .collect::<BTreeSet<_>>();
        let expected_external_ids = EXPECTED_ENUM_EXTERNAL_PRODUCER_IDS
            .into_iter()
            .collect::<BTreeSet<_>>();
        if actual_external_ids != expected_external_ids {
            return Err(MachineCatalogError::EnumExternalIdentityMismatch);
        }

        for kind in &self.kinds {
            validate_members(
                CatalogEntity::Kind,
                kind.kind.as_str(),
                "producer_ids",
                &kind.producer_ids,
                false,
            )?;
        }
        for producer in &self.producers {
            validate_members(
                CatalogEntity::Producer,
                producer.id.as_str(),
                "phase_epics",
                &producer.phase_epics,
                true,
            )?;
        }
        for unit in &self.units {
            validate_members(
                CatalogEntity::Unit,
                unit.id.as_str(),
                "producer_ids",
                &unit.producer_ids,
                true,
            )?;
            validate_members(
                CatalogEntity::Unit,
                unit.id.as_str(),
                "occurrence_families",
                &unit.occurrence_families,
                true,
            )?;
            validate_members(
                CatalogEntity::Unit,
                unit.id.as_str(),
                "phase_epics",
                &unit.phase_epics,
                true,
            )?;
        }

        self.validate_completion_owner_uniqueness()?;
        self.validate_kind_producer_closure()?;
        self.validate_unit_producer_closure()?;
        self.validate_unit_material()?;
        Ok(())
    }

    fn validate_status_counts(&self) -> Result<(), MachineCatalogError> {
        for (status, expected) in [
            (CatalogStatus::Active, 36),
            (CatalogStatus::Inactive, 22),
            (CatalogStatus::Starved, 5),
            (CatalogStatus::OptIn, 2),
        ] {
            let actual = self
                .kinds
                .iter()
                .filter(|kind| kind.status == status)
                .count();
            if actual != expected {
                return Err(MachineCatalogError::StatusCountMismatch {
                    status,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }

    fn validate_completion_owner_uniqueness(&self) -> Result<(), MachineCatalogError> {
        let mut owner_units = BTreeMap::new();
        for unit in &self.units {
            if owner_units
                .insert(unit.completion_owner.clone(), unit.id.clone())
                .is_some()
            {
                return Err(MachineCatalogError::RelationshipMismatch {
                    relation: CatalogRelation::DuplicateCompletionOwner,
                    id: unit.id.as_str().to_owned(),
                });
            }
        }
        Ok(())
    }

    fn validate_kind_producer_closure(&self) -> Result<(), MachineCatalogError> {
        for expected_kind in MonitorKind::ALL {
            let kind = self.kind(expected_kind).ok_or_else(|| {
                MachineCatalogError::RelationshipMismatch {
                    relation: CatalogRelation::KindProducer,
                    id: expected_kind.as_str().to_owned(),
                }
            })?;
            let registered = kind.producer_ids.iter().collect::<BTreeSet<_>>();
            let reverse = self
                .producers
                .iter()
                .filter(|producer| producer.monitor_kind == Some(expected_kind))
                .map(|producer| &producer.id)
                .collect::<BTreeSet<_>>();
            if registered != reverse {
                return Err(MachineCatalogError::RelationshipMismatch {
                    relation: CatalogRelation::KindProducer,
                    id: expected_kind.as_str().to_owned(),
                });
            }
        }
        Ok(())
    }

    fn validate_unit_producer_closure(&self) -> Result<(), MachineCatalogError> {
        for producer in &self.producers {
            if self.unit(&producer.unit_id).is_none() {
                return Err(MachineCatalogError::RelationshipMismatch {
                    relation: CatalogRelation::ProducerUnit,
                    id: producer.id.as_str().to_owned(),
                });
            }
        }

        for unit in &self.units {
            let registered = unit.producer_ids.iter().collect::<BTreeSet<_>>();
            let reverse = self
                .producers
                .iter()
                .filter(|producer| producer.unit_id == unit.id)
                .map(|producer| &producer.id)
                .collect::<BTreeSet<_>>();
            if registered != reverse {
                return Err(MachineCatalogError::RelationshipMismatch {
                    relation: CatalogRelation::UnitProducer,
                    id: unit.id.as_str().to_owned(),
                });
            }
        }
        Ok(())
    }

    fn validate_unit_material(&self) -> Result<(), MachineCatalogError> {
        for unit in &self.units {
            let producers = unit
                .producer_ids
                .iter()
                .map(|id| {
                    self.producer(id)
                        .ok_or_else(|| MachineCatalogError::RelationshipMismatch {
                            relation: CatalogRelation::UnitProducer,
                            id: unit.id.as_str().to_owned(),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;

            if producers
                .iter()
                .any(|producer| producer.completion_owner != unit.completion_owner)
            {
                return Err(MachineCatalogError::RelationshipMismatch {
                    relation: CatalogRelation::CompletionOwner,
                    id: unit.id.as_str().to_owned(),
                });
            }

            let registered_families = unit.occurrence_families.iter().collect::<BTreeSet<_>>();
            let producer_families = producers
                .iter()
                .map(|producer| &producer.occurrence_family)
                .collect::<BTreeSet<_>>();
            if registered_families != producer_families {
                return Err(MachineCatalogError::RelationshipMismatch {
                    relation: CatalogRelation::OccurrenceFamilies,
                    id: unit.id.as_str().to_owned(),
                });
            }

            let registered_phases = unit.phase_epics.iter().collect::<BTreeSet<_>>();
            let producer_phases = producers
                .iter()
                .flat_map(|producer| producer.phase_epics.iter())
                .collect::<BTreeSet<_>>();
            if registered_phases != producer_phases {
                return Err(MachineCatalogError::RelationshipMismatch {
                    relation: CatalogRelation::PhaseEpics,
                    id: unit.id.as_str().to_owned(),
                });
            }
        }
        Ok(())
    }
}

fn digest(bytes: &[u8]) -> Sha256Digest {
    let bytes: [u8; 32] = Sha256::digest(bytes).into();
    Sha256Digest::from_bytes(bytes)
}

fn validate_count(
    entity: CatalogEntity,
    expected: usize,
    actual: usize,
) -> Result<(), MachineCatalogError> {
    if actual != expected {
        return Err(MachineCatalogError::CountMismatch {
            entity,
            expected,
            actual,
        });
    }
    Ok(())
}

fn validate_members<T: Ord>(
    entity: CatalogEntity,
    id: &str,
    field: &'static str,
    values: &[T],
    require_nonempty: bool,
) -> Result<(), MachineCatalogError> {
    if require_nonempty && values.is_empty() {
        return Err(MachineCatalogError::EmptyMembers {
            entity,
            field,
            id: id.to_owned(),
        });
    }
    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err(MachineCatalogError::DuplicateMember {
            entity,
            field,
            id: id.to_owned(),
        });
    }
    Ok(())
}

fn parse_kind(raw: RawKind) -> Result<CatalogKindRegistration, MachineCatalogError> {
    let kind = MonitorKind::try_from(raw.kind.as_str()).map_err(|_| {
        MachineCatalogError::InvalidValue {
            entity: CatalogEntity::Kind,
            field: "kind",
        }
    })?;
    let producer_ids = raw
        .producer_ids
        .into_iter()
        .map(|value| {
            ProducerId::try_new(value).map_err(|_| MachineCatalogError::InvalidValue {
                entity: CatalogEntity::Kind,
                field: "producer_ids",
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(CatalogKindRegistration {
        kind,
        primary_phase: parse_phase(&raw.primary_phase, CatalogEntity::Kind)?,
        status: parse_status(&raw.status)?,
        producer_ids,
    })
}

fn parse_producer(raw: RawProducer) -> Result<CatalogProducerRegistration, MachineCatalogError> {
    let producer_id_text = raw.id.clone();
    let id = ProducerId::try_new(raw.id).map_err(|_| MachineCatalogError::InvalidValue {
        entity: CatalogEntity::Producer,
        field: "id",
    })?;
    if raw.kinds.len() > 1 {
        return Err(MachineCatalogError::ProducerKindCardinality {
            producer_id: producer_id_text,
            actual: raw.kinds.len(),
        });
    }
    let monitor_kind = raw
        .kinds
        .first()
        .map(|kind| MonitorKind::try_from(kind.as_str()))
        .transpose()
        .map_err(|_| MachineCatalogError::InvalidValue {
            entity: CatalogEntity::Producer,
            field: "kinds",
        })?;
    let phase_epics = raw
        .phase_epics
        .iter()
        .map(|phase| parse_phase(phase, CatalogEntity::Producer))
        .collect::<Result<_, _>>()?;
    let occurrence_family = OccurrenceFamily::try_new(raw.occurrence_family).map_err(|_| {
        MachineCatalogError::InvalidValue {
            entity: CatalogEntity::Producer,
            field: "occurrence_family",
        }
    })?;
    let completion_owner = CompletionOwnerId::try_new(raw.completion_owner).map_err(|_| {
        MachineCatalogError::InvalidValue {
            entity: CatalogEntity::Producer,
            field: "completion_owner",
        }
    })?;
    let unit_id =
        UnitId::try_new(raw.migration_unit_id).map_err(|_| MachineCatalogError::InvalidValue {
            entity: CatalogEntity::Producer,
            field: "migration_unit_id",
        })?;
    Ok(CatalogProducerRegistration {
        id,
        monitor_kind,
        phase_epics,
        occurrence_family,
        completion_owner,
        unit_id,
    })
}

fn parse_unit(raw: RawUnit) -> Result<CatalogUnitRegistration, MachineCatalogError> {
    let id = UnitId::try_new(raw.id).map_err(|_| MachineCatalogError::InvalidValue {
        entity: CatalogEntity::Unit,
        field: "id",
    })?;
    let completion_owner = CompletionOwnerId::try_new(raw.completion_owner).map_err(|_| {
        MachineCatalogError::InvalidValue {
            entity: CatalogEntity::Unit,
            field: "completion_owner",
        }
    })?;
    let producer_ids = raw
        .producer_ids
        .into_iter()
        .map(|value| {
            ProducerId::try_new(value).map_err(|_| MachineCatalogError::InvalidValue {
                entity: CatalogEntity::Unit,
                field: "producer_ids",
            })
        })
        .collect::<Result<_, _>>()?;
    let occurrence_families = raw
        .occurrence_families
        .into_iter()
        .map(|value| {
            OccurrenceFamily::try_new(value).map_err(|_| MachineCatalogError::InvalidValue {
                entity: CatalogEntity::Unit,
                field: "occurrence_families",
            })
        })
        .collect::<Result<_, _>>()?;
    let phase_epics = raw
        .phase_epics
        .iter()
        .map(|phase| parse_phase(phase, CatalogEntity::Unit))
        .collect::<Result<_, _>>()?;
    Ok(CatalogUnitRegistration {
        id,
        completion_owner,
        producer_ids,
        occurrence_families,
        phase_epics,
    })
}

fn parse_status(value: &str) -> Result<CatalogStatus, MachineCatalogError> {
    match value {
        "ACTIVE" => Ok(CatalogStatus::Active),
        "INACTIVE" => Ok(CatalogStatus::Inactive),
        "STARVED" => Ok(CatalogStatus::Starved),
        "OPT-IN" => Ok(CatalogStatus::OptIn),
        _ => Err(MachineCatalogError::InvalidValue {
            entity: CatalogEntity::Kind,
            field: "status",
        }),
    }
}

fn parse_phase(value: &str, entity: CatalogEntity) -> Result<PhaseEpic, MachineCatalogError> {
    match value {
        "盘前" => Ok(PhaseEpic::Preopen),
        "集合竞价" => Ok(PhaseEpic::Auction),
        "盘中" => Ok(PhaseEpic::Intraday),
        "盘后" => Ok(PhaseEpic::Postclose),
        _ => Err(MachineCatalogError::InvalidValue {
            entity,
            field: "phase",
        }),
    }
}

fn unique_index<K, I>(
    entity: CatalogEntity,
    entries: I,
) -> Result<BTreeMap<K, usize>, MachineCatalogError>
where
    K: Ord,
    I: IntoIterator<Item = (K, String, usize)>,
{
    let mut index = BTreeMap::new();
    for (key, label, position) in entries {
        if index.insert(key, position).is_some() {
            return Err(MachineCatalogError::DuplicateId { entity, id: label });
        }
    }
    Ok(index)
}

#[derive(Deserialize)]
struct RawMachineCatalog {
    schema_version: u32,
    status: String,
    baseline_commit: String,
    enum_evidence_id: String,
    kinds: Vec<RawKind>,
    producers: Vec<RawProducer>,
    migration_units: Vec<RawUnit>,
}

#[derive(Deserialize)]
struct RawKind {
    kind: String,
    primary_phase: String,
    status: String,
    producer_ids: Vec<String>,
}

#[derive(Deserialize)]
struct RawProducer {
    id: String,
    kinds: Vec<String>,
    phase_epics: Vec<String>,
    occurrence_family: String,
    completion_owner: String,
    migration_unit_id: String,
}

#[derive(Deserialize)]
struct RawUnit {
    id: String,
    completion_owner: String,
    producer_ids: Vec<String>,
    occurrence_families: Vec<String>,
    phase_epics: Vec<String>,
}
