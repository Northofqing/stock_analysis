//! Exact-prefix G5b selection candidates. This module does not own provider
//! readiness, the analysis window, durable cohort admission, or a day seal.
//! Encoded JSON is evidence to check, never a capability constructor.
//! The v2 envelope is strict. AlertRecord decoding preserves its existing
//! legacy compatibility; original raw bytes do not assert a stricter schema.

use super::alert_log::{
    validate_saved_input_head, AlertInputHeadUnknown, AlertRecord, LockedAlertInputPrefix,
    VerifiedAlertInputCutoff, VerifiedAlertInputLine,
};
use super::attribution_deep::DEEP_ATTRIBUTION_MAX_EVENTS;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SCHEMA: &str = "g5b-selection-v2";
const SELECTION_POLICY: &str = "g5b-top3-stable-level-append-order-v1";
const CUTOFF_POLICY: &str = "g5b-first-analysis-ready-prefix-v2";
const COHORT_DOMAIN: &str = "g5b-selection-cohort-v2";
const OCCURRENCE_DOMAIN: &str = "g5b-selected-raw-occurrence-v2";

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn domain_hash(domain: &str, bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain.as_bytes());
    digest.update([0]);
    digest.update(bytes);
    hex::encode(digest.finalize())
}

#[derive(Debug)]
pub(crate) enum G5bSelectionV2Error {
    Input(AlertInputHeadUnknown),
    Encoding(serde_json::Error),
    NoEligibleInput,
    BindingMismatch,
}

impl From<AlertInputHeadUnknown> for G5bSelectionV2Error {
    fn from(value: AlertInputHeadUnknown) -> Self {
        Self::Input(value)
    }
}

impl From<serde_json::Error> for G5bSelectionV2Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Encoding(value)
    }
}

impl std::fmt::Display for G5bSelectionV2Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(error) => write!(formatter, "G5b candidate input: {error:?}"),
            Self::Encoding(error) => write!(formatter, "G5b candidate encoding: {error}"),
            Self::NoEligibleInput => formatter.write_str("G5b candidate has no eligible input"),
            Self::BindingMismatch => formatter.write_str("G5b candidate exact binding mismatch"),
        }
    }
}

impl std::error::Error for G5bSelectionV2Error {}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cutoff {
    pub(crate) input_head_canonical: Vec<u8>,
    pub(crate) input_head_sha256: String,
    pub(crate) generation: u64,
    pub(crate) committed_offset: u64,
    pub(crate) prefix_sha256: String,
    pub(crate) source_identity: Option<SourceIdentity>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectedLine {
    pub(crate) line_ordinal: u64,
    pub(crate) start_offset: u64,
    pub(crate) end_offset: u64,
    pub(crate) raw_line_bytes: Vec<u8>,
    pub(crate) raw_line_sha256: String,
    pub(crate) record_canonical: Vec<u8>,
    pub(crate) record_sha256: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EncodedSelection {
    pub(crate) schema: String,
    pub(crate) business_date: NaiveDate,
    pub(crate) selection_policy: String,
    pub(crate) selection_policy_sha256: String,
    pub(crate) cutoff_policy: String,
    pub(crate) cutoff_policy_sha256: String,
    pub(crate) cutoff: Cutoff,
    pub(crate) selected: Vec<SelectedLine>,
}

/// Strictly checked encoded evidence, never a candidate or owner receipt.
pub(crate) struct G5bSelectionEvidence {
    encoded: EncodedSelection,
    canonical: Vec<u8>,
    cohort_preimage: Vec<u8>,
    occurrences: Vec<(String, Vec<u8>)>,
}

impl G5bSelectionEvidence {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, G5bSelectionV2Error> {
        let encoded: EncodedSelection = serde_json::from_slice(bytes)?;
        let mut canonical = serde_json::to_vec(&encoded)?;
        canonical.push(b'\n');
        let invalid = || G5bSelectionV2Error::BindingMismatch;
        if canonical != bytes
            || encoded.schema != SCHEMA
            || encoded.selection_policy != SELECTION_POLICY
            || encoded.selection_policy_sha256 != hash(SELECTION_POLICY.as_bytes())
            || encoded.cutoff_policy != CUTOFF_POLICY
            || encoded.cutoff_policy_sha256 != hash(CUTOFF_POLICY.as_bytes())
            || encoded.selected.is_empty()
            || encoded.selected.len() > DEEP_ATTRIBUTION_MAX_EVENTS
        {
            return Err(invalid());
        }
        let head =
            validate_saved_input_head(encoded.business_date, &encoded.cutoff.input_head_canonical)?;
        if hash(&encoded.cutoff.input_head_canonical) != encoded.cutoff.input_head_sha256
            || head.generation() != encoded.cutoff.generation
            || head.committed_offset() != encoded.cutoff.committed_offset
            || head.prefix_sha256() != encoded.cutoff.prefix_sha256
            || head.source_identity()
                != encoded
                    .cutoff
                    .source_identity
                    .as_ref()
                    .map(|v| (v.device, v.inode))
        {
            return Err(invalid());
        }
        let mut cohort_preimage = COHORT_DOMAIN.as_bytes().to_vec();
        cohort_preimage.push(0);
        cohort_preimage.extend_from_slice(bytes);
        let cohort_identity = hash(&cohort_preimage);
        let mut occurrences = Vec::new();
        let mut positions = std::collections::BTreeSet::new();
        let mut previous_rank = None;
        for line in &encoded.selected {
            let record: AlertRecord = serde_json::from_slice(&line.raw_line_bytes)?;
            let rank = match record.level.as_str() {
                "紧急" => 0,
                "重要" => 1,
                _ => 2,
            };
            if !record.is_production_eligible()
                || line.line_ordinal == 0
                || line.line_ordinal > head.generation()
                || line.end_offset <= line.start_offset
                || line.end_offset > head.committed_offset()
                || line.end_offset - line.start_offset != line.raw_line_bytes.len() as u64
                || line.raw_line_bytes.last() != Some(&b'\n')
                || line.raw_line_bytes[..line.raw_line_bytes.len() - 1].contains(&b'\n')
                || hash(&line.raw_line_bytes) != line.raw_line_sha256
                || serde_json::to_vec(&record)? != line.record_canonical
                || hash(&line.record_canonical) != line.record_sha256
                || !positions.insert((line.line_ordinal, line.start_offset, line.end_offset))
                || previous_rank.is_some_and(|(old_rank, old_ordinal)| {
                    (rank, line.line_ordinal) <= (old_rank, old_ordinal)
                })
            {
                return Err(invalid());
            }
            previous_rank = Some((rank, line.line_ordinal));
            let canonical = serde_json::to_vec(&OccurrencePreimage {
                business_date: encoded.business_date,
                cohort_identity: &cohort_identity,
                source_identity: &encoded.cutoff.source_identity,
                line_ordinal: line.line_ordinal,
                start_offset: line.start_offset,
                end_offset: line.end_offset,
                raw_line_sha256: &line.raw_line_sha256,
            })?;
            let mut preimage = OCCURRENCE_DOMAIN.as_bytes().to_vec();
            preimage.push(0);
            preimage.extend_from_slice(&canonical);
            occurrences.push((hash(&preimage), preimage));
        }
        Ok(Self {
            encoded,
            canonical,
            cohort_preimage,
            occurrences,
        })
    }
    pub(crate) fn encoded(&self) -> &EncodedSelection {
        &self.encoded
    }
    pub(crate) fn canonical(&self) -> &[u8] {
        &self.canonical
    }
    pub(crate) fn cohort_preimage(&self) -> &[u8] {
        &self.cohort_preimage
    }
    pub(crate) fn cohort_identity(&self) -> String {
        hash(&self.cohort_preimage)
    }
    pub(crate) fn occurrences(&self) -> &[(String, Vec<u8>)] {
        &self.occurrences
    }

    /// Verifies actual source without returning a candidate. A stored owner
    /// capability must be constructed separately by the attested coordinator.
    pub(crate) fn verify_locked_prefix(
        &self,
        prefix: &LockedAlertInputPrefix<'_>,
    ) -> Result<(), G5bSelectionV2Error> {
        let cutoff = prefix.cutoff_for_captured_head(&self.encoded.cutoff.input_head_canonical)?;
        let candidate = G5bSelectionV2Candidate::from_verified_cutoff(cutoff)?;
        candidate.verify_encoding_against_locked_prefix(prefix, &self.canonical)
    }
}

#[derive(Serialize)]
struct OccurrencePreimage<'a> {
    business_date: NaiveDate,
    cohort_identity: &'a str,
    source_identity: &'a Option<SourceIdentity>,
    line_ordinal: u64,
    start_offset: u64,
    end_offset: u64,
    raw_line_sha256: &'a str,
}

/// Original selected source and derived occurrence. Only the locked prefix
/// reader can supply the raw anchors used to construct this value.
pub(crate) struct G5bSelectedRawOccurrence {
    identity: String,
    line: SelectedLine,
    record: AlertRecord,
}

impl G5bSelectedRawOccurrence {
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
    pub(crate) fn ordinal(&self) -> u64 {
        self.line.line_ordinal
    }
    pub(crate) fn start_offset(&self) -> u64 {
        self.line.start_offset
    }
    pub(crate) fn end_offset(&self) -> u64 {
        self.line.end_offset
    }
    pub(crate) fn raw_bytes(&self) -> &[u8] {
        &self.line.raw_line_bytes
    }
    pub(crate) fn raw_sha256(&self) -> &str {
        &self.line.raw_line_sha256
    }
    pub(crate) fn record(&self) -> &AlertRecord {
        &self.record
    }
}

/// A private-constructed observation of top3 at one verified prefix. Its
/// policy names describe the intended owner contract; no window/provider
/// check has happened here. It cannot be deserialized from saved JSON.
pub(crate) struct G5bSelectionV2Candidate {
    encoded: EncodedSelection,
    canonical: Vec<u8>,
    cohort_identity: String,
    selected: Vec<G5bSelectedRawOccurrence>,
}

impl G5bSelectionV2Candidate {
    pub(crate) fn from_locked_prefix(
        prefix: &LockedAlertInputPrefix<'_>,
    ) -> Result<Self, G5bSelectionV2Error> {
        Self::from_verified_cutoff(prefix.current_cutoff()?)
    }

    fn from_verified_cutoff(cutoff: VerifiedAlertInputCutoff) -> Result<Self, G5bSelectionV2Error> {
        let mut selected = cutoff
            .lines()
            .iter()
            .filter(|line| line.record().is_production_eligible())
            .collect::<Vec<_>>();
        // This matches top_events_for_deep: level priority only, stable within
        // each level. Neither triggered_at nor raw hash changes append order.
        selected.sort_by_key(|line| match line.record().level.as_str() {
            "紧急" => 0u8,
            "重要" => 1,
            _ => 2,
        });
        selected.truncate(DEEP_ATTRIBUTION_MAX_EVENTS);
        if selected.is_empty() {
            return Err(G5bSelectionV2Error::NoEligibleInput);
        }
        let rows = selected
            .iter()
            .map(|line| encode_line(line))
            .collect::<Result<Vec<_>, _>>()?;
        let encoded = EncodedSelection {
            schema: SCHEMA.to_owned(),
            business_date: cutoff.head().business_date(),
            selection_policy: SELECTION_POLICY.to_owned(),
            selection_policy_sha256: hash(SELECTION_POLICY.as_bytes()),
            cutoff_policy: CUTOFF_POLICY.to_owned(),
            cutoff_policy_sha256: hash(CUTOFF_POLICY.as_bytes()),
            cutoff: Cutoff {
                input_head_canonical: cutoff.head_canonical().to_vec(),
                input_head_sha256: hash(cutoff.head_canonical()),
                generation: cutoff.head().generation(),
                committed_offset: cutoff.head().committed_offset(),
                prefix_sha256: cutoff.head().prefix_sha256().to_owned(),
                source_identity: cutoff
                    .source_identity()
                    .map(|(device, inode)| SourceIdentity { device, inode }),
            },
            selected: rows,
        };
        let mut canonical = serde_json::to_vec(&encoded)?;
        canonical.push(b'\n');
        let cohort_identity = domain_hash(COHORT_DOMAIN, &canonical);
        let occurrences = selected
            .iter()
            .zip(&encoded.selected)
            .map(|(raw, line)| {
                let identity = domain_hash(
                    OCCURRENCE_DOMAIN,
                    &serde_json::to_vec(&OccurrencePreimage {
                        business_date: encoded.business_date,
                        cohort_identity: &cohort_identity,
                        source_identity: &encoded.cutoff.source_identity,
                        line_ordinal: line.line_ordinal,
                        start_offset: line.start_offset,
                        end_offset: line.end_offset,
                        raw_line_sha256: &line.raw_line_sha256,
                    })?,
                );
                Ok(G5bSelectedRawOccurrence {
                    identity,
                    line: line.clone(),
                    record: raw.record().clone(),
                })
            })
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        Ok(Self {
            encoded,
            canonical,
            cohort_identity,
            selected: occurrences,
        })
    }

    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub(crate) fn cohort_identity(&self) -> &str {
        &self.cohort_identity
    }
    pub(crate) fn selected(&self) -> &[G5bSelectedRawOccurrence] {
        &self.selected
    }

    /// Check encoded evidence against this existing candidate and the actual
    /// current locked source. Legal suffixes preserve the original cutoff;
    /// arbitrary JSON never becomes a new trusted candidate.
    pub(crate) fn verify_encoding_against_locked_prefix(
        &self,
        prefix: &LockedAlertInputPrefix<'_>,
        bytes: &[u8],
    ) -> Result<(), G5bSelectionV2Error> {
        let decoded: EncodedSelection = serde_json::from_slice(bytes)?;
        let mut canonical = serde_json::to_vec(&decoded)?;
        canonical.push(b'\n');
        if canonical != bytes || decoded != self.encoded || bytes != self.canonical {
            return Err(G5bSelectionV2Error::BindingMismatch);
        }
        let cutoff = prefix.cutoff_for_captured_head(&self.encoded.cutoff.input_head_canonical)?;
        let actual = Self::from_verified_cutoff(cutoff)?;
        if actual.canonical != self.canonical || actual.cohort_identity != self.cohort_identity {
            return Err(G5bSelectionV2Error::BindingMismatch);
        }
        Ok(())
    }
}

fn encode_line(line: &VerifiedAlertInputLine) -> Result<SelectedLine, serde_json::Error> {
    let record_canonical = serde_json::to_vec(line.record())?;
    Ok(SelectedLine {
        line_ordinal: line.ordinal(),
        start_offset: line.start_offset(),
        end_offset: line.end_offset(),
        raw_line_bytes: line.raw_bytes().to_vec(),
        raw_line_sha256: line.raw_sha256().to_owned(),
        record_sha256: hash(&record_canonical),
        record_canonical,
    })
}

#[cfg(test)]
#[path = "g5b_selection_v2_tests.rs"]
mod tests;
