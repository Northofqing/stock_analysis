//! Canonical encodings for raw activation rows. Encoding validates content, not deployment.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;

use crate::monitor::push_job::{
    canonical_digest, canonical_preimage, CanonicalValue, Sha256Digest,
};

use super::activation::{ActivationManifest, PromotionJournalEntry};

pub(super) const MANIFEST_DOMAIN: &str = "ActivationManifestV1";
pub(super) const JOURNAL_DOMAIN: &str = "PromotionJournalV1";
pub(super) const PROMOTION_ID_DOMAIN: &str = "PromotionV1";

pub(super) fn manifest_canonical_bytes(manifest: &ActivationManifest) -> Vec<u8> {
    canonical_preimage(MANIFEST_DOMAIN, &manifest_fields(manifest))
}

pub(super) fn manifest_digest(manifest: &ActivationManifest) -> Sha256Digest {
    canonical_digest(MANIFEST_DOMAIN, &manifest_fields(manifest))
}

pub(super) fn journal_canonical_bytes(entry: &PromotionJournalEntry) -> Vec<u8> {
    canonical_preimage(JOURNAL_DOMAIN, &journal_fields(entry))
}

pub(super) fn journal_digest(entry: &PromotionJournalEntry) -> Sha256Digest {
    canonical_digest(JOURNAL_DOMAIN, &journal_fields(entry))
}

pub(super) fn promotion_event_id(unit_id: &str, generation: u64) -> Sha256Digest {
    canonical_digest(
        PROMOTION_ID_DOMAIN,
        &BTreeMap::from([
            ("generation", CanonicalValue::Unsigned(generation)),
            ("unit_id", string(unit_id)),
        ]),
    )
}

fn manifest_fields(manifest: &ActivationManifest) -> BTreeMap<&'static str, CanonicalValue> {
    BTreeMap::from([
        (
            "approved_at",
            CanonicalValue::Unsigned(manifest.approved_at),
        ),
        ("approved_by", string(&manifest.approved_by)),
        ("build_commit", string(manifest.build_commit.as_str())),
        ("build_sha256", string(manifest.build_sha256.as_str())),
        (
            "business_schema_sha256",
            string(manifest.business_schema_sha256.as_str()),
        ),
        ("catalog_sha256", string(manifest.catalog_sha256.as_str())),
        ("created_at", CanonicalValue::Unsigned(manifest.created_at)),
        ("desired_state", string(manifest.desired_state.as_str())),
        (
            "durable_schema_sha256",
            string(manifest.durable_schema_sha256.as_str()),
        ),
        ("evidence_sha256", string(manifest.evidence_sha256.as_str())),
        ("generation", CanonicalValue::Unsigned(manifest.generation)),
        ("physical_owner", string(&manifest.physical_owner)),
        (
            "previous_manifest_sha256",
            optional_digest(manifest.previous_manifest_sha256.as_ref()),
        ),
        (
            "rollback_target_sha256",
            optional_digest(manifest.rollback_target_sha256.as_ref()),
        ),
        (
            "source_contract_sha256",
            string(manifest.source_contract_sha256.as_str()),
        ),
        ("template_sha256", string(manifest.template_sha256.as_str())),
        ("unit_id", string(manifest.unit_id.as_str())),
        ("window_end", CanonicalValue::Unsigned(manifest.window_end)),
        (
            "window_start",
            CanonicalValue::Unsigned(manifest.window_start),
        ),
    ])
}

fn journal_fields(entry: &PromotionJournalEntry) -> BTreeMap<&'static str, CanonicalValue> {
    BTreeMap::from([
        ("action", string(entry.action.as_str())),
        ("actor", string(&entry.actor)),
        ("event_id", string(entry.event_id.as_str())),
        ("evidence_sha256", string(entry.evidence_sha256.as_str())),
        (
            "from_manifest_sha256",
            optional_digest(entry.from_manifest_sha256.as_ref()),
        ),
        ("generation", CanonicalValue::Unsigned(entry.generation)),
        ("occurred_at", CanonicalValue::Unsigned(entry.occurred_at)),
        (
            "previous_sha256",
            optional_digest(entry.previous_sha256.as_ref()),
        ),
        ("reason", string(&entry.reason)),
        (
            "rollback_target_sha256",
            optional_digest(entry.rollback_target_sha256.as_ref()),
        ),
        (
            "to_manifest_sha256",
            string(entry.to_manifest_sha256.as_str()),
        ),
        ("unit_id", string(entry.unit_id.as_str())),
        ("window_end", CanonicalValue::Unsigned(entry.window_end)),
        ("window_start", CanonicalValue::Unsigned(entry.window_start)),
    ])
}

fn optional_digest(value: Option<&Sha256Digest>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, |value| string(value.as_str()))
}

fn string(value: &str) -> CanonicalValue {
    CanonicalValue::String(value.to_owned())
}
