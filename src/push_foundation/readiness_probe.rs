//! Pure catalog inventory for a future read-only readiness probe.
//!
//! This candidate view is not an attested snapshot and grants no deployment or execution
//! authority.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};

use crate::monitor::push_job::{CatalogStatus, MachineCatalog, MonitorKind, ProducerId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DependencyAvailability {
    Available,
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReceiptMode {
    Strong,
    Compatibility,
}

impl ReceiptMode {
    fn satisfies(self, required: Self) -> bool {
        matches!(
            (required, self),
            (Self::Strong, Self::Strong) | (Self::Compatibility, _)
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReceiptReadiness {
    Available {
        required: ReceiptMode,
        observed: ReceiptMode,
    },
    Missing {
        required: ReceiptMode,
    },
}

impl ReceiptReadiness {
    fn is_ready(self) -> bool {
        match self {
            Self::Available { required, observed } => observed.satisfies(required),
            Self::Missing { .. } => false,
        }
    }

    fn observed(self) -> Option<ReceiptMode> {
        match self {
            Self::Available { observed, .. } => Some(observed),
            Self::Missing { .. } => None,
        }
    }
}

/// The eight named producer roles required before a producer can be considered ready.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProducerDependencyReadiness {
    pub(crate) producer_binding: DependencyAvailability,
    pub(crate) source_contract: DependencyAvailability,
    pub(crate) schedule_or_trigger: DependencyAvailability,
    pub(crate) presentation: DependencyAvailability,
    pub(crate) durable_policy: DependencyAvailability,
    pub(crate) receipt: ReceiptReadiness,
    pub(crate) feature_gate: DependencyAvailability,
    pub(crate) completion_policy: DependencyAvailability,
}

impl ProducerDependencyReadiness {
    fn is_ready(&self) -> bool {
        self.producer_binding == DependencyAvailability::Available
            && self.source_contract == DependencyAvailability::Available
            && self.schedule_or_trigger == DependencyAvailability::Available
            && self.presentation == DependencyAvailability::Available
            && self.durable_policy == DependencyAvailability::Available
            && self.receipt.is_ready()
            && self.feature_gate == DependencyAvailability::Available
            && self.completion_policy == DependencyAvailability::Available
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProducerReadinessFact {
    producer_id: ProducerId,
    dependencies: ProducerDependencyReadiness,
}

impl ProducerReadinessFact {
    pub(crate) fn new(producer_id: ProducerId, dependencies: ProducerDependencyReadiness) -> Self {
        Self {
            producer_id,
            dependencies,
        }
    }

    pub(crate) fn producer_id(&self) -> &ProducerId {
        &self.producer_id
    }

    pub(crate) fn dependencies(&self) -> &ProducerDependencyReadiness {
        &self.dependencies
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CandidateReadinessInventoryError {
    #[error("readiness fact references unknown producer {producer_id:?}")]
    UnknownProducer { producer_id: ProducerId },
    #[error("duplicate readiness fact for producer {producer_id:?}")]
    DuplicateProducerFact { producer_id: ProducerId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProducerCoverage {
    total: usize,
    ready_ids: Vec<ProducerId>,
    conditional_or_disabled_ids: Vec<ProducerId>,
    unassessed_ids: Vec<ProducerId>,
    failed_ids: Vec<ProducerId>,
}

impl ProducerCoverage {
    pub(crate) fn total(&self) -> usize {
        self.total
    }

    pub(crate) fn ready_ids(&self) -> &[ProducerId] {
        &self.ready_ids
    }

    pub(crate) fn conditional_or_disabled_ids(&self) -> &[ProducerId] {
        &self.conditional_or_disabled_ids
    }

    pub(crate) fn unassessed_ids(&self) -> &[ProducerId] {
        &self.unassessed_ids
    }

    pub(crate) fn failed_ids(&self) -> &[ProducerId] {
        &self.failed_ids
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidateReadinessInventory {
    push_total: usize,
    producer_total: usize,
    enum_external_producer_total: usize,
    ready_kind_ids: Vec<MonitorKind>,
    conditional_kind_ids: Vec<MonitorKind>,
    compat_kind_ids: Vec<MonitorKind>,
    inactive_kind_ids: Vec<MonitorKind>,
    schedule_unreachable_kind_ids: Vec<MonitorKind>,
    producer_missing_kind_ids: Vec<MonitorKind>,
    source_missing_kind_ids: Vec<MonitorKind>,
    presentation_missing_kind_ids: Vec<MonitorKind>,
    durable_policy_missing_kind_ids: Vec<MonitorKind>,
    producer_facts: Vec<ProducerReadinessFact>,
    producer_coverage: ProducerCoverage,
    enum_external_producer_coverage: ProducerCoverage,
}

impl CandidateReadinessInventory {
    pub(crate) fn evaluate(
        catalog: &MachineCatalog,
        facts: &[ProducerReadinessFact],
    ) -> Result<Self, CandidateReadinessInventoryError> {
        let facts = index_facts(catalog, facts)?;
        let mut ready_kind_ids = BTreeSet::new();
        let mut conditional_kind_ids = BTreeSet::new();
        let mut compat_kind_ids = BTreeSet::new();
        let mut inactive_kind_ids = BTreeSet::new();
        let mut schedule_unreachable_kind_ids = BTreeSet::new();
        let mut producer_missing_kind_ids = BTreeSet::new();
        let mut source_missing_kind_ids = BTreeSet::new();
        let mut presentation_missing_kind_ids = BTreeSet::new();
        let mut durable_policy_missing_kind_ids = BTreeSet::new();

        for registration in catalog.kinds() {
            let kind = registration.kind();
            match registration.status() {
                CatalogStatus::Active => {
                    let producer_facts = registration
                        .producer_ids()
                        .iter()
                        .map(|producer_id| facts.get(producer_id))
                        .collect::<Vec<_>>();
                    if producer_facts.is_empty() || producer_facts.iter().any(|fact| fact.is_none())
                    {
                        producer_missing_kind_ids.insert(kind);
                    }
                    if !producer_facts.is_empty()
                        && producer_facts
                            .iter()
                            .all(|fact| fact.is_some_and(|fact| fact.dependencies.is_ready()))
                    {
                        ready_kind_ids.insert(kind);
                    }
                }
                CatalogStatus::Inactive => {
                    inactive_kind_ids.insert(kind);
                }
                CatalogStatus::Starved | CatalogStatus::OptIn => {
                    conditional_kind_ids.insert(kind);
                }
            }

            for fact in registration
                .producer_ids()
                .iter()
                .filter_map(|producer_id| facts.get(producer_id))
            {
                let dependencies = &fact.dependencies;
                if dependencies.producer_binding == DependencyAvailability::Missing {
                    producer_missing_kind_ids.insert(kind);
                }
                if dependencies.source_contract == DependencyAvailability::Missing {
                    source_missing_kind_ids.insert(kind);
                }
                if dependencies.schedule_or_trigger == DependencyAvailability::Missing {
                    schedule_unreachable_kind_ids.insert(kind);
                }
                if dependencies.presentation == DependencyAvailability::Missing {
                    presentation_missing_kind_ids.insert(kind);
                }
                if dependencies.durable_policy == DependencyAvailability::Missing {
                    durable_policy_missing_kind_ids.insert(kind);
                }
                if dependencies.receipt.observed() == Some(ReceiptMode::Compatibility) {
                    compat_kind_ids.insert(kind);
                }
            }
        }

        let producer_coverage = build_producer_coverage(catalog, &facts, false);
        let enum_external_producer_coverage = build_producer_coverage(catalog, &facts, true);
        let producer_facts = facts.values().map(|fact| (*fact).clone()).collect();
        Ok(Self {
            push_total: catalog.kinds().len(),
            producer_total: catalog.producers().len(),
            enum_external_producer_total: catalog.enum_external_producers().count(),
            ready_kind_ids: ready_kind_ids.into_iter().collect(),
            conditional_kind_ids: conditional_kind_ids.into_iter().collect(),
            compat_kind_ids: compat_kind_ids.into_iter().collect(),
            inactive_kind_ids: inactive_kind_ids.into_iter().collect(),
            schedule_unreachable_kind_ids: schedule_unreachable_kind_ids.into_iter().collect(),
            producer_missing_kind_ids: producer_missing_kind_ids.into_iter().collect(),
            source_missing_kind_ids: source_missing_kind_ids.into_iter().collect(),
            presentation_missing_kind_ids: presentation_missing_kind_ids.into_iter().collect(),
            durable_policy_missing_kind_ids: durable_policy_missing_kind_ids.into_iter().collect(),
            producer_facts,
            producer_coverage,
            enum_external_producer_coverage,
        })
    }

    pub(crate) fn push_total(&self) -> usize {
        self.push_total
    }

    pub(crate) fn producer_total(&self) -> usize {
        self.producer_total
    }

    pub(crate) fn enum_external_producer_total(&self) -> usize {
        self.enum_external_producer_total
    }

    pub(crate) fn ready(&self) -> usize {
        self.ready_kind_ids.len()
    }

    pub(crate) fn conditional(&self) -> usize {
        self.conditional_kind_ids.len()
    }

    pub(crate) fn compat(&self) -> usize {
        self.compat_kind_ids.len()
    }

    pub(crate) fn inactive(&self) -> usize {
        self.inactive_kind_ids.len()
    }

    pub(crate) fn schedule_unreachable(&self) -> usize {
        self.schedule_unreachable_kind_ids.len()
    }

    pub(crate) fn producer_missing(&self) -> usize {
        self.producer_missing_kind_ids.len()
    }

    pub(crate) fn source_missing(&self) -> usize {
        self.source_missing_kind_ids.len()
    }

    pub(crate) fn presentation_missing(&self) -> usize {
        self.presentation_missing_kind_ids.len()
    }

    pub(crate) fn durable_policy_missing(&self) -> usize {
        self.durable_policy_missing_kind_ids.len()
    }

    pub(crate) fn ready_kind_ids(&self) -> &[MonitorKind] {
        &self.ready_kind_ids
    }

    pub(crate) fn conditional_kind_ids(&self) -> &[MonitorKind] {
        &self.conditional_kind_ids
    }

    pub(crate) fn compat_kind_ids(&self) -> &[MonitorKind] {
        &self.compat_kind_ids
    }

    pub(crate) fn inactive_kind_ids(&self) -> &[MonitorKind] {
        &self.inactive_kind_ids
    }

    pub(crate) fn schedule_unreachable_kind_ids(&self) -> &[MonitorKind] {
        &self.schedule_unreachable_kind_ids
    }

    pub(crate) fn producer_missing_kind_ids(&self) -> &[MonitorKind] {
        &self.producer_missing_kind_ids
    }

    pub(crate) fn source_missing_kind_ids(&self) -> &[MonitorKind] {
        &self.source_missing_kind_ids
    }

    pub(crate) fn presentation_missing_kind_ids(&self) -> &[MonitorKind] {
        &self.presentation_missing_kind_ids
    }

    pub(crate) fn durable_policy_missing_kind_ids(&self) -> &[MonitorKind] {
        &self.durable_policy_missing_kind_ids
    }

    pub(crate) fn producer_facts(&self) -> &[ProducerReadinessFact] {
        &self.producer_facts
    }

    pub(crate) fn producer_coverage(&self) -> &ProducerCoverage {
        &self.producer_coverage
    }

    pub(crate) fn enum_external_producer_coverage(&self) -> &ProducerCoverage {
        &self.enum_external_producer_coverage
    }
}

fn index_facts<'a>(
    catalog: &MachineCatalog,
    facts: &'a [ProducerReadinessFact],
) -> Result<BTreeMap<ProducerId, &'a ProducerReadinessFact>, CandidateReadinessInventoryError> {
    let mut indexed = BTreeMap::new();
    for fact in facts {
        if catalog.producer(fact.producer_id()).is_none() {
            return Err(CandidateReadinessInventoryError::UnknownProducer {
                producer_id: fact.producer_id().clone(),
            });
        }
        if indexed.insert(fact.producer_id().clone(), fact).is_some() {
            return Err(CandidateReadinessInventoryError::DuplicateProducerFact {
                producer_id: fact.producer_id().clone(),
            });
        }
    }
    Ok(indexed)
}

fn build_producer_coverage(
    catalog: &MachineCatalog,
    facts: &BTreeMap<ProducerId, &ProducerReadinessFact>,
    only_enum_external: bool,
) -> ProducerCoverage {
    let mut ready_ids = BTreeSet::new();
    let mut conditional_or_disabled_ids = BTreeSet::new();
    let mut unassessed_ids = BTreeSet::new();
    let mut failed_ids = BTreeSet::new();
    let producers = catalog
        .producers()
        .iter()
        .filter(|producer| !only_enum_external || producer.monitor_kind().is_none());
    let mut total = 0;
    for producer in producers {
        total += 1;
        let producer_id = producer.id();
        let conditional_or_disabled = producer
            .monitor_kind()
            .and_then(|kind| catalog.kind(kind))
            .is_some_and(|registration| registration.status() != CatalogStatus::Active);
        if conditional_or_disabled {
            conditional_or_disabled_ids.insert(producer_id.clone());
            continue;
        }
        let Some(fact) = facts.get(producer_id) else {
            unassessed_ids.insert(producer_id.clone());
            continue;
        };
        if !fact.dependencies.is_ready() {
            failed_ids.insert(producer_id.clone());
            continue;
        }
        ready_ids.insert(producer_id.clone());
    }
    ProducerCoverage {
        total,
        ready_ids: ready_ids.into_iter().collect(),
        conditional_or_disabled_ids: conditional_or_disabled_ids.into_iter().collect(),
        unassessed_ids: unassessed_ids.into_iter().collect(),
        failed_ids: failed_ids.into_iter().collect(),
    }
}
