//! Full-catalog deployment observations for W16 readiness.
//!
//! Values in this module are candidates assembled from raw activation facts and untrusted
//! deployment declarations. They do not authenticate a source, owner, binary, or execution right.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use crate::monitor::push_job::{
    canonical_preimage, namespace_value, raw_digest, CalendarId, CanonicalValue, MachineCatalog,
    Namespace, ProducerId, Sha256Digest, SourceContractId, SourceContractVersion, UnitId,
};

use super::activation::DesiredActivationState;
use super::activation_deployment::{
    validate_activation_calendar_declaration, ActivationCalendarClaims,
};
use super::activation_facts::{ActivationReconciliation, RawActivationFacts};
use super::activation_store::inspect_raw_activation_facts;
use super::operational_readiness::{required_kinds, DependencyKind, ReadinessScope};

const DEPLOYMENT_SET_DOMAIN: &str = "ActivationDeploymentSet/v1";
const DEPLOYMENT_SET_SCHEMA_VERSION: u64 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SourcePackageDeclaration {
    namespace: Namespace,
    unit_id: UnitId,
    generation: u64,
    manifest_sha256: Sha256Digest,
    source_binding_sha256: Sha256Digest,
}

impl SourcePackageDeclaration {
    pub(super) fn new(
        namespace: Namespace,
        unit_id: UnitId,
        generation: u64,
        manifest_sha256: Sha256Digest,
        source_binding_sha256: Sha256Digest,
    ) -> Self {
        Self {
            namespace,
            unit_id,
            generation,
            manifest_sha256,
            source_binding_sha256,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SharedDependencyDeclaration {
    kind: DependencyKind,
    contract_id: SourceContractId,
    contract_version: SourceContractVersion,
    sha256: Sha256Digest,
}

impl SharedDependencyDeclaration {
    pub(super) fn new(
        kind: DependencyKind,
        contract_id: SourceContractId,
        contract_version: SourceContractVersion,
        sha256: Sha256Digest,
    ) -> Self {
        Self {
            kind,
            contract_id,
            contract_version,
            sha256,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActivationDeploymentSetRequest {
    namespace: Namespace,
    calendar: ActivationCalendarClaims,
    observed_at: u64,
    enabled_producers: Vec<ProducerId>,
    recovery_units: Vec<UnitId>,
    source_packages: Vec<SourcePackageDeclaration>,
    shared_dependencies: Vec<SharedDependencyDeclaration>,
}

impl ActivationDeploymentSetRequest {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        namespace: Namespace,
        calendar: ActivationCalendarClaims,
        observed_at: u64,
        enabled_producers: Vec<ProducerId>,
        recovery_units: Vec<UnitId>,
        source_packages: Vec<SourcePackageDeclaration>,
        shared_dependencies: Vec<SharedDependencyDeclaration>,
    ) -> Self {
        Self {
            namespace,
            calendar,
            observed_at,
            enabled_producers,
            recovery_units,
            source_packages,
            shared_dependencies,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
struct CalendarDeclaration {
    calendar_id: CalendarId,
    authority_sha256: Sha256Digest,
    utc_offset_seconds: u64,
}

#[derive(Clone, Eq, PartialEq)]
enum UnitDeploymentObservation {
    Unregistered {
        unit_id: UnitId,
    },
    CaughtUp {
        unit_id: UnitId,
        generation: u64,
        manifest_sha256: Sha256Digest,
        journal_event_id: Sha256Digest,
        journal_sha256: Sha256Digest,
        desired_state: DesiredActivationState,
        physical_owner: String,
        build_commit: String,
        build_sha256: Sha256Digest,
        source_binding_sha256: Sha256Digest,
    },
}

impl UnitDeploymentObservation {
    fn unit_id(&self) -> &UnitId {
        match self {
            Self::Unregistered { unit_id } | Self::CaughtUp { unit_id, .. } => unit_id,
        }
    }

    fn is_registered(&self) -> bool {
        matches!(self, Self::CaughtUp { .. })
    }

    fn generation(&self) -> Option<u64> {
        match self {
            Self::Unregistered { .. } => None,
            Self::CaughtUp { generation, .. } => Some(*generation),
        }
    }
}

/// A deterministic, non-authorizing full-catalog deployment candidate.
#[derive(Clone, Eq, PartialEq)]
pub(super) struct ActivationDeploymentSet {
    namespace: Namespace,
    catalog_sha256: Sha256Digest,
    calendar: CalendarDeclaration,
    enabled_producers: Vec<ProducerId>,
    recovery_units: Vec<UnitId>,
    shared_dependencies: Vec<SharedDependencyDeclaration>,
    units: Vec<UnitDeploymentObservation>,
    canonical_bytes: Vec<u8>,
    deployment_set_sha256: Sha256Digest,
}

impl fmt::Debug for ActivationDeploymentSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivationDeploymentSet")
            .field("deployment_set_sha256", &self.deployment_set_sha256)
            .field("unit_count", &self.units.len())
            .field(
                "registered_unit_count",
                &self
                    .units
                    .iter()
                    .filter(|unit| unit.is_registered())
                    .count(),
            )
            .field("enabled_producer_count", &self.enabled_producers.len())
            .field("recovery_unit_count", &self.recovery_units.len())
            .finish_non_exhaustive()
    }
}

impl ActivationDeploymentSet {
    pub(super) fn sha256(&self) -> &Sha256Digest {
        &self.deployment_set_sha256
    }

    pub(super) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    pub(super) fn unit_generations(&self) -> Vec<(&UnitId, Option<u64>)> {
        self.units
            .iter()
            .map(|unit| (unit.unit_id(), unit.generation()))
            .collect()
    }

    pub(super) fn unit_ids_for_scope(
        &self,
        scope: &ReadinessScope,
    ) -> Result<Vec<UnitId>, ActivationDeploymentSetError> {
        let catalog =
            MachineCatalog::bundled().map_err(|_| ActivationDeploymentSetError::CatalogRejected)?;
        if catalog.catalog_sha256() != &self.catalog_sha256 {
            return Err(ActivationDeploymentSetError::ScopeRejected);
        }
        let enabled = self
            .enabled_producers
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        match scope {
            ReadinessScope::Core => {
                let mut units = self.recovery_units.iter().cloned().collect::<BTreeSet<_>>();
                for producer_id in &self.enabled_producers {
                    let producer = catalog
                        .producer(producer_id)
                        .ok_or(ActivationDeploymentSetError::ScopeRejected)?;
                    units.insert(producer.unit_id().clone());
                }
                Ok(units.into_iter().collect())
            }
            ReadinessScope::Producer {
                unit_id,
                producer_id,
            }
            | ReadinessScope::Occurrence {
                unit_id,
                producer_id,
                ..
            } => {
                let producer = catalog
                    .producer(producer_id)
                    .ok_or(ActivationDeploymentSetError::ScopeRejected)?;
                if producer.unit_id() != unit_id || !enabled.contains(producer_id) {
                    return Err(ActivationDeploymentSetError::ScopeRejected);
                }
                Ok(vec![unit_id.clone()])
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum ActivationDeploymentSetError {
    #[error("bundled deployment catalog is unavailable")]
    CatalogRejected,
    #[error("activation deployment facts are unavailable or invalid")]
    ActivationFactsRejected,
    #[error("activation deployment facts do not cover the catalog exactly")]
    UnitCoverageRejected,
    #[error("activation deployment has a pending unit")]
    PendingUnit,
    #[error("activation deployment configuration is invalid")]
    ConfigurationRejected,
    #[error("activation deployment calendar declaration is invalid")]
    CalendarRejected,
    #[error("activation deployment source declarations are invalid")]
    SourceDeclarationRejected,
    #[error("activation deployment shared dependencies are invalid")]
    SharedDependenciesRejected,
    #[error("activation deployment changed while being consumed")]
    DeploymentChanged,
    #[error("readiness scope is not enabled by this deployment")]
    ScopeRejected,
}

/// Read the complete raw activation history once and assemble a full-catalog candidate.
pub(super) fn read_activation_deployment_set(
    database: &Path,
    selected_unit: &UnitId,
    request: ActivationDeploymentSetRequest,
) -> Result<ActivationDeploymentSet, ActivationDeploymentSetError> {
    let facts = inspect_raw_activation_facts(database, selected_unit)
        .map_err(|_| ActivationDeploymentSetError::ActivationFactsRejected)?;
    construct_activation_deployment_set(&facts, request)
}

/// Re-read every input and reject reuse if any complete-set field changed.
pub(super) fn reread_activation_deployment_set(
    previous: &ActivationDeploymentSet,
    database: &Path,
    selected_unit: &UnitId,
    request: ActivationDeploymentSetRequest,
) -> Result<ActivationDeploymentSet, ActivationDeploymentSetError> {
    let current = read_activation_deployment_set(database, selected_unit, request)?;
    if &current != previous {
        return Err(ActivationDeploymentSetError::DeploymentChanged);
    }
    Ok(current)
}

pub(super) fn construct_activation_deployment_set(
    facts: &RawActivationFacts,
    request: ActivationDeploymentSetRequest,
) -> Result<ActivationDeploymentSet, ActivationDeploymentSetError> {
    let catalog =
        MachineCatalog::bundled().map_err(|_| ActivationDeploymentSetError::CatalogRejected)?;
    validate_fact_coverage(&catalog, facts)?;
    if request.calendar.namespace != request.namespace {
        return Err(ActivationDeploymentSetError::CalendarRejected);
    }
    validate_activation_calendar_declaration(&catalog, &request.calendar, request.observed_at)
        .map_err(|_| ActivationDeploymentSetError::CalendarRejected)?;

    let enabled_producers = validate_enabled_producers(&catalog, request.enabled_producers)?;
    let recovery_units = validate_recovery_units(&catalog, request.recovery_units)?;
    let shared_dependencies = validate_shared_dependencies(request.shared_dependencies)?;
    let sources = validate_source_keys(&catalog, &request.namespace, request.source_packages)?;
    let units = assemble_units(facts, &sources)?;

    let registered = units
        .iter()
        .map(|unit| (unit.unit_id(), unit.is_registered()))
        .collect::<BTreeMap<_, _>>();
    for producer_id in &enabled_producers {
        let unit_id = catalog
            .producer(producer_id)
            .ok_or(ActivationDeploymentSetError::ConfigurationRejected)?
            .unit_id();
        if registered.get(unit_id) != Some(&true) {
            return Err(ActivationDeploymentSetError::ConfigurationRejected);
        }
    }
    if recovery_units
        .iter()
        .any(|unit_id| registered.get(unit_id) != Some(&true))
    {
        return Err(ActivationDeploymentSetError::ConfigurationRejected);
    }

    let calendar = CalendarDeclaration {
        calendar_id: request.calendar.calendar_id,
        authority_sha256: request.calendar.authority_sha256,
        utc_offset_seconds: u64::try_from(request.calendar.utc_offset_seconds)
            .map_err(|_| ActivationDeploymentSetError::CalendarRejected)?,
    };
    let fields = deployment_set_fields(
        &request.namespace,
        catalog.catalog_sha256(),
        &calendar,
        &enabled_producers,
        &recovery_units,
        &shared_dependencies,
        &units,
    );
    let canonical_bytes = canonical_preimage(DEPLOYMENT_SET_DOMAIN, &fields);
    let deployment_set_sha256 = raw_digest(&canonical_bytes);
    Ok(ActivationDeploymentSet {
        namespace: request.namespace,
        catalog_sha256: catalog.catalog_sha256().clone(),
        calendar,
        enabled_producers,
        recovery_units,
        shared_dependencies,
        units,
        canonical_bytes,
        deployment_set_sha256,
    })
}

fn validate_fact_coverage(
    catalog: &MachineCatalog,
    facts: &RawActivationFacts,
) -> Result<(), ActivationDeploymentSetError> {
    let actual = facts
        .units()
        .iter()
        .map(|unit| unit.unit_id())
        .collect::<BTreeSet<_>>();
    let expected = catalog
        .units()
        .iter()
        .map(|unit| unit.id())
        .collect::<BTreeSet<_>>();
    if actual.len() != facts.units().len() || actual != expected {
        return Err(ActivationDeploymentSetError::UnitCoverageRejected);
    }
    Ok(())
}

fn validate_enabled_producers(
    catalog: &MachineCatalog,
    producers: Vec<ProducerId>,
) -> Result<Vec<ProducerId>, ActivationDeploymentSetError> {
    let mut unique = BTreeSet::new();
    for producer in producers {
        if catalog.producer(&producer).is_none() || !unique.insert(producer) {
            return Err(ActivationDeploymentSetError::ConfigurationRejected);
        }
    }
    Ok(unique.into_iter().collect())
}

fn validate_recovery_units(
    catalog: &MachineCatalog,
    units: Vec<UnitId>,
) -> Result<Vec<UnitId>, ActivationDeploymentSetError> {
    let mut unique = BTreeSet::new();
    for unit in units {
        if catalog.unit(&unit).is_none() || !unique.insert(unit) {
            return Err(ActivationDeploymentSetError::ConfigurationRejected);
        }
    }
    Ok(unique.into_iter().collect())
}

fn validate_shared_dependencies(
    dependencies: Vec<SharedDependencyDeclaration>,
) -> Result<Vec<SharedDependencyDeclaration>, ActivationDeploymentSetError> {
    let expected = required_kinds(&ReadinessScope::Core);
    let mut actual = BTreeMap::new();
    for dependency in dependencies {
        let kind = dependency.kind;
        if actual.insert(kind, dependency).is_some() {
            return Err(ActivationDeploymentSetError::SharedDependenciesRejected);
        }
    }
    if actual.keys().copied().collect::<BTreeSet<_>>() != expected {
        return Err(ActivationDeploymentSetError::SharedDependenciesRejected);
    }
    let mut validated = actual.into_values().collect::<Vec<_>>();
    validated.sort_by_key(|dependency| dependency.kind.as_str());
    Ok(validated)
}

fn validate_source_keys(
    catalog: &MachineCatalog,
    namespace: &Namespace,
    sources: Vec<SourcePackageDeclaration>,
) -> Result<BTreeMap<UnitId, SourcePackageDeclaration>, ActivationDeploymentSetError> {
    let mut by_unit = BTreeMap::new();
    for source in sources {
        if &source.namespace != namespace
            || catalog.unit(&source.unit_id).is_none()
            || source.generation == 0
            || by_unit.insert(source.unit_id.clone(), source).is_some()
        {
            return Err(ActivationDeploymentSetError::SourceDeclarationRejected);
        }
    }
    Ok(by_unit)
}

fn assemble_units(
    facts: &RawActivationFacts,
    sources: &BTreeMap<UnitId, SourcePackageDeclaration>,
) -> Result<Vec<UnitDeploymentObservation>, ActivationDeploymentSetError> {
    let mut units = Vec::with_capacity(facts.units().len());
    let mut used_sources = BTreeSet::new();
    for fact in facts.units() {
        match fact.reconciliation() {
            ActivationReconciliation::Pending { .. } => {
                return Err(ActivationDeploymentSetError::PendingUnit);
            }
            ActivationReconciliation::Unregistered => {
                if sources.contains_key(fact.unit_id()) {
                    return Err(ActivationDeploymentSetError::SourceDeclarationRejected);
                }
                units.push(UnitDeploymentObservation::Unregistered {
                    unit_id: fact.unit_id().clone(),
                });
            }
            ActivationReconciliation::CaughtUp { generation } => {
                let manifest = fact
                    .manifests()
                    .last()
                    .ok_or(ActivationDeploymentSetError::ActivationFactsRejected)?;
                let journal = fact
                    .journal()
                    .last()
                    .ok_or(ActivationDeploymentSetError::ActivationFactsRejected)?;
                let source = sources
                    .get(fact.unit_id())
                    .ok_or(ActivationDeploymentSetError::SourceDeclarationRejected)?;
                if manifest.generation() != generation
                    || journal.generation() != generation
                    || journal.to_manifest_sha256() != manifest.manifest_sha256()
                    || source.generation != generation
                    || &source.manifest_sha256 != manifest.manifest_sha256()
                    || &source.source_binding_sha256 != manifest.source_contract_sha256()
                {
                    return Err(ActivationDeploymentSetError::SourceDeclarationRejected);
                }
                used_sources.insert(fact.unit_id().clone());
                units.push(UnitDeploymentObservation::CaughtUp {
                    unit_id: fact.unit_id().clone(),
                    generation,
                    manifest_sha256: manifest.manifest_sha256().clone(),
                    journal_event_id: journal.event_id().clone(),
                    journal_sha256: journal.canonical_sha256().clone(),
                    desired_state: manifest.desired_state(),
                    physical_owner: manifest.physical_owner().to_owned(),
                    build_commit: manifest.build_commit().as_str().to_owned(),
                    build_sha256: manifest.build_sha256().clone(),
                    source_binding_sha256: source.source_binding_sha256.clone(),
                });
            }
        }
    }
    if used_sources.len() != sources.len() {
        return Err(ActivationDeploymentSetError::SourceDeclarationRejected);
    }
    units.sort_by(|left, right| left.unit_id().cmp(right.unit_id()));
    Ok(units)
}

fn deployment_set_fields(
    namespace: &Namespace,
    catalog_sha256: &Sha256Digest,
    calendar: &CalendarDeclaration,
    enabled_producers: &[ProducerId],
    recovery_units: &[UnitId],
    shared_dependencies: &[SharedDependencyDeclaration],
    units: &[UnitDeploymentObservation],
) -> BTreeMap<&'static str, CanonicalValue> {
    BTreeMap::from([
        (
            "schema_version",
            CanonicalValue::Unsigned(DEPLOYMENT_SET_SCHEMA_VERSION),
        ),
        ("namespace", namespace_value(namespace)),
        ("catalog_sha256", string(catalog_sha256.as_str())),
        (
            "calendar",
            CanonicalValue::Object(BTreeMap::from([
                ("calendar_id", string(calendar.calendar_id.as_str())),
                (
                    "authority_sha256",
                    string(calendar.authority_sha256.as_str()),
                ),
                (
                    "utc_offset_seconds",
                    CanonicalValue::Unsigned(calendar.utc_offset_seconds),
                ),
            ])),
        ),
        (
            "enabled_producers",
            CanonicalValue::Array(
                enabled_producers
                    .iter()
                    .map(|producer| string(producer.as_str()))
                    .collect(),
            ),
        ),
        (
            "recovery_units",
            CanonicalValue::Array(
                recovery_units
                    .iter()
                    .map(|unit| string(unit.as_str()))
                    .collect(),
            ),
        ),
        (
            "shared_dependencies",
            CanonicalValue::Array(
                shared_dependencies
                    .iter()
                    .map(shared_dependency_value)
                    .collect(),
            ),
        ),
        (
            "units",
            CanonicalValue::Array(units.iter().map(unit_value).collect()),
        ),
    ])
}

fn shared_dependency_value(dependency: &SharedDependencyDeclaration) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        ("dependency_kind", string(dependency.kind.as_str())),
        ("contract_id", string(dependency.contract_id.as_str())),
        (
            "contract_version",
            string(dependency.contract_version.as_str()),
        ),
        ("sha256", string(dependency.sha256.as_str())),
    ]))
}

fn unit_value(unit: &UnitDeploymentObservation) -> CanonicalValue {
    let fields = match unit {
        UnitDeploymentObservation::Unregistered { unit_id } => BTreeMap::from([
            ("unit_id", string(unit_id.as_str())),
            ("activation_status", string("Unregistered")),
            ("generation", CanonicalValue::Null),
            ("manifest_sha256", CanonicalValue::Null),
            ("journal_event_id", CanonicalValue::Null),
            ("journal_sha256", CanonicalValue::Null),
            ("desired_state", CanonicalValue::Null),
            ("physical_owner", CanonicalValue::Null),
            ("build_commit", CanonicalValue::Null),
            ("build_sha256", CanonicalValue::Null),
            ("source_binding_sha256", CanonicalValue::Null),
        ]),
        UnitDeploymentObservation::CaughtUp {
            unit_id,
            generation,
            manifest_sha256,
            journal_event_id,
            journal_sha256,
            desired_state,
            physical_owner,
            build_commit,
            build_sha256,
            source_binding_sha256,
        } => BTreeMap::from([
            ("unit_id", string(unit_id.as_str())),
            ("activation_status", string("CaughtUp")),
            ("generation", CanonicalValue::Unsigned(*generation)),
            ("manifest_sha256", string(manifest_sha256.as_str())),
            ("journal_event_id", string(journal_event_id.as_str())),
            ("journal_sha256", string(journal_sha256.as_str())),
            ("desired_state", string(desired_state.as_str())),
            ("physical_owner", string(physical_owner)),
            ("build_commit", string(build_commit)),
            ("build_sha256", string(build_sha256.as_str())),
            (
                "source_binding_sha256",
                string(source_binding_sha256.as_str()),
            ),
        ]),
    };
    CanonicalValue::Object(fields)
}

fn string(value: &str) -> CanonicalValue {
    CanonicalValue::String(value.to_owned())
}

/// Fixed codec sample for a literal golden test. It bypasses set construction deliberately so the
/// wire contract can be pinned independently of catalog and database fixture churn.
#[cfg(test)]
pub(super) fn deployment_set_codec_fixture(physical_owner: &str, build_commit: &str) -> Vec<u8> {
    use crate::monitor::push_job::RunId;

    let test_namespace = Namespace::test(
        RunId::try_new("golden-run".to_owned()).expect("TEST_CODE golden namespace"),
    );
    let calendar = CalendarDeclaration {
        calendar_id: CalendarId::try_new("golden-calendar".to_owned())
            .expect("TEST_CODE golden calendar"),
        authority_sha256: Sha256Digest::parse("TEST_CODE golden authority", &"b".repeat(64))
            .expect("TEST_CODE golden authority"),
        utc_offset_seconds: 28_800,
    };
    let dependency = |kind, id: &str, version: &str, sha: char| {
        SharedDependencyDeclaration::new(
            kind,
            SourceContractId::try_new(id.to_owned()).expect("TEST_CODE golden contract"),
            SourceContractVersion::try_new(version.to_owned()).expect("TEST_CODE golden version"),
            Sha256Digest::parse("TEST_CODE golden dependency", &sha.to_string().repeat(64))
                .expect("TEST_CODE golden dependency"),
        )
    };
    let dependencies = vec![
        dependency(DependencyKind::Audit, "golden-audit", "v1", '1'),
        dependency(DependencyKind::Durable, "golden-durable", "v2", '2'),
        dependency(DependencyKind::Manifest, "golden-manifest", "v3", '3'),
        dependency(DependencyKind::Namespace, "golden-namespace", "v4", '4'),
        dependency(DependencyKind::Schema, "golden-schema", "v5", '5'),
        dependency(
            DependencyKind::TypedAuthority,
            "golden-typed-authority",
            "v6",
            '6',
        ),
    ];
    let units = vec![
        UnitDeploymentObservation::CaughtUp {
            unit_id: UnitId::try_new("unit-a".to_owned()).expect("TEST_CODE golden unit"),
            generation: 7,
            manifest_sha256: Sha256Digest::parse("TEST_CODE golden manifest", &"d".repeat(64))
                .expect("TEST_CODE golden manifest"),
            journal_event_id: Sha256Digest::parse("TEST_CODE golden event", &"e".repeat(64))
                .expect("TEST_CODE golden event"),
            journal_sha256: Sha256Digest::parse("TEST_CODE golden journal", &"f".repeat(64))
                .expect("TEST_CODE golden journal"),
            desired_state: DesiredActivationState::Shadow,
            physical_owner: physical_owner.to_owned(),
            build_commit: build_commit.to_owned(),
            build_sha256: Sha256Digest::parse("TEST_CODE golden build", &"2".repeat(64))
                .expect("TEST_CODE golden build"),
            source_binding_sha256: Sha256Digest::parse(
                "TEST_CODE golden source binding",
                &"3".repeat(64),
            )
            .expect("TEST_CODE golden source binding"),
        },
        UnitDeploymentObservation::Unregistered {
            unit_id: UnitId::try_new("unit-b".to_owned()).expect("TEST_CODE golden unit"),
        },
    ];
    canonical_preimage(
        DEPLOYMENT_SET_DOMAIN,
        &deployment_set_fields(
            &test_namespace,
            &Sha256Digest::parse("TEST_CODE golden catalog", &"a".repeat(64))
                .expect("TEST_CODE golden catalog"),
            &calendar,
            &[ProducerId::try_new("producer-a".to_owned()).expect("TEST_CODE golden producer")],
            &[UnitId::try_new("unit-b".to_owned()).expect("TEST_CODE golden recovery")],
            &dependencies,
            &units,
        ),
    )
}
