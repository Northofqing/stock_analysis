//! Closed model archive snapshots, not delivery, counted or day-seal evidence.
use super::*;
use crate::durable_delivery::VerifiedG5bModelBundle;

const ARCHIVE_SCHEMA: &str = "g5b-model-archive-v2";
const ARCHIVE_POLICY: &str = "g5b-immutable-model-archive-snapshot-v2";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum G5bModelArchiveCoverageV2 {
    Partial,
    Full,
}
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveItem {
    selection_index: usize,
    occurrence_identity: String,
    attempt_identity: String,
    attempt_sha256: String,
    frozen_identity: String,
    frozen_sha256: String,
    handoff_canonical: Vec<u8>,
}
#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveSnapshot {
    schema: String,
    policy: String,
    business_date: NaiveDate,
    cohort_identity: String,
    selection_sha256: String,
    version: u64,
    previous_archive_identity: Option<String>,
    selected_count: usize,
    coverage: G5bModelArchiveCoverageV2,
    items: Vec<ArchiveItem>,
}

/// Immutable local model snapshot. Full means that every selected occurrence
/// has an archived model result; it says nothing about physical delivery.
pub struct G5bArchivedModelObservationV2 {
    identity: String,
    snapshot: ArchiveSnapshot,
    canonical: Vec<u8>,
}
impl G5bArchivedModelObservationV2 {
    pub fn archive_identity(&self) -> &str {
        &self.identity
    }
    pub fn version(&self) -> u64 {
        self.snapshot.version
    }
    pub fn coverage(&self) -> G5bModelArchiveCoverageV2 {
        self.snapshot.coverage
    }
    pub fn selected_count(&self) -> usize {
        self.snapshot.selected_count
    }
    pub fn archived_count(&self) -> usize {
        self.snapshot.items.len()
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
}

fn actual_items(bundle: &VerifiedG5bModelBundle) -> Result<Vec<ArchiveItem>> {
    let evidence = G5bSelectionEvidence::decode(bundle.cohort().selection_bytes())
        .map_err(|e| invalid(&e.to_string()))?;
    let mut items = Vec::new();
    for saved in bundle.members() {
        let Some(frozen) = saved.frozen().filter(|value| value.is_committed()) else {
            continue;
        };
        let attempt = saved
            .attempt()
            .filter(|value| value.is_committed())
            .ok_or_else(|| invalid("Committed Frozen has no actual Committed original Attempt"))?;
        let core = validate_handoff(frozen.bytes(), attempt.bytes(), attempt.identity())?;
        if core.member != member(&evidence, saved.index())?
            || core.member.occurrence_identity != saved.occurrence()
        {
            return Err(invalid("actual model bundle member differs"));
        }
        items.push(ArchiveItem {
            selection_index: saved.index(),
            occurrence_identity: saved.occurrence().to_owned(),
            attempt_identity: attempt.identity().to_owned(),
            attempt_sha256: hash(attempt.bytes()),
            frozen_identity: frozen.identity().to_owned(),
            frozen_sha256: hash(frozen.bytes()),
            handoff_canonical: frozen.bytes().to_vec(),
        });
    }
    Ok(items)
}
fn validate_snapshot(
    snapshot: &ArchiveSnapshot,
    bundle: &VerifiedG5bModelBundle,
    actual: &[ArchiveItem],
) -> Result<()> {
    validate_snapshot_evidence(snapshot, bundle.cohort().selection_bytes(), actual)
}
fn validate_snapshot_evidence(
    snapshot: &ArchiveSnapshot,
    selection: &[u8],
    actual: &[ArchiveItem],
) -> Result<()> {
    let evidence = G5bSelectionEvidence::decode(selection).map_err(|e| invalid(&e.to_string()))?;
    let count = evidence.encoded().selected.len();
    let coverage = if snapshot.items.len() == count {
        G5bModelArchiveCoverageV2::Full
    } else {
        G5bModelArchiveCoverageV2::Partial
    };
    if snapshot.schema != ARCHIVE_SCHEMA
        || snapshot.policy != ARCHIVE_POLICY
        || snapshot.business_date != evidence.encoded().business_date
        || snapshot.cohort_identity != evidence.cohort_identity()
        || snapshot.selection_sha256 != hash(selection)
        || snapshot.selected_count != count
        || snapshot.items.is_empty()
        || snapshot.items.len() > count
        || snapshot.version == 0
        || snapshot.version > count as u64
        || snapshot.coverage != coverage
    {
        return Err(invalid("closed archive snapshot differs"));
    }
    let mut previous = None;
    for item in &snapshot.items {
        if previous.is_some_and(|index| index >= item.selection_index)
            || !actual.iter().any(|value| value == item)
        {
            return Err(invalid(
                "archive contains duplicate, unordered or uncommitted model member",
            ));
        }
        previous = Some(item.selection_index);
    }
    Ok(())
}
fn saved_chain(
    bundle: &VerifiedG5bModelBundle,
    actual: &[ArchiveItem],
) -> Result<Vec<ArchiveSnapshot>> {
    let mut chain: Vec<ArchiveSnapshot> = Vec::new();
    for (index, artifact) in bundle.archives().iter().enumerate() {
        let snapshot: ArchiveSnapshot = decode(artifact.bytes())?;
        validate_snapshot(&snapshot, bundle, actual)?;
        let previous = index
            .checked_sub(1)
            .map(|index| bundle.archives()[index].identity().to_owned());
        if snapshot.version != index as u64 + 1 || snapshot.previous_archive_identity != previous {
            return Err(invalid("archive version chain differs"));
        }
        if let Some(previous) = chain.last() {
            if !bundle.archives()[index - 1].is_committed()
                || previous.items.len() >= snapshot.items.len()
                || !previous
                    .items
                    .iter()
                    .all(|item| snapshot.items.contains(item))
            {
                return Err(invalid(
                    "archive versions must append committed, strictly growing model sets",
                ));
            }
        }
        chain.push(snapshot);
    }
    Ok(chain)
}
/// Pure closed codec qualification only. The actual day owner separately
/// verifies every Committed original file and delivery/audit obligation.
pub(super) fn full_archive_identity(bundle: &VerifiedG5bModelBundle) -> Result<Option<String>> {
    let actual = actual_items(bundle)?;
    let chain = saved_chain(bundle, &actual)?;
    if actual.len() != bundle.cohort().selected_count()
        || bundle.archives().iter().any(|value| !value.is_committed())
    {
        return Ok(None);
    }
    match (chain.last(), bundle.archives().last()) {
        (Some(snapshot), Some(last))
            if snapshot.coverage == G5bModelArchiveCoverageV2::Full && snapshot.items == actual =>
        {
            Ok(Some(last.identity().to_owned()))
        }
        _ => Ok(None),
    }
}

fn next_snapshot(bundle: &VerifiedG5bModelBundle, items: Vec<ArchiveItem>) -> ArchiveSnapshot {
    let evidence = G5bSelectionEvidence::decode(bundle.cohort().selection_bytes())
        .expect("already validated bundle codec");
    ArchiveSnapshot {
        schema: ARCHIVE_SCHEMA.to_owned(),
        policy: ARCHIVE_POLICY.to_owned(),
        business_date: evidence.encoded().business_date,
        cohort_identity: bundle.cohort().identity(),
        selection_sha256: hash(bundle.cohort().selection_bytes()),
        version: bundle.archives().len() as u64 + 1,
        previous_archive_identity: bundle
            .archives()
            .last()
            .map(|value| value.identity().to_owned()),
        selected_count: bundle.cohort().selected_count(),
        coverage: if items.len() == bundle.cohort().selected_count() {
            G5bModelArchiveCoverageV2::Full
        } else {
            G5bModelArchiveCoverageV2::Partial
        },
        items,
    }
}

/// Short local operation. No provider/time inputs, model await or physical sink.
/// Replays only saved Prepared archive bytes and never heals Committed files.
pub fn archive_model_observations_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
) -> Result<Option<G5bArchivedModelObservationV2>> {
    let session = coordinator.g5b_day_session(date)?;
    // At most three growing versions; an interrupted last publication may
    // require one recovery pass before appending the current model set.
    for _ in 0..4 {
        let Some(bundle) = session.read_model_bundle()? else {
            return Ok(None);
        };
        let actual = actual_items(&bundle)?;
        let chain = saved_chain(&bundle, &actual)?;
        if let Some(last) = bundle.archives().last() {
            if !last.is_committed() {
                // This existing original intent is the only permitted replay.
                session.publish_model_archive(&bundle, last.intent())?;
                session.commit_model_archive(&bundle, last.intent())?;
                continue;
            }
            let snapshot = chain
                .into_iter()
                .last()
                .ok_or_else(|| invalid("archive chain absent"))?;
            if snapshot.items == actual {
                bundle.verify_files(&session)?;
                return Ok(Some(G5bArchivedModelObservationV2 {
                    identity: last.identity().to_owned(),
                    snapshot,
                    canonical: last.bytes().to_vec(),
                }));
            }
        }
        if actual.is_empty() {
            return Ok(None);
        }
        let snapshot = next_snapshot(&bundle, actual);
        let bytes = canonical(&snapshot)?;
        let intent = session.prepare_model_archive(&bundle, &bytes)?;
        // The post-prepare fresh bundle includes that exact original intent;
        // no arbitrary opaque Archive is adopted or orphan registered.
        let prepared = session
            .read_model_bundle()?
            .ok_or_else(|| invalid("prepared archive cohort absent"))?;
        let actual = actual_items(&prepared)?;
        saved_chain(&prepared, &actual)?;
        let last = prepared
            .archives()
            .last()
            .ok_or_else(|| invalid("prepared archive absent"))?;
        if last.identity() != intent.identity() || last.bytes() != bytes || last.is_committed() {
            return Err(invalid("new archive intent changed"));
        }
        session.publish_model_archive(&prepared, &intent)?;
        session.commit_model_archive(&prepared, &intent)?;
    }
    Err(invalid("bounded archive recovery did not converge"))
}

#[cfg(test)]
pub(crate) fn prepare_for_test(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
) -> Result<crate::durable_delivery::PreparedG5bArtifact> {
    let session = coordinator.g5b_day_session(date)?;
    session.validate_analysis_test_owner()?;
    // Test seam preserves real owner/files/codec; it stops at the actual
    // Prepared boundary instead of supplying payload facts or fake receipts.
    let bundle = session
        .read_model_bundle()?
        .ok_or_else(|| invalid("actual cohort absent"))?;
    let items = actual_items(&bundle)?;
    let chain = saved_chain(&bundle, &items)?;
    if !chain.is_empty() || items.is_empty() {
        return Err(invalid("test first archive boundary unavailable"));
    }
    let bytes = canonical(&next_snapshot(&bundle, items))?;
    Ok(session.prepare_model_archive(&bundle, &bytes)?)
}

#[cfg(test)]
pub(crate) fn bytes_for_test(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
) -> Result<Vec<u8>> {
    let session = coordinator.g5b_day_session(date)?;
    session.validate_analysis_test_owner()?;
    let bundle = session
        .read_model_bundle()?
        .ok_or_else(|| invalid("actual cohort absent"))?;
    let items = actual_items(&bundle)?;
    if !bundle.archives().is_empty() || items.is_empty() {
        return Err(invalid("first archive byte fixture unavailable"));
    }
    canonical(&next_snapshot(&bundle, items))
}

/// Pure codec projection shared by the SQL gate and the actual-file owner.
/// Partial archives qualify only their included member, never day completion.
pub(super) fn validate_archived_member(
    selection: &[u8],
    selected_index: usize,
    load: &dyn Fn(usize) -> Result<Option<(String, Vec<u8>, String, Vec<u8>)>>,
    archives: &[(String, Vec<u8>, bool)],
) -> Result<()> {
    let evidence = G5bSelectionEvidence::decode(selection).map_err(|e| invalid(&e.to_string()))?;
    let mut actual = Vec::new();
    for index in 0..evidence.encoded().selected.len() {
        if let Some((attempt_identity, attempt, frozen_identity, frozen)) = load(index)? {
            let core = validate_handoff(&frozen, &attempt, &attempt_identity)?;
            if core.member != member(&evidence, index)? {
                return Err(invalid("archived member raw anchors differ"));
            }
            actual.push(ArchiveItem {
                selection_index: index,
                occurrence_identity: core.member.occurrence_identity,
                attempt_identity,
                attempt_sha256: hash(&attempt),
                frozen_identity,
                frozen_sha256: hash(&frozen),
                handoff_canonical: frozen,
            });
        }
    }
    let mut previous: Option<ArchiveSnapshot> = None;
    let mut witnessed = false;
    for (index, (identity, bytes, committed)) in archives.iter().enumerate() {
        let snapshot: ArchiveSnapshot = decode(bytes)?;
        validate_snapshot_evidence(&snapshot, selection, &actual)?;
        let prev = index.checked_sub(1).map(|i| archives[i].0.clone());
        if snapshot.version != index as u64 + 1 || snapshot.previous_archive_identity != prev {
            return Err(invalid("counted archive chain differs"));
        }
        if let Some(previous) = &previous {
            if !archives[index - 1].2
                || previous.items.len() >= snapshot.items.len()
                || !previous
                    .items
                    .iter()
                    .all(|item| snapshot.items.contains(item))
            {
                return Err(invalid(
                    "counted archive chain does not grow Committed members",
                ));
            }
        }
        if *committed
            && snapshot
                .items
                .iter()
                .any(|item| item.selection_index == selected_index)
        {
            witnessed = true;
        }
        // Keep identity tied to the actual row even for the last Prepared version.
        if identity.is_empty() {
            return Err(invalid("archive row identity absent"));
        }
        previous = Some(snapshot);
    }
    if !witnessed {
        return Err(invalid("member has no actual Committed closed Archive"));
    }
    Ok(())
}
