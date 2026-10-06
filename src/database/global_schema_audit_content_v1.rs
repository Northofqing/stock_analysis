//! Ordinary audit content check within the original live reader/owner.
//! V1 content, economic replay and financial/native qualification remain absent.
use super::*;
use crate::database::order_audit::{
    CanonicalOrderAuditView, OrderAuditRecord, AUDIT_CHAIN_GENESIS,
};
use std::io::{self, Write};

#[derive(Default)]
struct AuditContentFacts {
    started: bool,
    callee_reached: bool,
    callee_returned: Option<bool>,
    return_retained: bool,
    checked_rows: usize,
    tail_row: Option<usize>,
}
struct AuditContentFrame {
    raw: RawV1AuditLinksFrame,
    facts: AuditContentFacts,
    result: Option<StorageResult<()>>,
    unaccepted: Option<StorageResult<()>>,
}
pub(in crate::database::global_schema_v1) struct AdditiveStorageLocalAuditContentChecked {
    frame: AuditContentFrame,
}
pub(in crate::database::global_schema_v1) struct AdditiveStorageAuditContentHeld {
    frame: AuditContentFrame,
}
impl AdditiveStorageAuditContentHeld {
    pub(in crate::database::global_schema_v1) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame
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
    pub(in crate::database::global_schema_v1) fn into_audit_content_hashes(
        self,
    ) -> std::result::Result<AdditiveStorageLocalAuditContentChecked, AdditiveStorageAuditContentHeld>
    {
        AuditContentFrame::new(RawV1AuditLinksFrame::new(self.frame)).run(false)
    }
}
impl AdditiveStorageLocalAuditContentChecked {
    pub(in crate::database::global_schema_v1) fn create_or_resume(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageAuditContentHeld> {
        AuditContentFrame::new(raw_v1_audit_links_cold_frame(source)).run(true)
    }
}
impl AuditContentFrame {
    fn new(raw: RawV1AuditLinksFrame) -> Self {
        Self {
            raw,
            facts: AuditContentFacts::default(),
            result: None,
            unaccepted: None,
        }
    }
    fn run(
        mut self,
        cold: bool,
    ) -> std::result::Result<AdditiveStorageLocalAuditContentChecked, AdditiveStorageAuditContentHeld>
    {
        if !self.raw.prepare(cold) || !self.raw.advance_links() || !self.finish() {
            return Err(AdditiveStorageAuditContentHeld { frame: self });
        }
        Ok(AdditiveStorageLocalAuditContentChecked { frame: self })
    }
    fn begin(&mut self) -> bool {
        if self.raw.first() {
            return false;
        }
        if self.raw.phase != RawV1AuditLinksPhase::LinksChecked
            || !self.raw.all_returns()
            || self.raw.input.phase != V1AuditInputPhase::InputsChecked
            || !self.raw.input.all_returns()
            || self.raw.input.genesis.owner.fee.local.readonly.active != Some(1)
            || self.raw.content_hashes != RawV1AuditContentHashes::NotChecked
            || self.facts.started
            || self.result.is_some()
        {
            self.raw.fail(storage_fail(
                "additive audit content lacks live input returns",
            ));
            return false;
        }
        // Fixed frame/scan allowance; each row pays before serializer/hash use.
        if let Err(first) = self
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
            self.raw.fail(first);
            return false;
        }
        self.facts.started = true;
        true
    }
    fn evaluate(&mut self) -> Option<StorageResult<()>> {
        if self.raw.first()
            || !self.facts.started
            || self.facts.callee_reached
            || self.result.is_some()
        {
            return None;
        }
        self.facts.callee_reached = true;
        let actual = self
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .loan()
            .and_then(|(_, _, work, _)| {
                audit_content_hashes(&self.raw.input.fields, work, &mut self.facts)
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
                self.raw.phase,
                RawV1AuditLinksPhase::LinksChecked | RawV1AuditLinksPhase::Refused
            )
        {
            return Err(actual);
        }
        self.result = Some(actual);
        self.facts.return_retained = true;
        if self.raw.first() {
            return Ok(());
        } // Preserve late return and original first error.
        if matches!(self.result, Some(Err(_))) {
            let first = self.result.take().unwrap().unwrap_err();
            self.raw.fail(first);
        } else {
            self.raw.content_hashes = RawV1AuditContentHashes::AuditOnlyChecked;
        }
        Ok(())
    }
    fn close_and_tail(&mut self) -> StorageResult<()> {
        if self.raw.first() {
            return Err(storage_fail(
                "additive audit content close after first error",
            ));
        }
        if self.raw.phase != RawV1AuditLinksPhase::LinksChecked
            || !self.raw.all_returns()
            || self.raw.content_hashes != RawV1AuditContentHashes::AuditOnlyChecked
            || !self.facts.started
            || !self.facts.callee_reached
            || self.facts.callee_returned != Some(true)
            || !self.facts.return_retained
            || !matches!(self.result, Some(Ok(())))
            || self.unaccepted.is_some()
        {
            return Err(storage_fail(
                "additive audit content close before actual result",
            ));
        }
        self.raw.input.close_and_tail()?;
        self.raw.phase = RawV1AuditLinksPhase::Complete;
        Ok(())
    }
    // Success here keeps the original second reader open for the next content
    // scan. Only the consuming entry's final close can produce a checked owner.
    fn advance_content(&mut self) -> bool {
        if !self.begin() {
            return false;
        }
        let Some(actual) = self.evaluate() else {
            self.raw
                .fail(storage_fail("additive audit content result not observed"));
            return false;
        };
        if let Err(actual) = self.retain(actual) {
            self.unaccepted = Some(actual);
            self.raw.fail(storage_fail(
                "additive audit content return already retained",
            ));
            return false;
        }
        if self.raw.first() {
            return false;
        }
        true
    }
    fn finish(&mut self) -> bool {
        if !self.advance_content() {
            return false;
        }
        match self.close_and_tail() {
            Ok(()) => true,
            Err(first) => {
                self.raw.fail(first);
                false
            }
        }
    }
}

#[path = "global_schema_v1_event_projection_content.rs"]
mod v1_event_projection;

fn real(row: &V1AuditInputRow, index: usize) -> StorageResult<f64> {
    match row.cells[index] {
        Some(V1AuditInputCell::RealBits(bits)) => Ok(f64::from_bits(bits)),
        _ => Err(storage_fail("additive audit content REAL absent")),
    }
}
fn optional_real(row: &V1AuditInputRow, index: usize) -> StorageResult<Option<f64>> {
    match row.cells[index] {
        Some(V1AuditInputCell::Null) => Ok(None),
        _ => real(row, index).map(Some),
    }
}
fn optional_text(row: &V1AuditInputRow, index: usize) -> StorageResult<Option<&str>> {
    match row.cells[index] {
        Some(V1AuditInputCell::Null) => Ok(None),
        _ => row.text(index).map(Some),
    }
}
fn borrowed_audit(row: &V1AuditInputRow) -> StorageResult<CanonicalOrderAuditView<'_>> {
    Ok(CanonicalOrderAuditView::new(
        row.integer(0)?,
        OrderAuditRecord {
            business_order_id: row.text(1)?,
            source: row.text(2)?,
            decision_basis: row.text(3)?,
            side: row.text(4)?,
            code: row.text(5)?,
            requested_price: real(row, 6)?,
            execution_price: optional_real(row, 7)?,
            quantity: row.integer(8)?,
            quote_observed_at: optional_text(row, 9)?,
            outcome: row.text(10)?,
            failure_reason: optional_text(row, 11)?,
        },
        row.text(12)?,
    ))
}
// Worst JSON escaping is six bytes per input byte. 2048 covers fixed keys,
// numeric tokens, punctuation, borrowed view, SHA/serde stack and hex output.
fn row_allowance(row: &V1AuditInputRow, previous: &str) -> StorageResult<u64> {
    let mut bytes = 2048u64
        .checked_add(previous.len() as u64)
        .ok_or_else(|| storage_fail("additive audit content extent overflow"))?;
    for cell in &row.cells {
        if let Some(V1AuditInputCell::Text(text)) = cell {
            bytes = (text.len() as u64)
                .checked_mul(6)
                .and_then(|n| bytes.checked_add(n))
                .ok_or_else(|| storage_fail("additive audit content extent overflow"))?;
        }
    }
    Ok(bytes)
}
struct AuditHashWriter {
    hash: Sha256,
    remaining: u64,
}
impl Write for AuditHashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.remaining = self
            .remaining
            .checked_sub(bytes.len() as u64)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
        self.hash.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn audit_content_hashes(
    fields: &V1AuditInputFields,
    work: &mut target::TargetWork,
    facts: &mut AuditContentFacts,
) -> StorageResult<()> {
    work.require_replay_clear()
        .map_err(GlobalSchemaV1Error::ReplayTerminal)?;
    let audits = &fields.rows[3];
    let chain = &fields.rows[4];
    if audits.len() != chain.len() {
        return Err(storage_fail("additive audit content length differs"));
    }
    let mut previous = AUDIT_CHAIN_GENESIS;
    for (index, (audit, evidence)) in audits.iter().zip(chain).enumerate() {
        let allowance = row_allowance(audit, previous)?;
        work.metadata(allowance)?;
        if !crate::database::order_audit::raw_order_audit_link_matches(
            audit.integer(0)?,
            evidence.integer(0)?,
            evidence.text(1)?,
            previous,
        ) {
            return Err(storage_fail("additive audit content predecessor differs"));
        }
        let view = borrowed_audit(audit)?;
        let mut writer = AuditHashWriter {
            hash: Sha256::new(),
            remaining: allowance,
        };
        writer.hash.update(b"BR086_ORDER_AUDIT_V1\0");
        writer.hash.update(previous.as_bytes());
        writer.hash.update(b"\0");
        serde_json::to_writer(&mut writer, &view)
            .map_err(|_| storage_fail("additive audit content serialization failed"))?;
        let mut expected = [0; 64];
        hex::encode_to_slice(writer.hash.finalize(), &mut expected)
            .map_err(|_| storage_fail("additive audit content digest extent differs"))?;
        if evidence.text(2)?.as_bytes() != expected {
            return Err(storage_fail("additive audit content hash differs"));
        }
        facts.checked_rows += 1;
        facts.tail_row = Some(index);
        previous = evidence.text(2)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "global_schema_audit_content_v1_tests.rs"]
mod tests;
