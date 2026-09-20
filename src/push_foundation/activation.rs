//! Raw activation persistence values. These types authenticate neither deployment nor authority.

#![cfg_attr(not(test), allow(dead_code))]

use std::fmt;

use crate::monitor::push_job::{GitSha40, Sha256Digest, UnitId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesiredActivationState {
    Disabled,
    Shadow,
    Active,
    Draining,
}

impl DesiredActivationState {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "Disabled" => Some(Self::Disabled),
            "Shadow" => Some(Self::Shadow),
            "Active" => Some(Self::Active),
            "Draining" => Some(Self::Draining),
            _ => None,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Shadow => "Shadow",
            Self::Active => "Active",
            Self::Draining => "Draining",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromotionAction {
    Initialize,
    EnterShadow,
    Activate,
    Drain,
    Disable,
    Rollback,
}

impl PromotionAction {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "Initialize" => Some(Self::Initialize),
            "EnterShadow" => Some(Self::EnterShadow),
            "Activate" => Some(Self::Activate),
            "Drain" => Some(Self::Drain),
            "Disable" => Some(Self::Disable),
            "Rollback" => Some(Self::Rollback),
            _ => None,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Initialize => "Initialize",
            Self::EnterShadow => "EnterShadow",
            Self::Activate => "Activate",
            Self::Drain => "Drain",
            Self::Disable => "Disable",
            Self::Rollback => "Rollback",
        }
    }
}

/// One complete row from `push_activation_manifests` after content-integrity checks.
///
/// The historical hashes and owner/approval strings are retained for later authentication. They
/// are claims from this database, not assertions about the currently deployed process.
#[derive(Clone, Eq, PartialEq)]
pub struct ActivationManifest {
    pub(super) manifest_sha256: Sha256Digest,
    pub(super) unit_id: UnitId,
    pub(super) generation: u64,
    pub(super) previous_manifest_sha256: Option<Sha256Digest>,
    pub(super) desired_state: DesiredActivationState,
    pub(super) physical_owner: String,
    pub(super) build_commit: GitSha40,
    pub(super) build_sha256: Sha256Digest,
    pub(super) catalog_sha256: Sha256Digest,
    pub(super) business_schema_sha256: Sha256Digest,
    pub(super) durable_schema_sha256: Sha256Digest,
    pub(super) template_sha256: Sha256Digest,
    pub(super) source_contract_sha256: Sha256Digest,
    pub(super) evidence_sha256: Sha256Digest,
    pub(super) approved_by: String,
    pub(super) approved_at: u64,
    pub(super) window_start: u64,
    pub(super) window_end: u64,
    pub(super) rollback_target_sha256: Option<Sha256Digest>,
    pub(super) created_at: u64,
}

impl fmt::Debug for ActivationManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivationManifest")
            .field("manifest_sha256", &self.manifest_sha256)
            .field("unit_id", &self.unit_id)
            .field("generation", &self.generation)
            .field("desired_state", &self.desired_state)
            .field(
                "has_rollback_target",
                &self.rollback_target_sha256.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl ActivationManifest {
    pub fn manifest_sha256(&self) -> &Sha256Digest {
        &self.manifest_sha256
    }
    pub fn unit_id(&self) -> &UnitId {
        &self.unit_id
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn previous_manifest_sha256(&self) -> Option<&Sha256Digest> {
        self.previous_manifest_sha256.as_ref()
    }
    pub fn desired_state(&self) -> DesiredActivationState {
        self.desired_state
    }
    pub fn physical_owner(&self) -> &str {
        &self.physical_owner
    }
    pub fn build_commit(&self) -> &GitSha40 {
        &self.build_commit
    }
    pub fn build_sha256(&self) -> &Sha256Digest {
        &self.build_sha256
    }
    pub fn catalog_sha256(&self) -> &Sha256Digest {
        &self.catalog_sha256
    }
    pub fn business_schema_sha256(&self) -> &Sha256Digest {
        &self.business_schema_sha256
    }
    pub fn durable_schema_sha256(&self) -> &Sha256Digest {
        &self.durable_schema_sha256
    }
    pub fn template_sha256(&self) -> &Sha256Digest {
        &self.template_sha256
    }
    pub fn source_contract_sha256(&self) -> &Sha256Digest {
        &self.source_contract_sha256
    }
    pub fn evidence_sha256(&self) -> &Sha256Digest {
        &self.evidence_sha256
    }
    pub fn approved_by(&self) -> &str {
        &self.approved_by
    }
    pub fn approved_at(&self) -> u64 {
        self.approved_at
    }
    pub fn window_start(&self) -> u64 {
        self.window_start
    }
    pub fn window_end(&self) -> u64 {
        self.window_end
    }
    pub fn rollback_target_sha256(&self) -> Option<&Sha256Digest> {
        self.rollback_target_sha256.as_ref()
    }
    pub fn created_at(&self) -> u64 {
        self.created_at
    }
}

/// One complete row from `push_promotion_journal` after content-integrity checks.
#[derive(Clone, Eq, PartialEq)]
pub struct PromotionJournalEntry {
    pub(super) event_id: Sha256Digest,
    pub(super) unit_id: UnitId,
    pub(super) generation: u64,
    pub(super) from_manifest_sha256: Option<Sha256Digest>,
    pub(super) to_manifest_sha256: Sha256Digest,
    pub(super) actor: String,
    pub(super) action: PromotionAction,
    pub(super) reason: String,
    pub(super) window_start: u64,
    pub(super) window_end: u64,
    pub(super) evidence_sha256: Sha256Digest,
    pub(super) rollback_target_sha256: Option<Sha256Digest>,
    pub(super) previous_sha256: Option<Sha256Digest>,
    pub(super) canonical_sha256: Sha256Digest,
    pub(super) occurred_at: u64,
}

impl fmt::Debug for PromotionJournalEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PromotionJournalEntry")
            .field("event_id", &self.event_id)
            .field("unit_id", &self.unit_id)
            .field("generation", &self.generation)
            .field("action", &self.action)
            .field("canonical_sha256", &self.canonical_sha256)
            .finish_non_exhaustive()
    }
}

impl PromotionJournalEntry {
    pub fn event_id(&self) -> &Sha256Digest {
        &self.event_id
    }
    pub fn unit_id(&self) -> &UnitId {
        &self.unit_id
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn from_manifest_sha256(&self) -> Option<&Sha256Digest> {
        self.from_manifest_sha256.as_ref()
    }
    pub fn to_manifest_sha256(&self) -> &Sha256Digest {
        &self.to_manifest_sha256
    }
    pub fn actor(&self) -> &str {
        &self.actor
    }
    pub fn action(&self) -> PromotionAction {
        self.action
    }
    pub fn reason(&self) -> &str {
        &self.reason
    }
    pub fn window_start(&self) -> u64 {
        self.window_start
    }
    pub fn window_end(&self) -> u64 {
        self.window_end
    }
    pub fn evidence_sha256(&self) -> &Sha256Digest {
        &self.evidence_sha256
    }
    pub fn rollback_target_sha256(&self) -> Option<&Sha256Digest> {
        self.rollback_target_sha256.as_ref()
    }
    pub fn previous_sha256(&self) -> Option<&Sha256Digest> {
        self.previous_sha256.as_ref()
    }
    pub fn canonical_sha256(&self) -> &Sha256Digest {
        &self.canonical_sha256
    }
    pub fn occurred_at(&self) -> u64 {
        self.occurred_at
    }
}
