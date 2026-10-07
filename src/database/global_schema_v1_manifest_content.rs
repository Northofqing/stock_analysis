//! Ordinary historical SeedManifest normalization in the original live reader.
//! This issues no paid-codec, native, provider, financial or approval authority.
use super::*;
use crate::trading::paper_ledger::{SeedManifest, FEE_MODEL, MONEY_MODEL};

#[derive(Default)]
struct ManifestContentFacts {
    started: bool,
    callee_reached: bool,
    callee_returned: Option<bool>,
    return_retained: bool,
    checked_accounts: usize,
    tail_account: Option<usize>,
}
struct ManifestContentFrame {
    v1: V1ContentFrame,
    facts: ManifestContentFacts,
    manifests: Vec<SeedManifest>,
    result: Option<StorageResult<()>>,
    unaccepted: Option<StorageResult<()>>,
}
pub(in crate::database::global_schema_v1) struct AdditiveStorageLocalV1ManifestContentChecked {
    frame: ManifestContentFrame,
}
pub(in crate::database::global_schema_v1) struct AdditiveStorageV1ManifestContentHeld {
    frame: ManifestContentFrame,
}
impl AdditiveStorageV1ManifestContentHeld {
    pub(in crate::database::global_schema_v1) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame
            .v1
            .audit
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .transform
            .first
            .as_ref()
            .unwrap()
    }
}
impl AdditiveStorageTransformed {
    pub(in crate::database::global_schema_v1) fn into_v1_manifest_content_hashes(
        self,
    ) -> std::result::Result<
        AdditiveStorageLocalV1ManifestContentChecked,
        AdditiveStorageV1ManifestContentHeld,
    > {
        ManifestContentFrame::new(V1ContentFrame::new(RawV1AuditLinksFrame::new(self.frame)))
            .run(false)
    }
}
impl AdditiveStorageLocalV1ManifestContentChecked {
    pub(in crate::database::global_schema_v1) fn create_or_resume(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageV1ManifestContentHeld> {
        ManifestContentFrame::new(V1ContentFrame::new(raw_v1_audit_links_cold_frame(source)))
            .run(true)
    }
}
impl ManifestContentFrame {
    fn new(v1: V1ContentFrame) -> Self {
        Self {
            v1,
            facts: ManifestContentFacts::default(),
            manifests: Vec::new(),
            result: None,
            unaccepted: None,
        }
    }
    fn run(
        mut self,
        cold: bool,
    ) -> std::result::Result<
        AdditiveStorageLocalV1ManifestContentChecked,
        AdditiveStorageV1ManifestContentHeld,
    > {
        if !self.v1.audit.raw.prepare(cold)
            || !self.v1.audit.raw.advance_links()
            || !self.v1.audit.advance_content()
            || !self.v1.advance_content()
            || !self.finish()
        {
            return Err(AdditiveStorageV1ManifestContentHeld { frame: self });
        }
        Ok(AdditiveStorageLocalV1ManifestContentChecked { frame: self })
    }
    fn v1_returned(&self) -> bool {
        self.v1.audit_returned()
            && self.v1.facts.started
            && self.v1.facts.callee_reached
            && self.v1.facts.callee_returned == Some(true)
            && self.v1.facts.return_retained
            && matches!(self.v1.result, Some(Ok(())))
            && self.v1.unaccepted.is_none()
    }
    fn begin(&mut self) -> bool {
        if self.v1.audit.raw.first() {
            return false;
        }
        if !self.v1_returned()
            || self.v1.audit.raw.content_hashes
                != RawV1AuditContentHashes::AuditAndV1EventProjectionChecked
            || self
                .v1
                .audit
                .raw
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .active
                != Some(1)
            || self.facts.started
            || self.result.is_some()
        {
            self.v1.audit.raw.fail(storage_fail(
                "additive manifest lacks live V1 content return",
            ));
            return false;
        }
        if let Err(first) = self
            .v1
            .audit
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .loan()
            .and_then(|(_, _, work, _)| work.metadata(256))
        {
            self.v1.audit.raw.fail(first);
            return false;
        }
        self.facts.started = true;
        true
    }
    fn evaluate(&mut self) -> Option<StorageResult<()>> {
        if self.v1.audit.raw.first()
            || !self.facts.started
            || self.facts.callee_reached
            || self.result.is_some()
        {
            return None;
        }
        self.facts.callee_reached = true;
        let actual = self
            .v1
            .audit
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .loan()
            .and_then(|(_, _, work, _)| {
                manifest_content_hashes(
                    &self.v1.audit.raw.input.fields,
                    work,
                    &mut self.facts,
                    &mut self.manifests,
                )
            });
        self.facts.callee_returned = Some(actual.is_ok());
        Some(actual)
    }
    fn retain(&mut self, actual: StorageResult<()>) -> std::result::Result<(), StorageResult<()>> {
        if !self.facts.started
            || !self.facts.callee_reached
            || self.facts.callee_returned != Some(actual.is_ok())
            || self.facts.return_retained
            || self.result.is_some()
            || !matches!(
                self.v1.audit.raw.phase,
                RawV1AuditLinksPhase::LinksChecked | RawV1AuditLinksPhase::Refused
            )
        {
            return Err(actual);
        }
        self.result = Some(actual);
        self.facts.return_retained = true;
        if self.v1.audit.raw.first() {
            return Ok(());
        }
        if matches!(self.result, Some(Err(_))) {
            let first = self.result.take().unwrap().unwrap_err();
            self.v1.audit.raw.fail(first);
        } else {
            self.v1.audit.raw.content_hashes =
                RawV1AuditContentHashes::AuditAndV1EventProjectionAndManifestChecked;
        }
        Ok(())
    }
    fn close_and_tail(&mut self) -> StorageResult<()> {
        if self.v1.audit.raw.first() {
            return Err(storage_fail("additive manifest close after first error"));
        }
        if !self.v1_returned()
            || self.v1.audit.raw.content_hashes
                != RawV1AuditContentHashes::AuditAndV1EventProjectionAndManifestChecked
            || !self.facts.started
            || !self.facts.callee_reached
            || self.facts.callee_returned != Some(true)
            || !self.facts.return_retained
            || !matches!(self.result, Some(Ok(())))
            || self.unaccepted.is_some()
            || self.facts.checked_accounts != self.v1.audit.raw.input.fields.rows[0].len()
            || self.manifests.len() != self.facts.checked_accounts
        {
            return Err(storage_fail("additive manifest close before actual result"));
        }
        self.v1.audit.raw.input.close_and_tail()?;
        self.v1.audit.raw.phase = RawV1AuditLinksPhase::Complete;
        Ok(())
    }
    fn finish(&mut self) -> bool {
        if !self.begin() {
            return false;
        }
        let Some(actual) = self.evaluate() else {
            self.v1
                .audit
                .raw
                .fail(storage_fail("additive manifest result not observed"));
            return false;
        };
        if let Err(actual) = self.retain(actual) {
            self.unaccepted = Some(actual);
            self.v1
                .audit
                .raw
                .fail(storage_fail("additive manifest return already retained"));
            return false;
        }
        if self.v1.audit.raw.first() {
            return false;
        }
        match self.close_and_tail() {
            Ok(()) => true,
            Err(first) => {
                self.v1.audit.raw.fail(first);
                false
            }
        }
    }
}

fn manifest_allowance(bytes: &str) -> StorageResult<u64> {
    // Fixed ordinary DTO escrow: text, nested finite vectors, decoder scratch,
    // retained DTO capacity and worst-case canonical JSON expansion. This is
    // conservative work accounting, not an attestation of native allocations.
    u64::try_from(bytes.len())
        .ok()
        .and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_mul(256))
        .and_then(|n| n.checked_add(8192))
        .ok_or_else(|| storage_fail("additive manifest work extent overflow"))
}
fn manifest_content_hashes(
    fields: &V1AuditInputFields,
    work: &mut target::TargetWork,
    facts: &mut ManifestContentFacts,
    manifests: &mut Vec<SeedManifest>,
) -> StorageResult<()> {
    work.require_replay_clear()
        .map_err(GlobalSchemaV1Error::ReplayTerminal)?;
    for (index, account) in fields.rows[0].iter().enumerate() {
        let bytes = account.text(3)?;
        let allowance = manifest_allowance(bytes)?;
        work.metadata(allowance)?; // Before the fixed decoder and any owned DTO.
        let manifest: SeedManifest = serde_json::from_str(bytes)
            .map_err(|_| storage_fail("additive manifest historical DTO invalid"))?;
        manifests.push(manifest); // Actual typed inputs survive later failures.
        let manifest = manifests.last().unwrap();
        if manifest.account_id != account.text(0)? || manifest.epoch_id != account.text(1)? {
            return Err(storage_fail("additive manifest account/epoch differs"));
        }
        if [
            &manifest.account_id,
            &manifest.epoch_id,
            &manifest.command_id,
            &manifest.source_reference,
            &manifest.approved_by,
        ]
        .iter()
        .any(|v| v.trim().is_empty())
            || manifest.source_hash.len() != 64
            || !manifest.source_hash.bytes().all(|c| c.is_ascii_hexdigit())
            || manifest.account_effective_at != manifest.cutover_at
            || manifest.positions_effective_at != manifest.cutover_at
        {
            return Err(storage_fail(
                "additive manifest seed identity/effective time invalid",
            ));
        }
        let mut writer = AuditHashWriter {
            hash: Sha256::new(),
            remaining: allowance,
        };
        // Historical binding normalizes the typed DTO, not the stored JSON bytes.
        serde_json::to_writer(&mut writer, &(1, MONEY_MODEL, FEE_MODEL, manifest))
            .map_err(|_| storage_fail("additive manifest content serialization failed"))?;
        if !digest_matches(writer.hash, account.text(2)?)? {
            return Err(storage_fail("additive manifest normalized hash differs"));
        }
        facts.checked_accounts += 1;
        facts.tail_account = Some(index);
    }
    Ok(())
}

#[cfg(test)]
#[path = "global_schema_v1_manifest_content_tests.rs"]
mod tests;
