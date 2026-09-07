//! Complete raw activation history, deliberately separated from rollout authority.

#![cfg_attr(not(test), allow(dead_code))]

use std::fmt;

use crate::monitor::push_job::UnitId;

use super::activation::{ActivationManifest, PromotionJournalEntry};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationReconciliation {
    /// This bundled-catalog unit has no manifest and no journal row.
    Unregistered,
    /// Every manifest has one exactly-bound journal entry.
    CaughtUp { generation: u64 },
    /// The final manifest is the sole unapplied generation. It is not evidence of execution.
    Pending {
        executed_generation: Option<u64>,
        pending_generation: u64,
    },
}

#[derive(Clone, Eq, PartialEq)]
pub struct UnitActivationFacts {
    pub(super) unit_id: UnitId,
    pub(super) manifests: Vec<ActivationManifest>,
    pub(super) journal: Vec<PromotionJournalEntry>,
    pub(super) reconciliation: ActivationReconciliation,
}

impl fmt::Debug for UnitActivationFacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnitActivationFacts")
            .field("unit_id", &self.unit_id)
            .field("manifest_count", &self.manifests.len())
            .field("journal_count", &self.journal.len())
            .field("reconciliation", &self.reconciliation)
            .finish()
    }
}

impl UnitActivationFacts {
    pub fn unit_id(&self) -> &UnitId {
        &self.unit_id
    }

    pub fn manifests(&self) -> &[ActivationManifest] {
        &self.manifests
    }

    pub fn journal(&self) -> &[PromotionJournalEntry] {
        &self.journal
    }

    pub fn reconciliation(&self) -> ActivationReconciliation {
        self.reconciliation
    }
}

/// All bundled-catalog units and their complete raw activation chains.
///
/// `selected_unit` is a convenience projection over `units`; validation is always performed for
/// the whole database first. No value here permits execution or attests the current deployment.
#[derive(Clone, Eq, PartialEq)]
pub struct RawActivationFacts {
    pub(super) units: Vec<UnitActivationFacts>,
    pub(super) selected_unit_index: usize,
}

impl fmt::Debug for RawActivationFacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let selected = &self.units[self.selected_unit_index];
        formatter
            .debug_struct("RawActivationFacts")
            .field("unit_count", &self.units.len())
            .field("selected_unit", &selected.unit_id)
            .field("selected_reconciliation", &selected.reconciliation)
            .finish_non_exhaustive()
    }
}

impl RawActivationFacts {
    pub fn units(&self) -> &[UnitActivationFacts] {
        &self.units
    }

    pub fn selected_unit(&self) -> &UnitActivationFacts {
        &self.units[self.selected_unit_index]
    }

    pub fn unit(&self, unit_id: &UnitId) -> Option<&UnitActivationFacts> {
        self.units.iter().find(|facts| &facts.unit_id == unit_id)
    }
}
