//! V1 event and stored projection digests within the original live reader.
//! Manifest canonicalization and economic replay remain separate obligations.
use super::*;

#[derive(Default)]
struct V1ContentFacts {
    started: bool,
    callee_reached: bool,
    callee_returned: Option<bool>,
    return_retained: bool,
    checked_events: usize,
    checked_heads: usize,
    tail_event: Option<usize>,
    tail_head: Option<usize>,
}
struct V1ContentFrame {
    audit: AuditContentFrame,
    facts: V1ContentFacts,
    result: Option<StorageResult<()>>,
    unaccepted: Option<StorageResult<()>>,
}
pub(in crate::database::global_schema_v1) struct AdditiveStorageLocalV1EventProjectionContentChecked
{
    frame: V1ContentFrame,
}
pub(in crate::database::global_schema_v1) struct AdditiveStorageV1EventProjectionContentHeld {
    frame: V1ContentFrame,
}
impl AdditiveStorageV1EventProjectionContentHeld {
    pub(in crate::database::global_schema_v1) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame
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
    pub(in crate::database::global_schema_v1) fn into_v1_event_projection_content_hashes(
        self,
    ) -> std::result::Result<
        AdditiveStorageLocalV1EventProjectionContentChecked,
        AdditiveStorageV1EventProjectionContentHeld,
    > {
        V1ContentFrame::new(RawV1AuditLinksFrame::new(self.frame)).run(false)
    }
}
impl AdditiveStorageLocalV1EventProjectionContentChecked {
    pub(in crate::database::global_schema_v1) fn create_or_resume(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageV1EventProjectionContentHeld> {
        V1ContentFrame::new(raw_v1_audit_links_cold_frame(source)).run(true)
    }
}
impl V1ContentFrame {
    fn new(raw: RawV1AuditLinksFrame) -> Self {
        Self {
            audit: AuditContentFrame::new(raw),
            facts: V1ContentFacts::default(),
            result: None,
            unaccepted: None,
        }
    }
    fn run(
        mut self,
        cold: bool,
    ) -> std::result::Result<
        AdditiveStorageLocalV1EventProjectionContentChecked,
        AdditiveStorageV1EventProjectionContentHeld,
    > {
        if !self.audit.raw.prepare(cold)
            || !self.audit.raw.advance_links()
            || !self.audit.advance_content()
            || !self.finish()
        {
            return Err(AdditiveStorageV1EventProjectionContentHeld { frame: self });
        }
        Ok(AdditiveStorageLocalV1EventProjectionContentChecked { frame: self })
    }
    fn audit_returned(&self) -> bool {
        self.audit.raw.phase == RawV1AuditLinksPhase::LinksChecked
            && self.audit.raw.all_returns()
            && self.audit.raw.input.phase == V1AuditInputPhase::InputsChecked
            && self.audit.raw.input.all_returns()
            && self.audit.facts.started
            && self.audit.facts.callee_reached
            && self.audit.facts.callee_returned == Some(true)
            && self.audit.facts.return_retained
            && matches!(self.audit.result, Some(Ok(())))
            && self.audit.unaccepted.is_none()
    }
    fn begin(&mut self) -> bool {
        if self.audit.raw.first() {
            return false;
        }
        if !self.audit_returned()
            || self.audit.raw.content_hashes != RawV1AuditContentHashes::AuditOnlyChecked
            || self.audit.raw.input.genesis.owner.fee.local.readonly.active != Some(1)
            || self.facts.started
            || self.result.is_some()
        {
            self.audit
                .raw
                .fail(storage_fail("additive V1 content lacks live audit return"));
            return false;
        }
        if let Err(first) = self
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
            self.audit.raw.fail(first);
            return false;
        }
        self.facts.started = true;
        true
    }
    fn evaluate(&mut self) -> Option<StorageResult<()>> {
        if self.audit.raw.first()
            || !self.facts.started
            || self.facts.callee_reached
            || self.result.is_some()
        {
            return None;
        }
        self.facts.callee_reached = true;
        let actual = self
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
                v1_event_projection_hashes(&self.audit.raw.input.fields, work, &mut self.facts)
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
                self.audit.raw.phase,
                RawV1AuditLinksPhase::LinksChecked | RawV1AuditLinksPhase::Refused
            )
        {
            return Err(actual);
        }
        self.result = Some(actual);
        self.facts.return_retained = true;
        if self.audit.raw.first() {
            return Ok(());
        }
        if matches!(self.result, Some(Err(_))) {
            let first = self.result.take().unwrap().unwrap_err();
            self.audit.raw.fail(first);
        } else {
            self.audit.raw.content_hashes =
                RawV1AuditContentHashes::AuditAndV1EventProjectionChecked;
        }
        Ok(())
    }
    fn close_and_tail(&mut self) -> StorageResult<()> {
        if self.audit.raw.first() {
            return Err(storage_fail("additive V1 content close after first error"));
        }
        if !self.audit_returned()
            || self.audit.raw.content_hashes
                != RawV1AuditContentHashes::AuditAndV1EventProjectionChecked
            || !self.facts.started
            || !self.facts.callee_reached
            || self.facts.callee_returned != Some(true)
            || !self.facts.return_retained
            || !matches!(self.result, Some(Ok(())))
            || self.unaccepted.is_some()
        {
            return Err(storage_fail(
                "additive V1 content close before actual result",
            ));
        }
        self.audit.raw.input.close_and_tail()?;
        self.audit.raw.phase = RawV1AuditLinksPhase::Complete;
        Ok(())
    }
    fn finish(&mut self) -> bool {
        if !self.begin() {
            return false;
        }
        let Some(actual) = self.evaluate() else {
            self.audit
                .raw
                .fail(storage_fail("additive V1 content result not observed"));
            return false;
        };
        if let Err(actual) = self.retain(actual) {
            self.unaccepted = Some(actual);
            self.audit
                .raw
                .fail(storage_fail("additive V1 content return already retained"));
            return false;
        }
        if self.audit.raw.first() {
            return false;
        }
        match self.close_and_tail() {
            Ok(()) => true,
            Err(first) => {
                self.audit.raw.fail(first);
                false
            }
        }
    }
}

fn digest_matches(hash: Sha256, expected: &str) -> StorageResult<bool> {
    let mut hex = [0; 64];
    hex::encode_to_slice(hash.finalize(), &mut hex)
        .map_err(|_| storage_fail("additive V1 content digest extent differs"))?;
    Ok(expected.as_bytes() == hex)
}
fn v1_event_projection_hashes(
    fields: &V1AuditInputFields,
    work: &mut target::TargetWork,
    facts: &mut V1ContentFacts,
) -> StorageResult<()> {
    work.require_replay_clear()
        .map_err(GlobalSchemaV1Error::ReplayTerminal)?;
    for (index, event) in fields.rows[1].iter().enumerate() {
        // Same cumulative meter. Payment precedes serialization and hashing;
        // six-fold text allowance covers JSON escaping without a payload copy.
        let allowance = row_allowance(event, "")?;
        work.metadata(allowance)?;
        let mut writer = AuditHashWriter {
            hash: Sha256::new(),
            remaining: allowance,
        };
        serde_json::to_writer(
            &mut writer,
            &(
                "PAPER_EVENT_V1",
                event.text(0)?,
                event.integer(1)?,
                event.text(2)?,
                event.text(3)?,
                event.text(5)?,
            ),
        )
        .map_err(|_| storage_fail("additive V1 event content serialization failed"))?;
        if !digest_matches(writer.hash, event.text(4)?)? {
            return Err(storage_fail("additive V1 event content hash differs"));
        }
        facts.checked_events += 1;
        facts.tail_event = Some(index);
    }
    for (index, head) in fields.rows[2].iter().enumerate() {
        let bytes = head.text(3)?.as_bytes();
        let allowance = (bytes.len() as u64)
            .checked_add(1024)
            .ok_or_else(|| storage_fail("additive V1 projection content extent overflow"))?;
        work.metadata(allowance)?;
        let mut hash = Sha256::new();
        hash.update(bytes);
        if !digest_matches(hash, head.text(4)?)? {
            return Err(storage_fail("additive V1 projection content hash differs"));
        }
        facts.checked_heads += 1;
        facts.tail_head = Some(index);
    }
    Ok(())
}

#[cfg(test)]
#[path = "global_schema_v1_event_projection_content_tests.rs"]
mod tests;
