//! Fixed Catalog6-to-8 data codec and retained Intent/Created/Copied storage.
//! No transformed Catalog8 reader, SQL/native/financial qualification is issued.
#![allow(dead_code)]
use super::super::global_schema_catalog_v1::ClosedAdditiveCatalog8Recipe;
use super::target::OriginalBackupBinding;
use sha2::{Digest, Sha256};

type CodecResult<T> = std::result::Result<T, AdditiveCodecFault>;
const MIB: u64 = 1024 * 1024;
const DOMAIN: &[u8] = b"stock_analysis.global_schema.additive_catalog8_target_record.v1";
const LEAVES: [&str; 6] = [
    "000-intent.json",
    "001-created-target.json",
    "002-copied-target.json",
    "003-transform-started.json",
    "004-transformed-target.json",
    "005-verification-started.json",
];
// These limits only bound the bytes observed by this decoder. They do not pay
// for an owned encoding, metadata allocator, SQL operation or filesystem work.
const INTENT_LIMIT: u64 = MIB;
const EVENT_LIMIT: u64 = 64 * 1024;
const HELD_LIMIT: u64 = 2 * MIB;
const JOURNAL_LIMIT: u64 = 32 * MIB;
const EXTENT_LIMIT: u64 = 16 * 1024 * MIB;
const FIXED_LIMITS: &[u8] = b"{\"extent\":17179869184,\"physical\":137438953472,\"metadata\":16777216,\"intent\":1048576,\"event\":65536,\"records\":2097152,\"journal\":33554432,\"review\":1048576}";

/// Declared record data only; equality does not establish a live inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DeclaredRecordNode {
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) links: u64,
    pub(super) owner: u32,
    pub(super) mode: u32,
}
// Directory links are a live stat observation, not an immutable inode key.
// The retained adapter still requires actual directory nlink > 0 on every fstat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DeclaredRecordDirectory {
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) owner: u32,
    pub(super) mode: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DeclaredRecordAnchor {
    pub(super) main_device: u64,
    pub(super) main_inode: u64,
    pub(super) managed: DeclaredRecordDirectory,
    pub(super) operation: DeclaredRecordDirectory,
}
/// The expected data must be supplied by a future retained storage adapter.
/// This type carries neither that adapter's authority nor a File/OwnedNode.
pub(super) struct AdditiveRecordIdentity<'a> {
    pub(super) anchor: DeclaredRecordAnchor,
    pub(super) record_nodes: [Option<DeclaredRecordNode>; 6],
    pub(super) target_node: DeclaredRecordNode,
    pub(super) original: &'a OriginalBackupBinding,
    pub(super) rows: &'a str,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdditiveCodecFault {
    Canonical,
    Identity,
    DuplicateRecordNode,
    Chain,
    Checksum,
    Gap,
    SlotLimit(u8),
    HeldLimit,
    JournalLimit,
    Overflow,
}
/// Non-Clone cumulative observation state. Ending a loan never refunds bytes.
pub(super) struct AdditiveRecordCodecState {
    observed_journal: u64,
    first_fault: Option<AdditiveCodecFault>,
}
impl AdditiveRecordCodecState {
    pub(super) fn new() -> Self {
        Self {
            observed_journal: 0,
            first_fault: None,
        }
    }
    pub(super) fn observed_journal(&self) -> u64 {
        self.observed_journal
    }
    pub(super) fn first_fault(&self) -> Option<AdditiveCodecFault> {
        self.first_fault
    }
    fn latch<T>(&mut self, fault: AdditiveCodecFault) -> CodecResult<T> {
        let first = *self.first_fault.get_or_insert(fault);
        Err(first)
    }
    fn observe_slots(&mut self, slots: &[Option<&[u8]>; 6]) -> CodecResult<()> {
        if let Some(first) = self.first_fault {
            return Err(first);
        }
        let mut held = 0_u64;
        for (slot, bytes) in slots.iter().enumerate() {
            if let Some(bytes) = bytes {
                let length = match u64::try_from(bytes.len()) {
                    Ok(length) => length,
                    Err(_) => return self.latch(AdditiveCodecFault::Overflow),
                };
                self.observed_journal = match self.observed_journal.checked_add(length) {
                    Some(total) => total,
                    None => return self.latch(AdditiveCodecFault::Overflow),
                };
                if self.observed_journal > JOURNAL_LIMIT {
                    return self.latch(AdditiveCodecFault::JournalLimit);
                }
                if length > if slot == 0 { INTENT_LIMIT } else { EVENT_LIMIT } {
                    return self.latch(AdditiveCodecFault::SlotLimit(slot as u8));
                }
                held = match held.checked_add(length) {
                    Some(total) => total,
                    None => return self.latch(AdditiveCodecFault::Overflow),
                };
                if held > HELD_LIMIT {
                    return self.latch(AdditiveCodecFault::HeldLimit);
                }
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AdditiveRecordStage {
    Empty,
    Intent,
    Created,
    Copied,
    TransformStarted,
    Transformed,
    VerificationStarted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BorrowedWitness<'a> {
    node: DeclaredRecordNode,
    length: u64,
    sha256: &'a str,
}
#[derive(Debug, Clone, Copy)]
struct BorrowedRecord<'a> {
    bytes: &'a [u8],
    checksum: [u8; 32],
    witness: Option<BorrowedWitness<'a>>,
}
#[derive(Debug)]
pub(super) struct AdditiveRecordPrefix<'bytes> {
    records: [Option<BorrowedRecord<'bytes>>; 6],
    count: usize,
}
impl AdditiveRecordPrefix<'_> {
    pub(super) fn stage(&self) -> AdditiveRecordStage {
        match self.count {
            0 => AdditiveRecordStage::Empty,
            1 => AdditiveRecordStage::Intent,
            2 => AdditiveRecordStage::Created,
            3 => AdditiveRecordStage::Copied,
            4 => AdditiveRecordStage::TransformStarted,
            5 => AdditiveRecordStage::Transformed,
            _ => AdditiveRecordStage::VerificationStarted,
        }
    }
    pub(super) fn count(&self) -> usize {
        self.count
    }
}

/// Decode only the six supplied buffers. This cannot observe an extra file in
/// a directory, actual inode identity, fsync, DDL or verification completion.
pub(super) fn decode_prefix<'bytes>(
    recipe: &ClosedAdditiveCatalog8Recipe,
    identity: &AdditiveRecordIdentity<'_>,
    slots: [Option<&'bytes [u8]>; 6],
    state: &mut AdditiveRecordCodecState,
) -> CodecResult<AdditiveRecordPrefix<'bytes>> {
    state.observe_slots(&slots)?;
    match decode_observed_prefix(recipe, identity, slots) {
        Ok(prefix) => Ok(prefix),
        Err(fault) => state.latch(fault),
    }
}
fn decode_observed_prefix<'bytes>(
    recipe: &ClosedAdditiveCatalog8Recipe,
    identity: &AdditiveRecordIdentity<'_>,
    slots: [Option<&'bytes [u8]>; 6],
) -> CodecResult<AdditiveRecordPrefix<'bytes>> {
    if identity.original.canonical.is_empty()
        || identity.rows.is_empty()
        || identity.original.length > EXTENT_LIMIT
        || !lower_hash(&identity.original.sha256)
    {
        return Err(AdditiveCodecFault::Identity);
    }
    let mut prefix = AdditiveRecordPrefix {
        records: [None; 6],
        count: 0,
    };
    let mut gap = false;
    for (slot, bytes) in slots.into_iter().enumerate() {
        let Some(bytes) = bytes else {
            gap = true;
            continue;
        };
        if gap {
            return Err(AdditiveCodecFault::Gap);
        }
        // Absent future slots need no invented inode. Compare only declarations
        // for buffers actually supplied, including duplicate inode aliases.
        let node = identity.record_nodes[slot].ok_or(AdditiveCodecFault::Identity)?;
        if node.links != 1 || node.mode != 0o600 {
            return Err(AdditiveCodecFault::Identity);
        }
        if identity.record_nodes[..slot]
            .iter()
            .flatten()
            .any(|other| other.device == node.device && other.inode == node.inode)
        {
            return Err(AdditiveCodecFault::DuplicateRecordNode);
        }
        let intent = prefix.records[0].map(|record| record.checksum);
        let predecessor = slot
            .checked_sub(1)
            .and_then(|index| prefix.records[index])
            .map(|record| record.checksum);
        let prior_witness = slot
            .checked_sub(1)
            .and_then(|index| prefix.records[index])
            .and_then(|record| record.witness);
        prefix.records[slot] = Some(decode_record(
            recipe,
            identity,
            slot,
            bytes,
            intent,
            predecessor,
            prior_witness,
        )?);
        prefix.count += 1;
    }
    Ok(prefix)
}
fn decode_record<'a>(
    recipe: &ClosedAdditiveCatalog8Recipe,
    identity: &AdditiveRecordIdentity<'_>,
    slot: usize,
    bytes: &'a [u8],
    intent: Option<[u8; 32]>,
    predecessor: Option<[u8; 32]>,
    prior_witness: Option<BorrowedWitness<'a>>,
) -> CodecResult<BorrowedRecord<'a>> {
    let mut cursor = Cursor { bytes, offset: 0 };
    cursor.literal(b"{\"sha256\":")?;
    let claimed = decode_hash(cursor.hash()?);
    cursor.literal(b",\"record\":")?;
    let start = cursor.offset;
    cursor.literal(b"{\"version\":")?;
    cursor.equal_number(1)?;
    cursor.literal(b",\"slot\":")?;
    cursor.equal_number(slot as u64)?;
    cursor.literal(b",\"leaf\":")?;
    cursor.equal_string(LEAVES[slot])?;
    cursor.literal(b",\"anchor\":")?;
    if cursor.anchor()? != identity.anchor {
        return Err(AdditiveCodecFault::Identity);
    }
    cursor.literal(b",\"self_inode\":")?;
    if Some(cursor.node()?) != identity.record_nodes[slot] {
        return Err(AdditiveCodecFault::Identity);
    }
    cursor.literal(b",\"intent\":")?;
    cursor.chain_hash(intent)?;
    cursor.literal(b",\"predecessor\":")?;
    cursor.chain_hash(predecessor)?;
    cursor.literal(b",\"transition\":")?;
    let witness = match slot {
        0 => {
            cursor.literal(b"{\"kind\":\"intent\",\"binding\":{\"recipe\":")?;
            cursor.equal_string(recipe.id())?;
            cursor.literal(b",\"original\":{\"canonical\":")?;
            cursor.equal_string(&identity.original.canonical)?;
            cursor.literal(b",\"length\":")?;
            cursor.equal_number(identity.original.length)?;
            cursor.literal(b",\"sha256\":")?;
            cursor.equal_string(&identity.original.sha256)?;
            cursor.literal(b"},\"rows\":")?;
            cursor.equal_string(identity.rows)?;
            cursor.literal(b",\"limits\":")?;
            cursor.literal(FIXED_LIMITS)?;
            cursor.literal(b"}}")?;
            None
        }
        1 => {
            cursor.literal(b"{\"kind\":\"created\",\"file\":")?;
            if cursor.node()? != identity.target_node {
                return Err(AdditiveCodecFault::Identity);
            }
            cursor.literal(b"}")?;
            None
        }
        2..=5 => {
            let kind: &[u8] = match slot {
                2 => b"{\"kind\":\"copied\",\"file\":",
                3 => b"{\"kind\":\"transform_started\",\"file\":",
                4 => b"{\"kind\":\"transformed\",\"file\":",
                _ => b"{\"kind\":\"verification_started\",\"file\":",
            };
            cursor.literal(kind)?;
            let witness = cursor.witness()?;
            if witness.node != identity.target_node || witness.length > EXTENT_LIMIT {
                return Err(AdditiveCodecFault::Identity);
            }
            if slot == 2
                && (witness.length != identity.original.length
                    || witness.sha256 != identity.original.sha256)
            {
                return Err(AdditiveCodecFault::Identity);
            }
            if matches!(slot, 3 | 5) && prior_witness != Some(witness) {
                return Err(AdditiveCodecFault::Chain);
            }
            cursor.literal(b"}")?;
            Some(witness)
        }
        _ => return Err(AdditiveCodecFault::Canonical),
    };
    cursor.literal(b"}")?;
    let end = cursor.offset;
    cursor.literal(b"}")?;
    if cursor.offset != bytes.len() {
        return Err(AdditiveCodecFault::Canonical);
    }
    let checksum = record_checksum(&bytes[start..end]);
    if claimed != checksum {
        return Err(AdditiveCodecFault::Checksum);
    }
    Ok(BorrowedRecord {
        bytes,
        checksum,
        witness,
    })
}
fn record_checksum(bytes: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(DOMAIN);
    digest.update([0]);
    digest.update(bytes);
    digest.finalize().into()
}
fn lower_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn decode_hash(value: &str) -> [u8; 32] {
    fn nibble(byte: u8) -> u8 {
        if byte <= b'9' {
            byte - b'0'
        } else {
            byte - b'a' + 10
        }
    }
    let mut hash = [0; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        hash[index] = nibble(pair[0]) * 16 + nibble(pair[1]);
    }
    hash
}
/// A fixed-schema cursor: no general JSON tree, decoder scratch or owned string.
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn literal(&mut self, literal: &[u8]) -> CodecResult<()> {
        let end = self
            .offset
            .checked_add(literal.len())
            .ok_or(AdditiveCodecFault::Overflow)?;
        if self.bytes.get(self.offset..end) != Some(literal) {
            return Err(AdditiveCodecFault::Canonical);
        }
        self.offset = end;
        Ok(())
    }
    fn number(&mut self) -> CodecResult<u64> {
        let start = self.offset;
        let mut value = 0_u64;
        while let Some(byte @ b'0'..=b'9') = self.bytes.get(self.offset).copied() {
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(u64::from(byte - b'0')))
                .ok_or(AdditiveCodecFault::Overflow)?;
            self.offset += 1;
        }
        if self.offset == start || (self.offset - start > 1 && self.bytes[start] == b'0') {
            return Err(AdditiveCodecFault::Canonical);
        }
        Ok(value)
    }
    fn equal_number(&mut self, expected: u64) -> CodecResult<()> {
        if self.number()? != expected {
            return Err(AdditiveCodecFault::Identity);
        }
        Ok(())
    }
    fn equal_string(&mut self, expected: &str) -> CodecResult<()> {
        self.literal(b"\"")?;
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in expected.bytes() {
            match byte {
                b'"' => self.literal(b"\\\"")?,
                b'\\' => self.literal(b"\\\\")?,
                b'\x08' => self.literal(b"\\b")?,
                b'\x0c' => self.literal(b"\\f")?,
                b'\n' => self.literal(b"\\n")?,
                b'\r' => self.literal(b"\\r")?,
                b'\t' => self.literal(b"\\t")?,
                0..=31 => self.literal(&[
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX[usize::from(byte >> 4)],
                    HEX[usize::from(byte & 15)],
                ])?,
                _ => self.literal(&[byte])?,
            }
        }
        self.literal(b"\"")
    }
    fn hash(&mut self) -> CodecResult<&'a str> {
        self.literal(b"\"")?;
        let end = self
            .offset
            .checked_add(64)
            .ok_or(AdditiveCodecFault::Overflow)?;
        let raw = self
            .bytes
            .get(self.offset..end)
            .ok_or(AdditiveCodecFault::Canonical)?;
        let hash = std::str::from_utf8(raw).map_err(|_| AdditiveCodecFault::Canonical)?;
        if !lower_hash(hash) {
            return Err(AdditiveCodecFault::Canonical);
        }
        self.offset = end;
        self.literal(b"\"")?;
        Ok(hash)
    }
    fn chain_hash(&mut self, expected: Option<[u8; 32]>) -> CodecResult<()> {
        match expected {
            None => self.literal(b"null"),
            Some(expected) => {
                if decode_hash(self.hash()?) != expected {
                    return Err(AdditiveCodecFault::Chain);
                }
                Ok(())
            }
        }
    }
    fn node(&mut self) -> CodecResult<DeclaredRecordNode> {
        self.literal(b"{\"device\":")?;
        let device = self.number()?;
        self.literal(b",\"inode\":")?;
        let inode = self.number()?;
        self.literal(b",\"links\":")?;
        let links = self.number()?;
        self.literal(b",\"owner\":")?;
        let owner = u32::try_from(self.number()?).map_err(|_| AdditiveCodecFault::Overflow)?;
        self.literal(b",\"mode\":")?;
        let mode = u32::try_from(self.number()?).map_err(|_| AdditiveCodecFault::Overflow)?;
        self.literal(b"}")?;
        Ok(DeclaredRecordNode {
            device,
            inode,
            links,
            owner,
            mode,
        })
    }
    fn directory(&mut self) -> CodecResult<DeclaredRecordDirectory> {
        self.literal(b"{\"device\":")?;
        let device = self.number()?;
        self.literal(b",\"inode\":")?;
        let inode = self.number()?;
        self.literal(b",\"owner\":")?;
        let owner = u32::try_from(self.number()?).map_err(|_| AdditiveCodecFault::Overflow)?;
        self.literal(b",\"mode\":")?;
        let mode = u32::try_from(self.number()?).map_err(|_| AdditiveCodecFault::Overflow)?;
        self.literal(b"}")?;
        if mode != 0o700 {
            return Err(AdditiveCodecFault::Identity);
        }
        Ok(DeclaredRecordDirectory {
            device,
            inode,
            owner,
            mode,
        })
    }
    fn anchor(&mut self) -> CodecResult<DeclaredRecordAnchor> {
        self.literal(b"{\"main_device\":")?;
        let main_device = self.number()?;
        self.literal(b",\"main_inode\":")?;
        let main_inode = self.number()?;
        self.literal(b",\"managed\":")?;
        let managed = self.directory()?;
        self.literal(b",\"operation\":")?;
        let operation = self.directory()?;
        self.literal(b"}")?;
        Ok(DeclaredRecordAnchor {
            main_device,
            main_inode,
            managed,
            operation,
        })
    }
    fn witness(&mut self) -> CodecResult<BorrowedWitness<'a>> {
        self.literal(b"{\"node\":")?;
        let node = self.node()?;
        self.literal(b",\"length\":")?;
        let length = self.number()?;
        self.literal(b",\"sha256\":")?;
        let sha256 = self.hash()?;
        self.literal(b"}")?;
        Ok(BorrowedWitness {
            node,
            length,
            sha256,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::global_schema_catalog_v1::additive_catalog6_recipe_for_test;
    use super::*;

    // These owned encoders construct test bytes only. Production has neither
    // an owned encoder nor a factory for an original/file/target capability.
    fn fixture_node(inode: u64) -> DeclaredRecordNode {
        DeclaredRecordNode {
            device: 1,
            inode,
            links: 1,
            owner: 501,
            mode: 0o600,
        }
    }
    fn fixture_identity<'a>(
        original: &'a OriginalBackupBinding,
        rows: &'a str,
    ) -> AdditiveRecordIdentity<'a> {
        AdditiveRecordIdentity {
            anchor: DeclaredRecordAnchor {
                main_device: 1,
                main_inode: 10,
                managed: DeclaredRecordDirectory {
                    device: 1,
                    inode: 20,
                    owner: 501,
                    mode: 0o700,
                },
                operation: DeclaredRecordDirectory {
                    device: 1,
                    inode: 30,
                    owner: 501,
                    mode: 0o700,
                },
            },
            record_nodes: std::array::from_fn(|slot| Some(fixture_node(100 + slot as u64))),
            target_node: fixture_node(90),
            original,
            rows,
        }
    }
    fn node_text(node: DeclaredRecordNode) -> String {
        format!(
            "{{\"device\":{},\"inode\":{},\"links\":{},\"owner\":{},\"mode\":{}}}",
            node.device, node.inode, node.links, node.owner, node.mode
        )
    }
    fn directory_text(node: DeclaredRecordDirectory) -> String {
        format!(
            "{{\"device\":{},\"inode\":{},\"owner\":{},\"mode\":{}}}",
            node.device, node.inode, node.owner, node.mode
        )
    }
    fn record_fixture(
        recipe: &ClosedAdditiveCatalog8Recipe,
        identity: &AdditiveRecordIdentity<'_>,
    ) -> [Vec<u8>; 6] {
        let mut hashes: [String; 6] = std::array::from_fn(|_| String::new());
        std::array::from_fn(|slot| {
            let anchor = identity.anchor;
            let anchor = format!(
                "{{\"main_device\":{},\"main_inode\":{},\"managed\":{},\"operation\":{}}}",
                anchor.main_device,
                anchor.main_inode,
                directory_text(anchor.managed),
                directory_text(anchor.operation)
            );
            let transition = match slot {
                0 => format!("{{\"kind\":\"intent\",\"binding\":{{\"recipe\":{},\"original\":{{\"canonical\":{},\"length\":{},\"sha256\":{}}},\"rows\":{},\"limits\":{}}}}}",
                    serde_json::to_string(recipe.id()).unwrap(), serde_json::to_string(&identity.original.canonical).unwrap(),
                    identity.original.length, serde_json::to_string(&identity.original.sha256).unwrap(),
                    serde_json::to_string(identity.rows).unwrap(), std::str::from_utf8(FIXED_LIMITS).unwrap()),
                1 => format!("{{\"kind\":\"created\",\"file\":{}}}", node_text(identity.target_node)),
                _ => {
                    let kind = ["", "", "copied", "transform_started", "transformed", "verification_started"][slot];
                    let (length, hash) = if slot < 4 { (identity.original.length, identity.original.sha256.clone()) }
                        else { (8192, "b".repeat(64)) };
                    format!("{{\"kind\":\"{kind}\",\"file\":{{\"node\":{},\"length\":{length},\"sha256\":\"{hash}\"}}}}", node_text(identity.target_node))
                }
            };
            let intent = if slot == 0 {
                "null".to_owned()
            } else {
                serde_json::to_string(&hashes[0]).unwrap()
            };
            let predecessor = if slot == 0 {
                "null".to_owned()
            } else {
                serde_json::to_string(&hashes[slot - 1]).unwrap()
            };
            let record = format!("{{\"version\":1,\"slot\":{slot},\"leaf\":{},\"anchor\":{anchor},\"self_inode\":{},\"intent\":{intent},\"predecessor\":{predecessor},\"transition\":{transition}}}",
                serde_json::to_string(LEAVES[slot]).unwrap(), node_text(identity.record_nodes[slot].unwrap()));
            hashes[slot] = hex::encode(record_checksum(record.as_bytes()));
            format!("{{\"sha256\":\"{}\",\"record\":{record}}}", hashes[slot]).into_bytes()
        })
    }
    fn prefix_slots(records: &[Vec<u8>; 6], count: usize) -> [Option<&[u8]>; 6] {
        std::array::from_fn(|slot| (slot < count).then_some(records[slot].as_slice()))
    }
    fn replace_record(records: &mut [Vec<u8>; 6], slot: usize, before: &str, after: &str) {
        let text = std::str::from_utf8(&records[slot]).unwrap();
        assert!(
            text.contains(before),
            "mutation must touch the intended bytes"
        );
        records[slot] = text.replacen(before, after, 1).into_bytes();
    }

    #[test]
    fn task6_additive_codec_validates_complete_prefixes_and_foreign_records() {
        let recipe = additive_catalog6_recipe_for_test();
        let original = OriginalBackupBinding {
            canonical: "source/quoted\"\\path\n中文\u{0001}".into(),
            length: 4096,
            sha256: "a".repeat(64),
        };
        let identity = fixture_identity(&original, "{\"rows\":\"retained transcript\"}");
        let records = record_fixture(&recipe, &identity);
        for count in 0..=6 {
            let mut state = AdditiveRecordCodecState::new();
            let prefix = decode_prefix(
                &recipe,
                &identity,
                prefix_slots(&records, count),
                &mut state,
            )
            .unwrap();
            assert_eq!(prefix.count(), count);
            assert_eq!(
                state.observed_journal(),
                records[..count]
                    .iter()
                    .map(|record| record.len() as u64)
                    .sum::<u64>()
            );
            if count > 0 {
                assert!(std::ptr::eq(
                    prefix.records[count - 1].unwrap().bytes.as_ptr(),
                    records[count - 1].as_ptr()
                ));
            }
            if count == 6 {
                assert_eq!(prefix.stage(), AdditiveRecordStage::VerificationStarted);
                assert_eq!(
                    prefix.records[2].unwrap().witness.unwrap().sha256,
                    original.sha256
                );
                assert_eq!(
                    prefix.records[4].unwrap().witness.unwrap().sha256,
                    "b".repeat(64)
                );
            }
        }
        // A real prefix requires no declarations for as-yet absent slots.
        let mut partial_identity = fixture_identity(&original, identity.rows);
        partial_identity.record_nodes[2..].fill(None);
        assert!(decode_prefix(
            &recipe,
            &partial_identity,
            prefix_slots(&records, 2),
            &mut AdditiveRecordCodecState::new()
        )
        .is_ok());
        let mut gap = prefix_slots(&records, 6);
        gap[1] = None;
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                gap,
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Gap
        );
        let mut duplicate = prefix_slots(&records, 3);
        duplicate[2] = Some(&records[1]);
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                duplicate,
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Identity
        );
        let mut foreign = fixture_identity(&original, identity.rows);
        foreign.anchor.main_inode += 1;
        assert_eq!(
            decode_prefix(
                &recipe,
                &foreign,
                prefix_slots(&records, 1),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Identity
        );
        let mut foreign_directory = fixture_identity(&original, identity.rows);
        foreign_directory.anchor.operation.inode += 1;
        assert_eq!(
            decode_prefix(
                &recipe,
                &foreign_directory,
                prefix_slots(&records, 1),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Identity
        );
        let mut old_directory_shape = records.clone();
        replace_record(
            &mut old_directory_shape,
            0,
            "\"managed\":{\"device\":1,\"inode\":20,\"owner\":501",
            "\"managed\":{\"device\":1,\"inode\":20,\"links\":2,\"owner\":501",
        );
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                prefix_slots(&old_directory_shape, 1),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Canonical
        );
        let other = OriginalBackupBinding {
            canonical: "foreign-source".into(),
            length: original.length,
            sha256: original.sha256.clone(),
        };
        assert!(decode_prefix(
            &recipe,
            &fixture_identity(&other, identity.rows),
            prefix_slots(&records, 1),
            &mut AdditiveRecordCodecState::new()
        )
        .is_err());
        let mut alias = fixture_identity(&original, identity.rows);
        alias.record_nodes[1] = alias.record_nodes[0];
        assert_eq!(
            decode_prefix(
                &recipe,
                &alias,
                prefix_slots(&records, 2),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::DuplicateRecordNode
        );
        let mut changed = records.clone();
        replace_record(&mut changed, 1, "\"inode\":101", "\"inode\":999");
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                prefix_slots(&changed, 2),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Identity
        );
        let mut changed = records.clone();
        let predecessor = format!(
            "\"predecessor\":\"{}\"",
            hex::encode(record_checksum(prefix_record_bytes(&records[0])))
        );
        replace_record(
            &mut changed,
            1,
            &predecessor,
            &format!("\"predecessor\":\"{}\"", "c".repeat(64)),
        );
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                prefix_slots(&changed, 2),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Chain
        );
        let mut changed = records.clone();
        replace_record(&mut changed, 5, "\"length\":8192", "\"length\":8193");
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                prefix_slots(&changed, 6),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Chain
        );
        let mut changed = records.clone();
        replace_record(&mut changed, 0, "\"version\":1", "\"version\":01");
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                prefix_slots(&changed, 1),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Canonical
        );
        let mut changed = records.clone();
        replace_record(
            &mut changed,
            0,
            "\"version\":1",
            "\"version\":1,\"unknown\":0",
        );
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                prefix_slots(&changed, 1),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Canonical
        );
        let mut foreign_rows = fixture_identity(&original, "foreign rows transcript");
        assert!(decode_prefix(
            &recipe,
            &foreign_rows,
            prefix_slots(&records, 1),
            &mut AdditiveRecordCodecState::new()
        )
        .is_err());
        foreign_rows.record_nodes[0] = None;
        assert_eq!(
            decode_prefix(
                &recipe,
                &foreign_rows,
                prefix_slots(&records, 1),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Identity
        );
        let mut old_domain = Sha256::new();
        old_domain.update(b"stock_analysis.global_schema.requalification_target_record.v1");
        old_domain.update([0]);
        old_domain.update(prefix_record_bytes(&records[0]));
        let mut changed = records.clone();
        changed[0][11..75].copy_from_slice(hex::encode(old_domain.finalize()).as_bytes());
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                prefix_slots(&changed, 1),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Checksum
        );
        let mut changed = records.clone();
        changed[0].push(b'\n');
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                prefix_slots(&changed, 1),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Canonical
        );
        let mut changed = records.clone();
        replace_record(
            &mut changed,
            0,
            "\"version\":1",
            "\"version\":18446744073709551616",
        );
        let mut overflow = AdditiveRecordCodecState::new();
        assert_eq!(
            decode_prefix(&recipe, &identity, prefix_slots(&changed, 1), &mut overflow)
                .unwrap_err(),
            AdditiveCodecFault::Overflow
        );
        assert_eq!(overflow.first_fault(), Some(AdditiveCodecFault::Overflow));
        let mut changed = records.clone();
        changed[0][11] = if changed[0][11] == b'a' { b'b' } else { b'a' };
        assert_eq!(
            decode_prefix(
                &recipe,
                &identity,
                prefix_slots(&changed, 1),
                &mut AdditiveRecordCodecState::new()
            )
            .unwrap_err(),
            AdditiveCodecFault::Checksum
        );
    }
    fn prefix_record_bytes(envelope: &[u8]) -> &[u8] {
        // Locate the fixed record field in fixture bytes; no JSON tree decoding.
        let marker = b",\"record\":";
        let start = envelope
            .windows(marker.len())
            .position(|window| window == marker)
            .unwrap()
            + marker.len();
        &envelope[start..envelope.len() - 1]
    }

    #[test]
    fn task6_additive_codec_observation_limits_are_sticky_across_moves() {
        // Pure byte-length admission, using actual slices, not synthetic u64
        // meter mutations or an assertion that these bytes form valid records.
        for slot in 0..6 {
            let limit = if slot == 0 { INTENT_LIMIT } else { EVENT_LIMIT } as usize;
            let exact = vec![0; limit];
            let mut slots = [None; 6];
            slots[slot] = Some(exact.as_slice());
            let mut state = AdditiveRecordCodecState::new();
            state.observe_slots(&slots).unwrap();
            assert_eq!(state.observed_journal(), limit as u64);
            let over = vec![0; limit + 1];
            slots[slot] = Some(over.as_slice());
            let mut short = AdditiveRecordCodecState::new();
            assert_eq!(
                short.observe_slots(&slots),
                Err(AdditiveCodecFault::SlotLimit(slot as u8))
            );
            let used = short.observed_journal();
            assert_eq!(
                short.observe_slots(&[None; 6]),
                Err(AdditiveCodecFault::SlotLimit(slot as u8))
            );
            assert_eq!(short.observed_journal(), used);
        }
        // Sum of all six individual maxima is below HELD_LIMIT. The redundant
        // held check is retained, without fabricating an unreachable overflow.
        let buffers: [Vec<u8>; 6] = std::array::from_fn(|slot| {
            vec![0; if slot == 0 { INTENT_LIMIT } else { EVENT_LIMIT } as usize]
        });
        let all = std::array::from_fn(|slot| Some(buffers[slot].as_slice()));
        let mut held = AdditiveRecordCodecState::new();
        held.observe_slots(&all).unwrap();
        assert_eq!(held.observed_journal(), INTENT_LIMIT + 5 * EVENT_LIMIT);
        assert!(held.observed_journal() < HELD_LIMIT);
        let intent = vec![0; INTENT_LIMIT as usize];
        let repeated = [Some(intent.as_slice()), None, None, None, None, None];
        let mut exact = AdditiveRecordCodecState::new();
        for _ in 0..32 {
            exact.observe_slots(&repeated).unwrap();
        }
        assert_eq!(exact.observed_journal(), JOURNAL_LIMIT);
        assert_eq!(
            exact.observe_slots(&[Some(&[0_u8]), None, None, None, None, None]),
            Err(AdditiveCodecFault::JournalLimit)
        );
        let attempted = exact.observed_journal();
        let mut moved = exact;
        assert_eq!(moved.first_fault(), Some(AdditiveCodecFault::JournalLimit));
        assert_eq!(
            moved.observe_slots(&[None; 6]),
            Err(AdditiveCodecFault::JournalLimit)
        );
        assert_eq!(moved.observed_journal(), attempted);
        let mut short = AdditiveRecordCodecState::new();
        for _ in 0..31 {
            short.observe_slots(&repeated).unwrap();
        }
        short
            .observe_slots(&[
                Some(&intent[..intent.len() - 1]),
                None,
                None,
                None,
                None,
                None,
            ])
            .unwrap();
        assert_eq!(short.observed_journal(), JOURNAL_LIMIT - 1);
        short
            .observe_slots(&[Some(&[0_u8]), None, None, None, None, None])
            .unwrap();
        assert_eq!(short.observed_journal(), JOURNAL_LIMIT);
        assert_eq!(
            short.observe_slots(&[Some(&[0_u8]), None, None, None, None, None]),
            Err(AdditiveCodecFault::JournalLimit)
        );

        let recipe = additive_catalog6_recipe_for_test();
        let original = OriginalBackupBinding {
            canonical: "retained-original".into(),
            length: 4096,
            sha256: "a".repeat(64),
        };
        let identity = fixture_identity(&original, "actual borrowed rows binding");
        let records = record_fixture(&recipe, &identity);
        let mut state = AdditiveRecordCodecState::new();
        let prefix =
            decode_prefix(&recipe, &identity, prefix_slots(&records, 6), &mut state).unwrap();
        let charged = state.observed_journal();
        drop(prefix);
        assert_eq!(state.observed_journal(), charged);
        let mut state = state;
        decode_prefix(&recipe, &identity, prefix_slots(&records, 1), &mut state).unwrap();
        assert_eq!(state.observed_journal(), charged + records[0].len() as u64);
        let mut bad = records.clone();
        bad[0].push(b' ');
        assert_eq!(
            decode_prefix(&recipe, &identity, prefix_slots(&bad, 1), &mut state).unwrap_err(),
            AdditiveCodecFault::Canonical
        );
        let charged = state.observed_journal();
        assert_eq!(
            decode_prefix(&recipe, &identity, prefix_slots(&records, 6), &mut state).unwrap_err(),
            AdditiveCodecFault::Canonical
        );
        assert_eq!(state.observed_journal(), charged);
    }
}

// Real additive storage prefix, ending at durable Copied. No write Connection,
// transform, readonly reader, verification or financial capability is issued.
use super::{prospective, rows, target, GlobalSchemaV1Error};
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, MetadataExt};

type StorageResult<T> = std::result::Result<T, GlobalSchemaV1Error>;
const STORAGE_MANAGED: &str = "global-schema-targets";
const STORAGE_OPERATION: &str = "upgrade-exact-amended-catalog6-to-catalog8-v1";
const STORAGE_TARGET: &str = "stock_analysis.db.target";
fn storage_fail(detail: &'static str) -> GlobalSchemaV1Error {
    prospective::refusal(detail)
}
fn storage_io<T>(result: io::Result<T>) -> StorageResult<T> {
    result.map_err(|_| storage_fail("additive descriptor IO failed"))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
struct StorageNode {
    device: u64,
    inode: u64,
    links: u64,
    owner: u32,
    mode: u32,
}
impl StorageNode {
    fn from_file(file: &File, directory: bool) -> StorageResult<Self> {
        let m = storage_io(file.metadata())?;
        if m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o7777 != if directory { 0o700 } else { 0o600 }
            || if directory {
                !m.is_dir() || m.nlink() == 0
            } else {
                !m.is_file() || m.nlink() != 1
            }
        {
            return Err(storage_fail("additive inode type/euid/mode/link mismatch"));
        }
        Ok(Self {
            device: m.dev(),
            inode: m.ino(),
            links: m.nlink(),
            owner: m.uid(),
            mode: m.mode() & 0o7777,
        })
    }
    fn declared(self) -> DeclaredRecordNode {
        DeclaredRecordNode {
            device: self.device,
            inode: self.inode,
            links: self.links,
            owner: self.owner,
            mode: self.mode,
        }
    }
}
// Files retain exact nlink=1 identity above. Directories have a separate key;
// their current nonzero link count is checked, never normalized or persisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
struct StorageDirectoryIdentity {
    device: u64,
    inode: u64,
    owner: u32,
    mode: u32,
}
impl StorageDirectoryIdentity {
    fn from_file(file: &File) -> StorageResult<Self> {
        let node = StorageNode::from_file(file, true)?;
        Ok(Self {
            device: node.device,
            inode: node.inode,
            owner: node.owner,
            mode: node.mode,
        })
    }
    fn declared(self) -> DeclaredRecordDirectory {
        DeclaredRecordDirectory {
            device: self.device,
            inode: self.inode,
            owner: self.owner,
            mode: self.mode,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
struct StorageAnchor {
    main_device: u64,
    main_inode: u64,
    managed: StorageDirectoryIdentity,
    operation: StorageDirectoryIdentity,
}
impl StorageAnchor {
    fn declared(self) -> DeclaredRecordAnchor {
        DeclaredRecordAnchor {
            main_device: self.main_device,
            main_inode: self.main_inode,
            managed: self.managed.declared(),
            operation: self.operation.declared(),
        }
    }
}
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
struct StorageWitness {
    node: StorageNode,
    length: u64,
    sha256: String,
}
#[derive(serde::Serialize)]
struct StorageIntent<'a> {
    recipe: &'a str,
    original: &'a OriginalBackupBinding,
    rows: &'a str,
    limits: target::Limits,
}
#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StorageTransition<'a> {
    Intent { binding: StorageIntent<'a> },
    Created { file: StorageNode },
    Copied { file: &'a StorageWitness },
    TransformStarted { file: &'a StorageWitness },
    Transformed { file: &'a StorageWitness },
    VerificationStarted { file: &'a StorageWitness },
}
#[derive(serde::Serialize)]
struct StorageRecord<'a> {
    version: u16,
    slot: u8,
    leaf: &'static str,
    anchor: StorageAnchor,
    self_inode: StorageNode,
    intent: Option<&'a str>,
    predecessor: Option<&'a str>,
    transition: StorageTransition<'a>,
}
#[derive(serde::Serialize)]
struct StorageEnvelope<'a, 'b> {
    sha256: &'a str,
    record: &'a StorageRecord<'b>,
}
struct StorageSaved {
    file: File,
    node: StorageNode,
    bytes: Vec<u8>,
    hash: String,
}
struct StoragePendingRecord {
    file: File,
    node: Option<StorageNode>,
    canonical: Vec<u8>,
    bytes: Vec<u8>,
    readback: Vec<u8>,
}

// One-use loan; its private fields cannot be made from an arbitrary File.
pub(super) struct DurableAdditiveCreatedPermit<'a> {
    slot: &'a mut Option<File>,
    rejected: &'a mut Option<File>,
    expected: StorageNode,
    descriptor: i32,
    taken: bool,
    failed: &'a mut bool,
    failure: &'a mut Option<GlobalSchemaV1Error>,
    post_copy_failure: &'a mut Option<GlobalSchemaV1Error>,
}
impl DurableAdditiveCreatedPermit<'_> {
    pub(super) fn take_file(&mut self) -> StorageResult<File> {
        if self.taken || self.slot.is_none() {
            *self.failed = true;
            return Err(storage_fail(
                "additive Created File already consumed/missing",
            ));
        }
        let file = self.slot.as_ref().unwrap();
        match StorageNode::from_file(file, false) {
            Ok(node) if node == self.expected && file.as_raw_fd() == self.descriptor => {}
            result => {
                *self.failed = true;
                *self.failure =
                    Some(result.err().unwrap_or_else(|| {
                        storage_fail("additive Created File changed before move")
                    }));
                return Err(storage_fail("additive Created permit move refused"));
            }
        }
        self.taken = true;
        Ok(self.slot.take().unwrap())
    }
    pub(super) fn retain_post_copy_failure(&mut self, error: GlobalSchemaV1Error) {
        // The one fixed T caller supplies only its actual outer loan return.
        *self.post_copy_failure = Some(error);
    }
    // Consume the permit so a second return is impossible. Park an actual File
    // before fstat/refusal; neither a bad return nor its diagnostic is dropped.
    pub(super) fn return_file(self, file: File) -> bool {
        if self.slot.is_some() {
            *self.rejected = Some(file);
            *self.failed = true;
            *self.failure = Some(storage_fail("additive return into occupied File slot"));
            return false;
        }
        *self.slot = Some(file);
        let file = self.slot.as_ref().unwrap();
        match StorageNode::from_file(file, false) {
            Ok(node)
                if self.taken && node == self.expected && file.as_raw_fd() == self.descriptor =>
            {
                true
            }
            result => {
                *self.failed = true;
                *self.failure =
                    Some(result.err().unwrap_or_else(|| {
                        storage_fail("additive copy returned a different File")
                    }));
                false
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StorageCut {
    None,
    AfterIntent,
    AfterTargetOpen,
    AfterCreated,
    AfterCopy,
    ExhaustBeforeCopy,
}

/// No Clone/Deserialize, and no cleanup Drop. Partial outputs and records are
/// retained on disk, and every acquired main File stays in this owner on Err.
pub(super) struct AdditiveStorageCopied {
    source: rows::AdditiveRowsTargetSource,
    managed: Option<File>,
    directory: Option<File>,
    anchor: Option<StorageAnchor>,
    fresh: bool,
    original: Option<OriginalBackupBinding>,
    rows: Option<String>,
    records: [Option<StorageSaved>; 6],
    pending: Option<StoragePendingRecord>,
    target: Option<File>,
    target_node: Option<StorageNode>,
    copied: Option<StorageWitness>,
    census_files: [Option<File>; 7],
    rejected_copy_return: Option<File>,
    copy_return_failed: bool,
    copy_return_error: Option<GlobalSchemaV1Error>,
    copy_origin_return_error: Option<GlobalSchemaV1Error>,
    codec: AdditiveRecordCodecState,
    copy_issued: bool,
}
pub(super) struct AdditiveStorageHeld {
    owner: AdditiveStorageCopied,
    first: GlobalSchemaV1Error,
}
impl AdditiveStorageHeld {
    pub(super) fn first_error(&self) -> &GlobalSchemaV1Error {
        &self.first
    }
}
impl AdditiveStorageCopied {
    pub(super) fn create(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageHeld> {
        Self::create_at_cut(source, StorageCut::None)
    }
    fn create_at_cut(
        source: rows::AdditiveRowsTargetSource,
        cut: StorageCut,
    ) -> std::result::Result<Self, AdditiveStorageHeld> {
        let mut owner = Self {
            source,
            managed: None,
            directory: None,
            anchor: None,
            fresh: false,
            original: None,
            rows: None,
            records: std::array::from_fn(|_| None),
            pending: None,
            target: None,
            target_node: None,
            copied: None,
            census_files: std::array::from_fn(|_| None),
            codec: AdditiveRecordCodecState::new(),
            copy_issued: false,
            rejected_copy_return: None,
            copy_return_failed: false,
            copy_return_error: None,
            copy_origin_return_error: None,
        };
        match owner.prepare_prefix(cut) {
            Ok(()) => Ok(owner),
            Err(first) => Err(AdditiveStorageHeld { owner, first }),
        }
    }
    fn stop(cut: StorageCut, reached: StorageCut) -> StorageResult<()> {
        #[cfg(test)]
        if cut == reached {
            return Err(storage_fail("additive fixed negative stop"));
        }
        let _ = (cut, reached);
        Ok(())
    }
    fn count(&self) -> usize {
        self.records
            .iter()
            .take_while(|record| record.is_some())
            .count()
    }
    fn directory(&self) -> StorageResult<&File> {
        self.directory
            .as_ref()
            .ok_or_else(|| storage_fail("additive operation FD missing"))
    }
    fn target(&self) -> StorageResult<&File> {
        self.target
            .as_ref()
            .ok_or_else(|| storage_fail("additive target FD missing"))
    }
    pub(super) fn revalidate(mut self) -> std::result::Result<Self, AdditiveStorageHeld> {
        let result = (|| {
            self.verify_prefix()?;
            let actual = self.fingerprint()?;
            if self.copied.as_ref() != Some(&actual) {
                return Err(storage_fail("additive retained Copied bytes changed"));
            }
            self.validate_origin()
        })();
        match result {
            Ok(()) => Ok(self),
            Err(first) => Err(AdditiveStorageHeld { owner: self, first }),
        }
    }
    fn prepare_prefix(&mut self, cut: StorageCut) -> StorageResult<()> {
        // These are the original journal/tail/metadata loans. Same original
        // RowsWork and target TargetWork survive every subsequent transition.
        let (core, _, work) = self.source.storage_parts()?;
        self.rows = Some(core.original_binding(work)?);
        self.original = Some(core.with_copy_origin(work, |loan, work| loan.binding(work))?);
        self.open_directories()?;
        self.load_prefix()?;
        if self.count() == 0 {
            self.emit(0)?;
        }
        Self::stop(cut, StorageCut::AfterIntent)?;
        if self.count() == 1 {
            if self.target.is_some() {
                return Err(storage_fail("additive unrecorded target; no adopt"));
            }
            self.verify_prefix()?;
            let file = storage_create(self.directory()?, STORAGE_TARGET)?;
            // Store the acquired File before fstat, sync or any refusal.
            self.target = Some(file);
            self.target_node = Some(StorageNode::from_file(self.target()?, false)?);
            Self::stop(cut, StorageCut::AfterTargetOpen)?;
            storage_io(self.target()?.sync_all())?;
            storage_io(self.directory()?.sync_all())?;
            self.require_empty_named()?;
            self.emit(1)?;
        }
        Self::stop(cut, StorageCut::AfterCreated)?;
        if self.count() == 2 {
            self.copy_from_original(cut)?;
            Self::stop(cut, StorageCut::AfterCopy)?;
            storage_io(self.target()?.sync_all())?;
            storage_io(self.directory()?.sync_all())?;
            self.copied = Some(self.fingerprint()?);
            let original = self.original.as_ref().unwrap();
            let copied = self.copied.as_ref().unwrap();
            if copied.length != original.length || copied.sha256 != original.sha256 {
                return Err(storage_fail("additive Copied is not exact original bytes"));
            }
            self.emit(2)?;
        }
        if self.count() != 3 {
            return Err(storage_fail("additive prefix is not Copied"));
        }
        self.verify_prefix()?;
        let actual = self.fingerprint()?;
        let original = self.original.as_ref().unwrap();
        if actual.length != original.length || actual.sha256 != original.sha256 {
            return Err(storage_fail("additive cold Copied bytes changed"));
        }
        self.copied = Some(actual);
        self.validate_origin()?;
        Ok(())
    }
    fn validate_origin(&mut self) -> StorageResult<()> {
        let expected = self.original.as_ref().unwrap();
        let (core, _, work) = self.source.storage_parts()?;
        core.with_copy_origin(work, |loan, work| {
            if loan.binding(work)? != *expected {
                return Err(storage_fail(
                    "additive original binding changed after Copied",
                ));
            }
            Ok(())
        })
    }
    fn copy_from_original(&mut self, cut: StorageCut) -> StorageResult<()> {
        if self.count() != 2
            || self.pending.is_some()
            || self.copy_return_failed
            || self.copy_issued
        {
            return Err(storage_fail(
                "additive durable Created copy obligation invalid",
            ));
        }
        self.verify_prefix()?;
        self.require_empty_named()?;
        let expected = self.target_node.unwrap();
        let descriptor = self.target()?.as_raw_fd();
        self.copy_issued = true;
        let (core, _, work) = self.source.storage_parts()?;
        #[cfg(test)]
        if cut == StorageCut::ExhaustBeforeCopy {
            let remaining = 16 * MIB - work.metadata_used();
            let _ = work.metadata(remaining + 1); // negative-only actual fixed meter
        }
        let _ = cut;
        // This is the only permit mint. It follows actual Created decode,
        // named/empty/FD census checks; it cannot be minted again in this owner.
        target::copy_additive_created(
            core,
            work,
            DurableAdditiveCreatedPermit {
                slot: &mut self.target,
                rejected: &mut self.rejected_copy_return,
                expected,
                descriptor,
                taken: false,
                failed: &mut self.copy_return_failed,
                failure: &mut self.copy_return_error,
                post_copy_failure: &mut self.copy_origin_return_error,
            },
        )
    }
    fn open_directories(&mut self) -> StorageResult<()> {
        let Self {
            source,
            managed,
            directory,
            anchor,
            fresh,
            ..
        } = self;
        let (core, _, work) = source.storage_parts()?;
        // Bound the real retained path components before repeated namespace
        // traversal; no caller path or limits are accepted.
        let allowance = core.with_namespace(|ns| {
            (ns.database_parent.path.as_os_str().len() as u64)
                .max(4096)
                .checked_mul(128)
                .and_then(|n| n.checked_add(32768))
                .ok_or_else(|| storage_fail("additive namespace work overflow"))
        })?;
        work.metadata(allowance)?;
        core.with_namespace(|ns| {
            ns.validate_unchanged()?;
            let main = storage_io(ns.database_parent.file.metadata())?;
            if !main.is_dir()
                || main.uid() != unsafe { libc::geteuid() }
                || main.mode() & 0o022 != 0
            {
                return Err(storage_fail("additive original parent is not private"));
            }
            let _ = storage_mkdir(&ns.database_parent.file, STORAGE_MANAGED)?;
            *managed = Some(storage_open_directory(
                &ns.database_parent.file,
                STORAGE_MANAGED,
            )?);
            let mf = managed.as_ref().unwrap();
            let mn = StorageDirectoryIdentity::from_file(mf)?;
            if mn.device != main.dev() {
                return Err(storage_fail("additive filesystem changed"));
            }
            *fresh = storage_mkdir(mf, STORAGE_OPERATION)?;
            *directory = Some(storage_open_directory(mf, STORAGE_OPERATION)?);
            let on = StorageDirectoryIdentity::from_file(directory.as_ref().unwrap())?;
            if on.device != main.dev() {
                return Err(storage_fail("additive operation crosses filesystem"));
            }
            // Capture actual directory identity after the operation's mkdir; links
            // remain a positive live stat check, not a persisted equality key.
            *anchor = Some(StorageAnchor {
                main_device: main.dev(),
                main_inode: main.ino(),
                managed: StorageDirectoryIdentity::from_file(mf)?,
                operation: on,
            });
            ns.validate_unchanged()
        })
    }
    fn validate_ancestors(&mut self) -> StorageResult<()> {
        let Self {
            source,
            managed,
            directory,
            anchor,
            ..
        } = self;
        let (core, _, _) = source.storage_parts()?;
        core.with_namespace(|ns| {
            ns.validate_unchanged()?;
            let main = storage_io(ns.database_parent.file.metadata())?;
            let anchor = anchor.ok_or_else(|| storage_fail("additive anchor missing"))?;
            if main.dev() != anchor.main_device || main.ino() != anchor.main_inode {
                return Err(storage_fail("additive original parent changed"));
            }
            let reopened = storage_open_directory(&ns.database_parent.file, STORAGE_MANAGED)?;
            let managed = managed
                .as_ref()
                .ok_or_else(|| storage_fail("additive managed FD missing"))?;
            if StorageDirectoryIdentity::from_file(&reopened)? != anchor.managed
                || StorageDirectoryIdentity::from_file(managed)? != anchor.managed
            {
                return Err(storage_fail("additive managed parent changed"));
            }
            let reopened = storage_open_directory(managed, STORAGE_OPERATION)?;
            if StorageDirectoryIdentity::from_file(&reopened)? != anchor.operation
                || StorageDirectoryIdentity::from_file(directory.as_ref().unwrap())?
                    != anchor.operation
            {
                return Err(storage_fail("additive operation directory changed"));
            }
            ns.validate_unchanged()
        })
    }
    fn load_prefix(&mut self) -> StorageResult<()> {
        self.validate_ancestors()?;
        let observed = self.census()?;
        let mut count = 0;
        while count < 6 && observed[count].is_some() {
            count += 1;
        }
        if observed[count..6].iter().any(Option::is_some)
            || count > 3
            || (self.fresh && count != 0)
            || (!self.fresh && count == 0)
        {
            return Err(storage_fail(
                "additive gap/partial/unsupported existing prefix",
            ));
        }
        if let Some(node) = observed[6] {
            self.target = Some(storage_open_file(self.directory()?, STORAGE_TARGET, true)?);
            self.target_node = Some(StorageNode::from_file(self.target()?, false)?);
            if self.target_node != Some(node) {
                return Err(storage_fail("additive target changed during census"));
            }
        }
        if (count >= 2) != self.target.is_some() {
            return Err(storage_fail("additive target presence lacks Created"));
        }
        for slot in 0..count {
            let file = storage_open_file(self.directory()?, LEAVES[slot], false)?;
            self.records[slot] = Some(StorageSaved {
                file,
                node: observed[slot].unwrap(),
                bytes: Vec::new(),
                hash: String::new(),
            });
            let saved = self.records[slot].as_mut().unwrap();
            if StorageNode::from_file(&saved.file, false)? != saved.node {
                return Err(storage_fail("additive record replaced after census"));
            }
            let (_, _, work) = self.source.storage_parts()?;
            saved.bytes = storage_read_record(&saved.file, slot, work)?;
        }
        self.decode_saved()?;
        if count == 2 {
            self.require_empty_named()?;
        }
        // Only exactly decoded existing facts may be resynced, never rewritten.
        for saved in self.records.iter().flatten() {
            storage_io(saved.file.sync_all())?;
        }
        storage_io(self.directory()?.sync_all())?;
        self.verify_prefix()
    }
    fn decode_saved(&mut self) -> StorageResult<()> {
        let Self {
            source,
            anchor,
            original,
            rows,
            records,
            target_node,
            codec,
            ..
        } = self;
        let (_, recipe, work) = source.storage_parts()?;
        let identity = AdditiveRecordIdentity {
            anchor: anchor.unwrap().declared(),
            record_nodes: std::array::from_fn(|slot| {
                records[slot].as_ref().map(|r| r.node.declared())
            }),
            target_node: target_node
                .map(StorageNode::declared)
                .unwrap_or(DeclaredRecordNode {
                    device: 0,
                    inode: 0,
                    links: 0,
                    owner: 0,
                    mode: 0,
                }),
            original: original.as_ref().unwrap(),
            rows: rows.as_deref().unwrap(),
        };
        let slots = std::array::from_fn(|slot| records[slot].as_ref().map(|r| r.bytes.as_slice()));
        let prefix = decode_prefix(recipe, &identity, slots, codec)
            .map_err(|_| storage_fail("additive canonical prefix refused"))?;
        let hashes: [Option<[u8; 32]>; 6] =
            std::array::from_fn(|slot| prefix.records[slot].map(|r| r.checksum));
        for (slot, hash) in hashes.into_iter().enumerate() {
            if let Some(hash) = hash {
                work.metadata(64)?;
                records[slot].as_mut().unwrap().hash = hex::encode(hash);
            }
        }
        Ok(())
    }
    fn census(&mut self) -> StorageResult<[Option<StorageNode>; 7]> {
        let Self {
            source,
            directory,
            census_files,
            ..
        } = self;
        let (_, _, work) = source.storage_parts()?;
        storage_census(directory.as_ref().unwrap(), work, census_files)
    }
    fn same_named(
        directory: &File,
        leaf: &str,
        file: &File,
        expected: StorageNode,
    ) -> StorageResult<()> {
        let named = storage_open_file(directory, leaf, false)?;
        if StorageNode::from_file(&named, false)? != expected
            || StorageNode::from_file(file, false)? != expected
        {
            return Err(storage_fail("additive held/named inode changed"));
        }
        Ok(())
    }
    fn require_empty_named(&mut self) -> StorageResult<()> {
        self.validate_ancestors()?;
        Self::same_named(
            self.directory()?,
            STORAGE_TARGET,
            self.target()?,
            self.target_node.unwrap(),
        )?;
        if storage_io(self.target()?.metadata())?.len() != 0 {
            return Err(storage_fail("additive partial Created; no recopy"));
        }
        self.validate_ancestors()
    }
    fn verify_prefix(&mut self) -> StorageResult<()> {
        if self.copy_return_failed {
            return Err(storage_fail("additive File-return failure remains held"));
        }
        self.source.storage_parts()?.2.metadata(65536)?;
        self.validate_ancestors()?;
        let observed = self.census()?;
        for slot in 0..6 {
            match (self.records[slot].as_ref(), observed[slot]) {
                (None, None) => {}
                (Some(saved), Some(node)) if saved.node == node => {}
                _ => {
                    return Err(storage_fail(
                        "additive fixed record presence/identity changed",
                    ))
                }
            }
        }
        if observed[6] != self.target_node {
            return Err(storage_fail("additive target presence changed"));
        }
        let Self {
            source,
            directory,
            records,
            target,
            target_node,
            ..
        } = self;
        let (_, _, work) = source.storage_parts()?;
        for (slot, saved) in records
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.as_ref().map(|r| (i, r)))
        {
            Self::same_named(
                directory.as_ref().unwrap(),
                LEAVES[slot],
                &saved.file,
                saved.node,
            )?;
            let bytes = storage_read_record(&saved.file, slot, work)?;
            if bytes != saved.bytes {
                return Err(storage_fail("additive retained record bytes changed"));
            }
        }
        if let Some(file) = target.as_ref() {
            Self::same_named(
                directory.as_ref().unwrap(),
                STORAGE_TARGET,
                file,
                target_node.unwrap(),
            )?;
        }
        self.decode_saved()?;
        self.validate_ancestors()
    }
    fn fingerprint(&mut self) -> StorageResult<StorageWitness> {
        self.validate_ancestors()?;
        let Self {
            source,
            target,
            directory,
            target_node,
            ..
        } = self;
        let (_, _, work) = source.storage_parts()?;
        let file = target.as_ref().unwrap();
        let node = StorageNode::from_file(file, false)?;
        if Some(node) != *target_node {
            return Err(storage_fail("additive target inode mismatch"));
        }
        Self::same_named(directory.as_ref().unwrap(), STORAGE_TARGET, file, node)?;
        let before = storage_file_stamp(file)?;
        let length = work.additive_hash_read(file)?;
        let mut hash = Sha256::new();
        let mut offset = 0;
        let mut buffer = [0_u8; 65536];
        while offset < length {
            let room = (length - offset).min(buffer.len() as u64) as usize;
            let n = storage_io(file.read_at(&mut buffer[..room], offset))?;
            if n == 0 {
                return Err(storage_fail("additive target shortened while hashing"));
            }
            hash.update(&buffer[..n]);
            offset += n as u64;
        }
        let mut sentinel = [0];
        if storage_io(file.read_at(&mut sentinel, length))? != 0
            || storage_file_stamp(file)? != before
        {
            return Err(storage_fail("additive target changed while hashing"));
        }
        Self::same_named(directory.as_ref().unwrap(), STORAGE_TARGET, file, node)?;
        Ok(StorageWitness {
            node,
            length,
            sha256: hex::encode(hash.finalize()),
        })
    }
    fn emit(&mut self, slot: usize) -> StorageResult<()> {
        if slot >= 3 || slot != self.count() || self.pending.is_some() {
            return Err(storage_fail("additive emit phase mismatch"));
        }
        self.verify_prefix()?;
        let file = storage_create(self.directory()?, LEAVES[slot])?;
        self.pending = Some(StoragePendingRecord {
            file,
            node: None,
            canonical: Vec::new(),
            bytes: Vec::new(),
            readback: Vec::new(),
        });
        let node = StorageNode::from_file(&self.pending.as_ref().unwrap().file, false)?;
        self.pending.as_mut().unwrap().node = Some(node);
        let Self {
            source,
            anchor,
            original,
            rows,
            records,
            copied,
            pending,
            target_node,
            ..
        } = self;
        let (_, recipe, work) = source.storage_parts()?;
        let transition = match slot {
            0 => StorageTransition::Intent {
                binding: StorageIntent {
                    recipe: recipe.id(),
                    original: original.as_ref().unwrap(),
                    rows: rows.as_deref().unwrap(),
                    limits: target::Limits::production(),
                },
            },
            1 => StorageTransition::Created {
                file: target_node.unwrap(),
            },
            _ => StorageTransition::Copied {
                file: copied.as_ref().unwrap(),
            },
        };
        let record = StorageRecord {
            version: 1,
            slot: slot as u8,
            leaf: LEAVES[slot],
            anchor: anchor.unwrap(),
            self_inode: node,
            intent: records[0].as_ref().map(|r| r.hash.as_str()),
            predecessor: slot
                .checked_sub(1)
                .and_then(|i| records[i].as_ref())
                .map(|r| r.hash.as_str()),
            transition,
        };
        let max = if slot == 0 { INTENT_LIMIT } else { EVENT_LIMIT };
        let pending = pending.as_mut().unwrap();
        pending.canonical = work.additive_encode(&record, max)?;
        work.metadata(64)?;
        let hash = hex::encode(record_checksum(&pending.canonical));
        pending.bytes = work.additive_encode(
            &StorageEnvelope {
                sha256: &hash,
                record: &record,
            },
            max,
        )?;
        let total = records
            .iter()
            .flatten()
            .try_fold(pending.bytes.len() as u64, |n, r| {
                n.checked_add(r.bytes.len() as u64)
            })
            .ok_or_else(|| storage_fail("additive held record total overflow"))?;
        if total > HELD_LIMIT {
            return Err(storage_fail("additive held record total exceeded"));
        }
        work.additive_record_write(&pending.bytes)?;
        storage_io(pending.file.write_all_at(&pending.bytes, 0))?;
        storage_io(pending.file.sync_all())?;
        pending.readback = storage_read_record(&pending.file, slot, work)?;
        if pending.readback != pending.bytes {
            return Err(storage_fail("additive record readback differs"));
        }
        // Promote only a real owned File. Before any further fallible validation,
        // the complete record is in the parent's retained slots.
        let pending = self.pending.take().unwrap();
        self.records[slot] = Some(StorageSaved {
            file: pending.file,
            node,
            bytes: pending.bytes,
            hash,
        });
        self.decode_saved()?;
        storage_io(self.directory()?.sync_all())?;
        self.verify_prefix()?;
        Ok(())
    }
}
fn storage_open_directory(parent: &File, name: &str) -> StorageResult<File> {
    let name = super::component_cstring(OsStr::new(name))
        .map_err(|_| storage_fail("additive fixed directory name"))?;
    // SAFETY: live single-component CString, retained dirfd, libc's platform
    // flags; a fresh O_DIRECTORY/no-follow/CLOEXEC FD is transferred once.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_DIRECTORY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(storage_fail("additive directory open refused"));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn storage_mkdir(parent: &File, name: &str) -> StorageResult<bool> {
    let name = super::component_cstring(OsStr::new(name))
        .map_err(|_| storage_fail("additive fixed mkdir name"))?;
    let code = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
    if code == 0 {
        storage_io(parent.sync_all())?;
        return Ok(true);
    }
    if io::Error::last_os_error().kind() == io::ErrorKind::AlreadyExists {
        Ok(false)
    } else {
        Err(storage_fail("additive directory create failed"))
    }
}
fn storage_open_file(parent: &File, name: &str, write: bool) -> StorageResult<File> {
    super::openat_component(
        parent,
        OsStr::new(name),
        if write {
            super::O_RDWR_FLAG
        } else {
            super::O_RDONLY_FLAG
        },
        false,
    )
    .map_err(|_| storage_fail("additive fixed file unavailable"))
}
fn storage_create(parent: &File, name: &str) -> StorageResult<File> {
    let name = super::component_cstring(OsStr::new(name))
        .map_err(|_| storage_fail("additive fixed create name"))?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR
                | libc::O_CREAT
                | libc::O_EXCL
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK,
            0o600 as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(storage_fail("additive exclusive create failed"));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn storage_file_stamp(file: &File) -> StorageResult<(u64, u64, u64, i64, i64, i64, i64)> {
    let m = storage_io(file.metadata())?;
    Ok((
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}
fn storage_read_record(
    file: &File,
    slot: usize,
    work: &mut target::TargetWork,
) -> StorageResult<Vec<u8>> {
    let before = storage_file_stamp(file)?;
    let size = work.additive_record_read(file, slot)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| storage_fail("additive record allocation"))?;
    bytes.resize(size, 0);
    storage_io(file.read_exact_at(&mut bytes, 0))?;
    let mut last = [0];
    if storage_io(file.read_at(&mut last, size as u64))? != 0 || storage_file_stamp(file)? != before
    {
        return Err(storage_fail("additive record extent/identity changed"));
    }
    Ok(bytes)
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
unsafe fn storage_errno() -> *mut libc::c_int {
    unsafe { libc::__error() }
}
#[cfg(any(target_os = "linux", target_os = "android"))]
unsafe fn storage_errno() -> *mut libc::c_int {
    unsafe { libc::__errno_location() }
}
struct StorageDirectoryStream {
    dir: Option<std::ptr::NonNull<libc::DIR>>,
}
impl StorageDirectoryStream {
    fn close(mut self) -> StorageResult<()> {
        let pointer = self.dir.take().unwrap();
        // closedir consumes this enumeration FD; never the held parent FD.
        if unsafe { libc::closedir(pointer.as_ptr()) } != 0 {
            return Err(storage_fail("additive census close failed"));
        }
        Ok(())
    }
}
impl Drop for StorageDirectoryStream {
    fn drop(&mut self) {
        if let Some(dir) = self.dir.take() {
            unsafe {
                libc::closedir(dir.as_ptr());
            }
        }
    }
}
fn storage_census(
    parent: &File,
    work: &mut target::TargetWork,
    held: &mut [Option<File>; 7],
) -> StorageResult<[Option<StorageNode>; 7]> {
    let before = storage_io(parent.metadata())?;
    let stamp = (
        before.dev(),
        before.ino(),
        before.nlink(),
        before.uid(),
        before.mode(),
        before.mtime(),
        before.mtime_nsec(),
        before.ctime(),
        before.ctime_nsec(),
    );
    let enumerator = storage_open_directory(parent, ".")?;
    let flags = unsafe { libc::fcntl(enumerator.as_raw_fd(), libc::F_GETFD) };
    if flags < 0 || flags & libc::FD_CLOEXEC == 0 {
        return Err(storage_fail("additive census FD is inheritable"));
    }
    let fd = enumerator.into_raw_fd();
    // SAFETY: fd is a fresh directory open, not dup of the main owner's offset;
    // success transfers it to libc DIR. On failure we still own/close the FD.
    let pointer = unsafe { libc::fdopendir(fd) };
    let Some(pointer) = std::ptr::NonNull::new(pointer) else {
        drop(unsafe { File::from_raw_fd(fd) });
        return Err(storage_fail("additive fdopendir failed"));
    };
    let stream = StorageDirectoryStream { dir: Some(pointer) };
    let mut nodes = [None; 7];
    let mut entries = 0;
    loop {
        unsafe {
            *storage_errno() = 0;
        }
        let entry = unsafe { libc::readdir(pointer.as_ptr()) };
        if entry.is_null() {
            if unsafe { *storage_errno() } != 0 {
                return Err(storage_fail("additive census readdir error, not EOF"));
            }
            break;
        }
        entries += 1;
        if entries > 9 {
            return Err(storage_fail("additive namespace has extra entries"));
        }
        // Bound raw names by the actual libc dirent array, never a handwritten ABI.
        let raw = unsafe { &(*entry).d_name };
        let length = raw
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| storage_fail("additive dirent missing terminator"))?;
        if length == 0 || length > 255 {
            return Err(storage_fail("additive dirent component extent"));
        }
        let mut buffer = [0_u8; 255];
        for (target, source) in buffer.iter_mut().zip(&raw[..length]) {
            *target = *source as u8;
        }
        let name = &buffer[..length];
        work.additive_census_entry(name)?;
        if name == b"." || name == b".." {
            continue;
        }
        let slot = if name == STORAGE_TARGET.as_bytes() {
            6
        } else {
            LEAVES
                .iter()
                .position(|leaf| leaf.as_bytes() == name)
                .ok_or_else(|| storage_fail("additive unknown namespace entry"))?
        };
        if nodes[slot].is_some() {
            return Err(storage_fail("additive repeated census entry"));
        }
        let fixed = if slot == 6 {
            STORAGE_TARGET
        } else {
            LEAVES[slot]
        };
        let file = storage_open_file(parent, fixed, false)?;
        // Park the actual named FD before node/alias checks, including failures.
        // Replacing an earlier census loan never removes the retained main pin.
        held[slot] = Some(file);
        let node = StorageNode::from_file(held[slot].as_ref().unwrap(), false)?;
        if node.device != before.dev()
            || nodes
                .iter()
                .flatten()
                .any(|other: &StorageNode| other.device == node.device && other.inode == node.inode)
        {
            return Err(storage_fail("additive census alias/filesystem mismatch"));
        }
        nodes[slot] = Some(node);
    }
    stream.close()?;
    let after = storage_io(parent.metadata())?;
    if stamp
        != (
            after.dev(),
            after.ino(),
            after.nlink(),
            after.uid(),
            after.mode(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
        )
    {
        return Err(storage_fail("additive namespace changed during census"));
    }
    Ok(nodes)
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    use std::os::unix::fs::{symlink, DirBuilderExt, OpenOptionsExt};
    fn path(owner: &AdditiveStorageCopied, leaf: &str) -> std::path::PathBuf {
        super::super::sqlite_open_route_from_retained_parent(
            owner.directory().unwrap(),
            OsStr::new(leaf),
        )
        .unwrap()
    }
    fn records(owner: &AdditiveStorageCopied) -> Vec<Vec<u8>> {
        owner
            .records
            .iter()
            .flatten()
            .map(|r| r.bytes.clone())
            .collect()
    }
    fn copied(original: rows::VerifiedUnapprovedOriginalRowsBackup) -> AdditiveStorageCopied {
        match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
            Ok(owner) => owner,
            Err(held) => panic!("actual prefix refused: {}", held.first_error()),
        }
    }
    #[test]
    fn task6_additive_storage_prefix_real_copy_and_cold_validation() {
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let mut owner = copied(original);
                assert_eq!(owner.count(), 3);
                let live_directory = owner.directory().unwrap().metadata().unwrap();
                assert!(live_directory.is_dir() && live_directory.nlink() > 0);
                assert_eq!(
                    StorageDirectoryIdentity::from_file(owner.directory().unwrap()).unwrap(),
                    owner.anchor.unwrap().operation
                );
                owner.verify_prefix().unwrap(); // Own durable entries may change live directory nlink.
                let witness = owner.copied.as_ref().unwrap();
                assert!(witness.length > 0);
                assert_eq!(witness.length, owner.original.as_ref().unwrap().length);
                assert_eq!(witness.sha256, owner.original.as_ref().unwrap().sha256);
                let mut magic = [0; 16];
                owner
                    .target()
                    .unwrap()
                    .read_exact_at(&mut magic, 0)
                    .unwrap();
                assert_eq!(&magic, b"SQLite format 3\0");
                assert_eq!(
                    unsafe { libc::fcntl(owner.target().unwrap().as_raw_fd(), libc::F_GETFD) }
                        & libc::FD_CLOEXEC,
                    libc::FD_CLOEXEC
                );
                let data = (
                    owner.anchor.unwrap(),
                    owner.target_node.unwrap(),
                    records(&owner),
                );
                let fd = owner.target().unwrap().as_raw_fd();
                let used = owner.source.storage_parts().unwrap().2.metadata_used();
                {
                    let (core, _, work) = owner.source.storage_parts().unwrap();
                    assert_eq!(core.observation().original_streams, 6);
                    work.metadata(17).unwrap();
                }
                assert_eq!(owner.target().unwrap().as_raw_fd(), fd);
                assert_eq!(
                    owner.source.storage_parts().unwrap().2.metadata_used(),
                    used + 17
                );
                drop(owner); // End old Work and release the actual source lease.
                data
            },
            |(anchor, target, bytes), original| {
                let mut owner = copied(original); // New genuine cap and one new meter.
                assert_eq!(owner.anchor, Some(anchor));
                assert_eq!(owner.target_node, Some(target));
                assert_eq!(records(&owner), bytes); // Cold prefix read, no rewrite/copy.
                assert_eq!(owner.count(), 3);
                assert!(!owner.copy_issued);
                assert_eq!(
                    owner
                        .source
                        .storage_parts()
                        .unwrap()
                        .0
                        .observation()
                        .original_streams,
                    6
                );
                drop(owner);
            },
        );
    }
    #[test]
    fn task6_additive_storage_prefix_rejects_namespace_and_partial_states() {
        for case in [
            "intent",
            "created",
            "extra",
            "gap",
            "record",
            "replacement",
            "directory",
            "hardlink",
            "symlink",
            "partial",
        ] {
            super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
                |original| {
                    let cut = if case == "intent" {
                        StorageCut::AfterIntent
                    } else {
                        StorageCut::AfterCreated
                    };
                    let mut held = match AdditiveStorageCopied::create_at_cut(
                        original.into_additive_target_source().unwrap(),
                        cut,
                    ) {
                        Err(held) => held,
                        Ok(_) => panic!("fixed cut did not stop"),
                    };
                    let owner = &mut held.owner;
                    let target = path(owner, STORAGE_TARGET);
                    match case {
                        "extra" => {
                            let file = std::fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .mode(0o600)
                                .open(target.parent().unwrap().join("006-extra.json"))
                                .unwrap();
                            file.sync_all().unwrap();
                        }
                        "gap" => std::fs::remove_file(path(owner, LEAVES[0])).unwrap(),
                        "record" => {
                            let file = &owner.records[0].as_ref().unwrap().file;
                            file.write_all_at(b"{", 0).unwrap();
                            file.set_len(1).unwrap();
                        }
                        "replacement" => {
                            std::fs::remove_file(&target).unwrap();
                            std::fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .mode(0o600)
                                .open(&target)
                                .unwrap();
                        }
                        "directory" => {
                            let operation = target.parent().unwrap();
                            std::fs::rename(
                                operation,
                                operation.with_file_name("TEST_CODE_replaced_operation"),
                            )
                            .unwrap();
                            std::fs::DirBuilder::new()
                                .mode(0o700)
                                .create(operation)
                                .unwrap();
                            assert!(owner.validate_ancestors().is_err()); // Same mode/count cannot replace the retained inode.
                        }
                        "hardlink" => {
                            std::fs::hard_link(&target, target.parent().unwrap().join("alias.bin"))
                                .unwrap()
                        }
                        "symlink" => {
                            std::fs::remove_file(&target).unwrap();
                            symlink("000-intent.json", &target).unwrap();
                        }
                        "partial" => owner.target().unwrap().write_all_at(b"x", 0).unwrap(),
                        _ => {}
                    }
                    let expected = (owner.count(), owner.target_node);
                    drop(held);
                    expected
                },
                |(prior_count, prior_target), original| {
                    let result = AdditiveStorageCopied::create(
                        original.into_additive_target_source().unwrap(),
                    );
                    if matches!(case, "intent" | "created") {
                        let owner = match result {
                            Ok(owner) => owner,
                            Err(held) => {
                                panic!("legal prefix resume failed: {}", held.first_error())
                            }
                        };
                        assert_eq!(owner.count(), 3);
                        if prior_count == 2 {
                            assert_eq!(owner.target_node, prior_target);
                        }
                        drop(owner);
                    } else {
                        let held = match result {
                            Err(held) => held,
                            Ok(_) => panic!("bad prefix was repaired into success"),
                        };
                        assert!(held.owner.copied.is_none());
                        assert!(
                            held.owner.records[2].is_none(),
                            "no following durable Copied after failure"
                        );
                        if case == "partial" {
                            assert_eq!(held.owner.target().unwrap().metadata().unwrap().len(), 1);
                        }
                        drop(held);
                    }
                },
            );
        }
    }
    #[test]
    fn task6_additive_storage_prefix_holds_copy_errors_and_one_meter() {
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut held = match AdditiveStorageCopied::create_at_cut(
                original.into_additive_target_source().unwrap(),
                StorageCut::AfterCreated,
            ) {
                Err(held) => held,
                Ok(_) => panic!("fixed Created cut did not stop"),
            };
            let first = format!("{}", held.first_error());
            let owner = &mut held.owner;
            owner.verify_prefix().unwrap();
            owner.require_empty_named().unwrap();
            let node = owner.target_node.unwrap();
            let fd = owner.target().unwrap().as_raw_fd();
            let used = owner.source.storage_parts().unwrap().2.metadata_used();
            let result = owner.copy_from_original(StorageCut::ExhaustBeforeCopy);
            assert!(result.is_err()); // Actual B loan refusal, not a synthetic File.
            assert_eq!(owner.target().unwrap().as_raw_fd(), fd);
            assert_eq!(
                StorageNode::from_file(owner.target().unwrap(), false).unwrap(),
                node
            );
            assert_eq!(owner.target().unwrap().metadata().unwrap().len(), 0);
            assert!(owner.source.storage_parts().unwrap().2.metadata_used() > 16 * MIB);
            assert!(owner.source.storage_parts().unwrap().2.metadata_used() > used);
            assert!(!owner.copy_return_failed);
            assert_eq!(
                format!("{}", held.first_error()),
                first,
                "secondary failure cannot replace first owned error"
            );
            let moved = held;
            assert_eq!(moved.owner.count(), 2);
            assert!(moved.owner.records[2].is_none());
            assert_eq!(moved.owner.target().unwrap().as_raw_fd(), fd);
            drop(moved);
        });
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let held = match AdditiveStorageCopied::create_at_cut(
                    original.into_additive_target_source().unwrap(),
                    StorageCut::AfterCopy,
                ) {
                    Err(held) => held,
                    Ok(_) => panic!("fixed post-copy cut did not stop"),
                };
                let witness = held.owner.target_node.unwrap();
                assert!(held.owner.target().unwrap().metadata().unwrap().len() > 0);
                assert_eq!(held.owner.count(), 2);
                drop(held);
                witness
            },
            |node, original| {
                let held = match AdditiveStorageCopied::create(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Err(held) => held,
                    Ok(_) => panic!("unrecorded copy was adopted"),
                };
                assert_eq!(held.owner.target_node, Some(node));
                assert_eq!(held.owner.count(), 2);
                assert!(held.owner.records[2].is_none());
                assert!(held.owner.target().unwrap().metadata().unwrap().len() > 0);
                drop(held);
            },
        );
    }
}

// A fixed ordinary SQLite writer. No readonly Catalog8 reader or Financial,
// native VFS, allocator/payment or provider qualification is issued here.
use rusqlite::{config::DbConfig, Connection, OpenFlags};
const WAL_LEAVES: [&str; 2] = [
    "stock_analysis.db.target-wal",
    "stock_analysis.db.target-shm",
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransformPhase {
    BeforeWriter,
    WriterHeld,
    Active,
    Committed,
    Checkpointed,
    Closed,
    Transformed,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TransformCut {
    None,
    AfterStarted,
    DuplicateDdl,
    BusyClose,
}
struct TransformSidecar {
    file: File,
    node: Option<StorageNode>,
    removed: bool,
}
// The frame owns the entire prefix. There is no second source, Work, pool,
// cloned File or self-reference. Lexical rusqlite Statements are not invented
// as frame resources: execute_batch/query_row drop them before returning.
struct TransformFrame {
    base: AdditiveStorageCopied,
    writer: Option<Connection>,
    phase: TransformPhase,
    route: Option<std::path::PathBuf>,
    uri: Option<String>,
    sidecars: [Option<TransformSidecar>; 2],
    census: [Option<File>; 9],
    sidecar_error: Option<GlobalSchemaV1Error>,
    transformed: Option<StorageWitness>,
    first: Option<GlobalSchemaV1Error>,
    begin_return: Option<bool>,
    commit_return: Option<bool>,
    rollback_return: Option<rusqlite::Result<()>>,
    rollback_autocommit: Option<bool>,
    checkpoint_return: Option<(i64, i64, i64)>,
    close_attempted: bool,
    extension_return: Option<(i32, i32)>,
    extension_omission_return: Option<i64>,
    #[cfg(test)]
    busy_vm: Option<std::ptr::NonNull<rusqlite::ffi::sqlite3_stmt>>,
    #[cfg(test)]
    busy_prepare_return: Option<i32>,
    #[cfg(test)]
    busy_finalize_return: Option<i32>,
}
pub(super) struct AdditiveStorageTransformed {
    frame: TransformFrame,
}
pub(super) struct AdditiveStorageTransformHeld {
    frame: TransformFrame,
}
impl AdditiveStorageTransformHeld {
    pub(super) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame.first.as_ref().unwrap()
    }
}
impl AdditiveStorageCopied {
    pub(super) fn into_transformed(
        self,
    ) -> std::result::Result<AdditiveStorageTransformed, AdditiveStorageTransformHeld> {
        TransformFrame::new(self).run(TransformCut::None)
    }
}
impl AdditiveStorageTransformed {
    // Separate cold phase dispatch. The old Copied create/load/run still
    // accepts exactly 0..3 and never adopts Started or a generation8 target.
    pub(super) fn create_or_resume_transformed(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageTransformHeld> {
        let base = AdditiveStorageCopied {
            source,
            managed: None,
            directory: None,
            anchor: None,
            fresh: false,
            original: None,
            rows: None,
            records: std::array::from_fn(|_| None),
            pending: None,
            target: None,
            target_node: None,
            copied: None,
            census_files: std::array::from_fn(|_| None),
            codec: AdditiveRecordCodecState::new(),
            copy_issued: false,
            rejected_copy_return: None,
            copy_return_failed: false,
            copy_return_error: None,
            copy_origin_return_error: None,
        };
        let mut frame = TransformFrame::new(base);
        match frame.load_cold() {
            Ok(()) => frame.run(TransformCut::None),
            Err(first) => {
                frame.first = Some(first);
                Err(AdditiveStorageTransformHeld { frame })
            }
        }
    }
}
impl TransformFrame {
    fn new(base: AdditiveStorageCopied) -> Self {
        Self {
            base,
            writer: None,
            phase: TransformPhase::BeforeWriter,
            route: None,
            uri: None,
            sidecars: std::array::from_fn(|_| None),
            census: std::array::from_fn(|_| None),
            sidecar_error: None,
            transformed: None,
            first: None,
            begin_return: None,
            commit_return: None,
            rollback_return: None,
            rollback_autocommit: None,
            checkpoint_return: None,
            close_attempted: false,
            extension_return: None,
            extension_omission_return: None,
            #[cfg(test)]
            busy_vm: None,
            #[cfg(test)]
            busy_prepare_return: None,
            #[cfg(test)]
            busy_finalize_return: None,
        }
    }
    fn run(
        mut self,
        cut: TransformCut,
    ) -> std::result::Result<AdditiveStorageTransformed, AdditiveStorageTransformHeld> {
        if self.first.is_some() {
            return Err(AdditiveStorageTransformHeld { frame: self });
        }
        match self.transform(cut) {
            Ok(()) => Ok(AdditiveStorageTransformed { frame: self }),
            Err(first) => {
                self.first = Some(first); // Retain before any cleanup return.
                self.rollback_once();
                Err(AdditiveStorageTransformHeld { frame: self })
            }
        }
    }
    fn load_cold(&mut self) -> StorageResult<()> {
        let (core, _, work) = self.base.source.storage_parts()?;
        self.base.rows = Some(core.original_binding(work)?);
        self.base.original = Some(core.with_copy_origin(work, |loan, work| loan.binding(work))?);
        self.base.open_directories()?;
        self.base.validate_ancestors()?;
        // No adoption of WAL/SHM in a cold process. Unknown prior commit or
        // live journal requires explicit recovery outside this slice.
        let observed = self.base.census()?;
        let mut count = 0;
        while count < 6 && observed[count].is_some() {
            count += 1;
        }
        if count > 5
            || observed[count..6].iter().any(Option::is_some)
            || (self.base.fresh && count != 0)
            || (!self.base.fresh && count == 0)
        {
            return Err(storage_fail(
                "additive cold transform prefix gap/unsupported phase",
            ));
        }
        if let Some(node) = observed[6] {
            self.base.target = Some(storage_open_file(
                self.base.directory()?,
                STORAGE_TARGET,
                true,
            )?);
            self.base.target_node = Some(StorageNode::from_file(self.base.target()?, false)?);
            if self.base.target_node != Some(node) {
                return Err(storage_fail("additive cold target changed"));
            }
        }
        if (count >= 2) != self.base.target.is_some() {
            return Err(storage_fail("additive cold target lacks Created"));
        }
        for slot in 0..count {
            let file = storage_open_file(self.base.directory()?, LEAVES[slot], false)?;
            self.base.records[slot] = Some(StorageSaved {
                file,
                node: observed[slot].unwrap(),
                bytes: Vec::new(),
                hash: String::new(),
            });
            let saved = self.base.records[slot].as_mut().unwrap();
            if StorageNode::from_file(&saved.file, false)? != saved.node {
                return Err(storage_fail("additive cold record replaced"));
            }
            let (_, _, work) = self.base.source.storage_parts()?;
            saved.bytes = storage_read_record(&saved.file, slot, work)?;
        }
        self.base.decode_saved()?;
        if count < 3 {
            if count == 0 {
                self.base.emit(0)?;
            }
            if self.base.count() == 1 {
                if self.base.target.is_some() {
                    return Err(storage_fail("additive cold unrecorded target"));
                }
                self.base.verify_prefix()?;
                self.base.target = Some(storage_create(self.base.directory()?, STORAGE_TARGET)?);
                self.base.target_node = Some(StorageNode::from_file(self.base.target()?, false)?);
                storage_io(self.base.target()?.sync_all())?;
                storage_io(self.base.directory()?.sync_all())?;
                self.base.require_empty_named()?;
                self.base.emit(1)?;
            }
            self.base.require_empty_named()?;
            self.base.copy_from_original(StorageCut::None)?;
            storage_io(self.base.target()?.sync_all())?;
            storage_io(self.base.directory()?.sync_all())?;
            self.base.copied = Some(self.base.fingerprint()?);
            self.require_original_witness()?;
            self.base.emit(2)?;
        } else if count < 5 {
            self.base.copied = Some(self.base.fingerprint()?);
            self.require_original_witness()?;
        } else {
            // Own the actual fingerprint, then compare with the already paid
            // borrowed canonical witness. Never derive COMMIT from this data.
            self.transformed = Some(self.base.fingerprint()?);
            self.require_transformed_witness()?;
            self.require_header(8)?;
            self.phase = TransformPhase::Transformed;
        }
        for record in self.base.records.iter().flatten() {
            storage_io(record.file.sync_all())?;
        }
        storage_io(self.base.directory()?.sync_all())?;
        self.base.verify_prefix()?;
        self.base.validate_origin()
    }
    fn require_original_witness(&self) -> StorageResult<()> {
        let original = self.base.original.as_ref().unwrap();
        let copied = self.base.copied.as_ref().unwrap();
        if copied.length != original.length || copied.sha256 != original.sha256 {
            return Err(storage_fail(
                "additive Started/partial target is not original Copied",
            ));
        }
        Ok(())
    }
    fn require_transformed_witness(&mut self) -> StorageResult<()> {
        let base = &mut self.base;
        let (_, recipe, _) = base.source.storage_parts()?;
        let identity = AdditiveRecordIdentity {
            anchor: base.anchor.unwrap().declared(),
            record_nodes: std::array::from_fn(|i| {
                base.records[i].as_ref().map(|r| r.node.declared())
            }),
            target_node: base.target_node.unwrap().declared(),
            original: base.original.as_ref().unwrap(),
            rows: base.rows.as_deref().unwrap(),
        };
        let prefix = decode_prefix(
            recipe,
            &identity,
            std::array::from_fn(|i| base.records[i].as_ref().map(|r| r.bytes.as_slice())),
            &mut base.codec,
        )
        .map_err(|_| storage_fail("additive transformed canonical witness refused"))?;
        let expected = prefix.records[4]
            .and_then(|r| r.witness)
            .ok_or_else(|| storage_fail("additive Transformed witness missing"))?;
        let actual = self.transformed.as_ref().unwrap();
        if expected.node != actual.node.declared()
            || expected.length != actual.length
            || expected.sha256 != actual.sha256
        {
            return Err(storage_fail("additive cold Transformed bytes changed"));
        }
        Ok(())
    }
    fn require_header(&mut self, generation: u32) -> StorageResult<()> {
        self.base.source.storage_parts()?.2.additive_header_read()?;
        AdditiveStorageCopied::same_named(
            self.base.directory()?,
            STORAGE_TARGET,
            self.base.target()?,
            self.base.target_node.unwrap(),
        )?;
        let before = storage_file_stamp(self.base.target()?)?;
        let mut bytes = [0; 100];
        storage_io(self.base.target()?.read_exact_at(&mut bytes, 0))?;
        if &bytes[..16] != b"SQLite format 3\0"
            || bytes[18..20] != [2, 2]
            || u32::from_be_bytes(bytes[60..64].try_into().unwrap()) != generation
            || storage_file_stamp(self.base.target()?)? != before
        {
            return Err(storage_fail("additive target WAL/header differs"));
        }
        Ok(())
    }
    fn transform(&mut self, cut: TransformCut) -> StorageResult<()> {
        if self.phase == TransformPhase::Transformed {
            self.base.verify_prefix()?;
            self.base.validate_origin()?;
            self.transformed = Some(self.base.fingerprint()?);
            self.require_transformed_witness()?;
            return self.require_header(8);
        }
        if self.phase != TransformPhase::BeforeWriter || !matches!(self.base.count(), 3 | 4) {
            return Err(storage_fail("additive writer phase mismatch"));
        }
        self.base.verify_prefix()?;
        self.base.validate_origin()?;
        let actual = self.base.fingerprint()?;
        self.base.copied = Some(actual);
        self.require_original_witness()?;
        self.require_header(6)?;
        if self.base.count() == 3 {
            self.emit_transform_record(3)?;
        }
        #[cfg(test)]
        if cut == TransformCut::AfterStarted {
            return Err(storage_fail("additive fixed Started stop"));
        }
        self.open_writer()?;
        self.configure_writer()?;
        // Whole query_row returns only after lexical Rows/reset/Statement Drop.
        let first_read = self.writer.as_ref().unwrap().query_row(
            "SELECT COUNT(*) FROM main.sqlite_schema",
            [],
            |row| row.get::<_, i64>(0),
        );
        // Even an actual read failure must not drop sidecars that were created.
        let pins = self.pin_sidecars();
        match first_read {
            Err(error) => {
                if let Err(error) = pins {
                    self.sidecar_error = Some(error);
                }
                return Err(transform_sql_error("materialize additive WAL", error));
            }
            Ok(_) => pins?,
        }
        self.verify_live()?;
        self.require_writer_pragmas()?;
        self.batch(FixedBatch::Begin)?;
        self.begin_return = Some(true);
        if self.writer.as_ref().unwrap().is_autocommit() {
            return Err(storage_fail(
                "additive BEGIN returned without active transaction",
            ));
        }
        self.phase = TransformPhase::Active;
        for slot in 0..4 {
            self.batch(FixedBatch::Schema7(slot))?;
        }
        #[cfg(test)]
        if cut == TransformCut::DuplicateDdl {
            self.batch(FixedBatch::Schema7(0))?;
        }
        for slot in 0..4 {
            self.batch(FixedBatch::Schema8(slot))?;
        }
        self.batch(FixedBatch::Header8)?;
        self.base
            .source
            .validate_additive_write_projection(self.writer.as_ref().unwrap())?;
        self.verify_live()?;
        self.base.validate_origin()?;
        self.batch(FixedBatch::Commit)?;
        self.commit_return = Some(true);
        if !self.writer.as_ref().unwrap().is_autocommit() {
            return Err(storage_fail("additive COMMIT returned still active"));
        }
        self.phase = TransformPhase::Committed;
        self.checkpoint()?;
        #[cfg(test)]
        if cut == TransformCut::BusyClose {
            self.prepare_busy_vm()?;
        }
        let _ = cut;
        self.close_writer()?;
        self.remove_sidecars()?;
        self.finish_transformed()
    }
    fn open_writer(&mut self) -> StorageResult<()> {
        self.base.source.storage_parts()?.2.metadata(32768)?;
        self.route = Some(
            super::super::sqlite_open_route_from_retained_parent(
                self.base.directory()?,
                OsStr::new(STORAGE_TARGET),
            )
            .map_err(|_| storage_fail("additive writer retained route unavailable"))?,
        );
        let route = self.route.as_ref().unwrap().as_os_str().as_bytes();
        let n = route
            .len()
            .checked_mul(3)
            .and_then(|n| n.checked_add(16))
            .ok_or_else(|| storage_fail("additive writer URI overflow"))?;
        self.base.source.storage_parts()?.2.metadata(n as u64)?;
        let mut uri = String::new();
        uri.try_reserve_exact(n)
            .map_err(|_| storage_fail("additive writer URI allocation"))?;
        uri.push_str("file:");
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        for byte in route {
            uri.push('%');
            uri.push(HEX[(byte >> 4) as usize] as char);
            uri.push(HEX[(byte & 15) as usize] as char);
        }
        uri.push_str("?mode=rw");
        self.uri = Some(uri);
        let returned = Connection::open_with_flags(
            self.uri.as_ref().unwrap(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        );
        // Park the actual Connection before all fallible return checks.
        self.writer = Some(returned.map_err(|e| transform_sql_error("open additive writer", e))?);
        self.phase = TransformPhase::WriterHeld;
        self.base.validate_ancestors()?;
        AdditiveStorageCopied::same_named(
            self.base.directory()?,
            STORAGE_TARGET,
            self.base.target()?,
            self.base.target_node.unwrap(),
        )
    }
    fn configure_writer(&mut self) -> StorageResult<()> {
        self.base.source.storage_parts()?.2.metadata(1024)?;
        let writer = self.writer.as_ref().unwrap();
        for (config, required) in [
            (DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true),
            (DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true),
            (DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true),
            (DbConfig::SQLITE_DBCONFIG_WRITABLE_SCHEMA, false),
        ] {
            if writer
                .set_db_config(config, required)
                .map_err(|e| transform_sql_error("configure additive writer", e))?
                != required
                || writer
                    .db_config(config)
                    .map_err(|e| transform_sql_error("observe additive writer config", e))?
                    != required
            {
                return Err(storage_fail("additive writer config return differs"));
            }
        }
        let mut actual = -1;
        // SAFETY: actual Connection is already owned in this frame; fixed
        // linked db_config signature/cut, short handle borrow, no pointer cache.
        let code = unsafe {
            rusqlite::ffi::sqlite3_db_config(
                writer.handle(),
                rusqlite::ffi::SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION,
                0_i32,
                &mut actual,
            )
        };
        self.extension_return = Some((code, actual));
        if code == rusqlite::ffi::SQLITE_OK && actual == 0 {
            return Ok(());
        }
        if (code == rusqlite::ffi::SQLITE_ERROR || code == rusqlite::ffi::SQLITE_MISUSE)
            && actual == -1
        {
            // Observe the fixed property fresh from this same linked SQLite implementation.
            // The writer is already held; this cut prepares no SQL or sidecar work.
            // Cached features and separate/global saved witnesses are not authority.
            let omitted = i64::from(unsafe {
                rusqlite::ffi::sqlite3_compileoption_used(b"OMIT_LOAD_EXTENSION\0".as_ptr().cast())
            });
            self.extension_omission_return = Some(omitted);
            if omitted == 1 {
                return Ok(());
            }
        }
        Err(storage_fail("additive extension disable not observed"))
    }
    fn pin_sidecars(&mut self) -> StorageResult<()> {
        for (slot, leaf) in WAL_LEAVES.iter().enumerate() {
            if self.sidecars[slot].is_some() {
                return Err(storage_fail("additive sidecar pin already issued"));
            }
            let file = storage_open_file(self.base.directory()?, leaf, true)?;
            self.sidecars[slot] = Some(TransformSidecar {
                file,
                node: None,
                removed: false,
            });
            let held = self.sidecars[slot].as_mut().unwrap();
            held.node = Some(StorageNode::from_file(&held.file, false)?);
        }
        Ok(())
    }
    fn require_writer_pragmas(&mut self) -> StorageResult<()> {
        self.base.source.storage_parts()?.2.metadata(4096)?;
        let writer = self.writer.as_ref().unwrap();
        let mode: String = writer
            .query_row("PRAGMA main.journal_mode", [], |r| r.get(0))
            .map_err(|e| transform_sql_error("read additive WAL mode", e))?;
        let foreign: i64 = writer
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .map_err(|e| transform_sql_error("read additive foreign keys", e))?;
        let page: i64 = writer
            .query_row("PRAGMA main.page_size", [], |r| r.get(0))
            .map_err(|e| transform_sql_error("read additive page size", e))?;
        let count: i64 = writer
            .query_row("PRAGMA main.page_count", [], |r| r.get(0))
            .map_err(|e| transform_sql_error("read additive page count", e))?;
        if mode != "wal"
            || foreign != 1
            || page < 512
            || page > 65536
            || !(page as u64).is_power_of_two()
            || count < 0
            || (count as u64)
                .checked_mul(page as u64)
                .is_none_or(|n| n > EXTENT_LIMIT)
        {
            return Err(storage_fail(
                "additive writer WAL/page/foreign-key contract differs",
            ));
        }
        let maximum = EXTENT_LIMIT / page as u64;
        // Fixed value derives only from the production extent and actual page,
        // not an arbitrary SQL/amount factory. Bound formatting before allocation.
        self.base.source.storage_parts()?.2.metadata(128)?;
        let sql = format!("PRAGMA main.max_page_count={maximum}");
        let actual: i64 = writer
            .query_row(&sql, [], |r| r.get(0))
            .map_err(|e| transform_sql_error("bound additive max page count", e))?;
        if actual < 0 || actual as u64 != maximum || count as u64 > maximum {
            return Err(storage_fail("additive max page count return differs"));
        }
        Ok(())
    }
    fn batch(&mut self, operation: FixedBatch) -> StorageResult<()> {
        if self.first.is_some() {
            return Err(storage_fail("additive first error blocks new writer work"));
        }
        let sql = operation.sql();
        self.base
            .source
            .storage_parts()?
            .2
            .metadata(sql.len() as u64 + 4096)?;
        let result = self.writer.as_ref().unwrap().execute_batch(sql);
        if result.is_err() {
            match operation {
                FixedBatch::Begin => self.begin_return = Some(false),
                FixedBatch::Commit => self.commit_return = Some(false),
                _ => {}
            }
        }
        result.map_err(|error| transform_sql_error(operation.name(), error))
    }
    fn rollback_once(&mut self) {
        // Cleanup after first failure issues no new ordinary debit/error/read.
        // Real is_autocommit decides whether the one fixed rollback is owed.
        let Some(writer) = self.writer.as_ref() else {
            return;
        };
        if self.rollback_return.is_some() || writer.is_autocommit() {
            return;
        }
        self.rollback_return = Some(writer.execute_batch("ROLLBACK"));
        self.rollback_autocommit = Some(writer.is_autocommit());
    }
    fn checkpoint(&mut self) -> StorageResult<()> {
        if self.phase != TransformPhase::Committed || self.commit_return != Some(true) {
            return Err(storage_fail(
                "additive checkpoint lacks actual COMMIT return",
            ));
        }
        self.base.source.storage_parts()?.2.metadata(4096)?;
        let result = self.writer.as_ref().unwrap().query_row(
            "PRAGMA main.wal_checkpoint(TRUNCATE)",
            [],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        );
        let tuple = result.map_err(|e| transform_sql_error("checkpoint additive WAL", e))?;
        self.checkpoint_return = Some(tuple);
        if tuple.0 != 0 || tuple.1 < 0 || tuple.2 < 0 || tuple.1 != tuple.2 {
            return Err(storage_fail(
                "additive checkpoint did not truncate completely",
            ));
        }
        self.verify_live()?;
        let file = &self.sidecars[0]
            .as_ref()
            .ok_or_else(|| storage_fail("additive WAL pin missing"))?
            .file;
        if self
            .base
            .source
            .storage_parts()?
            .2
            .additive_sidecar_extent(file)?
            != 0
        {
            return Err(storage_fail("additive WAL remains nonempty"));
        }
        self.phase = TransformPhase::Checkpointed;
        Ok(())
    }
    fn close_writer(&mut self) -> StorageResult<()> {
        if self.close_attempted || self.phase != TransformPhase::Checkpointed {
            return Err(storage_fail("additive consuming close phase"));
        }
        self.close_attempted = true;
        let writer = self
            .writer
            .take()
            .ok_or_else(|| storage_fail("additive writer owner missing"))?;
        match writer.close() {
            Ok(()) => {
                self.phase = TransformPhase::Closed;
                Ok(())
            }
            Err((writer, error)) => {
                self.writer = Some(writer); // Restore the exact returned owner first.
                Err(transform_sql_error("close additive writer", error))
            }
        }
    }
    fn remove_sidecars(&mut self) -> StorageResult<()> {
        if self.writer.is_some() || self.phase != TransformPhase::Closed {
            return Err(storage_fail(
                "additive sidecar removal before consuming close",
            ));
        }
        for (slot, leaf) in WAL_LEAVES.iter().enumerate() {
            // Target-local WAL then SHM.
            let held = self.sidecars[slot]
                .as_mut()
                .ok_or_else(|| storage_fail("additive tracked sidecar missing"))?;
            if held.removed {
                return Err(storage_fail("additive sidecar already removed"));
            }
            let expected = held
                .node
                .ok_or_else(|| storage_fail("additive sidecar node unknown"))?;
            let named = super::openat_component(
                self.base.directory()?,
                OsStr::new(leaf),
                super::O_RDONLY_FLAG,
                false,
            );
            match named {
                Ok(file) => {
                    if StorageNode::from_file(&file, false)? != expected
                        || StorageNode::from_file(&held.file, false)? != expected
                    {
                        return Err(storage_fail("additive sidecar replaced; no unlink"));
                    }
                    storage_io(super::unlinkat_component(
                        self.base.directory()?,
                        OsStr::new(leaf),
                    ))?;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    let m = storage_io(held.file.metadata())?;
                    if (m.dev(), m.ino(), m.uid(), m.mode() & 0o7777, m.nlink())
                        != (
                            expected.device,
                            expected.inode,
                            expected.owner,
                            expected.mode,
                            0,
                        )
                        || !m.is_file()
                    {
                        return Err(storage_fail(
                            "additive absent sidecar is not retained unlinked inode",
                        ));
                    }
                }
                Err(_) => return Err(storage_fail("additive sidecar final named lookup failed")),
            }
            let after = storage_io(held.file.metadata())?;
            if after.nlink() != 0 {
                return Err(storage_fail("additive sidecar still linked after remove"));
            }
            held.removed = true;
            storage_io(self.base.directory()?.sync_all())?;
        }
        // First-error cleanup performs only the fixed acquired-resource cuts;
        // no fresh record read/decode, debit or success qualification follows.
        if self.first.is_some() {
            Ok(())
        } else {
            self.base.verify_prefix()
        } // Normal closed census rejects all sidecars/extra names.
    }
    fn finish_transformed(&mut self) -> StorageResult<()> {
        if self.first.is_some()
            || self.phase != TransformPhase::Closed
            || self.writer.is_some()
            || self
                .sidecars
                .iter()
                .any(|s| !s.as_ref().is_some_and(|s| s.removed))
        {
            return Err(storage_fail("additive Transformed lacks resource cleanup"));
        }
        self.base.verify_prefix()?;
        self.base.validate_origin()?;
        storage_io(self.base.target()?.sync_all())?;
        storage_io(self.base.directory()?.sync_all())?;
        self.require_header(8)?;
        self.transformed = Some(self.base.fingerprint()?);
        self.emit_transform_record(4)?;
        self.phase = TransformPhase::Transformed;
        Ok(())
    }
    fn verify_live(&mut self) -> StorageResult<()> {
        self.base.source.storage_parts()?.2.metadata(65536)?;
        self.base.validate_ancestors()?;
        let (_, _, work) = self.base.source.storage_parts()?;
        let observed = storage_wal_census(
            self.base.directory.as_ref().unwrap(),
            work,
            &mut self.census,
        )?;
        for slot in 0..6 {
            if self.base.records[slot].as_ref().map(|r| r.node) != observed[slot] {
                return Err(storage_fail("additive live record changed"));
            }
        }
        if observed[6] != self.base.target_node {
            return Err(storage_fail("additive live target changed"));
        }
        for slot in 0..2 {
            let owned = self.sidecars[slot]
                .as_ref()
                .ok_or_else(|| storage_fail("additive live sidecar not owned"))?;
            if owned.removed || observed[7 + slot] != owned.node {
                return Err(storage_fail("additive live sidecar ownership differs"));
            }
            AdditiveStorageCopied::same_named(
                self.base.directory()?,
                WAL_LEAVES[slot],
                &owned.file,
                owned.node.unwrap(),
            )?;
        }
        for slot in 0..self.base.count() {
            let saved = self.base.records[slot].as_ref().unwrap();
            let (_, _, work) = self.base.source.storage_parts()?;
            if storage_read_record(&saved.file, slot, work)? != saved.bytes {
                return Err(storage_fail("additive live record bytes changed"));
            }
        }
        self.base.decode_saved()?;
        self.base.validate_ancestors()
    }
    fn emit_transform_record(&mut self, slot: usize) -> StorageResult<()> {
        if self.first.is_some()
            || slot != self.base.count()
            || !matches!(slot, 3 | 4)
            || self.base.pending.is_some()
            || (slot == 3 && self.phase != TransformPhase::BeforeWriter)
            || (slot == 4 && self.phase != TransformPhase::Closed)
        {
            return Err(storage_fail("additive transform record phase mismatch"));
        }
        self.base.verify_prefix()?;
        self.base.pending = Some(StoragePendingRecord {
            file: storage_create(self.base.directory()?, LEAVES[slot])?,
            node: None,
            canonical: Vec::new(),
            bytes: Vec::new(),
            readback: Vec::new(),
        });
        let node = StorageNode::from_file(&self.base.pending.as_ref().unwrap().file, false)?;
        self.base.pending.as_mut().unwrap().node = Some(node);
        let base = &mut self.base;
        let (_, _, work) = base.source.storage_parts()?;
        let transition = if slot == 3 {
            StorageTransition::TransformStarted {
                file: base.copied.as_ref().unwrap(),
            }
        } else {
            StorageTransition::Transformed {
                file: self.transformed.as_ref().unwrap(),
            }
        };
        let record = StorageRecord {
            version: 1,
            slot: slot as u8,
            leaf: LEAVES[slot],
            anchor: base.anchor.unwrap(),
            self_inode: node,
            intent: base.records[0].as_ref().map(|r| r.hash.as_str()),
            predecessor: base.records[slot - 1].as_ref().map(|r| r.hash.as_str()),
            transition,
        };
        let pending = base.pending.as_mut().unwrap();
        pending.canonical = work.additive_encode(&record, EVENT_LIMIT)?;
        work.metadata(64)?;
        let hash = hex::encode(record_checksum(&pending.canonical));
        pending.bytes = work.additive_encode(
            &StorageEnvelope {
                sha256: &hash,
                record: &record,
            },
            EVENT_LIMIT,
        )?;
        let total = base
            .records
            .iter()
            .flatten()
            .try_fold(pending.bytes.len() as u64, |n, r| {
                n.checked_add(r.bytes.len() as u64)
            })
            .ok_or_else(|| storage_fail("additive transform record held overflow"))?;
        if total > HELD_LIMIT {
            return Err(storage_fail("additive transform record held exceeded"));
        }
        work.additive_record_write(&pending.bytes)?;
        storage_io(pending.file.write_all_at(&pending.bytes, 0))?;
        storage_io(pending.file.sync_all())?;
        pending.readback = storage_read_record(&pending.file, slot, work)?;
        if pending.readback != pending.bytes {
            return Err(storage_fail("additive transform readback differs"));
        }
        let pending = base.pending.take().unwrap();
        base.records[slot] = Some(StorageSaved {
            file: pending.file,
            node,
            bytes: pending.bytes,
            hash,
        });
        base.decode_saved()?;
        storage_io(base.directory()?.sync_all())?;
        base.verify_prefix()
    }
    #[cfg(test)]
    fn prepare_busy_vm(&mut self) -> StorageResult<()> {
        if self.busy_prepare_return.is_some() || self.writer.is_none() {
            return Err(storage_fail("additive Busy VM already issued/no writer"));
        }
        let mut raw = std::ptr::null_mut();
        // SAFETY: the real Connection is already Held; fixed nul-terminated
        // SELECT1 only, actual returned nonnull pointer is parked even on Err.
        let code = unsafe {
            rusqlite::ffi::sqlite3_prepare_v2(
                self.writer.as_ref().unwrap().handle(),
                b"SELECT 1\0".as_ptr().cast(),
                -1,
                &mut raw,
                std::ptr::null_mut(),
            )
        };
        self.busy_vm = std::ptr::NonNull::new(raw);
        self.busy_prepare_return = Some(code);
        if code != rusqlite::ffi::SQLITE_OK || self.busy_vm.is_none() {
            return Err(storage_fail("additive actual Busy prepare failed"));
        }
        Ok(())
    }
    #[cfg(test)]
    fn finalize_busy_once(&mut self) -> StorageResult<()> {
        let vm = self
            .busy_vm
            .take()
            .ok_or_else(|| storage_fail("additive Busy VM missing/already consumed"))?;
        let code = unsafe { rusqlite::ffi::sqlite3_finalize(vm.as_ptr()) };
        self.busy_finalize_return = Some(code);
        if code != rusqlite::ffi::SQLITE_OK {
            return Err(storage_fail("additive actual Busy finalize failed"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
enum FixedBatch {
    Begin,
    Schema7(usize),
    Schema8(usize),
    Header8,
    Commit,
}
impl FixedBatch {
    fn sql(self) -> &'static str {
        match self {
            Self::Begin => "BEGIN IMMEDIATE",
            Self::Schema7(i) => {
                super::super::candidate_scope_observation_schema_v1::STATEMENTS[i].3
            }
            Self::Schema8(i) => super::super::investment_decision_schema_v1::STATEMENTS[i].3,
            Self::Header8 => "PRAGMA user_version=8",
            Self::Commit => "COMMIT",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Begin => "begin additive transform",
            Self::Schema7(_) => "create additive Catalog7",
            Self::Schema8(_) => "create additive Catalog8",
            Self::Header8 => "set additive generation8",
            Self::Commit => "commit additive transform",
        }
    }
}
fn transform_sql_error(operation: &'static str, source: rusqlite::Error) -> GlobalSchemaV1Error {
    GlobalSchemaV1Error::SelectionSqlite { operation, source }
}

fn storage_wal_census(
    parent: &File,
    work: &mut target::TargetWork,
    held: &mut [Option<File>; 9],
) -> StorageResult<[Option<StorageNode>; 9]> {
    let before = storage_io(parent.metadata())?;
    let stamp = (
        before.dev(),
        before.ino(),
        before.nlink(),
        before.uid(),
        before.mode(),
        before.mtime(),
        before.mtime_nsec(),
        before.ctime(),
        before.ctime_nsec(),
    );
    let enumerator = storage_open_directory(parent, ".")?;
    let flags = unsafe { libc::fcntl(enumerator.as_raw_fd(), libc::F_GETFD) };
    if flags < 0 || flags & libc::FD_CLOEXEC == 0 {
        return Err(storage_fail("additive census FD is inheritable"));
    }
    let fd = enumerator.into_raw_fd();
    // SAFETY: fd is a fresh directory open, not dup of the main owner's offset;
    // success transfers it to libc DIR. On failure we still own/close the FD.
    let pointer = unsafe { libc::fdopendir(fd) };
    let Some(pointer) = std::ptr::NonNull::new(pointer) else {
        drop(unsafe { File::from_raw_fd(fd) });
        return Err(storage_fail("additive fdopendir failed"));
    };
    let stream = StorageDirectoryStream { dir: Some(pointer) };
    let mut nodes = [None; 9];
    let mut entries = 0;
    loop {
        unsafe {
            *storage_errno() = 0;
        }
        let entry = unsafe { libc::readdir(pointer.as_ptr()) };
        if entry.is_null() {
            if unsafe { *storage_errno() } != 0 {
                return Err(storage_fail("additive census readdir error, not EOF"));
            }
            break;
        }
        entries += 1;
        if entries > 11 {
            return Err(storage_fail("additive namespace has extra entries"));
        }
        // Bound raw names by the actual libc dirent array, never a handwritten ABI.
        let raw = unsafe { &(*entry).d_name };
        let length = raw
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| storage_fail("additive dirent missing terminator"))?;
        if length == 0 || length > 255 {
            return Err(storage_fail("additive dirent component extent"));
        }
        let mut buffer = [0_u8; 255];
        for (target, source) in buffer.iter_mut().zip(&raw[..length]) {
            *target = *source as u8;
        }
        let name = &buffer[..length];
        work.additive_census_entry(name)?;
        if name == b"." || name == b".." {
            continue;
        }
        let slot = if name == STORAGE_TARGET.as_bytes() {
            6
        } else if name == WAL_LEAVES[0].as_bytes() {
            7
        } else if name == WAL_LEAVES[1].as_bytes() {
            8
        } else {
            LEAVES
                .iter()
                .position(|leaf| leaf.as_bytes() == name)
                .ok_or_else(|| storage_fail("additive unknown namespace entry"))?
        };
        if nodes[slot].is_some() {
            return Err(storage_fail("additive repeated census entry"));
        }
        let fixed = if slot == 6 {
            STORAGE_TARGET
        } else if slot >= 7 {
            WAL_LEAVES[slot - 7]
        } else {
            LEAVES[slot]
        };
        let file = storage_open_file(parent, fixed, false)?;
        // Park the actual named FD before node/alias checks, including failures.
        // Replacing an earlier census loan never removes the retained main pin.
        held[slot] = Some(file);
        let node = StorageNode::from_file(held[slot].as_ref().unwrap(), false)?;
        if node.device != before.dev()
            || nodes
                .iter()
                .flatten()
                .any(|other: &StorageNode| other.device == node.device && other.inode == node.inode)
        {
            return Err(storage_fail("additive census alias/filesystem mismatch"));
        }
        nodes[slot] = Some(node);
    }
    stream.close()?;
    let after = storage_io(parent.metadata())?;
    if stamp
        != (
            after.dev(),
            after.ino(),
            after.nlink(),
            after.uid(),
            after.mode(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
        )
    {
        return Err(storage_fail("additive namespace changed during census"));
    }
    Ok(nodes)
}

#[cfg(test)]
impl Drop for TransformFrame {
    fn drop(&mut self) {
        // This is fixture-only leak prevention, never a successful close or
        // record gate. Production has no raw VM bridge or cleanup Drop.
        if let Some(vm) = self.busy_vm.take() {
            self.busy_finalize_return =
                Some(unsafe { rusqlite::ffi::sqlite3_finalize(vm.as_ptr()) });
        }
    }
}
#[cfg(test)]
mod wal_tests {
    use super::*;
    fn actual_copied(
        original: rows::VerifiedUnapprovedOriginalRowsBackup,
    ) -> AdditiveStorageCopied {
        match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
            Ok(owner) => owner,
            Err(held) => panic!("genuine Copied refused: {}", held.first_error()),
        }
    }
    fn transformed(owner: AdditiveStorageCopied) -> AdditiveStorageTransformed {
        match owner.into_transformed() {
            Ok(owner) => owner,
            Err(held) => panic!("actual fixed transform refused: {}", held.first_error()),
        }
    }
    fn owned_records(frame: &TransformFrame) -> Vec<Vec<u8>> {
        frame
            .base
            .records
            .iter()
            .flatten()
            .map(|r| r.bytes.clone())
            .collect()
    }
    #[test]
    fn task6_additive_wal_transform_real_commit_and_cold_prefix() {
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let copied = actual_copied(original);
                let original_target = copied.target_node.unwrap();
                let allocation = copied.target().unwrap().as_raw_fd();
                let mut owner = transformed(copied);
                let frame = &mut owner.frame;
                assert_eq!(frame.phase, TransformPhase::Transformed);
                assert_eq!(frame.base.count(), 5);
                assert_eq!(frame.base.target_node, Some(original_target));
                assert_eq!(frame.base.target().unwrap().as_raw_fd(), allocation);
                assert_eq!(frame.begin_return, Some(true));
                assert_eq!(frame.commit_return, Some(true));
                match frame.extension_return {
                    Some((rusqlite::ffi::SQLITE_OK, 0)) => {
                        assert!(frame.extension_omission_return.is_none())
                    }
                    Some((code, -1))
                        if code == rusqlite::ffi::SQLITE_ERROR
                            || code == rusqlite::ffi::SQLITE_MISUSE =>
                    {
                        assert_eq!(frame.extension_omission_return, Some(1))
                    } // Actual same-writer query, not a fake config success.
                    other => panic!("extension safety has no genuine observation: {other:?}"),
                }
                assert!(frame.rollback_return.is_none());
                assert!(frame.writer.is_none());
                assert!(frame.sidecars.iter().all(|s| s.as_ref().unwrap().removed));
                assert_eq!(frame.checkpoint_return, Some((0, 0, 0)));
                frame.base.verify_prefix().unwrap();
                frame.require_header(8).unwrap();
                assert_eq!(
                    frame
                        .base
                        .source
                        .storage_parts()
                        .unwrap()
                        .0
                        .observation()
                        .original_streams,
                    6
                );
                let saved = (
                    frame.base.anchor.unwrap(),
                    original_target,
                    owned_records(frame),
                    frame.transformed.as_ref().unwrap().length,
                    frame.transformed.as_ref().unwrap().sha256.clone(),
                );
                drop(owner);
                saved // End source lease and Work before a new genuine cap.
            },
            |(anchor, node, bytes, length, hash), original| {
                let mut owner = match AdditiveStorageTransformed::create_or_resume_transformed(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("actual cold Transformed refused: {}", held.first_error()),
                };
                let frame = &mut owner.frame;
                assert_eq!(frame.phase, TransformPhase::Transformed);
                assert_eq!(frame.base.anchor, Some(anchor));
                assert_eq!(frame.base.target_node, Some(node));
                assert_eq!(owned_records(frame), bytes);
                assert_eq!(frame.transformed.as_ref().unwrap().length, length);
                assert_eq!(frame.transformed.as_ref().unwrap().sha256, hash);
                assert!(!frame.base.copy_issued && frame.writer.is_none());
                assert!(frame.begin_return.is_none() && frame.commit_return.is_none()); // Cold bytes are not a fresh commit fact.
                frame.base.verify_prefix().unwrap();
                frame.require_header(8).unwrap();
                drop(owner);
            },
        );
    }
    #[test]
    fn task6_additive_wal_transform_rollback_and_partial_cold_states() {
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let copied = actual_copied(original);
                let held = match TransformFrame::new(copied).run(TransformCut::AfterStarted) {
                    Err(held) => held,
                    Ok(_) => panic!("fixed Started cut did not stop"),
                };
                assert_eq!(held.frame.base.count(), 4);
                assert!(held.frame.writer.is_none());
                let saved = (
                    held.frame.base.target_node.unwrap(),
                    owned_records(&held.frame),
                );
                drop(held);
                saved
            },
            |(node, before), original| {
                let owner = match AdditiveStorageTransformed::create_or_resume_transformed(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("actual Started resume refused: {}", held.first_error()),
                };
                assert_eq!(owner.frame.base.target_node, Some(node));
                for (slot, bytes) in before.iter().enumerate() {
                    assert_eq!(
                        &owner.frame.base.records[slot].as_ref().unwrap().bytes,
                        bytes
                    );
                }
                assert_eq!(owner.frame.base.count(), 5);
                drop(owner);
            },
        );
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let copied = actual_copied(original);
                let mut held = match TransformFrame::new(copied).run(TransformCut::DuplicateDdl) {
                    Err(held) => held,
                    Ok(_) => panic!("actual duplicate CREATE unexpectedly succeeded"),
                };
                assert!(matches!(
                    held.first_error(),
                    GlobalSchemaV1Error::SelectionSqlite {
                        operation: "create additive Catalog7",
                        ..
                    }
                ));
                let frame = &mut held.frame;
                assert_eq!(frame.base.count(), 4);
                assert_eq!(frame.begin_return, Some(true));
                assert!(matches!(frame.rollback_return.as_ref(), Some(Ok(()))));
                assert_eq!(frame.rollback_autocommit, Some(true));
                assert!(frame.commit_return.is_none());
                assert!(frame.writer.is_some());
                let writer = frame.writer.as_ref().unwrap();
                assert_eq!(
                    writer
                        .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    6
                );
                assert_eq!(writer.query_row("SELECT COUNT(*) FROM main.sqlite_schema WHERE name='candidate_scope_observations_v1'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
                let spent = frame.base.source.storage_parts().unwrap().2.metadata_used();
                frame.rollback_once(); // No second rollback or work/debit.
                assert_eq!(
                    frame.base.source.storage_parts().unwrap().2.metadata_used(),
                    spent
                );
                let first = held.first_error().to_string();
                // Test-only complete real cleanup leaves a cold safe Started prefix;
                // it never emits Transformed or changes the retained first error.
                let writer = held.frame.writer.take().unwrap();
                match writer.close() {
                    Ok(()) => held.frame.phase = TransformPhase::Closed,
                    Err((writer, error)) => {
                        held.frame.writer = Some(writer);
                        panic!("actual rollback writer close: {error}");
                    }
                }
                held.frame.remove_sidecars().unwrap();
                assert_eq!(held.first_error().to_string(), first);
                // Unknown changed target bytes cannot be adopted on next cold entry.
                held.frame
                    .base
                    .target()
                    .unwrap()
                    .write_all_at(b"X", 0)
                    .unwrap();
                let node = held.frame.base.target_node.unwrap();
                drop(held);
                node
            },
            |node, original| {
                let held = match AdditiveStorageTransformed::create_or_resume_transformed(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Err(held) => held,
                    Ok(_) => panic!("cold damaged Started was repaired"),
                };
                assert_eq!(held.frame.base.target_node, Some(node));
                assert_eq!(held.frame.base.count(), 4);
                assert!(held.frame.writer.is_none());
                assert!(held.frame.base.records[4].is_none());
                drop(held);
            },
        );
    }
    #[test]
    fn task6_additive_wal_transform_busy_close_and_cumulative_owner() {
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let copied = actual_copied(original);
            let fd = copied.target().unwrap().as_raw_fd();
            let node = copied.target_node.unwrap();
            let mut held = match TransformFrame::new(copied).run(TransformCut::BusyClose) {
                Err(held) => held,
                Ok(_) => panic!("real unfinalized SELECT1 did not make close Busy"),
            };
            assert!(
                matches!(held.first_error(), GlobalSchemaV1Error::SelectionSqlite { operation: "close additive writer", source }
                if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy))
            );
            let first = held.first_error().to_string();
            let frame = &mut held.frame;
            assert!(frame.writer.is_some());
            assert!(frame.busy_vm.is_some());
            assert_eq!(frame.busy_prepare_return, Some(rusqlite::ffi::SQLITE_OK));
            assert!(frame.busy_finalize_return.is_none());
            assert_eq!(frame.base.count(), 4);
            assert_eq!(frame.base.target_node, Some(node));
            assert_eq!(frame.base.target().unwrap().as_raw_fd(), fd);
            let used = frame.base.source.storage_parts().unwrap().2.metadata_used();
            {
                let (_, _, work) = frame.base.source.storage_parts().unwrap();
                work.metadata(17).unwrap();
            }
            assert_eq!(
                frame.base.source.storage_parts().unwrap().2.metadata_used(),
                used + 17
            );
            frame.finalize_busy_once().unwrap();
            assert!(frame.finalize_busy_once().is_err());
            assert_eq!(frame.busy_finalize_return, Some(rusqlite::ffi::SQLITE_OK));
            // Fixed resource cleanup may now consume this exact returned writer.
            // The first Busy remains, and no successful record is published.
            let writer = frame.writer.take().unwrap();
            match writer.close() {
                Ok(()) => frame.phase = TransformPhase::Closed,
                Err((writer, error)) => {
                    frame.writer = Some(writer);
                    panic!("actual post-finalize close: {error}");
                }
            }
            frame.remove_sidecars().unwrap();
            assert!(frame.base.records[4].is_none());
            assert_eq!(held.first_error().to_string(), first);
            let mut moved = held;
            assert_eq!(moved.frame.base.target().unwrap().as_raw_fd(), fd);
            let (_, _, work) = moved.frame.base.source.storage_parts().unwrap();
            let remaining = 16 * MIB - work.metadata_used();
            assert!(work.metadata(remaining + 1).is_err()); // Actual fixed same meter, negative-only.
            let attempted = work.metadata_used();
            assert!(work.metadata(1).is_err());
            assert!(work.metadata_used() >= attempted); // No reconstruction/refund.
            assert_eq!(moved.frame.base.count(), 4);
            assert_eq!(moved.first_error().to_string(), first);
            drop(moved);
        });
    }
}

// Minted only after a complete Transformed proof in the same whole owning
// frame. No Clone/Copy/default/serde, public constructor, getter or new meter.
pub(super) struct Retained8FramePermit {
    _private: (),
}
#[derive(Default)]
struct Retained8ReadFacts {
    query_only_set: bool,
    query_only: Option<i64>,
    readonly: Option<bool>,
    first_read: Option<i64>,
    close_attempted: bool,
    closed: bool,
    original_tail_validated: bool,
}
struct Retained8Frame {
    transform: TransformFrame,
    permit: Option<Retained8FramePermit>,
    reader: Option<Connection>,
    active: Option<usize>,
    routes: [Option<std::path::PathBuf>; 2],
    uris: [Option<String>; 2],
    facts: [Retained8ReadFacts; 2],
    cleanup_close: Option<rusqlite::Result<()>>,
    #[cfg(test)]
    busy_vm: Option<std::ptr::NonNull<rusqlite::ffi::sqlite3_stmt>>,
    #[cfg(test)]
    busy_prepare: Option<i32>,
    #[cfg(test)]
    busy_finalize: Option<i32>,
}
pub(super) struct AdditiveStorageReadonlyCompared {
    frame: Retained8Frame,
}
pub(super) struct AdditiveStorageReadonlyHeld {
    frame: Retained8Frame,
}
impl AdditiveStorageReadonlyHeld {
    pub(super) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame.transform.first.as_ref().unwrap()
    }
}
impl AdditiveStorageTransformed {
    pub(super) fn into_readonly_compared(
        self,
    ) -> std::result::Result<AdditiveStorageReadonlyCompared, AdditiveStorageReadonlyHeld> {
        Retained8Frame::new(self.frame).run(false)
    }
}
impl AdditiveStorageReadonlyCompared {
    // Reader-only cold dispatch: no mkdir, writer, DDL, COMMIT or repair path.
    // Existing slot5 is start-only history; both pairs use the new genuine cap.
    pub(super) fn create_or_resume_readonly_compared(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageReadonlyHeld> {
        let base = AdditiveStorageCopied {
            source,
            managed: None,
            directory: None,
            anchor: None,
            fresh: false,
            original: None,
            rows: None,
            records: std::array::from_fn(|_| None),
            pending: None,
            target: None,
            target_node: None,
            copied: None,
            census_files: std::array::from_fn(|_| None),
            codec: AdditiveRecordCodecState::new(),
            copy_issued: false,
            rejected_copy_return: None,
            copy_return_failed: false,
            copy_return_error: None,
            copy_origin_return_error: None,
        };
        Retained8Frame::new(TransformFrame::new(base)).run(true)
    }
}
impl Retained8Frame {
    fn new(transform: TransformFrame) -> Self {
        Self {
            transform,
            permit: None,
            reader: None,
            active: None,
            routes: std::array::from_fn(|_| None),
            uris: std::array::from_fn(|_| None),
            facts: std::array::from_fn(|_| Retained8ReadFacts::default()),
            cleanup_close: None,
            #[cfg(test)]
            busy_vm: None,
            #[cfg(test)]
            busy_prepare: None,
            #[cfg(test)]
            busy_finalize: None,
        }
    }
    fn run(
        mut self,
        cold: bool,
    ) -> std::result::Result<AdditiveStorageReadonlyCompared, AdditiveStorageReadonlyHeld> {
        if self.transform.first.is_some() {
            return Err(AdditiveStorageReadonlyHeld { frame: self });
        }
        let prepared = if cold {
            self.load_cold().and_then(|()| self.prepare())
        } else {
            self.prepare()
        };
        if let Err(first) = prepared {
            self.transform.first = Some(first);
            return Err(AdditiveStorageReadonlyHeld { frame: self });
        }
        if !self.step() || !self.step() {
            return Err(AdditiveStorageReadonlyHeld { frame: self });
        }
        Ok(AdditiveStorageReadonlyCompared { frame: self })
    }
    fn load_cold(&mut self) -> StorageResult<()> {
        let b = &mut self.transform.base;
        let (core, _, work) = b.source.storage_parts()?;
        b.rows = Some(core.original_binding(work)?);
        b.original = Some(core.with_copy_origin(work, |loan, work| loan.binding(work))?);
        work.metadata(32768)?;
        core.with_namespace(|ns| {
            ns.validate_unchanged()?;
            let main = storage_io(ns.database_parent.file.metadata())?;
            if !main.is_dir()
                || main.uid() != unsafe { libc::geteuid() }
                || main.mode() & 0o022 != 0
            {
                return Err(storage_fail(
                    "additive cold readonly original parent is not private",
                ));
            }
            b.managed = Some(storage_open_directory(
                &ns.database_parent.file,
                STORAGE_MANAGED,
            )?);
            b.directory = Some(storage_open_directory(
                b.managed.as_ref().unwrap(),
                STORAGE_OPERATION,
            )?);
            let managed = StorageDirectoryIdentity::from_file(b.managed.as_ref().unwrap())?;
            let operation = StorageDirectoryIdentity::from_file(b.directory.as_ref().unwrap())?;
            if managed.device != main.dev() || operation.device != main.dev() {
                return Err(storage_fail("additive cold readonly filesystem changed"));
            }
            b.anchor = Some(StorageAnchor {
                main_device: main.dev(),
                main_inode: main.ino(),
                managed,
                operation,
            });
            ns.validate_unchanged()
        })?;
        let observed = b.census()?; // Strict actual EOF, no sidecars/extra names.
        let count = observed[..6].iter().take_while(|n| n.is_some()).count();
        if !matches!(count, 5 | 6)
            || observed[count..6].iter().any(Option::is_some)
            || observed[6].is_none()
        {
            return Err(storage_fail(
                "additive cold readonly requires exact Transformed/VerificationStarted",
            ));
        }
        b.target = Some(storage_open_file(b.directory()?, STORAGE_TARGET, false)?);
        b.target_node = Some(StorageNode::from_file(b.target()?, false)?);
        if b.target_node != observed[6] {
            return Err(storage_fail("additive cold readonly target changed"));
        }
        for slot in 0..count {
            let file = storage_open_file(b.directory()?, LEAVES[slot], false)?;
            b.records[slot] = Some(StorageSaved {
                file,
                node: observed[slot].unwrap(),
                bytes: Vec::new(),
                hash: String::new(),
            });
            let saved = b.records[slot].as_mut().unwrap();
            if StorageNode::from_file(&saved.file, false)? != saved.node {
                return Err(storage_fail("additive cold readonly record replaced"));
            }
            saved.bytes = storage_read_record(&saved.file, slot, b.source.storage_parts()?.2)?;
        }
        b.decode_saved()?;
        self.transform.transformed = Some(b.fingerprint()?);
        self.transform.require_transformed_witness()?;
        self.transform.require_header(8)?;
        self.transform.phase = TransformPhase::Transformed;
        Ok(())
    }
    fn prepare(&mut self) -> StorageResult<()> {
        if self.permit.is_some()
            || self.reader.is_some()
            || self.transform.first.is_some()
            || self.transform.phase != TransformPhase::Transformed
            || self.transform.writer.is_some()
            || self.transform.sidecar_error.is_some()
            || self.transform.rollback_return.is_some()
            || self.transform.sidecars.iter().flatten().any(|s| !s.removed)
            || self.transform.base.pending.is_some()
            || !matches!(self.transform.base.count(), 5 | 6)
        {
            return Err(storage_fail(
                "additive readonly entry lacks clean Transformed custody",
            ));
        }
        // These old zero-pair wrappers are used only before the one permit mint.
        self.transform.base.verify_prefix()?;
        self.transform.transformed = Some(self.transform.base.fingerprint()?);
        self.transform.require_transformed_witness()?;
        self.transform.require_header(8)?;
        self.permit = Some(Retained8FramePermit { _private: () });
        self.revalidate_target()?;
        if self.transform.base.count() == 5 {
            self.emit_verification_started()?;
        }
        self.revalidate_target()
    }
    fn loan(
        &mut self,
    ) -> StorageResult<(
        &mut rows::OriginalRowsTargetSource,
        &ClosedAdditiveCatalog8Recipe,
        &mut target::TargetWork,
        u64,
    )> {
        self.transform.base.source.retained8_parts(
            self.permit
                .as_ref()
                .ok_or_else(|| storage_fail("additive retained8 permit missing"))?,
        )
    }
    fn ancestors(&mut self) -> StorageResult<()> {
        let Self {
            transform, permit, ..
        } = self;
        let b = &mut transform.base;
        let (core, _, _, _) = b.source.retained8_parts(permit.as_ref().unwrap())?;
        core.with_namespace(|ns| {
            ns.validate_unchanged()?;
            let main = storage_io(ns.database_parent.file.metadata())?;
            let anchor = b.anchor.unwrap();
            if main.dev() != anchor.main_device || main.ino() != anchor.main_inode {
                return Err(storage_fail("additive readonly original parent changed"));
            }
            let reopened = storage_open_directory(&ns.database_parent.file, STORAGE_MANAGED)?;
            let managed = b.managed.as_ref().unwrap();
            if StorageDirectoryIdentity::from_file(&reopened)? != anchor.managed
                || StorageDirectoryIdentity::from_file(managed)? != anchor.managed
            {
                return Err(storage_fail("additive readonly managed parent changed"));
            }
            let reopened = storage_open_directory(managed, STORAGE_OPERATION)?;
            if StorageDirectoryIdentity::from_file(&reopened)? != anchor.operation
                || StorageDirectoryIdentity::from_file(b.directory.as_ref().unwrap())?
                    != anchor.operation
            {
                return Err(storage_fail(
                    "additive readonly operation directory changed",
                ));
            }
            ns.validate_unchanged()
        })
    }
    fn decode_retained(&mut self) -> StorageResult<()> {
        let Self {
            transform, permit, ..
        } = self;
        let b = &mut transform.base;
        let (_, recipe, work, _) = b.source.retained8_parts(permit.as_ref().unwrap())?;
        let identity = AdditiveRecordIdentity {
            anchor: b.anchor.unwrap().declared(),
            record_nodes: std::array::from_fn(|i| b.records[i].as_ref().map(|r| r.node.declared())),
            target_node: b.target_node.unwrap().declared(),
            original: b.original.as_ref().unwrap(),
            rows: b.rows.as_deref().unwrap(),
        };
        let prefix = decode_prefix(
            recipe,
            &identity,
            std::array::from_fn(|i| b.records[i].as_ref().map(|r| r.bytes.as_slice())),
            &mut b.codec,
        )
        .map_err(|_| storage_fail("additive readonly canonical prefix refused"))?;
        let expected = prefix.records[4]
            .and_then(|r| r.witness)
            .ok_or_else(|| storage_fail("additive readonly Transformed witness missing"))?;
        let actual = transform.transformed.as_ref().unwrap();
        if expected.node != actual.node.declared()
            || expected.length != actual.length
            || expected.sha256 != actual.sha256
        {
            return Err(storage_fail(
                "additive readonly Transformed witness differs",
            ));
        }
        let hashes: [Option<[u8; 32]>; 6] =
            std::array::from_fn(|i| prefix.records[i].map(|r| r.checksum));
        for (slot, hash) in hashes.into_iter().enumerate() {
            if let Some(hash) = hash {
                work.metadata(64)?;
                b.records[slot].as_mut().unwrap().hash = hex::encode(hash);
            }
        }
        Ok(())
    }
    fn fingerprint(&mut self) -> StorageResult<StorageWitness> {
        let Self {
            transform, permit, ..
        } = self;
        let b = &mut transform.base;
        let (_, _, work, _) = b.source.retained8_parts(permit.as_ref().unwrap())?;
        let file = b.target.as_ref().unwrap();
        let node = StorageNode::from_file(file, false)?;
        if Some(node) != b.target_node {
            return Err(storage_fail("additive readonly target inode differs"));
        }
        AdditiveStorageCopied::same_named(
            b.directory.as_ref().unwrap(),
            STORAGE_TARGET,
            file,
            node,
        )?;
        let before = storage_file_stamp(file)?;
        let length = work.additive_hash_read(file)?;
        let mut hash = Sha256::new();
        let mut offset = 0;
        let mut buffer = [0; 65536];
        while offset < length {
            let room = (length - offset).min(buffer.len() as u64) as usize;
            let n = storage_io(file.read_at(&mut buffer[..room], offset))?;
            if n == 0 {
                return Err(storage_fail("additive readonly target shortened"));
            }
            hash.update(&buffer[..n]);
            offset += n as u64;
        }
        let mut sentinel = [0];
        if storage_io(file.read_at(&mut sentinel, length))? != 0
            || storage_file_stamp(file)? != before
        {
            return Err(storage_fail("additive readonly target changed during hash"));
        }
        AdditiveStorageCopied::same_named(
            b.directory.as_ref().unwrap(),
            STORAGE_TARGET,
            file,
            node,
        )?;
        Ok(StorageWitness {
            node,
            length,
            sha256: hex::encode(hash.finalize()),
        })
    }
    fn revalidate_target(&mut self) -> StorageResult<()> {
        // Called only on the normal path. No success revalidation after first.
        if self.transform.first.is_some() {
            return Err(storage_fail("additive readonly tail after first error"));
        }
        self.loan()?.2.metadata(65536)?;
        self.ancestors()?;
        let Self {
            transform, permit, ..
        } = self;
        let b = &mut transform.base;
        let count = b.count();
        let (_, _, work, _) = b.source.retained8_parts(permit.as_ref().unwrap())?;
        let observed = storage_census(b.directory.as_ref().unwrap(), work, &mut b.census_files)?;
        if !matches!(count, 5 | 6)
            || b.pending.is_some()
            || b.copy_return_failed
            || observed[6] != b.target_node
        {
            return Err(storage_fail(
                "additive readonly fixed prefix/target obligation differs",
            ));
        }
        for slot in 0..6 {
            if b.records[slot].as_ref().map(|r| r.node) != observed[slot] {
                return Err(storage_fail(
                    "additive readonly record presence/inode differs",
                ));
            }
            if let Some(saved) = b.records[slot].as_ref() {
                AdditiveStorageCopied::same_named(
                    b.directory.as_ref().unwrap(),
                    LEAVES[slot],
                    &saved.file,
                    saved.node,
                )?;
                if storage_read_record(&saved.file, slot, work)? != saved.bytes {
                    return Err(storage_fail("additive readonly record bytes changed"));
                }
            }
        }
        // The owned initial transcript/limits and minted binding stay in this
        // frame. Pure target boundaries do not re-encode that immutable proof.
        // The real original loan validates before/after each typed pair; after
        // consuming close, original_tail validates its retained physical tail.
        self.decode_retained()?;
        if self.fingerprint()? != *self.transform.transformed.as_ref().unwrap() {
            return Err(storage_fail(
                "additive readonly retained target bytes changed",
            ));
        }
        self.loan()?.2.additive_header_read()?;
        let before = storage_file_stamp(self.transform.base.target()?)?;
        let mut header = [0; 100];
        storage_io(self.transform.base.target()?.read_exact_at(&mut header, 0))?;
        if &header[..16] != b"SQLite format 3\0"
            || header[18..20] != [2, 2]
            || u32::from_be_bytes(header[60..64].try_into().unwrap()) != 8
            || storage_file_stamp(self.transform.base.target()?)? != before
        {
            return Err(storage_fail("additive readonly header changed"));
        }
        // Preserve each acquired route/URI and recheck routing after close.
        for slot in 0..2 {
            if self.routes[slot].is_some() {
                self.loan()?.2.metadata(32768)?;
                let actual = super::super::sqlite_open_route_from_retained_parent(
                    self.transform.base.directory()?,
                    OsStr::new(STORAGE_TARGET),
                )
                .map_err(|_| storage_fail("additive readonly retained route disappeared"))?;
                if self.routes[slot].as_ref() != Some(&actual) {
                    return Err(storage_fail("additive readonly route changed"));
                }
            }
        }
        self.ancestors()
    }
    fn emit_verification_started(&mut self) -> StorageResult<()> {
        if self.transform.first.is_some()
            || self.reader.is_some()
            || self.transform.base.count() != 5
            || self.transform.base.pending.is_some()
            || self.loan()?.3 != 0
        {
            return Err(storage_fail("additive VerificationStarted phase differs"));
        }
        self.revalidate_target()?;
        let b = &mut self.transform.base;
        b.pending = Some(StoragePendingRecord {
            file: storage_create(b.directory()?, LEAVES[5])?,
            node: None,
            canonical: Vec::new(),
            bytes: Vec::new(),
            readback: Vec::new(),
        });
        let node = StorageNode::from_file(&b.pending.as_ref().unwrap().file, false)?;
        b.pending.as_mut().unwrap().node = Some(node);
        let (_, _, work, _) = b.source.retained8_parts(self.permit.as_ref().unwrap())?;
        let record = StorageRecord {
            version: 1,
            slot: 5,
            leaf: LEAVES[5],
            anchor: b.anchor.unwrap(),
            self_inode: node,
            intent: b.records[0].as_ref().map(|r| r.hash.as_str()),
            predecessor: b.records[4].as_ref().map(|r| r.hash.as_str()),
            transition: StorageTransition::VerificationStarted {
                file: self.transform.transformed.as_ref().unwrap(),
            },
        };
        let pending = b.pending.as_mut().unwrap();
        pending.canonical = work.additive_encode(&record, EVENT_LIMIT)?;
        work.metadata(64)?;
        let hash = hex::encode(record_checksum(&pending.canonical));
        pending.bytes = work.additive_encode(
            &StorageEnvelope {
                sha256: &hash,
                record: &record,
            },
            EVENT_LIMIT,
        )?;
        let total = b
            .records
            .iter()
            .flatten()
            .try_fold(pending.bytes.len() as u64, |n, r| {
                n.checked_add(r.bytes.len() as u64)
            })
            .ok_or_else(|| storage_fail("additive VerificationStarted held overflow"))?;
        if total > HELD_LIMIT {
            return Err(storage_fail("additive VerificationStarted held exceeded"));
        }
        work.additive_record_write(&pending.bytes)?;
        storage_io(pending.file.write_all_at(&pending.bytes, 0))?;
        storage_io(pending.file.sync_all())?;
        pending.readback = storage_read_record(&pending.file, 5, work)?;
        if pending.bytes != pending.readback {
            return Err(storage_fail(
                "additive VerificationStarted readback differs",
            ));
        }
        let pending = b.pending.take().unwrap();
        b.records[5] = Some(StorageSaved {
            file: pending.file,
            node,
            bytes: pending.bytes,
            hash,
        });
        self.decode_retained()?;
        storage_io(self.transform.base.directory()?.sync_all())?;
        self.revalidate_target()
    }
    fn open_reader(&mut self, slot: usize) -> StorageResult<()> {
        if slot >= 2
            || self.reader.is_some()
            || self.active.is_some()
            || self.facts[slot].close_attempted
            || self.transform.first.is_some()
            || self.transform.base.count() != 6
            || (slot > 0 && !self.facts[slot - 1].original_tail_validated)
        {
            return Err(storage_fail("additive readonly open phase differs"));
        }
        self.loan()?.2.metadata(32768)?;
        self.routes[slot] = Some(
            super::super::sqlite_open_route_from_retained_parent(
                self.transform.base.directory()?,
                OsStr::new(STORAGE_TARGET),
            )
            .map_err(|_| storage_fail("additive readonly route unavailable"))?,
        );
        let route = self.routes[slot].as_ref().unwrap().as_os_str().as_bytes();
        let n = route
            .len()
            .checked_mul(3)
            .and_then(|n| n.checked_add(32))
            .ok_or_else(|| storage_fail("additive readonly URI overflow"))?;
        self.transform
            .base
            .source
            .retained8_parts(self.permit.as_ref().unwrap())?
            .2
            .metadata(n as u64)?;
        let mut uri = String::new();
        uri.try_reserve_exact(n)
            .map_err(|_| storage_fail("additive readonly URI allocation"))?;
        uri.push_str("file:");
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        for byte in route {
            uri.push('%');
            uri.push(HEX[(byte >> 4) as usize] as char);
            uri.push(HEX[(byte & 15) as usize] as char);
        }
        uri.push_str("?mode=ro&immutable=1");
        self.uris[slot] = Some(uri);
        self.reader = Some(
            Connection::open_with_flags(
                self.uris[slot].as_ref().unwrap(),
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(|e| transform_sql_error("open additive readonly", e))?,
        );
        self.active = Some(slot); // Park owner before all fallible post-open cuts.
        self.loan()?.2.metadata(4096)?;
        let reader = self.reader.as_ref().unwrap();
        reader
            .execute_batch("PRAGMA query_only=ON")
            .map_err(|e| transform_sql_error("set additive readonly query-only", e))?;
        self.facts[slot].query_only_set = true;
        let actual: i64 = reader
            .query_row("PRAGMA query_only", [], |r| r.get(0))
            .map_err(|e| transform_sql_error("observe additive readonly query-only", e))?;
        self.facts[slot].query_only = Some(actual);
        let readonly = reader
            .is_readonly(rusqlite::DatabaseName::Main)
            .map_err(|e| transform_sql_error("observe additive readonly flag", e))?;
        self.facts[slot].readonly = Some(readonly);
        let first_read: i64 = reader
            .query_row("SELECT COUNT(*) FROM main.sqlite_schema", [], |r| r.get(0))
            .map_err(|e| transform_sql_error("first additive readonly read", e))?;
        self.facts[slot].first_read = Some(first_read);
        if actual != 1 || !readonly || first_read <= 0 {
            return Err(storage_fail("additive readonly facts differ"));
        }
        self.revalidate_target()
    }
    fn close_reader(&mut self, slot: usize) -> StorageResult<()> {
        if self.active != Some(slot) || self.facts[slot].close_attempted {
            return Err(storage_fail(
                "additive readonly consuming close phase differs",
            ));
        }
        self.facts[slot].close_attempted = true;
        let reader = self
            .reader
            .take()
            .ok_or_else(|| storage_fail("additive readonly owner missing"))?;
        match reader.close() {
            Ok(()) => {
                self.facts[slot].closed = true;
                self.active = None;
                Ok(())
            }
            Err((reader, error)) => {
                self.reader = Some(reader); // Retain actual returned Connection before error mapping.
                Err(transform_sql_error("close additive readonly", error))
            }
        }
    }
    fn original_tail(&mut self, slot: usize) -> StorageResult<()> {
        if slot >= 2
            || self.transform.first.is_some()
            || self.reader.is_some()
            || self.active.is_some()
            || !self.facts[slot].closed
            || self.facts[slot].original_tail_validated
        {
            return Err(storage_fail(
                "additive readonly original tail phase differs",
            ));
        }
        let (core, _, _, completed) = self.loan()?;
        if completed != slot as u64 + 1 {
            return Err(storage_fail("additive readonly original tail pair differs"));
        }
        core.validate_without_hooks()?; // Same original RowsWork; actual full closed-source tail.
        self.facts[slot].original_tail_validated = true;
        Ok(())
    }
    fn step(&mut self) -> bool {
        if self.transform.first.is_some() {
            return false;
        } // No new error/read/debit after first.
        let result = (|| {
            let completed = self.loan()?.3;
            if completed >= 2 {
                return Err(storage_fail("additive readonly third pair refused"));
            }
            self.revalidate_target()?;
            let slot = completed as usize;
            self.open_reader(slot)?;
            self.transform
                .base
                .source
                .compare_projection_core(self.reader.as_ref().unwrap())?;
            self.close_reader(slot)?;
            self.original_tail(slot)?;
            self.revalidate_target()?;
            if self.loan()?.3 != completed + 1 {
                return Err(storage_fail("additive readonly pair tail differs"));
            }
            Ok(())
        })();
        match result {
            Ok(()) => true,
            Err(first) => {
                self.transform.first = Some(first);
                self.cleanup_reader_once();
                false
            }
        }
    }
    fn cleanup_reader_once(&mut self) {
        // Fixed acquired-resource cleanup only. No new diagnostic or debit.
        if self.transform.first.is_none() || self.cleanup_close.is_some() {
            return;
        }
        #[cfg(test)]
        if self.busy_vm.is_some() {
            return;
        }
        let attempted = self
            .active
            .is_some_and(|slot| self.facts[slot].close_attempted);
        #[cfg(test)]
        let finalized = self.busy_finalize == Some(rusqlite::ffi::SQLITE_OK);
        #[cfg(not(test))]
        let finalized = false;
        // A failed consuming close stays Held. Only the cfg real-VM fixture
        // can prove new finalize progress; it cannot issue a production tail.
        if attempted && !finalized {
            return;
        }
        let Some(reader) = self.reader.take() else {
            return;
        };
        match reader.close() {
            Ok(()) => {
                self.cleanup_close = Some(Ok(()));
                self.active = None;
            }
            Err((reader, error)) => {
                self.reader = Some(reader);
                self.cleanup_close = Some(Err(error));
            }
        }
    }
    #[cfg(test)]
    fn prepare_busy_vm(&mut self) {
        assert!(self.reader.is_some() && self.busy_prepare.is_none());
        let mut raw = std::ptr::null_mut();
        let code = unsafe {
            rusqlite::ffi::sqlite3_prepare_v2(
                self.reader.as_ref().unwrap().handle(),
                b"SELECT 1\0".as_ptr().cast(),
                -1,
                &mut raw,
                std::ptr::null_mut(),
            )
        };
        self.busy_vm = std::ptr::NonNull::new(raw);
        self.busy_prepare = Some(code);
        assert_eq!(code, rusqlite::ffi::SQLITE_OK);
        assert!(self.busy_vm.is_some());
    }
    #[cfg(test)]
    fn finalize_busy_once(&mut self) -> bool {
        let Some(vm) = self.busy_vm.take() else {
            return false;
        };
        let code = unsafe { rusqlite::ffi::sqlite3_finalize(vm.as_ptr()) };
        self.busy_finalize = Some(code);
        code == rusqlite::ffi::SQLITE_OK
    }
}
#[cfg(test)]
impl Drop for Retained8Frame {
    fn drop(&mut self) {
        // Fixture-only leak prevention; never a successful consuming close/tail.
        if let Some(vm) = self.busy_vm.take() {
            self.busy_finalize = Some(unsafe { rusqlite::ffi::sqlite3_finalize(vm.as_ptr()) });
        }
    }
}

#[cfg(test)]
mod readonly8_tests {
    use super::*;
    fn real_transformed(
        original: rows::VerifiedUnapprovedOriginalRowsBackup,
    ) -> AdditiveStorageTransformed {
        let copied =
            match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
                Ok(owner) => owner,
                Err(held) => panic!("real Copied refused: {}", held.first_error()),
            };
        match copied.into_transformed() {
            Ok(owner) => owner,
            Err(held) => panic!("real WAL refused: {}", held.first_error()),
        }
    }
    fn compared(owner: AdditiveStorageTransformed) -> AdditiveStorageReadonlyCompared {
        match owner.into_readonly_compared() {
            Ok(owner) => owner,
            Err(held) => panic!("real readonly8 refused: {}", held.first_error()),
        }
    }
    fn cold(
        original: rows::VerifiedUnapprovedOriginalRowsBackup,
    ) -> AdditiveStorageReadonlyCompared {
        match AdditiveStorageReadonlyCompared::create_or_resume_readonly_compared(
            original.into_additive_target_source().unwrap(),
        ) {
            Ok(owner) => owner,
            Err(held) => panic!("real cold readonly8 refused: {}", held.first_error()),
        }
    }
    fn records(frame: &Retained8Frame) -> Vec<(StorageNode, Vec<u8>)> {
        frame
            .transform
            .base
            .records
            .iter()
            .flatten()
            .map(|r| (r.node, r.bytes.clone()))
            .collect()
    }
    fn assert_two_pairs(frame: &mut Retained8Frame) {
        assert!(
            frame.reader.is_none() && frame.active.is_none() && frame.transform.writer.is_none()
        );
        assert!(frame.transform.first.is_none());
        assert_eq!(frame.transform.base.count(), 6);
        for facts in &frame.facts {
            assert!(
                facts.query_only_set
                    && facts.close_attempted
                    && facts.closed
                    && facts.original_tail_validated
            );
            assert_eq!(facts.query_only, Some(1));
            assert_eq!(facts.readonly, Some(true));
            assert!(facts.first_read.is_some_and(|n| n > 0));
        }
        let (core, _, _, count) = frame.loan().unwrap();
        assert_eq!(count, 2);
        assert_eq!(core.observation().target_streams, 4);
        assert_eq!(core.observation().original_streams, 6);
        frame.revalidate_target().unwrap();
    }
    #[test]
    fn task6_additive_readonly8_two_real_pairs_and_cold5_cold6() {
        // Cold5 starts no writer and emits only the one start-history slot.
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = real_transformed(original);
                let saved = (
                    owner.frame.base.target_node.unwrap(),
                    owner
                        .frame
                        .base
                        .records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                );
                drop(owner);
                saved
            },
            |(node, prefix), original| {
                let mut owner = cold(original);
                assert_two_pairs(&mut owner.frame);
                assert_eq!(owner.frame.transform.base.target_node, Some(node));
                assert_eq!(&records(&owner.frame)[..5], prefix.as_slice());
                assert!(
                    owner.frame.transform.begin_return.is_none()
                        && owner.frame.transform.commit_return.is_none()
                );
                assert!(!owner.frame.transform.base.copy_issued);
                drop(owner);
            },
        );
        // Normal ownership consumes the actual same source/File/Work once.
        // Cold6 reuses record bytes/inodes, not pair counters or an old cap.
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = real_transformed(original);
                let fd = owner.frame.base.target().unwrap().as_raw_fd();
                let node = owner.frame.base.target_node.unwrap();
                let mut owner = compared(owner);
                assert_two_pairs(&mut owner.frame);
                assert_eq!(owner.frame.transform.base.target().unwrap().as_raw_fd(), fd);
                assert_eq!(owner.frame.transform.base.target_node, Some(node));
                let saved = (node, records(&owner.frame));
                drop(owner);
                saved
            },
            |(node, prefix), original| {
                let mut owner = cold(original);
                assert_two_pairs(&mut owner.frame);
                assert_eq!(owner.frame.transform.base.target_node, Some(node));
                assert_eq!(records(&owner.frame), prefix);
                assert!(
                    owner.frame.transform.begin_return.is_none()
                        && owner.frame.transform.commit_return.is_none()
                );
                drop(owner);
            },
        );
    }
    #[test]
    fn task6_additive_readonly8_drift_and_incomplete_prefix_stop_next_pair() {
        for cut in 0..6 {
            super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
                let mut frame = Retained8Frame::new(real_transformed(original).frame);
                frame.prepare().unwrap();
                assert!(frame.step());
                assert!(
                    frame.reader.is_none()
                        && frame.facts[0].closed
                        && frame.facts[0].original_tail_validated
                        && !frame.facts[1].close_attempted
                        && !frame.facts[1].original_tail_validated
                );
                let target_fd = frame.transform.base.target().unwrap().as_raw_fd();
                let route = frame.routes[0].as_ref().unwrap().clone();
                match cut {
                    0 => frame.transform.base.records[4]
                        .as_ref()
                        .unwrap()
                        .file
                        .write_all_at(b"X", 0)
                        .unwrap(),
                    1 => {
                        let file = storage_create(
                            frame.transform.base.directory().unwrap(),
                            WAL_LEAVES[0],
                        )
                        .unwrap();
                        file.write_all_at(b"TEST_CODE_WAL", 0).unwrap();
                        drop(file);
                    }
                    2 => {
                        std::fs::remove_file(route.with_file_name(LEAVES[5])).unwrap();
                        std::fs::hard_link(
                            route.with_file_name(LEAVES[4]),
                            route.with_file_name(LEAVES[5]),
                        )
                        .unwrap();
                    }
                    3 => {
                        let bytes = std::fs::read(&route).unwrap();
                        std::fs::remove_file(&route).unwrap();
                        let replacement = storage_create(
                            frame.transform.base.directory().unwrap(),
                            STORAGE_TARGET,
                        )
                        .unwrap();
                        replacement.write_all_at(&bytes, 0).unwrap();
                        drop(replacement);
                    }
                    _ => {
                        // Genuine target drift after the first reader's real close.
                        // It cannot be adopted by replacing the Transformed witness.
                        let writer =
                            Connection::open_with_flags(&route, OpenFlags::SQLITE_OPEN_READ_WRITE)
                                .unwrap();
                        let sql = if cut == 4 {
                            "ALTER TABLE ledger ADD COLUMN TEST_CODE_EXTRA TEXT"
                        } else {
                            "UPDATE ledger SET cash=13.5 WHERE id=1"
                        };
                        writer.execute_batch(sql).unwrap();
                        writer
                            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                            .unwrap();
                        match writer.close() {
                            Ok(()) => {}
                            Err((writer, error)) => {
                                drop(writer);
                                panic!("negative writer close: {error}");
                            }
                        }
                    }
                }
                assert!(!frame.step());
                assert!(frame.transform.first.is_some());
                assert!(frame.reader.is_none() && frame.routes[1].is_none());
                assert_eq!(frame.loan().unwrap().3, 1); // No next pair or fresh allowance.
                assert_eq!(
                    frame.transform.base.target().unwrap().as_raw_fd(),
                    target_fd
                );
                let first = frame.transform.first.as_ref().unwrap().to_string();
                let used = frame.loan().unwrap().2.metadata_used();
                assert!(!frame.step());
                assert_eq!(frame.loan().unwrap().2.metadata_used(), used);
                assert_eq!(frame.transform.first.as_ref().unwrap().to_string(), first);
                drop(frame);
            });
        }
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = Retained8Frame::new(real_transformed(original).frame);
            frame.prepare().unwrap();
            frame.open_reader(0).unwrap();
            frame
                .transform
                .base
                .source
                .compare_projection_core(frame.reader.as_ref().unwrap())
                .unwrap();
            frame.close_reader(0).unwrap();
            assert!(frame.facts[0].closed);
            let original_path = frame
                .loan()
                .unwrap()
                .0
                .with_namespace(|ns| Ok(ns.database_parent.path.join(&ns.database_leaf)))
                .unwrap();
            let original_bytes = std::fs::read(&original_path).unwrap();
            let target_stamp = storage_file_stamp(frame.transform.base.target().unwrap()).unwrap();
            let target_sha256 = frame.transform.transformed.as_ref().unwrap().sha256.clone();
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&original_path)
                .unwrap();
            let offset = 100;
            let changed = [original_bytes[offset] ^ 1];
            file.write_all_at(&changed, offset as u64).unwrap();
            file.sync_all().unwrap();
            let result = frame.original_tail(0);
            file.write_all_at(&original_bytes[offset..offset + 1], offset as u64)
                .unwrap();
            file.sync_all().unwrap();
            drop(file); // Restore the actual fixture, never the source meter.
            let first = result.unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail }
                if detail == "rows original main bytes changed")
            ); // Not a metadata-budget refusal.
            frame.transform.first = Some(first);
            assert!(!frame.facts[0].original_tail_validated && frame.reader.is_none());
            assert_eq!(
                storage_file_stamp(frame.transform.base.target().unwrap()).unwrap(),
                target_stamp
            );
            assert_eq!(
                frame.transform.transformed.as_ref().unwrap().sha256,
                target_sha256
            );
            assert_eq!(frame.loan().unwrap().3, 1);
            assert!(frame.routes[1].is_none());
            let first = frame.transform.first.as_ref().unwrap().to_string();
            let used = frame.loan().unwrap().0.observation().metadata_work;
            assert!(!frame.step());
            assert_eq!(frame.loan().unwrap().0.observation().metadata_work, used);
            assert_eq!(frame.transform.first.as_ref().unwrap().to_string(), first);
            drop(frame);
        });
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let copied = match AdditiveStorageCopied::create(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("real Copied refused: {}", held.first_error()),
                };
                let held = match TransformFrame::new(copied).run(TransformCut::AfterStarted) {
                    Err(held) => held,
                    Ok(_) => panic!("fixed Started did not stop"),
                };
                assert_eq!(held.frame.base.count(), 4);
                drop(held);
            },
            |(), original| {
                let held = match AdditiveStorageReadonlyCompared::create_or_resume_readonly_compared(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Err(held) => held,
                    Ok(_) => panic!("readonly entry repaired incomplete Started"),
                };
                assert!(held.frame.reader.is_none() && held.frame.transform.writer.is_none());
                assert!(held.frame.permit.is_none());
                assert!(held.frame.transform.base.records[5].is_none());
                drop(held);
            },
        );
    }
    #[test]
    fn task6_additive_readonly8_busy_return_and_cumulative_single_owner() {
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = Retained8Frame::new(real_transformed(original).frame);
            frame.prepare().unwrap();
            let fd = frame.transform.base.target().unwrap().as_raw_fd();
            frame.open_reader(0).unwrap();
            frame
                .transform
                .base
                .source
                .compare_projection_core(frame.reader.as_ref().unwrap())
                .unwrap();
            frame.prepare_busy_vm();
            let first = frame.close_reader(0).unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSqlite { operation: "close additive readonly", source }
                if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy))
            );
            frame.transform.first = Some(first);
            assert!(
                frame.reader.is_some()
                    && frame.busy_vm.is_some()
                    && !frame.facts[0].closed
                    && !frame.facts[0].original_tail_validated
            );
            let first = frame.transform.first.as_ref().unwrap().to_string();
            let used = frame.loan().unwrap().2.metadata_used();
            {
                let (_, _, _, count) = frame.loan().unwrap();
                assert_eq!(count, 1);
            }
            assert_eq!(frame.transform.base.target().unwrap().as_raw_fd(), fd); // Loan Drop has not closed target.
            assert!(!frame.step());
            frame.cleanup_reader_once();
            assert!(frame.reader.is_some());
            assert_eq!(frame.loan().unwrap().2.metadata_used(), used);
            assert!(frame.finalize_busy_once());
            assert!(!frame.finalize_busy_once());
            frame.cleanup_reader_once();
            assert!(matches!(frame.cleanup_close, Some(Ok(()))));
            assert!(frame.reader.is_none());
            assert!(!frame.step());
            let mut moved = frame;
            assert_eq!(moved.transform.base.target().unwrap().as_raw_fd(), fd);
            assert_eq!(moved.transform.first.as_ref().unwrap().to_string(), first);
            assert_eq!(moved.loan().unwrap().2.metadata_used(), used);
            drop(moved);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut owner = compared(real_transformed(original));
            assert_two_pairs(&mut owner.frame);
            let used = owner.frame.loan().unwrap().2.metadata_used();
            assert!(!owner.frame.step());
            assert_eq!(owner.frame.loan().unwrap().3, 2);
            assert_eq!(owner.frame.loan().unwrap().2.metadata_used(), used); // Third pair denied before fresh tail IO.
            assert!(owner.frame.reader.is_none());
            drop(owner);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = Retained8Frame::new(real_transformed(original).frame);
            frame.prepare().unwrap();
            let work = frame.loan().unwrap().2;
            let remaining = 16 * MIB - work.metadata_used();
            assert!(work.metadata(remaining + 1).is_err());
            let used = frame.loan().unwrap().2.metadata_used();
            assert!(!frame.step());
            assert!(frame.transform.first.is_some());
            assert!(frame.reader.is_none() && frame.routes[0].is_none());
            let spent = frame.loan().unwrap().2.metadata_used();
            assert!(spent >= used);
            assert!(!frame.step());
            assert_eq!(frame.loan().unwrap().2.metadata_used(), spent);
            drop(frame);
        });
    }
}

// Ordinary, local integrity completion on the second already-open reader.
// This owns the same complete frame; it issues no VerifiedTarget or financial,
// native/provider, formatter-payment or new reader capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalCompletionPhase {
    Fresh,
    FirstClosed,
    SecondCompared,
    ChecksComplete,
    ReaderClosed,
    Complete,
    Refused,
}
#[derive(Default)]
struct LocalIntegrityFacts {
    started: bool,
    integrity_row: bool,
    integrity_ok: bool,
    integrity_eof: bool,
    foreign_keys_started: bool,
    foreign_keys_eof: bool,
    scopes_ended: bool,
    returned_ok: bool,
}
struct LocalCompletionFrame {
    readonly: Retained8Frame,
    phase: LocalCompletionPhase,
    integrity: LocalIntegrityFacts,
}
pub(super) struct AdditiveStorageLocalIntegrityChecked {
    frame: LocalCompletionFrame,
}
pub(super) struct AdditiveStorageLocalIntegrityHeld {
    frame: LocalCompletionFrame,
}
impl AdditiveStorageLocalIntegrityHeld {
    pub(super) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame.readonly.transform.first.as_ref().unwrap()
    }
}
impl AdditiveStorageTransformed {
    pub(super) fn into_local_integrity_checked(
        self,
    ) -> std::result::Result<AdditiveStorageLocalIntegrityChecked, AdditiveStorageLocalIntegrityHeld>
    {
        LocalCompletionFrame::new(self.frame).run(false)
    }
}
impl AdditiveStorageLocalIntegrityChecked {
    // Fixed cold5/cold6 reader-only entry. The new genuine source carries its
    // own ended-old-process lease and one TargetWork; no arbitrary connection.
    pub(super) fn create_or_resume_local_integrity_checked(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageLocalIntegrityHeld> {
        let base = AdditiveStorageCopied {
            source,
            managed: None,
            directory: None,
            anchor: None,
            fresh: false,
            original: None,
            rows: None,
            records: std::array::from_fn(|_| None),
            pending: None,
            target: None,
            target_node: None,
            copied: None,
            census_files: std::array::from_fn(|_| None),
            codec: AdditiveRecordCodecState::new(),
            copy_issued: false,
            rejected_copy_return: None,
            copy_return_failed: false,
            copy_return_error: None,
            copy_origin_return_error: None,
        };
        LocalCompletionFrame::new(TransformFrame::new(base)).run(true)
    }
}
impl LocalCompletionFrame {
    fn new(transform: TransformFrame) -> Self {
        Self {
            readonly: Retained8Frame::new(transform),
            phase: LocalCompletionPhase::Fresh,
            integrity: LocalIntegrityFacts::default(),
        }
    }
    fn run(
        mut self,
        cold: bool,
    ) -> std::result::Result<AdditiveStorageLocalIntegrityChecked, AdditiveStorageLocalIntegrityHeld>
    {
        if !self.start(cold) || !self.advance_second() {
            return Err(AdditiveStorageLocalIntegrityHeld { frame: self });
        }
        Ok(AdditiveStorageLocalIntegrityChecked { frame: self })
    }
    fn fail(&mut self, first: GlobalSchemaV1Error) {
        if self.readonly.transform.first.is_none() {
            self.readonly.transform.first = Some(first);
        }
        self.phase = LocalCompletionPhase::Refused;
        self.readonly.cleanup_reader_once(); // Acquired-resource cleanup, no new debit or diagnostic.
    }
    fn start(&mut self, cold: bool) -> bool {
        if self.readonly.transform.first.is_some() {
            return false;
        }
        if self.phase != LocalCompletionPhase::Fresh {
            self.fail(storage_fail("additive integrity start phase differs"));
            return false;
        }
        let prepared = if cold {
            self.readonly
                .load_cold()
                .and_then(|()| self.readonly.prepare())
        } else {
            self.readonly.prepare()
        };
        if let Err(first) = prepared {
            self.fail(first);
            return false;
        }
        if !self.readonly.step() {
            self.phase = LocalCompletionPhase::Refused;
            return false;
        }
        self.phase = LocalCompletionPhase::FirstClosed;
        true
    }
    fn compare_second(&mut self) -> StorageResult<()> {
        if self.phase != LocalCompletionPhase::FirstClosed
            || self.readonly.transform.first.is_some()
            || self.readonly.loan()?.3 != 1
            || !self.readonly.facts[0].original_tail_validated
        {
            return Err(storage_fail("additive integrity second pair phase differs"));
        }
        // Same production typed core; the first step and all old step bodies
        // stay untouched. The second reader remains live for these fixed checks.
        self.readonly.revalidate_target()?;
        self.readonly.open_reader(1)?;
        self.readonly
            .transform
            .base
            .source
            .compare_projection_core(self.readonly.reader.as_ref().unwrap())?;
        self.phase = LocalCompletionPhase::SecondCompared;
        Ok(())
    }
    fn check_second(&mut self) -> StorageResult<()> {
        if self.phase != LocalCompletionPhase::SecondCompared
            || self.readonly.transform.first.is_some()
            || self.readonly.active != Some(1)
            || self.readonly.loan()?.3 != 2
        {
            return Err(storage_fail("additive integrity reader phase differs"));
        }
        let Retained8Frame {
            transform,
            permit,
            reader,
            ..
        } = &mut self.readonly;
        let (_, _, work, _) = transform
            .base
            .source
            .retained8_parts(permit.as_ref().unwrap())?;
        local_integrity_queries(reader.as_ref().unwrap(), work, &mut self.integrity)?;
        self.phase = LocalCompletionPhase::ChecksComplete;
        Ok(())
    }
    fn close_and_tail(&mut self) -> StorageResult<()> {
        if self.phase != LocalCompletionPhase::ChecksComplete
            || self.readonly.transform.first.is_some()
            || !self.integrity.returned_ok
            || !self.integrity.scopes_ended
        {
            return Err(storage_fail(
                "additive integrity close before checks returned",
            ));
        }
        self.readonly.close_reader(1)?;
        self.phase = LocalCompletionPhase::ReaderClosed;
        // The exact existing full original tail is retained, after actual close
        // and before the final target tail. No third original loan/pair/reader.
        self.readonly.original_tail(1)?;
        self.readonly.revalidate_target()?;
        if self.readonly.loan()?.3 != 2
            || !self.readonly.facts[1].closed
            || !self.readonly.facts[1].original_tail_validated
            || self.readonly.reader.is_some()
        {
            return Err(storage_fail("additive integrity final tails differ"));
        }
        self.phase = LocalCompletionPhase::Complete;
        Ok(())
    }
    fn advance_second(&mut self) -> bool {
        // First primary and a terminal meter stop every new ordinary operation.
        // Each fixed query precharges the same TargetWork, which checks terminal.
        if self.readonly.transform.first.is_some() {
            return false;
        }
        let result = self
            .compare_second()
            .and_then(|()| self.check_second())
            .and_then(|()| self.close_and_tail());
        match result {
            Ok(()) => true,
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
}

// Private, fixed SQL core. Borrowing an arbitrary test Connection proves only
// these ordinary SQL predicates, never a retained owner or completion permit.
fn local_integrity_queries(
    connection: &Connection,
    work: &mut target::TargetWork,
    facts: &mut LocalIntegrityFacts,
) -> StorageResult<()> {
    if facts.started {
        return Err(storage_fail("additive integrity queries already started"));
    }
    work.metadata(4096 + b"PRAGMA main.integrity_check(1)".len() as u64)?;
    facts.started = true;
    let result = (|| {
        let mut integrity_statement = connection
            .prepare("PRAGMA main.integrity_check(1)")
            .map_err(|e| transform_sql_error("prepare additive integrity", e))?;
        let mut integrity_rows = integrity_statement
            .query([])
            .map_err(|e| transform_sql_error("query additive integrity", e))?;
        let Some(row) = integrity_rows
            .next()
            .map_err(|e| transform_sql_error("step additive integrity", e))?
        else {
            return Err(storage_fail("additive integrity result missing"));
        };
        facts.integrity_row = true;
        if !matches!(row.get_ref(0).map_err(|e| transform_sql_error("read additive integrity", e))?,
            rusqlite::types::ValueRef::Text(bytes) if bytes == b"ok")
        {
            return Err(storage_fail("additive integrity check refused"));
        }
        facts.integrity_ok = true;
        if integrity_rows
            .next()
            .map_err(|e| transform_sql_error("finish additive integrity", e))?
            .is_some()
        {
            return Err(storage_fail("additive integrity has extra result"));
        }
        facts.integrity_eof = true;
        work.metadata(4096 + b"PRAGMA main.foreign_key_check".len() as u64)?;
        facts.foreign_keys_started = true;
        let mut foreign_statement = connection
            .prepare("PRAGMA main.foreign_key_check")
            .map_err(|e| transform_sql_error("prepare additive foreign keys", e))?;
        let mut foreign_rows = foreign_statement
            .query([])
            .map_err(|e| transform_sql_error("query additive foreign keys", e))?;
        if foreign_rows
            .next()
            .map_err(|e| transform_sql_error("step additive foreign keys", e))?
            .is_some()
        {
            return Err(storage_fail("additive foreign key violation"));
        }
        facts.foreign_keys_eof = true;
        Ok(())
    })();
    // Both actual Rows/Statements leave their callee scopes before returning
    // this owned Result. This does not fabricate ignored reset/finalize codes.
    facts.scopes_ended = true;
    facts.returned_ok = result.is_ok();
    result
}

#[cfg(test)]
mod local_integrity_tests {
    use super::*;
    fn transformed(
        original: rows::VerifiedUnapprovedOriginalRowsBackup,
    ) -> AdditiveStorageTransformed {
        let copied =
            match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
                Ok(owner) => owner,
                Err(held) => panic!("local integrity Copied: {}", held.first_error()),
            };
        match copied.into_transformed() {
            Ok(owner) => owner,
            Err(held) => panic!("local integrity WAL: {}", held.first_error()),
        }
    }
    fn checked(owner: AdditiveStorageTransformed) -> AdditiveStorageLocalIntegrityChecked {
        match owner.into_local_integrity_checked() {
            Ok(owner) => owner,
            Err(held) => panic!("local integrity: {}", held.first_error()),
        }
    }
    fn cold(
        original: rows::VerifiedUnapprovedOriginalRowsBackup,
    ) -> AdditiveStorageLocalIntegrityChecked {
        match AdditiveStorageLocalIntegrityChecked::create_or_resume_local_integrity_checked(
            original.into_additive_target_source().unwrap(),
        ) {
            Ok(owner) => owner,
            Err(held) => panic!("cold local integrity: {}", held.first_error()),
        }
    }
    fn records(frame: &LocalCompletionFrame) -> Vec<(StorageNode, Vec<u8>)> {
        frame
            .readonly
            .transform
            .base
            .records
            .iter()
            .flatten()
            .map(|r| (r.node, r.bytes.clone()))
            .collect()
    }
    fn complete(frame: &mut LocalCompletionFrame) {
        assert_eq!(frame.phase, LocalCompletionPhase::Complete);
        let facts = &frame.integrity;
        assert!(
            facts.integrity_ok
                && facts.integrity_eof
                && facts.foreign_keys_eof
                && facts.scopes_ended
                && facts.returned_ok
        );
        assert!(frame.readonly.reader.is_none() && frame.readonly.active.is_none());
        assert!(frame
            .readonly
            .facts
            .iter()
            .all(|r| r.closed && r.original_tail_validated));
        let (core, _, _, count) = frame.readonly.loan().unwrap();
        assert_eq!(count, 2);
        assert_eq!(core.observation().target_streams, 4);
        assert_eq!(core.observation().original_streams, 6);
    }
    #[test]
    fn task6_additive_final_integrity_same_owner_and_cold5_cold6() {
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = transformed(original);
                let saved = (
                    owner.frame.base.target_node.unwrap(),
                    owner
                        .frame
                        .base
                        .records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                );
                drop(owner);
                saved
            },
            |(node, prefix), original| {
                let mut owner = cold(original);
                complete(&mut owner.frame);
                assert_eq!(owner.frame.readonly.transform.base.target_node, Some(node));
                assert_eq!(&records(&owner.frame)[..5], prefix.as_slice());
                assert!(
                    owner.frame.readonly.transform.begin_return.is_none()
                        && owner.frame.readonly.transform.commit_return.is_none()
                );
                drop(owner);
            },
        );
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = transformed(original);
                let fd = owner.frame.base.target().unwrap().as_raw_fd();
                let mut frame = LocalCompletionFrame::new(owner.frame);
                assert!(frame.start(false));
                let used = frame.readonly.loan().unwrap().2.metadata_used();
                assert!(frame.advance_second());
                complete(&mut frame);
                assert!(frame.readonly.loan().unwrap().2.metadata_used() > used);
                assert_eq!(
                    frame.readonly.transform.base.target().unwrap().as_raw_fd(),
                    fd
                );
                let saved = (
                    frame.readonly.transform.base.target_node.unwrap(),
                    records(&frame),
                );
                let owner = AdditiveStorageLocalIntegrityChecked { frame };
                drop(owner);
                saved
            },
            |(node, prefix), original| {
                let mut owner = cold(original);
                complete(&mut owner.frame);
                assert_eq!(owner.frame.readonly.transform.base.target_node, Some(node));
                assert_eq!(records(&owner.frame), prefix);
                drop(owner);
            },
        );
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut owner = checked(transformed(original));
            complete(&mut owner.frame);
            drop(owner);
        });
    }
    #[test]
    fn task6_additive_final_integrity_real_bad_results_and_retained_drift() {
        for foreign_key in [false, true] {
            super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
                let mut source = original.into_additive_target_source().unwrap();
                let connection = Connection::open_in_memory().unwrap();
                if foreign_key {
                    connection.execute_batch("PRAGMA foreign_keys=OFF; CREATE TABLE p(id INTEGER PRIMARY KEY); CREATE TABLE c(pid INTEGER REFERENCES p(id)); INSERT INTO c VALUES(7)").unwrap();
                } else {
                    connection.execute_batch("CREATE TABLE c(v INTEGER CHECK(v>0)); PRAGMA ignore_check_constraints=ON; INSERT INTO c VALUES(-1); PRAGMA ignore_check_constraints=OFF").unwrap();
                }
                // This real SQL predicate fixture intentionally has no retained
                // completion permit. Its same fixed production core reaches
                // the bad result, rather than stopping at target drift first.
                let mut facts = LocalIntegrityFacts::default();
                let work = source.storage_parts().unwrap().2;
                let first = local_integrity_queries(&connection, work, &mut facts).unwrap_err();
                let expected = if foreign_key {
                    "additive foreign key violation"
                } else {
                    "additive integrity check refused"
                };
                assert!(
                    matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == expected)
                );
                assert!(
                    facts.started
                        && facts.integrity_row
                        && facts.scopes_ended
                        && !facts.returned_ok
                );
                assert_eq!(facts.foreign_keys_started, foreign_key);
                assert_eq!(facts.integrity_eof, foreign_key);
                assert!(!facts.foreign_keys_eof);
                match connection.close() {
                    Ok(()) => {}
                    Err((connection, error)) => {
                        drop(connection);
                        panic!("bad SQL fixture close: {error}");
                    }
                }
                drop(source);
            });
        }
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = LocalCompletionFrame::new(transformed(original).frame);
            assert!(frame.start(false));
            let file = frame.readonly.transform.base.target().unwrap();
            let mut byte = [0];
            file.read_exact_at(&mut byte, 100).unwrap();
            file.write_all_at(&[byte[0] ^ 1], 100).unwrap();
            file.sync_all().unwrap();
            let passed = frame.advance_second();
            frame
                .readonly
                .transform
                .base
                .target()
                .unwrap()
                .write_all_at(&byte, 100)
                .unwrap();
            frame
                .readonly
                .transform
                .base
                .target()
                .unwrap()
                .sync_all()
                .unwrap(); // Fixture cleanup only.
            assert!(!passed);
            assert_eq!(frame.phase, LocalCompletionPhase::Refused);
            assert!(matches!(frame.readonly.transform.first.as_ref().unwrap(),
                GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == "additive readonly retained target bytes changed"));
            assert!(!frame.integrity.started && frame.readonly.routes[1].is_none());
            let used = frame.readonly.loan().unwrap().2.metadata_used();
            assert!(!frame.advance_second());
            assert_eq!(frame.readonly.loan().unwrap().2.metadata_used(), used);
            assert_eq!(frame.readonly.loan().unwrap().3, 1);
            drop(frame);
        });
    }
    #[test]
    fn task6_additive_final_integrity_second_close_busy_and_first_error_hold() {
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = LocalCompletionFrame::new(transformed(original).frame);
            assert!(frame.start(false));
            frame.compare_second().unwrap();
            assert_eq!(frame.readonly.active, Some(1));
            assert!(!frame.integrity.started && !frame.readonly.facts[1].close_attempted);
            frame.check_second().unwrap();
            frame.readonly.prepare_busy_vm();
            let fd = frame.readonly.transform.base.target().unwrap().as_raw_fd();
            let first = frame.close_and_tail().unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSqlite { operation: "close additive readonly", source }
                if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy))
            );
            frame.fail(first);
            assert!(frame.readonly.reader.is_some() && frame.readonly.busy_vm.is_some());
            assert!(frame.integrity.returned_ok && frame.integrity.scopes_ended);
            assert!(
                !frame.readonly.facts[1].closed && !frame.readonly.facts[1].original_tail_validated
            );
            let used = frame.readonly.loan().unwrap().2.metadata_used();
            let original_used = frame.readonly.loan().unwrap().0.observation().metadata_work;
            let first = frame.readonly.transform.first.as_ref().unwrap().to_string();
            {
                let (_, _, _, count) = frame.readonly.loan().unwrap();
                assert_eq!(count, 2);
            }
            assert!(!frame.advance_second());
            frame.readonly.cleanup_reader_once();
            assert!(frame.readonly.reader.is_some());
            assert!(frame.readonly.finalize_busy_once());
            assert!(!frame.readonly.finalize_busy_once());
            frame.readonly.cleanup_reader_once();
            assert!(matches!(frame.readonly.cleanup_close, Some(Ok(()))));
            assert!(
                frame.readonly.reader.is_none() && !frame.readonly.facts[1].original_tail_validated
            );
            let mut held = AdditiveStorageLocalIntegrityHeld { frame };
            assert_eq!(held.first_error().to_string(), first);
            assert_eq!(
                held.frame
                    .readonly
                    .transform
                    .base
                    .target()
                    .unwrap()
                    .as_raw_fd(),
                fd
            );
            assert_eq!(held.frame.readonly.loan().unwrap().2.metadata_used(), used);
            assert_eq!(
                held.frame
                    .readonly
                    .loan()
                    .unwrap()
                    .0
                    .observation()
                    .metadata_work,
                original_used
            );
            assert!(!held.frame.advance_second());
            drop(held);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = LocalCompletionFrame::new(transformed(original).frame);
            assert!(frame.start(false));
            frame.compare_second().unwrap();
            // Actual typed pair completion is not a fixed integrity/FK return.
            let first = frame.close_and_tail().unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail }
                if detail == "additive integrity close before checks returned")
            );
            assert!(!frame.readonly.facts[1].close_attempted);
            frame.fail(first);
            assert!(!frame.integrity.started && !frame.readonly.facts[1].original_tail_validated);
            assert!(matches!(frame.readonly.cleanup_close, Some(Ok(()))));
            let first = frame.readonly.transform.first.as_ref().unwrap().to_string();
            let used = frame.readonly.loan().unwrap().2.metadata_used();
            assert!(!frame.advance_second());
            assert_eq!(frame.readonly.loan().unwrap().2.metadata_used(), used);
            assert_eq!(
                frame.readonly.transform.first.as_ref().unwrap().to_string(),
                first
            );
            drop(frame);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = LocalCompletionFrame::new(transformed(original).frame);
            assert!(frame.start(false));
            let work = frame.readonly.loan().unwrap().2;
            let remaining = 16 * MIB - work.metadata_used();
            assert!(work.metadata(remaining + 1).is_err());
            assert!(!frame.advance_second());
            assert!(!frame.integrity.started && frame.readonly.reader.is_none());
            let used = frame.readonly.loan().unwrap().2.metadata_used();
            assert!(!frame.advance_second());
            assert_eq!(frame.readonly.loan().unwrap().2.metadata_used(), used);
            drop(frame);
        });
    }
}

// A fixed ordinary fee read on the second retained reader, before its consuming
// close. No third pair, replay engine, Financial/native/payment issuer or pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FeeManifestPhase {
    Fresh,
    ReaderChecked,
    FieldsReturned,
    Validated,
    Complete,
    Refused,
}
#[derive(Default)]
struct RetainedFeeManifestFields {
    schema_id: Option<String>,
    policy_instance_id: Option<String>,
    descriptor_sha256: Option<String>,
    descriptor_bytes: Option<Vec<u8>>,
}
#[derive(Default)]
struct FeeManifestReadFacts {
    started: bool,
    count: Option<i64>,
    types_checked: bool,
    extents: [Option<u64>; 4],
    charged: bool,
    row: bool,
    eof: bool,
    scopes_ended: bool,
    returned: Option<bool>,
    validation_started: bool,
}
struct FeeManifestFrame {
    local: LocalCompletionFrame,
    phase: FeeManifestPhase,
    fields: RetainedFeeManifestFields,
    read: FeeManifestReadFacts,
    read_return: Option<StorageResult<()>>,
    validation:
        Option<std::result::Result<(), super::super::paper_book_v2_schema::StagedPaperBookV2Error>>,
}
pub(super) struct AdditiveStorageRetainedFeeManifest {
    frame: FeeManifestFrame,
}
pub(super) struct AdditiveStorageFeeManifestHeld {
    frame: FeeManifestFrame,
}
impl AdditiveStorageFeeManifestHeld {
    pub(super) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame.local.readonly.transform.first.as_ref().unwrap()
    }
}
impl AdditiveStorageRetainedFeeManifest {
    pub(super) fn policy_instance_id(&self) -> &str {
        self.frame.fields.policy_instance_id.as_deref().unwrap()
    }
    pub(super) fn create_or_resume(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageFeeManifestHeld> {
        // The same fixed cold5/6 whole source, with no writer or fresh lease.
        let base = AdditiveStorageCopied {
            source,
            managed: None,
            directory: None,
            anchor: None,
            fresh: false,
            original: None,
            rows: None,
            records: std::array::from_fn(|_| None),
            pending: None,
            target: None,
            target_node: None,
            copied: None,
            census_files: std::array::from_fn(|_| None),
            codec: AdditiveRecordCodecState::new(),
            copy_issued: false,
            rejected_copy_return: None,
            copy_return_failed: false,
            copy_return_error: None,
            copy_origin_return_error: None,
        };
        FeeManifestFrame::new(TransformFrame::new(base)).run(true)
    }
}
impl AdditiveStorageTransformed {
    pub(super) fn into_retained_fee_manifest(
        self,
    ) -> std::result::Result<AdditiveStorageRetainedFeeManifest, AdditiveStorageFeeManifestHeld>
    {
        FeeManifestFrame::new(self.frame).run(false)
    }
}
impl FeeManifestFrame {
    fn new(transform: TransformFrame) -> Self {
        Self {
            local: LocalCompletionFrame::new(transform),
            phase: FeeManifestPhase::Fresh,
            fields: RetainedFeeManifestFields::default(),
            read: FeeManifestReadFacts::default(),
            read_return: None,
            validation: None,
        }
    }
    fn fail(&mut self, first: GlobalSchemaV1Error) {
        self.local.fail(first);
        self.phase = FeeManifestPhase::Refused;
    }
    fn run(
        mut self,
        cold: bool,
    ) -> std::result::Result<AdditiveStorageRetainedFeeManifest, AdditiveStorageFeeManifestHeld>
    {
        if !self.start(cold) || !self.finish() {
            return Err(AdditiveStorageFeeManifestHeld { frame: self });
        }
        Ok(AdditiveStorageRetainedFeeManifest { frame: self })
    }
    fn start(&mut self, cold: bool) -> bool {
        if self.local.readonly.transform.first.is_some() {
            return false;
        }
        if self.phase != FeeManifestPhase::Fresh {
            self.fail(storage_fail("additive fee start phase differs"));
            return false;
        }
        if !self.local.start(cold) {
            self.phase = FeeManifestPhase::Refused;
            return false;
        }
        let result = self
            .local
            .compare_second()
            .and_then(|()| self.local.check_second());
        match result {
            Ok(()) => {
                self.phase = FeeManifestPhase::ReaderChecked;
                true
            }
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
    fn acquire_fields(&mut self) -> StorageResult<()> {
        if self.local.readonly.transform.first.is_some() {
            return Err(storage_fail("additive fee read after first error"));
        }
        if self.phase != FeeManifestPhase::ReaderChecked
            || self.local.readonly.active != Some(1)
            || self.local.phase != LocalCompletionPhase::ChecksComplete
            || !self.local.integrity.returned_ok
            || !self.local.integrity.scopes_ended
            || self.read_return.is_some()
        {
            return Err(storage_fail(
                "additive fee read lacks second retained reader",
            ));
        }
        let Retained8Frame {
            transform,
            permit,
            reader,
            ..
        } = &mut self.local.readonly;
        let (_, _, work, completed) = transform
            .base
            .source
            .retained8_parts(permit.as_ref().unwrap())?;
        if completed != 2 {
            return Err(storage_fail("additive fee read pair count differs"));
        }
        let loan = RetainedCatalog8BusinessReadLoan {
            connection: reader.as_ref().unwrap(),
            work,
            fields: &mut self.fields,
            facts: &mut self.read,
        };
        // Park the actual owned callee result before inspecting its return.
        self.read_return = Some(loan.read_fee_manifest());
        match self.read_return.as_ref().unwrap() {
            Ok(()) => {
                self.phase = FeeManifestPhase::FieldsReturned;
                Ok(())
            }
            Err(_) => Err(self.read_return.take().unwrap().unwrap_err()),
        }
    }
    fn validate_fields(&mut self) -> StorageResult<()> {
        if self.local.readonly.transform.first.is_some() {
            return Err(storage_fail("additive fee validation after first error"));
        }
        if self.phase != FeeManifestPhase::FieldsReturned
            || !matches!(self.read_return, Some(Ok(())))
            || self.read.returned != Some(true)
            || !self.read.eof
            || !self.read.scopes_ended
            || self.validation.is_some()
        {
            return Err(storage_fail(
                "additive fee validation before whole read returned",
            ));
        }
        // Actual returned copies already reside in this frame. The initial fixed
        // charge paid for both original validator outputs; no new limit/meter.
        self.local
            .readonly
            .loan()?
            .2
            .require_replay_clear()
            .map_err(GlobalSchemaV1Error::ReplayTerminal)?;
        self.read.validation_started = true;
        self.validation = Some(
            super::super::paper_book_v2_schema::verify_fee_manifest_fields(
                self.fields.schema_id.as_deref().unwrap(),
                self.fields.policy_instance_id.as_deref().unwrap(),
                self.fields.descriptor_sha256.as_deref().unwrap(),
                self.fields.descriptor_bytes.as_deref().unwrap(),
            ),
        );
        if self.validation.as_ref().unwrap().is_err() {
            // Retain the real shared validator Result and all acquired fields;
            // fixed ordinary wrapper text is covered by the initial reservation.
            return Err(storage_fail(
                "additive fee manifest common validator refused",
            ));
        }
        self.phase = FeeManifestPhase::Validated;
        Ok(())
    }
    fn close_and_tail(&mut self) -> StorageResult<()> {
        if self.local.readonly.transform.first.is_some() {
            return Err(storage_fail("additive fee close after first error"));
        }
        if self.phase != FeeManifestPhase::Validated
            || !matches!(self.validation, Some(Ok(())))
            || !matches!(self.read_return, Some(Ok(())))
            || self.read.returned != Some(true)
            || !self.read.eof
            || !self.read.scopes_ended
        {
            return Err(storage_fail(
                "additive fee close before actual read and validation returned",
            ));
        }
        self.local.close_and_tail()?;
        self.phase = FeeManifestPhase::Complete;
        Ok(())
    }
    fn finish(&mut self) -> bool {
        if self.local.readonly.transform.first.is_some() {
            return false;
        }
        let result = self
            .acquire_fields()
            .and_then(|()| self.validate_fields())
            .and_then(|()| self.close_and_tail());
        match result {
            Ok(()) => true,
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
}
// Only A constructs this short loan from its second actual reader, current
// permit and unique work. There is no path/Connection getter or callback.
struct RetainedCatalog8BusinessReadLoan<'a> {
    connection: &'a Connection,
    work: &'a mut target::TargetWork,
    fields: &'a mut RetainedFeeManifestFields,
    facts: &'a mut FeeManifestReadFacts,
}
impl RetainedCatalog8BusinessReadLoan<'_> {
    fn read_fee_manifest(self) -> StorageResult<()> {
        if self.facts.started {
            return Err(storage_fail("additive fee read already started"));
        }
        const COUNT: &str = "SELECT COUNT(*) FROM main.paper_book_v2_fee_manifest";
        const TYPES: &str = "SELECT COUNT(*) FROM main.paper_book_v2_fee_manifest WHERE typeof(singleton)!='integer' OR singleton!=1 OR typeof(schema_id)!='text' OR typeof(policy_instance_id)!='text' OR typeof(descriptor_sha256)!='text' OR typeof(descriptor_bytes)!='blob'";
        const EXTENTS: &str = "SELECT length(CAST(schema_id AS BLOB)),length(CAST(policy_instance_id AS BLOB)),length(CAST(descriptor_sha256 AS BLOB)),length(descriptor_bytes) FROM main.paper_book_v2_fee_manifest ORDER BY singleton";
        const FIELDS: &str = "SELECT schema_id,policy_instance_id,descriptor_sha256,descriptor_bytes FROM main.paper_book_v2_fee_manifest ORDER BY singleton";
        self.work
            .metadata(4096 + (COUNT.len() + TYPES.len() + EXTENTS.len() + FIELDS.len()) as u64)?;
        self.facts.started = true;
        let result = (|| {
            let count: i64 = self
                .connection
                .query_row(COUNT, [], |r| r.get(0))
                .map_err(|e| transform_sql_error("count additive fee manifest", e))?;
            self.facts.count = Some(count);
            if count != 1 {
                return Err(storage_fail(
                    "additive fee manifest requires exactly one row",
                ));
            }
            let invalid: i64 = self
                .connection
                .query_row(TYPES, [], |r| r.get(0))
                .map_err(|e| transform_sql_error("type additive fee manifest", e))?;
            if invalid != 0 {
                return Err(storage_fail("additive fee manifest types differ"));
            }
            self.facts.types_checked = true;
            let lengths = self
                .connection
                .query_row(EXTENTS, [], |r| {
                    Ok([
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ])
                })
                .map_err(|e| transform_sql_error("extent additive fee manifest", e))?;
            let mut sizes = [0usize; 4];
            let mut total = 0u64;
            for (i, length) in lengths.into_iter().enumerate() {
                let length = u64::try_from(length)
                    .map_err(|_| storage_fail("additive fee manifest negative extent"))?;
                self.facts.extents[i] = Some(length);
                sizes[i] = usize::try_from(length)
                    .map_err(|_| storage_fail("additive fee manifest extent overflow"))?;
                total = total
                    .checked_add(length)
                    .ok_or_else(|| storage_fail("additive fee manifest extent overflow"))?;
            }
            if sizes[3] == 0 {
                return Err(storage_fail("additive fee manifest descriptor empty"));
            }
            let validator_bytes =
                super::super::paper_book_v2_schema::fee_manifest_validation_owned_bytes() as u64;
            self.work.metadata(
                total
                    .checked_add(validator_bytes)
                    .ok_or_else(|| storage_fail("additive fee manifest extent overflow"))?,
            )?;
            self.facts.charged = true;
            let mut statement = self
                .connection
                .prepare(FIELDS)
                .map_err(|e| transform_sql_error("prepare additive fee manifest", e))?;
            let mut rows = statement
                .query([])
                .map_err(|e| transform_sql_error("query additive fee manifest", e))?;
            let row = rows
                .next()
                .map_err(|e| transform_sql_error("step additive fee manifest", e))?
                .ok_or_else(|| storage_fail("additive fee manifest row disappeared"))?;
            self.facts.row = true;
            // Each actual allocation moves immediately into the whole frame
            // before any subsequent field/EOF failure. No collection/row clone.
            self.fields.schema_id = Some(fee_manifest_text(row, 0, sizes[0])?);
            self.fields.policy_instance_id = Some(fee_manifest_text(row, 1, sizes[1])?);
            self.fields.descriptor_sha256 = Some(fee_manifest_text(row, 2, sizes[2])?);
            let bytes = match row
                .get_ref(3)
                .map_err(|e| transform_sql_error("read additive fee descriptor", e))?
            {
                rusqlite::types::ValueRef::Blob(bytes) if bytes.len() == sizes[3] => bytes,
                _ => return Err(storage_fail("additive fee descriptor extent/type changed")),
            };
            let mut descriptor = Vec::new();
            descriptor
                .try_reserve_exact(bytes.len())
                .map_err(|_| storage_fail("additive fee descriptor allocation failed"))?;
            descriptor.extend_from_slice(bytes);
            self.fields.descriptor_bytes = Some(descriptor);
            if rows
                .next()
                .map_err(|e| transform_sql_error("finish additive fee manifest", e))?
                .is_some()
            {
                return Err(storage_fail("additive fee manifest extra row"));
            }
            self.facts.eof = true;
            Ok(())
        })();
        // Real query_row scopes and fixed Rows/Statement end before this actual
        // return. Their ignored driver Drop results are not manufactured facts.
        self.facts.scopes_ended = true;
        self.facts.returned = Some(result.is_ok());
        result
    }
}
fn fee_manifest_text(
    row: &rusqlite::Row<'_>,
    index: usize,
    expected: usize,
) -> StorageResult<String> {
    let bytes = match row
        .get_ref(index)
        .map_err(|e| transform_sql_error("read additive fee text", e))?
    {
        rusqlite::types::ValueRef::Text(bytes) if bytes.len() == expected => bytes,
        _ => return Err(storage_fail("additive fee text extent/type changed")),
    };
    let text =
        std::str::from_utf8(bytes).map_err(|_| storage_fail("additive fee text is not UTF8"))?;
    let mut value = String::new();
    value
        .try_reserve_exact(bytes.len())
        .map_err(|_| storage_fail("additive fee text allocation failed"))?;
    value.push_str(text);
    Ok(value)
}

#[cfg(test)]
mod retained_fee_manifest_tests {
    use super::*;
    fn transformed(
        original: rows::VerifiedUnapprovedOriginalRowsBackup,
    ) -> AdditiveStorageTransformed {
        let copied =
            match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
                Ok(owner) => owner,
                Err(held) => panic!("fee real Copied: {}", held.first_error()),
            };
        match copied.into_transformed() {
            Ok(owner) => owner,
            Err(held) => panic!("fee real WAL: {}", held.first_error()),
        }
    }
    fn assert_complete(owner: &mut AdditiveStorageRetainedFeeManifest) {
        let f = &mut owner.frame;
        assert_eq!(f.phase, FeeManifestPhase::Complete);
        assert_eq!(f.read.count, Some(1));
        assert!(f.read.types_checked && f.read.charged && f.read.row && f.read.eof);
        assert!(f.read.scopes_ended && f.read.returned == Some(true) && f.read.validation_started);
        assert!(matches!(f.validation, Some(Ok(()))) && matches!(f.read_return, Some(Ok(()))));
        assert!(f.local.readonly.reader.is_none() && f.local.readonly.active.is_none());
        assert!(f
            .local
            .readonly
            .facts
            .iter()
            .all(|r| r.closed && r.original_tail_validated));
        assert_eq!(f.local.readonly.loan().unwrap().3, 2);
        let policy =
            crate::performance::fee_policy::AShareFeePolicyV2::fixed_compatibility_assumption();
        assert_eq!(
            f.fields.descriptor_bytes.as_deref().unwrap(),
            policy.canonical_bytes().as_slice()
        );
        assert_eq!(
            f.fields.descriptor_sha256.as_deref().unwrap(),
            policy.descriptor_hash()
        );
        assert_eq!(owner.policy_instance_id(), policy.instance_id());
    }
    #[test]
    fn task6_retained_fee_manifest_nonempty_same_frame_and_cold() {
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = transformed(original);
                let fd = owner.frame.base.target().unwrap().as_raw_fd();
                let mut read = match owner.into_retained_fee_manifest() {
                    Ok(owner) => owner,
                    Err(held) => panic!("fee real read: {}", held.first_error()),
                };
                assert_complete(&mut read);
                assert_eq!(
                    read.frame
                        .local
                        .readonly
                        .transform
                        .base
                        .target()
                        .unwrap()
                        .as_raw_fd(),
                    fd
                );
                let saved = (
                    read.frame
                        .local
                        .readonly
                        .transform
                        .base
                        .target_node
                        .unwrap(),
                    read.frame
                        .local
                        .readonly
                        .transform
                        .base
                        .records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                    read.policy_instance_id().to_owned(),
                );
                drop(read);
                saved
            },
            |(node, records, policy), original| {
                let mut read = match AdditiveStorageRetainedFeeManifest::create_or_resume(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("fee real cold read: {}", held.first_error()),
                };
                assert_complete(&mut read);
                assert_eq!(
                    read.frame.local.readonly.transform.base.target_node,
                    Some(node)
                );
                assert_eq!(
                    read.frame
                        .local
                        .readonly
                        .transform
                        .base
                        .records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                    records
                );
                assert_eq!(read.policy_instance_id(), policy);
                drop(read);
            },
        );
    }
    #[test]
    fn task6_retained_fee_manifest_typed_extent_and_common_validator_refusals() {
        // These real SQL fixtures reach the fixed data gate without pretending
        // to possess a complete retained owner or bypassing its drift checks.
        for case in ["type", "descriptor", "id", "schema", "missing", "extra"] {
            super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
                let mut source = original.into_additive_target_source().unwrap();
                let connection = Connection::open_in_memory().unwrap();
                connection.execute_batch("CREATE TABLE paper_book_v2_fee_manifest(singleton,schema_id,policy_instance_id,descriptor_sha256,descriptor_bytes)").unwrap();
                let policy = crate::performance::fee_policy::AShareFeePolicyV2::fixed_compatibility_assumption();
                if case != "missing" {
                    connection
                        .execute(
                            "INSERT INTO paper_book_v2_fee_manifest VALUES(1,?1,?2,?3,?4)",
                            rusqlite::params![
                                "paper-book-v2-fee-manifest/v1",
                                policy.instance_id(),
                                policy.descriptor_hash(),
                                policy.canonical_bytes()
                            ],
                        )
                        .unwrap();
                }
                match case {
                    "type" => {
                        connection.execute("UPDATE paper_book_v2_fee_manifest SET descriptor_bytes='wrong type'", []).unwrap();
                    }
                    "descriptor" => {
                        connection
                            .execute(
                                "UPDATE paper_book_v2_fee_manifest SET descriptor_bytes=X'01'",
                                [],
                            )
                            .unwrap();
                    }
                    "id" => {
                        connection.execute("UPDATE paper_book_v2_fee_manifest SET policy_instance_id='wrong-instance'", []).unwrap();
                    }
                    "schema" => {
                        connection
                            .execute(
                                "UPDATE paper_book_v2_fee_manifest SET schema_id='wrong-schema'",
                                [],
                            )
                            .unwrap();
                    }
                    "extra" => {
                        connection.execute("INSERT INTO paper_book_v2_fee_manifest SELECT 2,schema_id,policy_instance_id,descriptor_sha256,descriptor_bytes FROM paper_book_v2_fee_manifest", []).unwrap();
                    }
                    _ => (),
                }
                let mut fields = RetainedFeeManifestFields::default();
                let mut facts = FeeManifestReadFacts::default();
                let work = source.storage_parts().unwrap().2;
                let before = work.metadata_used();
                let actual = RetainedCatalog8BusinessReadLoan {
                    connection: &connection,
                    work: &mut *work,
                    fields: &mut fields,
                    facts: &mut facts,
                }
                .read_fee_manifest();
                assert!(work.metadata_used() > before && facts.started && facts.scopes_ended);
                if matches!(case, "descriptor" | "id" | "schema") {
                    actual.unwrap();
                    assert!(
                        facts.eof
                            && facts.returned == Some(true)
                            && fields.descriptor_bytes.is_some()
                    );
                    let refusal =
                        super::super::super::paper_book_v2_schema::verify_fee_manifest_fields(
                            fields.schema_id.as_deref().unwrap(),
                            fields.policy_instance_id.as_deref().unwrap(),
                            fields.descriptor_sha256.as_deref().unwrap(),
                            fields.descriptor_bytes.as_deref().unwrap(),
                        );
                    assert!(matches!(refusal, Err(super::super::super::paper_book_v2_schema::StagedPaperBookV2Error::ManifestMismatch)));
                } else {
                    let error = actual.unwrap_err();
                    let expected = if case == "type" {
                        "additive fee manifest types differ"
                    } else {
                        "additive fee manifest requires exactly one row"
                    };
                    assert!(
                        matches!(error, GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == expected)
                    );
                    assert!(
                        !facts.charged
                            && !facts.row
                            && !facts.eof
                            && fields.descriptor_bytes.is_none()
                    );
                }
                connection.close().unwrap();
                drop(source);
            });
        }
    }
    #[test]
    fn task6_retained_fee_manifest_same_work_late_busy_and_route_hold() {
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = FeeManifestFrame::new(transformed(original).frame);
            assert!(frame.start(false));
            let work = frame.local.readonly.loan().unwrap().2;
            let remaining = 16 * MIB - work.metadata_used();
            assert!(work.metadata(remaining).is_ok());
            assert!(!frame.finish());
            assert!(!frame.read.started && frame.fields.schema_id.is_none());
            let first = frame
                .local
                .readonly
                .transform
                .first
                .as_ref()
                .unwrap()
                .to_string();
            let used = frame.local.readonly.loan().unwrap().2.metadata_used();
            assert!(!frame.finish());
            assert_eq!(frame.local.readonly.loan().unwrap().2.metadata_used(), used);
            assert_eq!(
                frame
                    .local
                    .readonly
                    .transform
                    .first
                    .as_ref()
                    .unwrap()
                    .to_string(),
                first
            );
            drop(frame);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = FeeManifestFrame::new(transformed(original).frame);
            assert!(frame.start(false));
            frame.acquire_fields().unwrap();
            assert!(frame.fields.descriptor_bytes.is_some() && frame.validation.is_none());
            let first = frame.close_and_tail().unwrap_err(); // Actual row/EOF is not a validator return.
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail }
                if detail == "additive fee close before actual read and validation returned")
            );
            frame.fail(first);
            assert!(!frame.read.validation_started && frame.fields.descriptor_bytes.is_some());
            assert!(!frame.finish());
            assert!(!frame.local.readonly.facts[1].original_tail_validated);
            drop(frame);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = FeeManifestFrame::new(transformed(original).frame);
            assert!(frame.start(false));
            frame.acquire_fields().unwrap();
            frame.validate_fields().unwrap();
            frame.local.readonly.prepare_busy_vm();
            let first = frame.close_and_tail().unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSqlite { operation: "close additive readonly", source }
                if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy))
            );
            frame.fail(first);
            assert!(
                frame.local.readonly.reader.is_some() && frame.fields.descriptor_bytes.is_some()
            );
            assert!(
                matches!(frame.validation, Some(Ok(())))
                    && !frame.local.readonly.facts[1].original_tail_validated
            );
            let used = frame.local.readonly.loan().unwrap().2.metadata_used();
            assert!(!frame.finish());
            assert_eq!(frame.local.readonly.loan().unwrap().2.metadata_used(), used);
            assert!(frame.local.readonly.finalize_busy_once());
            frame.local.readonly.cleanup_reader_once();
            assert!(matches!(frame.local.readonly.cleanup_close, Some(Ok(()))));
            assert!(
                frame.fields.descriptor_bytes.is_some()
                    && !frame.local.readonly.facts[1].original_tail_validated
            );
            drop(frame);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = FeeManifestFrame::new(transformed(original).frame);
            assert!(frame.start(false));
            let first = frame.close_and_tail().unwrap_err();
            frame.fail(first); // Unknown whole read never permits close success.
            assert!(
                !frame.read.started && frame.read.returned.is_none() && frame.validation.is_none()
            );
            assert!(!frame.finish());
            assert!(!frame.local.readonly.facts[1].original_tail_validated);
            drop(frame);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = FeeManifestFrame::new(transformed(original).frame);
            assert!(frame.local.start(false));
            let file = frame.local.readonly.transform.base.target().unwrap();
            let mut original_byte = [0];
            file.read_exact_at(&mut original_byte, 100).unwrap();
            file.write_all_at(&[original_byte[0] ^ 1], 100).unwrap();
            file.sync_all().unwrap();
            let actual = frame.local.compare_second();
            frame
                .local
                .readonly
                .transform
                .base
                .target()
                .unwrap()
                .write_all_at(&original_byte, 100)
                .unwrap();
            frame
                .local
                .readonly
                .transform
                .base
                .target()
                .unwrap()
                .sync_all()
                .unwrap(); // Cleanup only; first remains Held.
            let first = actual.unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail }
                if detail == "additive readonly retained target bytes changed")
            );
            frame.fail(first);
            assert!(!frame.read.started && frame.local.readonly.routes[1].is_none());
            assert!(!frame.finish());
            drop(frame);
        });
    }
}

// Ordinary fixed owner/account linkage. Genesis/event bodies and Financial
// replay are deliberately not checked or qualified by this retained frame.
#[derive(Default)]
struct OwnerLinkageRow {
    account_id: Option<String>,
    epoch_id: Option<String>,
    manifest_hash: Option<String>,
    active_generation: Option<i64>,
    active_epoch_id: Option<String>,
    active_manifest_hash: Option<String>,
    owner_revision: Option<i64>,
    cutover_id: Option<Option<String>>,
    manifest_bytes: Option<Vec<u8>>,
    fee_policy_instance_id: Option<String>,
    v1_epoch_id: Option<String>,
    v1_manifest_hash: Option<String>,
    v1_head_version: Option<i64>,
    v1_head_hash: Option<String>,
    v1_projection_hash: Option<String>,
    account_cutover_id: Option<String>,
}
#[derive(Default)]
struct OwnerLinkageReadFacts {
    started: bool,
    count: Option<i64>,
    types_checked: bool,
    extent: Option<u64>,
    charged: bool,
    acquired: usize,
    eof: bool,
    scopes_ended: bool,
    returned: Option<bool>,
}
struct OwnerLinkageFields {
    rows: [Vec<OwnerLinkageRow>; 5],
}
impl Default for OwnerLinkageFields {
    fn default() -> Self {
        Self {
            rows: std::array::from_fn(|_| Vec::new()),
        }
    }
}
#[derive(Clone, Copy)]
enum OwnerLinkageQuery {
    OldAccounts,
    Owners,
    Accounts,
    Events,
    Heads,
}
impl OwnerLinkageQuery {
    const ALL: [Self; 5] = [
        Self::OldAccounts,
        Self::Owners,
        Self::Accounts,
        Self::Events,
        Self::Heads,
    ];
    fn slot(self) -> usize {
        match self {
            Self::OldAccounts => 0,
            Self::Owners => 1,
            Self::Accounts => 2,
            Self::Events => 3,
            Self::Heads => 4,
        }
    }
    fn sql(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::OldAccounts => (
                "SELECT COUNT(*) FROM main.paper_ledger_account",
                "SELECT COUNT(*) FROM main.paper_ledger_account WHERE typeof(account_id)!='text' OR typeof(epoch_id)!='text' OR typeof(manifest_hash)!='text'",
                "SELECT coalesce(sum(length(CAST(account_id AS BLOB))+length(CAST(epoch_id AS BLOB))+length(CAST(manifest_hash AS BLOB))),0) FROM main.paper_ledger_account",
                "SELECT account_id,epoch_id,manifest_hash FROM main.paper_ledger_account ORDER BY account_id",
            ),
            Self::Owners => (
                "SELECT COUNT(*) FROM main.paper_book_owner_v2",
                "SELECT COUNT(*) FROM main.paper_book_owner_v2 WHERE typeof(account_id)!='text' OR typeof(active_generation)!='integer' OR typeof(active_epoch_id)!='text' OR typeof(active_manifest_hash)!='text' OR typeof(owner_revision)!='integer' OR typeof(cutover_id) NOT IN ('null','text')",
                "SELECT coalesce(sum(length(CAST(account_id AS BLOB))+length(CAST(active_epoch_id AS BLOB))+length(CAST(active_manifest_hash AS BLOB))+coalesce(length(CAST(cutover_id AS BLOB)),0)),0) FROM main.paper_book_owner_v2",
                "SELECT account_id,active_generation,active_epoch_id,active_manifest_hash,owner_revision,cutover_id FROM main.paper_book_owner_v2 ORDER BY account_id",
            ),
            Self::Accounts => (
                "SELECT COUNT(*) FROM main.paper_book_v2_account",
                "SELECT COUNT(*) FROM main.paper_book_v2_account WHERE typeof(account_id)!='text' OR typeof(epoch_id)!='text' OR typeof(manifest_hash)!='text' OR typeof(manifest_bytes)!='blob' OR typeof(fee_policy_instance_id)!='text' OR typeof(v1_epoch_id)!='text' OR typeof(v1_manifest_hash)!='text' OR typeof(v1_head_version)!='integer' OR typeof(v1_head_hash)!='text' OR typeof(v1_projection_hash)!='text' OR typeof(cutover_id)!='text'",
                "SELECT coalesce(sum(length(CAST(account_id AS BLOB))+length(CAST(epoch_id AS BLOB))+length(CAST(manifest_hash AS BLOB))+length(manifest_bytes)+length(CAST(fee_policy_instance_id AS BLOB))+length(CAST(v1_epoch_id AS BLOB))+length(CAST(v1_manifest_hash AS BLOB))+length(CAST(v1_head_hash AS BLOB))+length(CAST(v1_projection_hash AS BLOB))+length(CAST(cutover_id AS BLOB))),0) FROM main.paper_book_v2_account",
                "SELECT account_id,epoch_id,manifest_hash,manifest_bytes,fee_policy_instance_id,v1_epoch_id,v1_manifest_hash,v1_head_version,v1_head_hash,v1_projection_hash,cutover_id FROM main.paper_book_v2_account ORDER BY account_id",
            ),
            Self::Events => (
                "SELECT COUNT(*) FROM main.paper_book_v2_event",
                "SELECT COUNT(*) FROM main.paper_book_v2_event WHERE typeof(account_id)!='text'",
                "SELECT coalesce(sum(length(CAST(account_id AS BLOB))),0) FROM main.paper_book_v2_event",
                "SELECT account_id FROM main.paper_book_v2_event ORDER BY account_id",
            ),
            Self::Heads => (
                "SELECT COUNT(*) FROM main.paper_book_v2_head",
                "SELECT COUNT(*) FROM main.paper_book_v2_head WHERE typeof(account_id)!='text'",
                "SELECT coalesce(sum(length(CAST(account_id AS BLOB))),0) FROM main.paper_book_v2_head",
                "SELECT account_id FROM main.paper_book_v2_head ORDER BY account_id",
            ),
        }
    }
}
#[derive(PartialEq, Eq)]
enum OwnerLinkagePhase {
    Fresh,
    ReaderChecked,
    RowsReturned,
    RelationsChecked,
    Complete,
    Refused,
}
struct OwnerLinkageFrame {
    fee: FeeManifestFrame,
    phase: OwnerLinkagePhase,
    fields: OwnerLinkageFields,
    reads: [OwnerLinkageReadFacts; 5],
    returns: [Option<StorageResult<()>>; 5],
    relations_returned: Option<bool>,
}
pub(super) struct AdditiveStorageRetainedOwnerLinkage {
    frame: OwnerLinkageFrame,
}
pub(super) struct AdditiveStorageOwnerLinkageHeld {
    frame: OwnerLinkageFrame,
}
impl AdditiveStorageOwnerLinkageHeld {
    pub(super) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame
            .fee
            .local
            .readonly
            .transform
            .first
            .as_ref()
            .unwrap()
    }
}
impl AdditiveStorageRetainedOwnerLinkage {
    // Ordinary borrowed data only, never a full owner/financial capability.
    pub(super) fn active_accounts(&self) -> impl Iterator<Item = (&str, i64, &str, &str)> {
        self.frame.fields.rows[1].iter().map(|r| {
            (
                r.account_id.as_deref().unwrap(),
                r.active_generation.unwrap(),
                r.active_epoch_id.as_deref().unwrap(),
                r.active_manifest_hash.as_deref().unwrap(),
            )
        })
    }
    pub(super) fn create_or_resume(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageOwnerLinkageHeld> {
        let base = AdditiveStorageCopied {
            source,
            managed: None,
            directory: None,
            anchor: None,
            fresh: false,
            original: None,
            rows: None,
            records: std::array::from_fn(|_| None),
            pending: None,
            target: None,
            target_node: None,
            copied: None,
            census_files: std::array::from_fn(|_| None),
            codec: AdditiveRecordCodecState::new(),
            copy_issued: false,
            rejected_copy_return: None,
            copy_return_failed: false,
            copy_return_error: None,
            copy_origin_return_error: None,
        };
        OwnerLinkageFrame::new(TransformFrame::new(base)).run(true)
    }
}
impl AdditiveStorageTransformed {
    pub(super) fn into_retained_owner_linkage(
        self,
    ) -> std::result::Result<AdditiveStorageRetainedOwnerLinkage, AdditiveStorageOwnerLinkageHeld>
    {
        OwnerLinkageFrame::new(self.frame).run(false)
    }
}
impl OwnerLinkageFrame {
    fn new(transform: TransformFrame) -> Self {
        Self {
            fee: FeeManifestFrame::new(transform),
            phase: OwnerLinkagePhase::Fresh,
            fields: OwnerLinkageFields::default(),
            reads: std::array::from_fn(|_| OwnerLinkageReadFacts::default()),
            returns: std::array::from_fn(|_| None),
            relations_returned: None,
        }
    }
    fn fail(&mut self, first: GlobalSchemaV1Error) {
        self.fee.fail(first);
        self.phase = OwnerLinkagePhase::Refused;
    }
    fn run(
        mut self,
        cold: bool,
    ) -> std::result::Result<AdditiveStorageRetainedOwnerLinkage, AdditiveStorageOwnerLinkageHeld>
    {
        if !self.start(cold) || !self.finish() {
            return Err(AdditiveStorageOwnerLinkageHeld { frame: self });
        }
        Ok(AdditiveStorageRetainedOwnerLinkage { frame: self })
    }
    fn start(&mut self, cold: bool) -> bool {
        if self.fee.local.readonly.transform.first.is_some() {
            return false;
        }
        if self.phase != OwnerLinkagePhase::Fresh {
            self.fail(storage_fail("additive owner start phase differs"));
            return false;
        }
        if !self.fee.start(cold) {
            self.phase = OwnerLinkagePhase::Refused;
            return false;
        }
        let result = self
            .fee
            .acquire_fields()
            .and_then(|()| self.fee.validate_fields());
        match result {
            Ok(()) => {
                self.phase = OwnerLinkagePhase::ReaderChecked;
                true
            }
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
    fn read_fixed(&mut self, query: OwnerLinkageQuery) -> StorageResult<()> {
        if self.phase != OwnerLinkagePhase::ReaderChecked
            || self.fee.phase != FeeManifestPhase::Validated
            || self.fee.local.readonly.active != Some(1)
            || !matches!(self.fee.validation, Some(Ok(())))
        {
            return Err(storage_fail("additive owner lacks validated second reader"));
        }
        let Retained8Frame {
            transform,
            permit,
            reader,
            ..
        } = &mut self.fee.local.readonly;
        let (_, _, work, completed) = transform
            .base
            .source
            .retained8_parts(permit.as_ref().unwrap())?;
        if completed != 2 {
            return Err(storage_fail("additive owner pair count differs"));
        }
        OwnerLinkageReadLoan {
            connection: reader.as_ref().unwrap(),
            work,
            rows: &mut self.fields.rows[query.slot()],
            facts: &mut self.reads[query.slot()],
        }
        .read(query)
    }
    fn advance_reads(&mut self) -> bool {
        if self.fee.local.readonly.transform.first.is_some() {
            return false;
        }
        if self.phase != OwnerLinkagePhase::ReaderChecked {
            self.fail(storage_fail("additive owner read phase differs"));
            return false;
        }
        for query in OwnerLinkageQuery::ALL {
            let i = query.slot();
            if self.returns[i].is_some() || self.reads[i].started {
                self.fail(storage_fail("additive owner query already reached"));
                return false;
            }
            // The actual owned result lands before first-error inspection. Every
            // field acquired by the callee already resides in this same frame.
            let actual = self.read_fixed(query);
            self.returns[i] = Some(actual);
            if self.returns[i].as_ref().unwrap().is_err() {
                let first = self.returns[i].take().unwrap().unwrap_err();
                self.fail(first);
                return false;
            }
        }
        self.phase = OwnerLinkagePhase::RowsReturned;
        true
    }
    fn all_returns(&self) -> bool {
        self.returns.iter().all(|r| matches!(r, Some(Ok(()))))
            && self
                .reads
                .iter()
                .all(|r| r.returned == Some(true) && r.eof && r.scopes_ended)
    }
    fn validate_fields(&mut self) -> StorageResult<()> {
        if self.phase != OwnerLinkagePhase::RowsReturned
            || !self.all_returns()
            || self.relations_returned.is_some()
        {
            return Err(storage_fail(
                "additive owner relations before whole returns",
            ));
        }
        self.fee.local.readonly.loan()?.2.metadata(1024)?;
        let actual = owner_linkage_relations(&self.fields);
        self.relations_returned = Some(actual.is_ok());
        actual?;
        self.phase = OwnerLinkagePhase::RelationsChecked;
        Ok(())
    }
    fn close_and_tail(&mut self) -> StorageResult<()> {
        if self.phase != OwnerLinkagePhase::RelationsChecked
            || self.relations_returned != Some(true)
            || !self.all_returns()
        {
            return Err(storage_fail(
                "additive owner close before relations returned",
            ));
        }
        self.fee.close_and_tail()?;
        self.phase = OwnerLinkagePhase::Complete;
        Ok(())
    }
    fn finish(&mut self) -> bool {
        if self.fee.local.readonly.transform.first.is_some() {
            return false;
        }
        if !self.advance_reads() {
            return false;
        }
        let actual = self.validate_fields().and_then(|()| self.close_and_tail());
        match actual {
            Ok(()) => true,
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
}
// Only the fixed frame lends its existing actual second Connection and work.
struct OwnerLinkageReadLoan<'a> {
    connection: &'a Connection,
    work: &'a mut target::TargetWork,
    rows: &'a mut Vec<OwnerLinkageRow>,
    facts: &'a mut OwnerLinkageReadFacts,
}
impl OwnerLinkageReadLoan<'_> {
    fn read(self, query: OwnerLinkageQuery) -> StorageResult<()> {
        if self.facts.started || !self.rows.is_empty() {
            return Err(storage_fail("additive owner query already started"));
        }
        let (count_sql, type_sql, extent_sql, fields_sql) = query.sql();
        self.work.metadata(
            4096 + (count_sql.len() + type_sql.len() + extent_sql.len() + fields_sql.len()) as u64,
        )?;
        self.facts.started = true;
        let actual = (|| {
            let count: i64 = self
                .connection
                .query_row(count_sql, [], |r| r.get(0))
                .map_err(|e| transform_sql_error("count additive owner rows", e))?;
            self.facts.count = Some(count);
            let count = usize::try_from(count)
                .map_err(|_| storage_fail("additive owner count overflow"))?;
            let invalid: i64 = self
                .connection
                .query_row(type_sql, [], |r| r.get(0))
                .map_err(|e| transform_sql_error("type additive owner rows", e))?;
            if invalid != 0 {
                return Err(storage_fail("additive owner row types differ"));
            }
            self.facts.types_checked = true;
            let extent: i64 = self
                .connection
                .query_row(extent_sql, [], |r| r.get(0))
                .map_err(|e| transform_sql_error("extent additive owner rows", e))?;
            let mut remaining = u64::try_from(extent)
                .map_err(|_| storage_fail("additive owner extent overflow"))?;
            self.facts.extent = Some(remaining);
            let slots = (count as u64)
                .checked_mul(std::mem::size_of::<OwnerLinkageRow>() as u64)
                .ok_or_else(|| storage_fail("additive owner capacity overflow"))?;
            self.work.metadata(
                slots
                    .checked_add(remaining)
                    .ok_or_else(|| storage_fail("additive owner capacity overflow"))?,
            )?;
            self.facts.charged = true;
            self.rows
                .try_reserve_exact(count)
                .map_err(|_| storage_fail("additive owner row allocation failed"))?;
            let mut statement = self
                .connection
                .prepare(fields_sql)
                .map_err(|e| transform_sql_error("prepare additive owner rows", e))?;
            let mut rows = statement
                .query([])
                .map_err(|e| transform_sql_error("query additive owner rows", e))?;
            while let Some(row) = rows
                .next()
                .map_err(|e| transform_sql_error("step additive owner rows", e))?
            {
                if self.rows.len() == count {
                    return Err(storage_fail("additive owner extra row"));
                }
                self.rows.push(OwnerLinkageRow::default());
                self.facts.acquired = self.rows.len();
                let slot = self.rows.last_mut().unwrap();
                match query {
                    OwnerLinkageQuery::OldAccounts => {
                        slot.account_id = Some(owner_linkage_text(row, 0, &mut remaining)?);
                        slot.epoch_id = Some(owner_linkage_text(row, 1, &mut remaining)?);
                        slot.manifest_hash = Some(owner_linkage_text(row, 2, &mut remaining)?);
                    }
                    OwnerLinkageQuery::Owners => {
                        slot.account_id = Some(owner_linkage_text(row, 0, &mut remaining)?);
                        slot.active_generation = Some(owner_linkage_integer(row, 1)?);
                        slot.active_epoch_id = Some(owner_linkage_text(row, 2, &mut remaining)?);
                        slot.active_manifest_hash =
                            Some(owner_linkage_text(row, 3, &mut remaining)?);
                        slot.owner_revision = Some(owner_linkage_integer(row, 4)?);
                        slot.cutover_id =
                            Some(owner_linkage_nullable_text(row, 5, &mut remaining)?);
                    }
                    OwnerLinkageQuery::Accounts => {
                        slot.account_id = Some(owner_linkage_text(row, 0, &mut remaining)?);
                        slot.epoch_id = Some(owner_linkage_text(row, 1, &mut remaining)?);
                        slot.manifest_hash = Some(owner_linkage_text(row, 2, &mut remaining)?);
                        slot.manifest_bytes = Some(owner_linkage_blob(row, 3, &mut remaining)?);
                        slot.fee_policy_instance_id =
                            Some(owner_linkage_text(row, 4, &mut remaining)?);
                        slot.v1_epoch_id = Some(owner_linkage_text(row, 5, &mut remaining)?);
                        slot.v1_manifest_hash = Some(owner_linkage_text(row, 6, &mut remaining)?);
                        slot.v1_head_version = Some(owner_linkage_integer(row, 7)?);
                        slot.v1_head_hash = Some(owner_linkage_text(row, 8, &mut remaining)?);
                        slot.v1_projection_hash = Some(owner_linkage_text(row, 9, &mut remaining)?);
                        slot.account_cutover_id =
                            Some(owner_linkage_text(row, 10, &mut remaining)?);
                    }
                    OwnerLinkageQuery::Events => {
                        slot.account_id = Some(owner_linkage_text(row, 0, &mut remaining)?);
                    }
                    OwnerLinkageQuery::Heads => {
                        slot.account_id = Some(owner_linkage_text(row, 0, &mut remaining)?);
                    }
                }
            }
            if self.rows.len() != count || remaining != 0 {
                return Err(storage_fail("additive owner count/extent changed"));
            }
            self.facts.eof = true;
            Ok(())
        })();
        // Actual query_row and Rows/Statement lexical scopes end on both paths.
        // Ignored driver Drop results are not rewritten as successful facts.
        self.facts.scopes_ended = true;
        self.facts.returned = Some(actual.is_ok());
        actual
    }
}
fn owner_linkage_claim(length: usize, remaining: &mut u64) -> StorageResult<()> {
    *remaining = remaining
        .checked_sub(length as u64)
        .ok_or_else(|| storage_fail("additive owner field extent changed"))?;
    Ok(())
}
fn owner_linkage_text(
    row: &rusqlite::Row<'_>,
    index: usize,
    remaining: &mut u64,
) -> StorageResult<String> {
    let bytes = match row
        .get_ref(index)
        .map_err(|e| transform_sql_error("read additive owner text", e))?
    {
        rusqlite::types::ValueRef::Text(b) => b,
        _ => return Err(storage_fail("additive owner field type changed")),
    };
    owner_linkage_claim(bytes.len(), remaining)?;
    let value =
        std::str::from_utf8(bytes).map_err(|_| storage_fail("additive owner text is not UTF8"))?;
    let mut owned = String::new();
    owned
        .try_reserve_exact(bytes.len())
        .map_err(|_| storage_fail("additive owner text allocation failed"))?;
    owned.push_str(value);
    Ok(owned)
}
fn owner_linkage_nullable_text(
    row: &rusqlite::Row<'_>,
    index: usize,
    remaining: &mut u64,
) -> StorageResult<Option<String>> {
    if matches!(
        row.get_ref(index)
            .map_err(|e| transform_sql_error("read additive owner nullable text", e))?,
        rusqlite::types::ValueRef::Null
    ) {
        Ok(None)
    } else {
        owner_linkage_text(row, index, remaining).map(Some)
    }
}
fn owner_linkage_blob(
    row: &rusqlite::Row<'_>,
    index: usize,
    remaining: &mut u64,
) -> StorageResult<Vec<u8>> {
    let bytes = match row
        .get_ref(index)
        .map_err(|e| transform_sql_error("read additive owner blob", e))?
    {
        rusqlite::types::ValueRef::Blob(b) => b,
        _ => return Err(storage_fail("additive owner field type changed")),
    };
    owner_linkage_claim(bytes.len(), remaining)?;
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(bytes.len())
        .map_err(|_| storage_fail("additive owner blob allocation failed"))?;
    owned.extend_from_slice(bytes);
    Ok(owned)
}
fn owner_linkage_integer(row: &rusqlite::Row<'_>, index: usize) -> StorageResult<i64> {
    match row
        .get_ref(index)
        .map_err(|e| transform_sql_error("read additive owner integer", e))?
    {
        rusqlite::types::ValueRef::Integer(v) => Ok(v),
        _ => Err(storage_fail("additive owner field type changed")),
    }
}
fn owner_linkage_relations(fields: &OwnerLinkageFields) -> StorageResult<()> {
    let [old, owners, accounts, events, heads] = &fields.rows;
    for rows in &fields.rows {
        if rows
            .windows(2)
            .any(|w| w[0].account_id.as_deref().unwrap() >= w[1].account_id.as_deref().unwrap())
        {
            return Err(storage_fail(
                "additive owner duplicate or unordered account",
            ));
        }
    }
    if owners.len() != old.len() {
        return Err(storage_fail("additive owner backfill gap"));
    }
    for (i, row) in old.iter().enumerate() {
        if old[..i].iter().any(|v| v.epoch_id == row.epoch_id) {
            return Err(storage_fail("additive owner duplicate V1 epoch"));
        }
    }
    for rows in [owners, accounts, events, heads] {
        if rows
            .iter()
            .any(|r| !old.iter().any(|v| v.account_id == r.account_id))
        {
            return Err(storage_fail("additive owner orphan row"));
        }
    }
    for row in old {
        let owner = owners
            .iter()
            .find(|r| r.account_id == row.account_id)
            .ok_or_else(|| storage_fail("additive owner missing owner"))?;
        let account = accounts.iter().find(|r| r.account_id == row.account_id);
        let event = events.iter().find(|r| r.account_id == row.account_id);
        let head = heads.iter().find(|r| r.account_id == row.account_id);
        match owner.active_generation.unwrap() {
            1 => {
                if !crate::trading::paper_book_v2::owner_v1_fields_match(
                    owner.owner_revision.unwrap(),
                    owner.cutover_id.as_ref().unwrap().as_deref(),
                    owner.active_epoch_id.as_deref().unwrap(),
                    owner.active_manifest_hash.as_deref().unwrap(),
                    row.epoch_id.as_deref().unwrap(),
                    row.manifest_hash.as_deref().unwrap(),
                ) || account.is_some()
                    || event.is_some()
                    || head.is_some()
                {
                    return Err(storage_fail("additive owner V1Active mismatch"));
                }
            }
            2 => {
                let account =
                    account.ok_or_else(|| storage_fail("additive owner missing V2 account"))?;
                if event.is_none() || head.is_none() {
                    return Err(storage_fail("additive owner missing V2 genesis roster"));
                }
                if !crate::trading::paper_book_v2::owner_v2_fields_match(
                    owner.owner_revision.unwrap(),
                    owner.cutover_id.as_ref().unwrap().as_deref(),
                    owner.active_epoch_id.as_deref().unwrap(),
                    owner.active_manifest_hash.as_deref().unwrap(),
                    account.epoch_id.as_deref().unwrap(),
                    account.manifest_hash.as_deref().unwrap(),
                    account.account_cutover_id.as_deref().unwrap(),
                ) || old.iter().any(|v| v.epoch_id == account.epoch_id)
                {
                    return Err(storage_fail("additive owner V2Active mismatch"));
                }
            }
            _ => return Err(storage_fail("additive owner unknown generation")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod retained_owner_linkage_tests {
    use super::*;
    fn transformed(
        original: rows::VerifiedUnapprovedOriginalRowsBackup,
    ) -> AdditiveStorageTransformed {
        let copied =
            match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
                Ok(owner) => owner,
                Err(held) => panic!("owner linkage real Copied: {}", held.first_error()),
            };
        match copied.into_transformed() {
            Ok(owner) => owner,
            Err(held) => panic!("owner linkage real WAL: {}", held.first_error()),
        }
    }
    fn assert_complete(owner: &mut AdditiveStorageRetainedOwnerLinkage) {
        let f = &mut owner.frame;
        assert!(
            f.phase == OwnerLinkagePhase::Complete && f.fee.phase == FeeManifestPhase::Complete
        );
        assert!(f.all_returns() && f.relations_returned == Some(true));
        assert!(f.reads.iter().all(|r| r.started
            && r.types_checked
            && r.charged
            && r.eof
            && r.scopes_ended));
        assert!(f.fields.rows.iter().all(|r| !r.is_empty()));
        assert!(f.fields.rows[2]
            .iter()
            .all(|r| !r.manifest_bytes.as_ref().unwrap().is_empty()));
        assert!(
            f.fee.fields.descriptor_bytes.is_some() && matches!(f.fee.validation, Some(Ok(())))
        );
        assert!(f.fee.local.readonly.reader.is_none() && f.fee.local.readonly.active.is_none());
        assert!(f
            .fee
            .local
            .readonly
            .facts
            .iter()
            .all(|r| r.closed && r.original_tail_validated));
        assert_eq!(f.fee.local.readonly.loan().unwrap().3, 2);
        assert!(owner
            .active_accounts()
            .all(|(_, generation, _, _)| generation == 2));
    }
    #[test]
    fn task6_retained_owner_linkage_nonempty_same_reader_and_cold() {
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = transformed(original);
                let fd = owner.frame.base.target().unwrap().as_raw_fd();
                let mut linked = match owner.into_retained_owner_linkage() {
                    Ok(owner) => owner,
                    Err(held) => panic!("owner linkage real read: {}", held.first_error()),
                };
                assert_complete(&mut linked);
                assert_eq!(
                    linked
                        .frame
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .target()
                        .unwrap()
                        .as_raw_fd(),
                    fd
                );
                let saved = (
                    linked
                        .frame
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .target_node
                        .unwrap(),
                    linked
                        .active_accounts()
                        .map(|(id, generation, epoch, hash)| {
                            (id.to_owned(), generation, epoch.to_owned(), hash.to_owned())
                        })
                        .collect::<Vec<_>>(),
                    linked
                        .frame
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                );
                drop(linked);
                saved
            },
            |(node, accounts, records), original| {
                let mut linked = match AdditiveStorageRetainedOwnerLinkage::create_or_resume(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("owner linkage real cold read: {}", held.first_error()),
                };
                assert_complete(&mut linked);
                assert_eq!(
                    linked.frame.fee.local.readonly.transform.base.target_node,
                    Some(node)
                );
                assert_eq!(
                    linked
                        .active_accounts()
                        .map(|(id, generation, epoch, hash)| (
                            id.to_owned(),
                            generation,
                            epoch.to_owned(),
                            hash.to_owned()
                        ))
                        .collect::<Vec<_>>(),
                    accounts
                );
                assert_eq!(
                    linked
                        .frame
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                    records
                );
                drop(linked);
            },
        );
    }
    fn fixed_gate_connection() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE paper_ledger_account(account_id,epoch_id,manifest_hash);
            CREATE TABLE paper_book_owner_v2(account_id,active_generation,active_epoch_id,active_manifest_hash,owner_revision,cutover_id);
            CREATE TABLE paper_book_v2_account(account_id,epoch_id,manifest_hash,manifest_bytes,fee_policy_instance_id,v1_epoch_id,v1_manifest_hash,v1_head_version,v1_head_hash,v1_projection_hash,cutover_id);
            CREATE TABLE paper_book_v2_event(account_id); CREATE TABLE paper_book_v2_head(account_id);
            INSERT INTO paper_ledger_account VALUES('a','old','hash-old');
            INSERT INTO paper_book_owner_v2 VALUES('a',2,'new','hash-new',2,'cut');
            INSERT INTO paper_book_v2_account VALUES('a','new','hash-new',X'01','fee','old','hash-old',1,'head','projection','cut');
            INSERT INTO paper_book_v2_event VALUES('a'); INSERT INTO paper_book_v2_head VALUES('a');").unwrap();
        c
    }
    #[test]
    fn task6_retained_owner_linkage_typed_presence_epoch_and_drift_refusals() {
        // Arbitrary in-memory SQL proves only these fixed data gates, not a
        // retained owner, complete genesis verifier, or financial capability.
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut source = original.into_additive_target_source().unwrap();
            for case in [
                "v2",
                "v1",
                "type",
                "utf8",
                "missing",
                "duplicate",
                "orphan",
                "revision",
                "cutover",
                "epoch",
                "old_epoch",
                "generation",
                "v1_event",
                "missing_head",
            ] {
                let c = fixed_gate_connection();
                let sql = match case {
                    "v1" => "UPDATE paper_book_owner_v2 SET active_generation=1,active_epoch_id='old',active_manifest_hash='hash-old',owner_revision=1,cutover_id=NULL; DELETE FROM paper_book_v2_account; DELETE FROM paper_book_v2_event; DELETE FROM paper_book_v2_head;",
                    "type" => "UPDATE paper_book_v2_account SET manifest_bytes='text';",
                    "utf8" => "UPDATE paper_book_owner_v2 SET active_epoch_id=CAST(X'ff' AS TEXT);",
                    "missing" => "DELETE FROM paper_book_owner_v2;",
                    "duplicate" => "INSERT INTO paper_book_v2_event SELECT * FROM paper_book_v2_event;",
                    "orphan" => "INSERT INTO paper_book_v2_head VALUES('orphan');",
                    "revision" => "UPDATE paper_book_owner_v2 SET owner_revision=1;",
                    "cutover" => "UPDATE paper_book_owner_v2 SET cutover_id='wrong';",
                    "epoch" => "UPDATE paper_book_owner_v2 SET active_epoch_id='old'; UPDATE paper_book_v2_account SET epoch_id='old';",
                    "old_epoch" => "INSERT INTO paper_ledger_account VALUES('b','old','other-hash'); INSERT INTO paper_book_owner_v2 VALUES('b',1,'old','other-hash',1,NULL);",
                    "generation" => "UPDATE paper_book_owner_v2 SET active_generation=3;",
                    "v1_event" => "UPDATE paper_book_owner_v2 SET active_generation=1,active_epoch_id='old',active_manifest_hash='hash-old',owner_revision=1,cutover_id=NULL; DELETE FROM paper_book_v2_account; DELETE FROM paper_book_v2_head;",
                    "missing_head" => "DELETE FROM paper_book_v2_head;", _ => "",
                };
                c.execute_batch(sql).unwrap();
                let mut fields = OwnerLinkageFields::default();
                let mut facts: [OwnerLinkageReadFacts; 5] =
                    std::array::from_fn(|_| OwnerLinkageReadFacts::default());
                let work = source.storage_parts().unwrap().2;
                let before = work.metadata_used();
                let actual = (|| {
                    for query in OwnerLinkageQuery::ALL {
                        OwnerLinkageReadLoan {
                            connection: &c,
                            work: &mut *work,
                            rows: &mut fields.rows[query.slot()],
                            facts: &mut facts[query.slot()],
                        }
                        .read(query)?;
                    }
                    work.metadata(1024)?;
                    owner_linkage_relations(&fields)
                })();
                assert!(work.metadata_used() > before);
                if matches!(case, "v1" | "v2") {
                    actual.unwrap();
                    assert!(facts
                        .iter()
                        .all(|r| r.eof && r.scopes_ended && r.returned == Some(true)));
                } else {
                    let expected = match case {
                        "type" => "additive owner row types differ",
                        "utf8" => "additive owner text is not UTF8",
                        "missing" => "additive owner backfill gap",
                        "duplicate" => "additive owner duplicate or unordered account",
                        "orphan" => "additive owner orphan row",
                        "old_epoch" => "additive owner duplicate V1 epoch",
                        "generation" => "additive owner unknown generation",
                        "v1_event" => "additive owner V1Active mismatch",
                        "missing_head" => "additive owner missing V2 genesis roster",
                        _ => "additive owner V2Active mismatch",
                    };
                    assert!(
                        matches!(actual.unwrap_err(), GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == expected)
                    );
                    if case == "type" {
                        assert!(!facts[2].charged && fields.rows[2].is_empty());
                    }
                    if case == "utf8" {
                        assert!(
                            facts[1].charged
                                && fields.rows[1][0].account_id.is_some()
                                && fields.rows[1][0].active_epoch_id.is_none()
                        );
                    }
                }
                c.close().unwrap();
            }
            drop(source);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = OwnerLinkageFrame::new(transformed(original).frame);
            assert!(f.fee.local.start(false));
            let file = f.fee.local.readonly.transform.base.target().unwrap();
            let mut byte = [0];
            file.read_exact_at(&mut byte, 100).unwrap();
            file.write_all_at(&[byte[0] ^ 1], 100).unwrap();
            file.sync_all().unwrap();
            let actual = f.fee.local.compare_second();
            f.fee
                .local
                .readonly
                .transform
                .base
                .target()
                .unwrap()
                .write_all_at(&byte, 100)
                .unwrap();
            f.fee
                .local
                .readonly
                .transform
                .base
                .target()
                .unwrap()
                .sync_all()
                .unwrap(); // Cleanup, never success.
            let first = actual.unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail }
                if detail == "additive readonly retained target bytes changed")
            );
            f.fail(first);
            assert!(!f.finish() && f.reads.iter().all(|r| !r.started));
            assert!(f.fields.rows.iter().all(Vec::is_empty));
            drop(f);
        });
    }
    #[test]
    fn task6_retained_owner_linkage_same_work_unknown_late_and_busy_hold() {
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = OwnerLinkageFrame::new(transformed(original).frame);
            assert!(f.start(false));
            let work = f.fee.local.readonly.loan().unwrap().2;
            let remaining = 16 * MIB - work.metadata_used();
            work.metadata(remaining).unwrap();
            assert!(!f.finish());
            assert!(f.reads.iter().all(|r| !r.started));
            assert!(f.fields.rows.iter().all(Vec::is_empty));
            let first = f
                .fee
                .local
                .readonly
                .transform
                .first
                .as_ref()
                .unwrap()
                .to_string();
            let used = f.fee.local.readonly.loan().unwrap().2.metadata_used();
            assert!(!f.finish());
            assert_eq!(f.fee.local.readonly.loan().unwrap().2.metadata_used(), used);
            assert_eq!(
                f.fee
                    .local
                    .readonly
                    .transform
                    .first
                    .as_ref()
                    .unwrap()
                    .to_string(),
                first
            );
            drop(f);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = OwnerLinkageFrame::new(transformed(original).frame);
            assert!(f.start(false));
            let actual = f.read_fixed(OwnerLinkageQuery::OldAccounts);
            actual.as_ref().unwrap();
            let pointer = f.fields.rows[0][0].account_id.as_ref().unwrap().as_ptr();
            assert!(f.reads[0].eof && f.reads[0].scopes_ended && f.returns[0].is_none());
            let first = f.close_and_tail().unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail }
                if detail == "additive owner close before relations returned")
            );
            f.fail(first);
            f.returns[0] = Some(actual); // The actual late result stays owned even after first.
            assert!(matches!(f.returns[0], Some(Ok(()))));
            assert_eq!(
                f.fields.rows[0][0].account_id.as_ref().unwrap().as_ptr(),
                pointer
            );
            let used = f.fee.local.readonly.loan().unwrap().2.metadata_used();
            assert!(!f.finish());
            assert_eq!(f.fee.local.readonly.loan().unwrap().2.metadata_used(), used);
            assert!(f.reads[1..].iter().all(|r| !r.started) && f.relations_returned.is_none());
            assert!(!f.fee.local.readonly.facts[1].original_tail_validated);
            drop(f);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = OwnerLinkageFrame::new(transformed(original).frame);
            assert!(f.start(false));
            assert!(f.advance_reads());
            f.validate_fields().unwrap();
            f.fee.local.readonly.prepare_busy_vm();
            let first = f.close_and_tail().unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSqlite { operation: "close additive readonly", source }
                if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy))
            );
            f.fail(first);
            assert!(
                f.fee.local.readonly.reader.is_some()
                    && f.fields.rows[2][0].manifest_bytes.is_some()
            );
            let used = f.fee.local.readonly.loan().unwrap().2.metadata_used();
            assert!(!f.finish());
            assert_eq!(f.fee.local.readonly.loan().unwrap().2.metadata_used(), used);
            assert!(!f.fee.local.readonly.facts[1].original_tail_validated);
            assert!(f.fee.local.readonly.finalize_busy_once());
            f.fee.local.readonly.cleanup_reader_once();
            assert!(matches!(f.fee.local.readonly.cleanup_close, Some(Ok(()))));
            assert!(
                f.fields.rows[2][0].manifest_bytes.is_some()
                    && !f.fee.local.readonly.facts[1].original_tail_validated
            );
            drop(f);
        });
    }
}

// Fixed ordinary Genesis/current-head inputs, never verified replay or authority.
// The unfinished owner frame lends its existing second reader and one work pool.
#[derive(Default)]
struct GenesisInputRow {
    account_id: Option<String>,
    sequence: Option<i64>,
    command_id: Option<String>,
    previous_hash: Option<String>,
    event_hash: Option<String>,
    kind: Option<String>,
    payload: Option<Vec<u8>>,
    version: Option<i64>,
    projection_bytes: Option<Vec<u8>>,
    projection_hash: Option<String>,
}
#[derive(Default)]
struct GenesisInputReadFacts {
    started: bool,
    count: Option<i64>,
    types_checked: bool,
    extent: Option<u64>,
    charged: bool,
    acquired: usize,
    eof: bool,
    scopes_ended: bool,
    returned: Option<bool>,
}
struct GenesisInputFields {
    rows: [Vec<GenesisInputRow>; 2],
}
impl Default for GenesisInputFields {
    fn default() -> Self {
        Self {
            rows: std::array::from_fn(|_| Vec::new()),
        }
    }
}
#[derive(Clone, Copy)]
enum GenesisInputQuery {
    Event,
    Head,
}
impl GenesisInputQuery {
    const ALL: [Self; 2] = [Self::Event, Self::Head];
    fn slot(self) -> usize {
        match self {
            Self::Event => 0,
            Self::Head => 1,
        }
    }
    fn sql(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::Event => (
                "SELECT COUNT(*) FROM main.paper_book_v2_event WHERE seq=1",
                "SELECT COUNT(*) FROM main.paper_book_v2_event WHERE seq=1 AND (typeof(account_id)!='text' OR typeof(seq)!='integer' OR typeof(command_id)!='text' OR typeof(previous_hash)!='text' OR typeof(event_hash)!='text' OR typeof(kind)!='text' OR typeof(payload)!='blob')",
                "SELECT coalesce(sum(length(CAST(account_id AS BLOB))+length(CAST(command_id AS BLOB))+length(CAST(previous_hash AS BLOB))+length(CAST(event_hash AS BLOB))+length(CAST(kind AS BLOB))+length(payload)),0) FROM main.paper_book_v2_event WHERE seq=1",
                "SELECT account_id,seq,command_id,previous_hash,event_hash,kind,payload FROM main.paper_book_v2_event WHERE seq=1 ORDER BY account_id",
            ),
            Self::Head => (
                "SELECT COUNT(*) FROM main.paper_book_v2_head",
                "SELECT COUNT(*) FROM main.paper_book_v2_head WHERE typeof(account_id)!='text' OR typeof(version)!='integer' OR typeof(event_hash)!='text' OR typeof(projection_bytes)!='blob' OR typeof(projection_hash)!='text'",
                "SELECT coalesce(sum(length(CAST(account_id AS BLOB))+length(CAST(event_hash AS BLOB))+length(projection_bytes)+length(CAST(projection_hash AS BLOB))),0) FROM main.paper_book_v2_head",
                "SELECT account_id,version,event_hash,projection_bytes,projection_hash FROM main.paper_book_v2_head ORDER BY account_id",
            ),
        }
    }
}
#[derive(PartialEq, Eq)]
enum GenesisInputPhase {
    Fresh,
    ReaderChecked,
    RowsReturned,
    InputsChecked,
    Complete,
    Refused,
}
struct GenesisFieldsFrame {
    owner: OwnerLinkageFrame,
    phase: GenesisInputPhase,
    fields: GenesisInputFields,
    reads: [GenesisInputReadFacts; 2],
    returns: [Option<StorageResult<()>>; 2],
    relations_returned: Option<bool>,
}
pub(super) struct AdditiveStorageRetainedGenesisInputs {
    frame: GenesisFieldsFrame,
}
pub(super) struct AdditiveStorageGenesisInputsHeld {
    frame: GenesisFieldsFrame,
}
impl AdditiveStorageGenesisInputsHeld {
    pub(super) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame
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
    pub(super) fn into_retained_genesis_inputs(
        self,
    ) -> std::result::Result<AdditiveStorageRetainedGenesisInputs, AdditiveStorageGenesisInputsHeld>
    {
        GenesisFieldsFrame::new(self.frame).run(false)
    }
}
impl AdditiveStorageRetainedGenesisInputs {
    pub(super) fn create_or_resume(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageGenesisInputsHeld> {
        let base = AdditiveStorageCopied {
            source,
            managed: None,
            directory: None,
            anchor: None,
            fresh: false,
            original: None,
            rows: None,
            records: std::array::from_fn(|_| None),
            pending: None,
            target: None,
            target_node: None,
            copied: None,
            census_files: std::array::from_fn(|_| None),
            codec: AdditiveRecordCodecState::new(),
            copy_issued: false,
            rejected_copy_return: None,
            copy_return_failed: false,
            copy_return_error: None,
            copy_origin_return_error: None,
        };
        GenesisFieldsFrame::new(TransformFrame::new(base)).run(true)
    }
}
impl GenesisFieldsFrame {
    fn new(transform: TransformFrame) -> Self {
        Self {
            owner: OwnerLinkageFrame::new(transform),
            phase: GenesisInputPhase::Fresh,
            fields: GenesisInputFields::default(),
            reads: std::array::from_fn(|_| GenesisInputReadFacts::default()),
            returns: std::array::from_fn(|_| None),
            relations_returned: None,
        }
    }
    fn fail(&mut self, first: GlobalSchemaV1Error) {
        self.owner.fail(first);
        self.phase = GenesisInputPhase::Refused;
    }
    fn run(
        mut self,
        cold: bool,
    ) -> std::result::Result<AdditiveStorageRetainedGenesisInputs, AdditiveStorageGenesisInputsHeld>
    {
        if !self.start(cold) || !self.finish() {
            return Err(AdditiveStorageGenesisInputsHeld { frame: self });
        }
        Ok(AdditiveStorageRetainedGenesisInputs { frame: self })
    }
    fn start(&mut self, cold: bool) -> bool {
        if self.owner.fee.local.readonly.transform.first.is_some() {
            return false;
        }
        if self.phase != GenesisInputPhase::Fresh {
            self.fail(storage_fail("additive genesis start phase differs"));
            return false;
        }
        if !self.owner.start(cold) || !self.owner.advance_reads() {
            self.phase = GenesisInputPhase::Refused;
            return false;
        }
        match self.owner.validate_fields() {
            Ok(()) => {
                self.phase = GenesisInputPhase::ReaderChecked;
                true
            }
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
    fn read_fixed(&mut self, query: GenesisInputQuery) -> StorageResult<()> {
        if self.owner.fee.local.readonly.transform.first.is_some() {
            return Err(storage_fail("additive genesis read after first error"));
        }
        if self.phase != GenesisInputPhase::ReaderChecked
            || self.owner.phase != OwnerLinkagePhase::RelationsChecked
            || self.owner.relations_returned != Some(true)
            || !self.owner.all_returns()
            || self.owner.fee.local.readonly.active != Some(1)
        {
            return Err(storage_fail(
                "additive genesis lacks validated second reader",
            ));
        }
        let Retained8Frame {
            transform,
            permit,
            reader,
            ..
        } = &mut self.owner.fee.local.readonly;
        let (_, _, work, completed) = transform
            .base
            .source
            .retained8_parts(permit.as_ref().unwrap())?;
        if completed != 2 {
            return Err(storage_fail("additive genesis pair count differs"));
        }
        GenesisInputReadLoan {
            connection: reader.as_ref().unwrap(),
            work,
            rows: &mut self.fields.rows[query.slot()],
            facts: &mut self.reads[query.slot()],
        }
        .read(query)
    }
    fn advance_reads(&mut self) -> bool {
        if self.owner.fee.local.readonly.transform.first.is_some() {
            return false;
        }
        if self.phase != GenesisInputPhase::ReaderChecked {
            self.fail(storage_fail("additive genesis read phase differs"));
            return false;
        }
        for query in GenesisInputQuery::ALL {
            let i = query.slot();
            if self.returns[i].is_some() || self.reads[i].started {
                self.fail(storage_fail("additive genesis query already reached"));
                return false;
            }
            // Owned field children land directly in this frame. The actual
            // whole Result is parked before any first-error inspection.
            self.returns[i] = Some(self.read_fixed(query));
            if self.returns[i].as_ref().unwrap().is_err() {
                let first = self.returns[i].take().unwrap().unwrap_err();
                self.fail(first);
                return false;
            }
        }
        self.phase = GenesisInputPhase::RowsReturned;
        true
    }
    fn all_returns(&self) -> bool {
        self.returns.iter().all(|r| matches!(r, Some(Ok(()))))
            && self.reads.iter().all(|r| {
                r.returned == Some(true) && r.eof && r.scopes_ended && r.types_checked && r.charged
            })
    }
    fn validate_fields(&mut self) -> StorageResult<()> {
        if self.owner.fee.local.readonly.transform.first.is_some() {
            return Err(storage_fail(
                "additive genesis validation after first error",
            ));
        }
        if self.phase != GenesisInputPhase::RowsReturned
            || !self.all_returns()
            || self.relations_returned.is_some()
        {
            return Err(storage_fail("additive genesis inputs before whole returns"));
        }
        self.owner.fee.local.readonly.loan()?.2.metadata(1024)?;
        let actual = genesis_input_rosters(&self.owner.fields, &self.fields);
        self.relations_returned = Some(actual.is_ok());
        actual?;
        self.phase = GenesisInputPhase::InputsChecked;
        Ok(())
    }
    fn close_and_tail(&mut self) -> StorageResult<()> {
        if self.owner.fee.local.readonly.transform.first.is_some() {
            return Err(storage_fail("additive genesis close after first error"));
        }
        if self.phase != GenesisInputPhase::InputsChecked
            || self.relations_returned != Some(true)
            || !self.all_returns()
        {
            return Err(storage_fail(
                "additive genesis close before inputs returned",
            ));
        }
        self.owner.close_and_tail()?;
        self.phase = GenesisInputPhase::Complete;
        Ok(())
    }
    fn finish(&mut self) -> bool {
        if self.owner.fee.local.readonly.transform.first.is_some() {
            return false;
        }
        if !self.advance_reads() {
            return false;
        }
        let actual = self.validate_fields().and_then(|()| self.close_and_tail());
        match actual {
            Ok(()) => true,
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
}
struct GenesisInputReadLoan<'a> {
    connection: &'a Connection,
    work: &'a mut target::TargetWork,
    rows: &'a mut Vec<GenesisInputRow>,
    facts: &'a mut GenesisInputReadFacts,
}
impl GenesisInputReadLoan<'_> {
    fn read(mut self, query: GenesisInputQuery) -> StorageResult<()> {
        if let Err(first) = self.preflight(query) {
            self.facts.scopes_ended = true;
            self.facts.returned = Some(false);
            return Err(first);
        }
        self.acquire(query)
    }
    fn preflight(&mut self, query: GenesisInputQuery) -> StorageResult<()> {
        if self.facts.started || !self.rows.is_empty() {
            return Err(storage_fail("additive genesis query already started"));
        }
        let (count_sql, type_sql, extent_sql, fields_sql) = query.sql();
        self.work.metadata(
            4096 + (count_sql.len() + type_sql.len() + extent_sql.len() + fields_sql.len()) as u64,
        )?;
        self.facts.started = true;
        let count: i64 = self
            .connection
            .query_row(count_sql, [], |r| r.get(0))
            .map_err(|e| transform_sql_error("count additive genesis rows", e))?;
        self.facts.count = Some(count);
        let count =
            u64::try_from(count).map_err(|_| storage_fail("additive genesis count overflow"))?;
        let invalid: i64 = self
            .connection
            .query_row(type_sql, [], |r| r.get(0))
            .map_err(|e| transform_sql_error("type additive genesis rows", e))?;
        if invalid != 0 {
            return Err(storage_fail("additive genesis row types differ"));
        }
        self.facts.types_checked = true;
        let extent: i64 = self
            .connection
            .query_row(extent_sql, [], |r| r.get(0))
            .map_err(|e| transform_sql_error("extent additive genesis rows", e))?;
        let extent =
            u64::try_from(extent).map_err(|_| storage_fail("additive genesis extent overflow"))?;
        self.facts.extent = Some(extent);
        let slots = count
            .checked_mul(std::mem::size_of::<GenesisInputRow>() as u64)
            .ok_or_else(|| storage_fail("additive genesis capacity overflow"))?;
        self.work.metadata(
            slots
                .checked_add(extent)
                .ok_or_else(|| storage_fail("additive genesis capacity overflow"))?,
        )?;
        self.facts.charged = true;
        Ok(())
    }
    fn acquire(self, query: GenesisInputQuery) -> StorageResult<()> {
        let actual = (|| {
            if !self.facts.started
                || !self.facts.charged
                || self.facts.returned.is_some()
                || !self.rows.is_empty()
            {
                return Err(storage_fail(
                    "additive genesis acquire lacks fixed preflight",
                ));
            }
            let count = usize::try_from(self.facts.count.unwrap())
                .map_err(|_| storage_fail("additive genesis count overflow"))?;
            let mut remaining = self.facts.extent.unwrap();
            self.rows
                .try_reserve_exact(count)
                .map_err(|_| storage_fail("additive genesis row allocation failed"))?;
            let mut statement = self
                .connection
                .prepare(query.sql().3)
                .map_err(|e| transform_sql_error("prepare additive genesis rows", e))?;
            let mut rows = statement
                .query([])
                .map_err(|e| transform_sql_error("query additive genesis rows", e))?;
            while let Some(row) = rows
                .next()
                .map_err(|e| transform_sql_error("step additive genesis rows", e))?
            {
                if self.rows.len() == count {
                    return Err(storage_fail("additive genesis extra row"));
                }
                self.rows.push(GenesisInputRow::default());
                self.facts.acquired = self.rows.len();
                let slot = self.rows.last_mut().unwrap();
                slot.account_id = Some(owner_linkage_text(row, 0, &mut remaining)?);
                match query {
                    GenesisInputQuery::Event => {
                        slot.sequence = Some(owner_linkage_integer(row, 1)?);
                        slot.command_id = Some(owner_linkage_text(row, 2, &mut remaining)?);
                        slot.previous_hash = Some(owner_linkage_text(row, 3, &mut remaining)?);
                        slot.event_hash = Some(owner_linkage_text(row, 4, &mut remaining)?);
                        slot.kind = Some(owner_linkage_text(row, 5, &mut remaining)?);
                        slot.payload = Some(owner_linkage_blob(row, 6, &mut remaining)?);
                    }
                    GenesisInputQuery::Head => {
                        slot.version = Some(owner_linkage_integer(row, 1)?);
                        slot.event_hash = Some(owner_linkage_text(row, 2, &mut remaining)?);
                        slot.projection_bytes = Some(owner_linkage_blob(row, 3, &mut remaining)?);
                        slot.projection_hash = Some(owner_linkage_text(row, 4, &mut remaining)?);
                    }
                }
            }
            self.facts.eof = true;
            if self.rows.len() != count || remaining != 0 {
                return Err(storage_fail("additive genesis count/extent changed"));
            }
            Ok(())
        })();
        // Actual Rows/Statement scopes end independently of this owned return.
        // Their ignored Drop results are not converted to successful facts.
        self.facts.scopes_ended = true;
        self.facts.returned = Some(actual.is_ok());
        actual
    }
}
fn genesis_input_rosters(
    owner: &OwnerLinkageFields,
    fields: &GenesisInputFields,
) -> StorageResult<()> {
    let accounts = &owner.rows[2];
    for rows in &fields.rows {
        if rows
            .windows(2)
            .any(|w| w[0].account_id.as_deref().unwrap() >= w[1].account_id.as_deref().unwrap())
        {
            return Err(storage_fail(
                "additive genesis duplicate or unordered account",
            ));
        }
        if rows.len() != accounts.len()
            || rows
                .iter()
                .zip(accounts)
                .any(|(a, b)| a.account_id != b.account_id)
        {
            return Err(storage_fail("additive genesis input roster differs"));
        }
    }
    // A selected seq1 row and current head stay independent. No head version,
    // hash, payload, canonical replay or audit assertion is manufactured here.
    if fields.rows[0].iter().any(|r| r.sequence != Some(1)) {
        return Err(storage_fail("additive genesis sequence differs"));
    }
    Ok(())
}

#[cfg(test)]
mod retained_genesis_input_tests {
    use super::*;
    fn transformed(
        original: rows::VerifiedUnapprovedOriginalRowsBackup,
    ) -> AdditiveStorageTransformed {
        let copied =
            match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
                Ok(owner) => owner,
                Err(held) => panic!("genesis inputs real Copied: {}", held.first_error()),
            };
        match copied.into_transformed() {
            Ok(owner) => owner,
            Err(held) => panic!("genesis inputs real WAL: {}", held.first_error()),
        }
    }
    fn complete(frame: &mut GenesisFieldsFrame) {
        assert!(
            frame.phase == GenesisInputPhase::Complete
                && frame.owner.phase == OwnerLinkagePhase::Complete
        );
        assert!(frame.all_returns() && frame.relations_returned == Some(true));
        assert!(frame.reads.iter().all(|r| r.started
            && r.types_checked
            && r.charged
            && r.eof
            && r.scopes_ended));
        assert!(
            !frame.fields.rows[0].is_empty()
                && frame.fields.rows[0].len() == frame.owner.fields.rows[2].len()
        );
        for (i, r) in frame.fields.rows[0].iter().enumerate() {
            assert!(r.account_id.is_some() && r.sequence == Some(1) && r.command_id.is_some());
            assert!(r.previous_hash.is_some() && r.event_hash.is_some() && r.kind.is_some());
            assert!(!r.payload.as_ref().unwrap().is_empty());
            assert_eq!(r.kind.as_deref(), Some("Genesis"));
            assert_eq!(r.previous_hash, frame.owner.fields.rows[2][i].v1_head_hash);
        }
        for (i, r) in frame.fields.rows[1].iter().enumerate() {
            assert!(
                r.account_id.is_some()
                    && r.version.is_some()
                    && r.event_hash.is_some()
                    && r.projection_hash.is_some()
            );
            assert!(!r.projection_bytes.as_ref().unwrap().is_empty());
            assert_eq!(r.version, Some(1));
            assert_eq!(r.event_hash, frame.fields.rows[0][i].event_hash);
            assert_eq!(
                r.projection_hash,
                frame.owner.fields.rows[2][i].v1_projection_hash
            );
        }
        assert!(
            frame.owner.fee.local.readonly.reader.is_none()
                && frame.owner.fee.local.readonly.active.is_none()
        );
        assert!(frame
            .owner
            .fee
            .local
            .readonly
            .facts
            .iter()
            .all(|r| r.closed && r.original_tail_validated));
        assert_eq!(frame.owner.fee.local.readonly.loan().unwrap().3, 2);
    }
    fn fingerprint(
        frame: &GenesisFieldsFrame,
    ) -> (
        Vec<(String, i64, String, String, String, String, Vec<u8>)>,
        Vec<(String, i64, String, Vec<u8>, String)>,
    ) {
        // Test comparison only; this does not charge/mint production authority.
        (
            frame.fields.rows[0]
                .iter()
                .map(|r| {
                    (
                        r.account_id.as_ref().unwrap().clone(),
                        r.sequence.unwrap(),
                        r.command_id.as_ref().unwrap().clone(),
                        r.previous_hash.as_ref().unwrap().clone(),
                        r.event_hash.as_ref().unwrap().clone(),
                        r.kind.as_ref().unwrap().clone(),
                        r.payload.as_ref().unwrap().clone(),
                    )
                })
                .collect(),
            frame.fields.rows[1]
                .iter()
                .map(|r| {
                    (
                        r.account_id.as_ref().unwrap().clone(),
                        r.version.unwrap(),
                        r.event_hash.as_ref().unwrap().clone(),
                        r.projection_bytes.as_ref().unwrap().clone(),
                        r.projection_hash.as_ref().unwrap().clone(),
                    )
                })
                .collect(),
        )
    }
    #[test]
    fn task6_retained_genesis_input_same_reader_and_cold() {
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = transformed(original);
                let node = owner.frame.base.target_node.unwrap();
                let prefix = owner
                    .frame
                    .base
                    .records
                    .iter()
                    .flatten()
                    .map(|r| (r.node, r.bytes.clone()))
                    .collect::<Vec<_>>();
                assert_eq!(prefix.len(), 5);
                drop(owner);
                (node, prefix)
            },
            |(node, prefix), original| {
                let mut retained = match AdditiveStorageRetainedGenesisInputs::create_or_resume(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("genesis inputs cold5: {}", held.first_error()),
                };
                complete(&mut retained.frame);
                assert_eq!(
                    retained
                        .frame
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .target_node,
                    Some(node)
                );
                assert_eq!(
                    &retained
                        .frame
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>()[..5],
                    prefix.as_slice()
                );
                assert!(retained
                    .frame
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .begin_return
                    .is_none());
                drop(retained);
            },
        );
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = transformed(original);
                let fd = owner.frame.base.target().unwrap().as_raw_fd();
                let mut retained = match owner.into_retained_genesis_inputs() {
                    Ok(owner) => owner,
                    Err(held) => panic!("genesis inputs warm: {}", held.first_error()),
                };
                complete(&mut retained.frame);
                assert_eq!(
                    retained
                        .frame
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .target()
                        .unwrap()
                        .as_raw_fd(),
                    fd
                );
                let saved = (
                    retained
                        .frame
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .target_node
                        .unwrap(),
                    fingerprint(&retained.frame),
                    retained
                        .frame
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                );
                drop(retained);
                saved
            },
            |(node, fields, records), original| {
                let mut retained = match AdditiveStorageRetainedGenesisInputs::create_or_resume(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("genesis inputs cold6: {}", held.first_error()),
                };
                complete(&mut retained.frame);
                assert_eq!(fingerprint(&retained.frame), fields);
                assert_eq!(
                    retained
                        .frame
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .target_node,
                    Some(node)
                );
                assert_eq!(
                    retained
                        .frame
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                    records
                );
                drop(retained);
            },
        );
    }
    fn fixed_gate_connection() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE paper_book_v2_event(account_id,seq,command_id,previous_hash,event_hash,kind,payload);
            CREATE TABLE paper_book_v2_head(account_id,version,event_hash,projection_bytes,projection_hash);
            INSERT INTO paper_book_v2_event VALUES('a',1,'command','previous','event','Genesis',X'0102');
            INSERT INTO paper_book_v2_head VALUES('a',1,'event',X'0304','projection');").unwrap();
        c
    }
    fn gate_roster() -> OwnerLinkageFields {
        let mut owner = OwnerLinkageFields::default();
        owner.rows[2].push(OwnerLinkageRow {
            account_id: Some("a".to_owned()),
            ..OwnerLinkageRow::default()
        });
        owner
    }
    #[test]
    fn task6_retained_genesis_input_typed_extent_presence_and_drift() {
        // These real in-memory SQL mutations test only the fixed data callee;
        // they do not build a retained frame, fake proof or verified genesis.
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut source = original.into_additive_target_source().unwrap();
            for case in [
                "plain",
                "head_later",
                "type",
                "head_type",
                "utf8",
                "missing",
                "sequence",
                "duplicate",
                "orphan",
            ] {
                let c = fixed_gate_connection();
                c.execute_batch(match case {
                    "head_later" => "UPDATE paper_book_v2_head SET version=3;",
                    "type" => "UPDATE paper_book_v2_event SET payload='text';",
                    "head_type" => "UPDATE paper_book_v2_head SET projection_bytes='text';",
                    "utf8" => "UPDATE paper_book_v2_event SET command_id=CAST(X'ff' AS TEXT);",
                    "missing" => "DELETE FROM paper_book_v2_event;",
                    "sequence" => "UPDATE paper_book_v2_event SET seq=2;",
                    "duplicate" => {
                        "INSERT INTO paper_book_v2_event SELECT * FROM paper_book_v2_event;"
                    }
                    "orphan" => "UPDATE paper_book_v2_head SET account_id='orphan';",
                    _ => "",
                })
                .unwrap();
                let mut fields = GenesisInputFields::default();
                let mut facts: [GenesisInputReadFacts; 2] =
                    std::array::from_fn(|_| GenesisInputReadFacts::default());
                let work = source.storage_parts().unwrap().2;
                let actual = (|| {
                    for query in GenesisInputQuery::ALL {
                        GenesisInputReadLoan {
                            connection: &c,
                            work: &mut *work,
                            rows: &mut fields.rows[query.slot()],
                            facts: &mut facts[query.slot()],
                        }
                        .read(query)?;
                    }
                    work.metadata(1024)?;
                    genesis_input_rosters(&gate_roster(), &fields)
                })();
                if matches!(case, "plain" | "head_later") {
                    actual.unwrap();
                    assert!(facts
                        .iter()
                        .all(|r| r.eof && r.scopes_ended && r.returned == Some(true)));
                    assert_eq!(fields.rows[0][0].sequence, Some(1));
                    if case == "head_later" {
                        assert_eq!(fields.rows[1][0].version, Some(3));
                    }
                } else {
                    let expected = match case {
                        "type" | "head_type" => "additive genesis row types differ",
                        "utf8" => "additive owner text is not UTF8",
                        "duplicate" => "additive genesis duplicate or unordered account",
                        _ => "additive genesis input roster differs",
                    };
                    assert!(
                        matches!(actual.unwrap_err(), GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == expected)
                    );
                    if case == "type" {
                        assert!(fields.rows[0].is_empty() && !facts[0].charged);
                    }
                    if case == "head_type" {
                        assert!(
                            !fields.rows[0].is_empty()
                                && fields.rows[1].is_empty()
                                && !facts[1].charged
                        );
                    }
                    if case == "utf8" {
                        assert!(
                            facts[0].charged
                                && fields.rows[0][0].account_id.is_some()
                                && fields.rows[0][0].command_id.is_none()
                        );
                    }
                }
                c.close().unwrap();
            }
            for growth in [false, true] {
                let c = fixed_gate_connection();
                let mut rows = Vec::new();
                let mut facts = GenesisInputReadFacts::default();
                let mut loan = GenesisInputReadLoan {
                    connection: &c,
                    work: source.storage_parts().unwrap().2,
                    rows: &mut rows,
                    facts: &mut facts,
                };
                loan.preflight(GenesisInputQuery::Event).unwrap();
                c.execute_batch(if growth {
                    "UPDATE paper_book_v2_event SET payload=X'0102030405';"
                } else {
                    "UPDATE paper_book_v2_event SET payload=X'01';"
                })
                .unwrap();
                let first = loan.acquire(GenesisInputQuery::Event).unwrap_err();
                let expected = if growth {
                    "additive owner field extent changed"
                } else {
                    "additive genesis count/extent changed"
                };
                assert!(
                    matches!(first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == expected)
                );
                assert!(facts.scopes_ended && facts.returned == Some(false));
                assert_eq!(facts.eof, !growth);
                assert!(rows[0].account_id.is_some() && rows[0].kind.is_some());
                assert_eq!(rows[0].payload.is_some(), !growth);
                c.close().unwrap();
            }
            drop(source);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = GenesisFieldsFrame::new(transformed(original).frame);
            assert!(f.start(false));
            assert!(f.advance_reads());
            f.validate_fields().unwrap();
            let file = f.owner.fee.local.readonly.transform.base.target().unwrap();
            let mut byte = [0];
            file.read_exact_at(&mut byte, 100).unwrap();
            file.write_all_at(&[byte[0] ^ 1], 100).unwrap();
            file.sync_all().unwrap();
            let actual = f.close_and_tail();
            f.owner
                .fee
                .local
                .readonly
                .transform
                .base
                .target()
                .unwrap()
                .write_all_at(&byte, 100)
                .unwrap();
            f.owner
                .fee
                .local
                .readonly
                .transform
                .base
                .target()
                .unwrap()
                .sync_all()
                .unwrap(); // Fixture cleanup only.
            let first = actual.unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail }
                if detail == "additive readonly retained target bytes changed")
            );
            assert!(
                f.owner.fee.local.readonly.reader.is_none()
                    && f.owner.fee.local.readonly.active.is_none()
            );
            assert!(
                f.owner.fee.local.readonly.facts[1].closed
                    && f.owner.fee.local.readonly.facts[1].original_tail_validated
            );
            assert!(
                f.owner.fee.local.phase == LocalCompletionPhase::ReaderClosed
                    && f.phase == GenesisInputPhase::InputsChecked
            );
            let used = f.owner.fee.local.readonly.loan().unwrap().2.metadata_used();
            f.fail(first);
            assert!(
                f.fields.rows[0][0].payload.is_some()
                    && f.fields.rows[1][0].projection_bytes.is_some()
            );
            let primary = f.owner.fee.local.readonly.transform.first.as_ref().unwrap()
                as *const GlobalSchemaV1Error;
            for _ in 0..2 {
                assert!(!f.finish() && f.phase == GenesisInputPhase::Refused);
                assert_eq!(
                    f.owner.fee.local.readonly.transform.first.as_ref().unwrap()
                        as *const GlobalSchemaV1Error,
                    primary
                );
                assert_eq!(
                    f.owner.fee.local.readonly.loan().unwrap().2.metadata_used(),
                    used
                );
                assert!(
                    f.owner.fee.local.readonly.facts[1].closed
                        && f.owner.fee.local.readonly.facts[1].original_tail_validated
                );
                assert!(
                    f.owner.fee.local.readonly.reader.is_none()
                        && f.owner.fee.local.readonly.active.is_none()
                );
                assert!(f.owner.fee.local.phase != LocalCompletionPhase::Complete);
            }
            drop(f);
        });
    }
    #[test]
    fn task6_retained_genesis_input_same_work_unknown_late_and_busy() {
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = GenesisFieldsFrame::new(transformed(original).frame);
            assert!(f.start(false));
            let work = f.owner.fee.local.readonly.loan().unwrap().2;
            let remaining = 16 * MIB - work.metadata_used();
            work.metadata(remaining).unwrap();
            assert!(
                !f.finish()
                    && f.fields.rows.iter().all(Vec::is_empty)
                    && f.reads.iter().all(|r| !r.started)
            );
            let first = f
                .owner
                .fee
                .local
                .readonly
                .transform
                .first
                .as_ref()
                .unwrap()
                .to_string();
            let used = f.owner.fee.local.readonly.loan().unwrap().2.metadata_used();
            assert!(!f.finish());
            assert_eq!(
                f.owner.fee.local.readonly.loan().unwrap().2.metadata_used(),
                used
            );
            assert_eq!(
                f.owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .first
                    .as_ref()
                    .unwrap()
                    .to_string(),
                first
            );
            drop(f);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = GenesisFieldsFrame::new(transformed(original).frame);
            assert!(f.start(false));
            let actual = f.read_fixed(GenesisInputQuery::Event);
            actual.as_ref().unwrap();
            let pointer = f.fields.rows[0][0].payload.as_ref().unwrap().as_ptr();
            assert!(f.reads[0].eof && f.reads[0].scopes_ended && f.returns[0].is_none());
            let first = f.close_and_tail().unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail }
                if detail == "additive genesis close before inputs returned")
            );
            f.fail(first);
            f.returns[0] = Some(actual); // A real late return is retained after the barrier.
            assert_eq!(
                f.fields.rows[0][0].payload.as_ref().unwrap().as_ptr(),
                pointer
            );
            assert!(
                matches!(f.returns[0], Some(Ok(())))
                    && !f.reads[1].started
                    && f.relations_returned.is_none()
            );
            let used = f.owner.fee.local.readonly.loan().unwrap().2.metadata_used();
            assert!(!f.finish());
            assert_eq!(
                f.owner.fee.local.readonly.loan().unwrap().2.metadata_used(),
                used
            );
            assert!(!f.owner.fee.local.readonly.facts[1].original_tail_validated);
            drop(f);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = GenesisFieldsFrame::new(transformed(original).frame);
            assert!(f.start(false));
            assert!(f.advance_reads());
            f.validate_fields().unwrap();
            f.owner.fee.local.readonly.prepare_busy_vm();
            let first = f.close_and_tail().unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSqlite { operation: "close additive readonly", source }
                if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy))
            );
            f.fail(first);
            assert!(
                f.owner.fee.local.readonly.reader.is_some()
                    && f.fields.rows[0][0].payload.is_some()
            );
            let used = f.owner.fee.local.readonly.loan().unwrap().2.metadata_used();
            assert!(!f.finish());
            assert_eq!(
                f.owner.fee.local.readonly.loan().unwrap().2.metadata_used(),
                used
            );
            assert!(!f.owner.fee.local.readonly.facts[1].original_tail_validated);
            assert!(f.owner.fee.local.readonly.finalize_busy_once());
            f.owner.fee.local.readonly.cleanup_reader_once();
            assert!(matches!(
                f.owner.fee.local.readonly.cleanup_close,
                Some(Ok(()))
            ));
            assert!(
                f.fields.rows[1][0].projection_bytes.is_some()
                    && !f.owner.fee.local.readonly.facts[1].original_tail_validated
            );
            drop(f);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = GenesisFieldsFrame::new(transformed(original).frame);
            assert!(f.start(false));
            let first = f.close_and_tail().unwrap_err();
            f.fail(first);
            assert!(
                f.reads.iter().all(|r| !r.started && r.returned.is_none())
                    && f.returns.iter().all(Option::is_none)
            );
            assert!(
                !f.finish()
                    && f.fields.rows.iter().all(Vec::is_empty)
                    && !f.owner.fee.local.readonly.facts[1].original_tail_validated
            );
            drop(f);
        });
    }
}

// Ordinary fixed V1/audit material; no replay/hash/audit validity is inferred.
// Null is an observed cell, None an unacquired cell. REAL stores returned bits.
#[cfg_attr(test, derive(Clone, Debug, PartialEq, Eq))]
enum V1AuditInputCell {
    Text(String),
    Integer(i64),
    RealBits(u64),
    Null,
}
#[cfg_attr(test, derive(Clone, Debug, PartialEq, Eq))]
struct V1AuditInputRow {
    cells: [Option<V1AuditInputCell>; 13],
}
impl Default for V1AuditInputRow {
    fn default() -> Self {
        Self {
            cells: std::array::from_fn(|_| None),
        }
    }
}
impl V1AuditInputRow {
    fn text(&self, index: usize) -> StorageResult<&str> {
        match &self.cells[index] {
            Some(V1AuditInputCell::Text(v)) => Ok(v),
            _ => Err(storage_fail("additive V1/audit owned text absent")),
        }
    }
    fn integer(&self, index: usize) -> StorageResult<i64> {
        match &self.cells[index] {
            Some(V1AuditInputCell::Integer(v)) => Ok(*v),
            _ => Err(storage_fail("additive V1/audit owned integer absent")),
        }
    }
}
#[derive(Default)]
struct V1AuditInputReadFacts {
    started: bool,
    count: Option<i64>,
    types_checked: bool,
    extent: Option<u64>,
    charged: bool,
    acquired: usize,
    eof: bool,
    scopes_ended: bool,
    returned: Option<bool>,
}
struct V1AuditInputFields {
    rows: [Vec<V1AuditInputRow>; 5],
}
impl Default for V1AuditInputFields {
    fn default() -> Self {
        Self {
            rows: std::array::from_fn(|_| Vec::new()),
        }
    }
}
#[derive(Clone, Copy)]
enum V1AuditInputColumn {
    Text,
    NullableText,
    Integer,
    NullableInteger,
    Real,
    NullableReal,
}
#[derive(Clone, Copy)]
enum V1AuditInputQuery {
    Accounts,
    Events,
    Heads,
    Audits,
    Chain,
}
impl V1AuditInputQuery {
    const ALL: [Self; 5] = [
        Self::Accounts,
        Self::Events,
        Self::Heads,
        Self::Audits,
        Self::Chain,
    ];
    fn slot(self) -> usize {
        match self {
            Self::Accounts => 0,
            Self::Events => 1,
            Self::Heads => 2,
            Self::Audits => 3,
            Self::Chain => 4,
        }
    }
    fn columns(self) -> &'static [V1AuditInputColumn] {
        use V1AuditInputColumn::*;
        match self {
            Self::Accounts => &[Text, Text, Text, Text],
            Self::Events => &[
                Text,
                Integer,
                Text,
                Text,
                Text,
                Text,
                NullableText,
                NullableText,
                Integer,
                NullableInteger,
                NullableInteger,
            ],
            Self::Heads => &[Text, Integer, Text, Text, Text],
            Self::Audits => &[
                Integer,
                Text,
                Text,
                Text,
                Text,
                Text,
                Real,
                NullableReal,
                Integer,
                NullableText,
                Text,
                NullableText,
                Text,
            ],
            Self::Chain => &[Integer, Text, Text],
        }
    }
    fn sql(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::Accounts => (
                "SELECT COUNT(*) FROM main.paper_ledger_account",
                "SELECT COUNT(*) FROM main.paper_ledger_account WHERE typeof(account_id)!='text' OR typeof(epoch_id)!='text' OR typeof(manifest_hash)!='text' OR typeof(manifest_bytes)!='text'",
                "SELECT coalesce(sum(length(CAST(account_id AS BLOB))+length(CAST(epoch_id AS BLOB))+length(CAST(manifest_hash AS BLOB))+length(CAST(manifest_bytes AS BLOB))),0) FROM main.paper_ledger_account",
                "SELECT account_id,epoch_id,manifest_hash,manifest_bytes FROM main.paper_ledger_account ORDER BY account_id",
            ),
            Self::Events => (
                "SELECT COUNT(*) FROM main.paper_ledger_event",
                "SELECT COUNT(*) FROM main.paper_ledger_event WHERE typeof(account_id)!='text' OR typeof(seq)!='integer' OR typeof(command_id)!='text' OR typeof(previous_hash)!='text' OR typeof(event_hash)!='text' OR typeof(payload)!='text' OR typeof(business_plan_id) NOT IN ('null','text') OR typeof(intent_hash) NOT IN ('null','text') OR typeof(is_terminal)!='integer' OR typeof(paper_trade_id) NOT IN ('null','integer') OR typeof(order_audit_id) NOT IN ('null','integer')",
                "SELECT coalesce(sum(length(CAST(account_id AS BLOB))+length(CAST(command_id AS BLOB))+length(CAST(previous_hash AS BLOB))+length(CAST(event_hash AS BLOB))+length(CAST(payload AS BLOB))+coalesce(length(CAST(business_plan_id AS BLOB)),0)+coalesce(length(CAST(intent_hash AS BLOB)),0)),0) FROM main.paper_ledger_event",
                "SELECT account_id,seq,command_id,previous_hash,event_hash,payload,business_plan_id,intent_hash,is_terminal,paper_trade_id,order_audit_id FROM main.paper_ledger_event ORDER BY account_id,seq",
            ),
            Self::Heads => (
                "SELECT COUNT(*) FROM main.paper_ledger_head",
                "SELECT COUNT(*) FROM main.paper_ledger_head WHERE typeof(account_id)!='text' OR typeof(version)!='integer' OR typeof(event_hash)!='text' OR typeof(projection_bytes)!='text' OR typeof(projection_hash)!='text'",
                "SELECT coalesce(sum(length(CAST(account_id AS BLOB))+length(CAST(event_hash AS BLOB))+length(CAST(projection_bytes AS BLOB))+length(CAST(projection_hash AS BLOB))),0) FROM main.paper_ledger_head",
                "SELECT account_id,version,event_hash,projection_bytes,projection_hash FROM main.paper_ledger_head ORDER BY account_id",
            ),
            Self::Audits => (
                "SELECT COUNT(*) FROM main.order_audit",
                "SELECT COUNT(*) FROM main.order_audit WHERE typeof(id)!='integer' OR typeof(business_order_id)!='text' OR typeof(source)!='text' OR typeof(decision_basis)!='text' OR typeof(side)!='text' OR typeof(code)!='text' OR typeof(requested_price)!='real' OR typeof(execution_price) NOT IN ('null','real') OR typeof(quantity)!='integer' OR typeof(quote_observed_at) NOT IN ('null','text') OR typeof(outcome)!='text' OR typeof(failure_reason) NOT IN ('null','text') OR typeof(created_at)!='text'",
                "SELECT coalesce(sum(length(CAST(business_order_id AS BLOB))+length(CAST(source AS BLOB))+length(CAST(decision_basis AS BLOB))+length(CAST(side AS BLOB))+length(CAST(code AS BLOB))+coalesce(length(CAST(quote_observed_at AS BLOB)),0)+length(CAST(outcome AS BLOB))+coalesce(length(CAST(failure_reason AS BLOB)),0)+length(CAST(created_at AS BLOB))),0) FROM main.order_audit",
                "SELECT id,business_order_id,source,decision_basis,side,code,requested_price,execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at FROM main.order_audit ORDER BY id",
            ),
            Self::Chain => (
                "SELECT COUNT(*) FROM main.order_audit_chain",
                "SELECT COUNT(*) FROM main.order_audit_chain WHERE typeof(order_audit_id)!='integer' OR typeof(previous_hash)!='text' OR typeof(record_hash)!='text'",
                "SELECT coalesce(sum(length(CAST(previous_hash AS BLOB))+length(CAST(record_hash AS BLOB))),0) FROM main.order_audit_chain",
                "SELECT order_audit_id,previous_hash,record_hash FROM main.order_audit_chain ORDER BY order_audit_id",
            ),
        }
    }
}
#[derive(PartialEq, Eq)]
enum V1AuditInputPhase {
    Fresh,
    ReaderChecked,
    RowsReturned,
    InputsChecked,
    Complete,
    Refused,
}
struct V1AuditInputsFrame {
    genesis: GenesisFieldsFrame,
    phase: V1AuditInputPhase,
    fields: V1AuditInputFields,
    reads: [V1AuditInputReadFacts; 5],
    returns: [Option<StorageResult<()>>; 5],
    rosters_returned: Option<bool>,
}
pub(super) struct AdditiveStorageRetainedV1AuditInputs {
    frame: V1AuditInputsFrame,
}
pub(super) struct AdditiveStorageV1AuditInputsHeld {
    frame: V1AuditInputsFrame,
}
impl AdditiveStorageV1AuditInputsHeld {
    pub(super) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame
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
    pub(super) fn into_retained_v1_audit_inputs(
        self,
    ) -> std::result::Result<AdditiveStorageRetainedV1AuditInputs, AdditiveStorageV1AuditInputsHeld>
    {
        V1AuditInputsFrame::new(self.frame).run(false)
    }
}
impl AdditiveStorageRetainedV1AuditInputs {
    pub(super) fn create_or_resume(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageV1AuditInputsHeld> {
        let base = AdditiveStorageCopied {
            source,
            managed: None,
            directory: None,
            anchor: None,
            fresh: false,
            original: None,
            rows: None,
            records: std::array::from_fn(|_| None),
            pending: None,
            target: None,
            target_node: None,
            copied: None,
            census_files: std::array::from_fn(|_| None),
            codec: AdditiveRecordCodecState::new(),
            copy_issued: false,
            rejected_copy_return: None,
            copy_return_failed: false,
            copy_return_error: None,
            copy_origin_return_error: None,
        };
        V1AuditInputsFrame::new(TransformFrame::new(base)).run(true)
    }
}
impl V1AuditInputsFrame {
    fn new(transform: TransformFrame) -> Self {
        Self {
            genesis: GenesisFieldsFrame::new(transform),
            phase: V1AuditInputPhase::Fresh,
            fields: V1AuditInputFields::default(),
            reads: std::array::from_fn(|_| V1AuditInputReadFacts::default()),
            returns: std::array::from_fn(|_| None),
            rosters_returned: None,
        }
    }
    fn fail(&mut self, first: GlobalSchemaV1Error) {
        self.genesis.fail(first);
        self.phase = V1AuditInputPhase::Refused;
    }
    fn run(
        mut self,
        cold: bool,
    ) -> std::result::Result<AdditiveStorageRetainedV1AuditInputs, AdditiveStorageV1AuditInputsHeld>
    {
        if !self.start(cold) || !self.finish() {
            return Err(AdditiveStorageV1AuditInputsHeld { frame: self });
        }
        Ok(AdditiveStorageRetainedV1AuditInputs { frame: self })
    }
    fn start(&mut self, cold: bool) -> bool {
        if self
            .genesis
            .owner
            .fee
            .local
            .readonly
            .transform
            .first
            .is_some()
        {
            return false;
        }
        if self.phase != V1AuditInputPhase::Fresh {
            self.fail(storage_fail("additive V1/audit start phase differs"));
            return false;
        }
        if !self.genesis.start(cold) || !self.genesis.advance_reads() {
            self.phase = V1AuditInputPhase::Refused;
            return false;
        }
        match self.genesis.validate_fields() {
            Ok(()) => {
                self.phase = V1AuditInputPhase::ReaderChecked;
                true
            }
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
    fn read_fixed(&mut self, query: V1AuditInputQuery) -> StorageResult<()> {
        if self
            .genesis
            .owner
            .fee
            .local
            .readonly
            .transform
            .first
            .is_some()
        {
            return Err(storage_fail("additive V1/audit read after first error"));
        }
        if self.phase != V1AuditInputPhase::ReaderChecked
            || self.genesis.phase != GenesisInputPhase::InputsChecked
            || self.genesis.relations_returned != Some(true)
            || !self.genesis.all_returns()
            || self.genesis.owner.fee.local.readonly.active != Some(1)
        {
            return Err(storage_fail(
                "additive V1/audit lacks validated second reader",
            ));
        }
        let Retained8Frame {
            transform,
            permit,
            reader,
            ..
        } = &mut self.genesis.owner.fee.local.readonly;
        let (_, _, work, completed) = transform
            .base
            .source
            .retained8_parts(permit.as_ref().unwrap())?;
        if completed != 2 {
            return Err(storage_fail("additive V1/audit pair count differs"));
        }
        V1AuditInputReadLoan {
            connection: reader.as_ref().unwrap(),
            work,
            rows: &mut self.fields.rows[query.slot()],
            facts: &mut self.reads[query.slot()],
        }
        .read(query)
    }
    fn advance_reads(&mut self) -> bool {
        if self
            .genesis
            .owner
            .fee
            .local
            .readonly
            .transform
            .first
            .is_some()
        {
            return false;
        }
        if self.phase != V1AuditInputPhase::ReaderChecked {
            self.fail(storage_fail("additive V1/audit read phase differs"));
            return false;
        }
        for query in V1AuditInputQuery::ALL {
            let i = query.slot();
            if self.returns[i].is_some() || self.reads[i].started {
                self.fail(storage_fail("additive V1/audit query already reached"));
                return false;
            }
            // Children already live in this owning frame; park the actual Result
            // before inspecting it, and move only its first owned error onward.
            self.returns[i] = Some(self.read_fixed(query));
            if self.returns[i].as_ref().unwrap().is_err() {
                let first = self.returns[i].take().unwrap().unwrap_err();
                self.fail(first);
                return false;
            }
        }
        self.phase = V1AuditInputPhase::RowsReturned;
        true
    }
    fn all_returns(&self) -> bool {
        self.returns.iter().all(|r| matches!(r, Some(Ok(()))))
            && self.reads.iter().all(|r| {
                r.returned == Some(true) && r.eof && r.scopes_ended && r.types_checked && r.charged
            })
    }
    fn validate_fields(&mut self) -> StorageResult<()> {
        if self
            .genesis
            .owner
            .fee
            .local
            .readonly
            .transform
            .first
            .is_some()
        {
            return Err(storage_fail(
                "additive V1/audit validation after first error",
            ));
        }
        if self.phase != V1AuditInputPhase::RowsReturned
            || !self.all_returns()
            || self.rosters_returned.is_some()
        {
            return Err(storage_fail(
                "additive V1/audit inputs before whole returns",
            ));
        }
        self.genesis
            .owner
            .fee
            .local
            .readonly
            .loan()?
            .2
            .metadata(1024)?;
        let actual = v1_audit_input_rosters(&self.genesis.owner.fields.rows[0], &self.fields);
        self.rosters_returned = Some(actual.is_ok());
        actual?;
        self.phase = V1AuditInputPhase::InputsChecked;
        Ok(())
    }
    fn close_and_tail(&mut self) -> StorageResult<()> {
        if self
            .genesis
            .owner
            .fee
            .local
            .readonly
            .transform
            .first
            .is_some()
        {
            return Err(storage_fail("additive V1/audit close after first error"));
        }
        if self.phase != V1AuditInputPhase::InputsChecked
            || self.rosters_returned != Some(true)
            || !self.all_returns()
        {
            return Err(storage_fail(
                "additive V1/audit close before inputs returned",
            ));
        }
        self.genesis.close_and_tail()?;
        self.phase = V1AuditInputPhase::Complete;
        Ok(())
    }
    fn finish(&mut self) -> bool {
        if self
            .genesis
            .owner
            .fee
            .local
            .readonly
            .transform
            .first
            .is_some()
        {
            return false;
        }
        if !self.advance_reads() {
            return false;
        }
        let actual = self.validate_fields().and_then(|()| self.close_and_tail());
        match actual {
            Ok(()) => true,
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
}
struct V1AuditInputReadLoan<'a> {
    connection: &'a Connection,
    work: &'a mut target::TargetWork,
    rows: &'a mut Vec<V1AuditInputRow>,
    facts: &'a mut V1AuditInputReadFacts,
}
impl V1AuditInputReadLoan<'_> {
    fn read(mut self, query: V1AuditInputQuery) -> StorageResult<()> {
        if let Err(first) = self.preflight(query) {
            self.facts.scopes_ended = true;
            self.facts.returned = Some(false);
            return Err(first);
        }
        self.acquire(query)
    }
    fn preflight(&mut self, query: V1AuditInputQuery) -> StorageResult<()> {
        if self.facts.started || !self.rows.is_empty() {
            return Err(storage_fail("additive V1/audit query already started"));
        }
        let (count_sql, type_sql, extent_sql, fields_sql) = query.sql();
        self.work.metadata(
            4096 + (count_sql.len() + type_sql.len() + extent_sql.len() + fields_sql.len()) as u64,
        )?;
        self.facts.started = true;
        let count: i64 = self
            .connection
            .query_row(count_sql, [], |r| r.get(0))
            .map_err(|e| transform_sql_error("count additive V1/audit rows", e))?;
        self.facts.count = Some(count);
        let count =
            u64::try_from(count).map_err(|_| storage_fail("additive V1/audit count overflow"))?;
        let invalid: i64 = self
            .connection
            .query_row(type_sql, [], |r| r.get(0))
            .map_err(|e| transform_sql_error("type additive V1/audit rows", e))?;
        if invalid != 0 {
            return Err(storage_fail("additive V1/audit row types differ"));
        }
        self.facts.types_checked = true;
        let extent: i64 = self
            .connection
            .query_row(extent_sql, [], |r| r.get(0))
            .map_err(|e| transform_sql_error("extent additive V1/audit rows", e))?;
        let extent =
            u64::try_from(extent).map_err(|_| storage_fail("additive V1/audit extent overflow"))?;
        self.facts.extent = Some(extent);
        let slots = count
            .checked_mul(std::mem::size_of::<V1AuditInputRow>() as u64)
            .ok_or_else(|| storage_fail("additive V1/audit capacity overflow"))?;
        self.work.metadata(
            slots
                .checked_add(extent)
                .ok_or_else(|| storage_fail("additive V1/audit capacity overflow"))?,
        )?;
        self.facts.charged = true;
        Ok(())
    }
    fn acquire(self, query: V1AuditInputQuery) -> StorageResult<()> {
        let actual = (|| {
            if !self.facts.started
                || !self.facts.charged
                || self.facts.returned.is_some()
                || !self.rows.is_empty()
            {
                return Err(storage_fail(
                    "additive V1/audit acquire lacks fixed preflight",
                ));
            }
            let count = usize::try_from(self.facts.count.unwrap())
                .map_err(|_| storage_fail("additive V1/audit count overflow"))?;
            let mut remaining = self.facts.extent.unwrap();
            self.rows
                .try_reserve_exact(count)
                .map_err(|_| storage_fail("additive V1/audit row allocation failed"))?;
            let mut statement = self
                .connection
                .prepare(query.sql().3)
                .map_err(|e| transform_sql_error("prepare additive V1/audit rows", e))?;
            let mut rows = statement
                .query([])
                .map_err(|e| transform_sql_error("query additive V1/audit rows", e))?;
            while let Some(row) = rows
                .next()
                .map_err(|e| transform_sql_error("step additive V1/audit rows", e))?
            {
                if self.rows.len() == count {
                    return Err(storage_fail("additive V1/audit extra row"));
                }
                self.rows.push(V1AuditInputRow::default());
                self.facts.acquired = self.rows.len();
                let slot = self.rows.last_mut().unwrap();
                for (index, column) in query.columns().iter().enumerate() {
                    // A successful actual field return enters its slot immediately.
                    slot.cells[index] =
                        Some(v1_audit_input_cell(row, index, *column, &mut remaining)?);
                }
            }
            self.facts.eof = true;
            if self.rows.len() != count || remaining != 0 {
                return Err(storage_fail("additive V1/audit count/extent changed"));
            }
            Ok(())
        })();
        // Both lexical driver scopes ended, even on Err; ignored Drop results
        // remain ignored, independently of the actual owning whole return.
        self.facts.scopes_ended = true;
        self.facts.returned = Some(actual.is_ok());
        actual
    }
}
fn v1_audit_input_cell(
    row: &rusqlite::Row<'_>,
    index: usize,
    column: V1AuditInputColumn,
    remaining: &mut u64,
) -> StorageResult<V1AuditInputCell> {
    use rusqlite::types::ValueRef;
    use V1AuditInputColumn::*;
    let raw = row
        .get_ref(index)
        .map_err(|e| transform_sql_error("read additive V1/audit field", e))?;
    match (column, raw) {
        (NullableText | NullableInteger | NullableReal, ValueRef::Null) => {
            Ok(V1AuditInputCell::Null)
        }
        (Text | NullableText, ValueRef::Text(_)) => {
            owner_linkage_text(row, index, remaining).map(V1AuditInputCell::Text)
        }
        (Integer | NullableInteger, ValueRef::Integer(v)) => Ok(V1AuditInputCell::Integer(v)),
        (Real | NullableReal, ValueRef::Real(v)) => Ok(V1AuditInputCell::RealBits(v.to_bits())),
        _ => Err(storage_fail("additive V1/audit field type changed")),
    }
}
fn v1_audit_input_rosters(
    old: &[OwnerLinkageRow],
    fields: &V1AuditInputFields,
) -> StorageResult<()> {
    let [accounts, events, heads, audits, chain] = &fields.rows;
    for (slot, query) in V1AuditInputQuery::ALL.iter().enumerate() {
        if fields.rows[slot]
            .iter()
            .any(|r| r.cells[..query.columns().len()].iter().any(Option::is_none))
        {
            return Err(storage_fail("additive V1/audit unacquired field"));
        }
    }
    if accounts
        .windows(2)
        .any(|w| w[0].text(0).unwrap() >= w[1].text(0).unwrap())
        || accounts.len() != old.len()
        || accounts.iter().zip(old).any(|(a, b)| {
            a.text(0).unwrap() != b.account_id.as_deref().unwrap()
                || a.text(1).unwrap() != b.epoch_id.as_deref().unwrap()
                || a.text(2).unwrap() != b.manifest_hash.as_deref().unwrap()
        })
    {
        return Err(storage_fail("additive V1/audit account roster differs"));
    }
    if events.windows(2).any(|w| {
        (w[0].text(0).unwrap(), w[0].integer(1).unwrap())
            >= (w[1].text(0).unwrap(), w[1].integer(1).unwrap())
    }) {
        return Err(storage_fail(
            "additive V1/audit duplicate or unordered event key",
        ));
    }
    if heads
        .windows(2)
        .any(|w| w[0].text(0).unwrap() >= w[1].text(0).unwrap())
        || heads.len() != accounts.len()
        || heads
            .iter()
            .zip(accounts)
            .any(|(h, a)| h.text(0).unwrap() != a.text(0).unwrap())
        || events.iter().any(|e| {
            accounts
                .binary_search_by(|a| a.text(0).unwrap().cmp(e.text(0).unwrap()))
                .is_err()
        })
        || accounts.iter().any(|a| {
            !events
                .iter()
                .any(|e| e.text(0).unwrap() == a.text(0).unwrap())
        })
    {
        return Err(storage_fail("additive V1/audit event/head roster differs"));
    }
    if audits
        .windows(2)
        .any(|w| w[0].integer(0).unwrap() >= w[1].integer(0).unwrap())
        || chain
            .windows(2)
            .any(|w| w[0].integer(0).unwrap() >= w[1].integer(0).unwrap())
        || audits.len() != chain.len()
        || audits
            .iter()
            .zip(chain)
            .any(|(a, c)| a.integer(0).unwrap() != c.integer(0).unwrap())
    {
        return Err(storage_fail("additive V1/audit id roster differs"));
    }
    // These are raw field associations only: no JSON/hash, economic replay,
    // audit-chain validation, verified snapshot or layout/provider is issued.
    Ok(())
}

#[cfg(test)]
mod retained_v1_audit_input_tests {
    use super::*;
    fn transformed(
        original: rows::VerifiedUnapprovedOriginalRowsBackup,
    ) -> AdditiveStorageTransformed {
        let copied =
            match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
                Ok(owner) => owner,
                Err(held) => panic!("V1/audit real Copied: {}", held.first_error()),
            };
        match copied.into_transformed() {
            Ok(owner) => owner,
            Err(held) => panic!("V1/audit real WAL: {}", held.first_error()),
        }
    }
    fn complete(f: &mut V1AuditInputsFrame) {
        assert!(
            f.phase == V1AuditInputPhase::Complete
                && f.genesis.phase == GenesisInputPhase::Complete
        );
        assert!(f.all_returns() && f.rosters_returned == Some(true));
        assert!(f.reads.iter().all(|r| r.started
            && r.charged
            && r.eof
            && r.scopes_ended
            && r.returned == Some(true)));
        assert!(f.fields.rows.iter().all(|r| !r.is_empty()));
        assert!(f.fields.rows[1].len() > f.fields.rows[0].len()); // Actual V1 multi-event rows, separate from Genesis.
        assert!(f.fields.rows[1]
            .iter()
            .any(|r| matches!(&r.cells[6], Some(V1AuditInputCell::Null))));
        assert!(f.fields.rows[1]
            .iter()
            .any(|r| matches!(&r.cells[6], Some(V1AuditInputCell::Text(_)))));
        assert!(
            matches!(&f.fields.rows[3][0].cells[6], Some(V1AuditInputCell::RealBits(v)) if *v == 10.0_f64.to_bits())
        );
        assert!(
            matches!(&f.fields.rows[3][0].cells[7], Some(V1AuditInputCell::RealBits(v)) if *v == 10.0_f64.to_bits())
        );
        assert!(
            f.genesis.owner.fee.local.readonly.reader.is_none()
                && f.genesis.owner.fee.local.readonly.active.is_none()
        );
        assert!(f
            .genesis
            .owner
            .fee
            .local
            .readonly
            .facts
            .iter()
            .all(|r| r.closed && r.original_tail_validated));
        assert_eq!(f.genesis.owner.fee.local.readonly.loan().unwrap().3, 2);
    }
    #[test]
    fn task6_retained_v1_audit_input_nonempty_same_reader_and_cold() {
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = transformed(original);
                let fd = owner.frame.base.target().unwrap().as_raw_fd();
                let mut retained = match owner.into_retained_v1_audit_inputs() {
                    Ok(owner) => owner,
                    Err(held) => panic!("V1/audit warm: {}", held.first_error()),
                };
                complete(&mut retained.frame);
                let base = &retained
                    .frame
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .base;
                assert_eq!(base.target().unwrap().as_raw_fd(), fd);
                let saved = (
                    base.target_node.unwrap(),
                    base.records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                    retained.frame.fields.rows.clone(),
                ); // Owned test snapshot only, never a cap/payment.
                drop(retained);
                saved
            },
            |(node, records, fields), original| {
                let mut retained = match AdditiveStorageRetainedV1AuditInputs::create_or_resume(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("V1/audit cold6: {}", held.first_error()),
                };
                complete(&mut retained.frame);
                assert_eq!(retained.frame.fields.rows, fields);
                assert!(retained
                    .frame
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .begin_return
                    .is_none());
                let base = &retained
                    .frame
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .base;
                assert_eq!(base.target_node, Some(node));
                assert_eq!(
                    base.records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                    records
                );
                drop(retained);
            },
        );
    }
    fn gate_connection() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE paper_ledger_account(account_id,epoch_id,manifest_hash,manifest_bytes);
            CREATE TABLE paper_ledger_event(account_id,seq,command_id,previous_hash,event_hash,payload,business_plan_id,intent_hash,is_terminal,paper_trade_id,order_audit_id);
            CREATE TABLE paper_ledger_head(account_id,version,event_hash,projection_bytes,projection_hash);
            CREATE TABLE order_audit(id,business_order_id,source,decision_basis,side,code,requested_price,execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at);
            CREATE TABLE order_audit_chain(order_audit_id,previous_hash,record_hash);
            INSERT INTO paper_ledger_account VALUES('a','epoch','manifest','{}');
            INSERT INTO paper_ledger_event VALUES('a',1,'seed','previous','event1','{}',NULL,NULL,0,NULL,NULL),
                ('a',2,'order','event1','event2','{}','plan','intent',1,1,1);
            INSERT INTO paper_ledger_head VALUES('a',2,'event2','{}','projection');
            INSERT INTO order_audit VALUES(1,'order','source','basis','buy','code',10.0,10.0,100,NULL,'Filled',NULL,'time');
            INSERT INTO order_audit_chain VALUES(1,'previous','record');").unwrap();
        c
    }
    fn gate_roster() -> Vec<OwnerLinkageRow> {
        vec![OwnerLinkageRow {
            account_id: Some("a".into()),
            epoch_id: Some("epoch".into()),
            manifest_hash: Some("manifest".into()),
            ..OwnerLinkageRow::default()
        }]
    }
    fn read_gate(
        c: &Connection,
        work: &mut target::TargetWork,
        fields: &mut V1AuditInputFields,
        facts: &mut [V1AuditInputReadFacts; 5],
    ) -> StorageResult<()> {
        for query in V1AuditInputQuery::ALL {
            V1AuditInputReadLoan {
                connection: c,
                work: &mut *work,
                rows: &mut fields.rows[query.slot()],
                facts: &mut facts[query.slot()],
            }
            .read(query)?;
        }
        work.metadata(1024)?;
        v1_audit_input_rosters(&gate_roster(), fields)
    }
    #[test]
    fn task6_retained_v1_audit_input_typed_nullable_extent_and_drift() {
        // These memory databases test fixed SQL cells only, not retained issuer/replay.
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut source = original.into_additive_target_source().unwrap();
            for case in [
                "plain",
                "null_real",
                "raw_gap",
                "type",
                "nullable_type",
                "real_type",
                "utf8",
                "missing",
                "foreign",
                "duplicate",
                "chain",
            ] {
                let c = gate_connection();
                c.execute_batch(match case {
                    "null_real" => "UPDATE order_audit SET execution_price=NULL;",
                    "raw_gap" => "UPDATE paper_ledger_event SET seq=5 WHERE seq=2;",
                    "type" => "UPDATE paper_ledger_account SET manifest_bytes=X'0102';",
                    "nullable_type" => "UPDATE paper_ledger_event SET business_plan_id=7 WHERE seq=2;",
                    "real_type" => "UPDATE order_audit SET requested_price=X'01';",
                    "utf8" => "UPDATE paper_ledger_event SET command_id=CAST(X'ff' AS TEXT) WHERE seq=1;",
                    "missing" => "DELETE FROM paper_ledger_head;",
                    "foreign" => "UPDATE paper_ledger_event SET account_id='foreign';",
                    "duplicate" => "INSERT INTO paper_ledger_event SELECT * FROM paper_ledger_event WHERE seq=1;",
                    "chain" => "UPDATE order_audit_chain SET order_audit_id=2;", _ => "",
                }).unwrap();
                let mut fields = V1AuditInputFields::default();
                let mut facts: [V1AuditInputReadFacts; 5] =
                    std::array::from_fn(|_| V1AuditInputReadFacts::default());
                let actual = read_gate(
                    &c,
                    source.storage_parts().unwrap().2,
                    &mut fields,
                    &mut facts,
                );
                if matches!(case, "plain" | "null_real" | "raw_gap") {
                    actual.unwrap();
                    assert!(facts
                        .iter()
                        .all(|r| r.eof && r.scopes_ended && r.returned == Some(true)));
                    assert_eq!(fields.rows[1].len(), 2); // Does not validate the economic/event chain.
                    if case == "null_real" {
                        assert!(matches!(
                            &fields.rows[3][0].cells[7],
                            Some(V1AuditInputCell::Null)
                        ));
                    } else {
                        assert!(
                            matches!(&fields.rows[3][0].cells[7], Some(V1AuditInputCell::RealBits(v)) if *v == 10.0_f64.to_bits())
                        );
                    }
                    if case == "raw_gap" {
                        assert_eq!(fields.rows[1][1].integer(1).unwrap(), 5);
                    }
                } else {
                    let expected = match case {
                        "type" | "nullable_type" | "real_type" => {
                            "additive V1/audit row types differ"
                        }
                        "utf8" => "additive owner text is not UTF8",
                        "duplicate" => "additive V1/audit duplicate or unordered event key",
                        "chain" => "additive V1/audit id roster differs",
                        _ => "additive V1/audit event/head roster differs",
                    };
                    assert!(
                        matches!(actual.unwrap_err(), GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == expected)
                    );
                    if case == "type" {
                        assert!(fields.rows[0].is_empty() && !facts[0].charged);
                    }
                    if case == "nullable_type" {
                        assert!(
                            !fields.rows[0].is_empty()
                                && fields.rows[1].is_empty()
                                && !facts[1].charged
                        );
                    }
                    if case == "real_type" {
                        assert!(
                            !fields.rows[2].is_empty()
                                && fields.rows[3].is_empty()
                                && !facts[3].charged
                        );
                    }
                    if case == "utf8" {
                        assert!(
                            facts[1].charged
                                && fields.rows[1][0].cells[0].is_some()
                                && fields.rows[1][0].cells[1].is_some()
                                && fields.rows[1][0].cells[2].is_none()
                        );
                    }
                }
                c.close().unwrap();
            }
            for change in ["grow", "shrink", "extra"] {
                let c = gate_connection();
                let mut rows = Vec::new();
                let mut facts = V1AuditInputReadFacts::default();
                let mut loan = V1AuditInputReadLoan {
                    connection: &c,
                    work: source.storage_parts().unwrap().2,
                    rows: &mut rows,
                    facts: &mut facts,
                };
                loan.preflight(V1AuditInputQuery::Events).unwrap();
                c.execute_batch(match change {
                    "grow" => "UPDATE paper_ledger_event SET payload='longer' WHERE seq=2;",
                    "shrink" => "UPDATE paper_ledger_event SET payload='' WHERE seq=2;",
                    _ => "INSERT INTO paper_ledger_event VALUES('a',3,'extra','event2','event3','{}',NULL,NULL,0,NULL,NULL);",
                }).unwrap();
                let first = loan.acquire(V1AuditInputQuery::Events).unwrap_err();
                let expected = match change {
                    "grow" => "additive owner field extent changed",
                    "shrink" => "additive V1/audit count/extent changed",
                    _ => "additive V1/audit extra row",
                };
                assert!(
                    matches!(first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == expected)
                );
                assert!(facts.scopes_ended && facts.returned == Some(false));
                assert_eq!(facts.eof, change == "shrink");
                assert!(rows[0].cells[..11].iter().all(Option::is_some));
                c.close().unwrap();
            }
            drop(source);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = V1AuditInputsFrame::new(transformed(original).frame);
            assert!(f.start(false));
            assert!(f.advance_reads());
            f.validate_fields().unwrap();
            let file = f
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .base
                .target()
                .unwrap();
            let mut byte = [0];
            file.read_exact_at(&mut byte, 100).unwrap();
            file.write_all_at(&[byte[0] ^ 1], 100).unwrap();
            file.sync_all().unwrap();
            let actual = f.close_and_tail();
            let file = f
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .base
                .target()
                .unwrap();
            file.write_all_at(&byte, 100).unwrap();
            file.sync_all().unwrap(); // Cleanup only.
            let first = actual.unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == "additive readonly retained target bytes changed")
            );
            assert!(
                f.genesis.owner.fee.local.readonly.reader.is_none()
                    && f.genesis.owner.fee.local.readonly.facts[1].closed
                    && f.genesis.owner.fee.local.readonly.facts[1].original_tail_validated
            );
            f.fail(first);
            let primary = f
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .first
                .as_ref()
                .unwrap() as *const GlobalSchemaV1Error;
            let used = f
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .unwrap()
                .2
                .metadata_used();
            assert!(!f.finish() && f.phase == V1AuditInputPhase::Refused);
            assert_eq!(
                f.genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .first
                    .as_ref()
                    .unwrap() as *const GlobalSchemaV1Error,
                primary
            );
            assert_eq!(
                f.genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .loan()
                    .unwrap()
                    .2
                    .metadata_used(),
                used
            );
            assert!(
                f.fields.rows.iter().all(|r| !r.is_empty())
                    && f.genesis.owner.fee.local.phase != LocalCompletionPhase::Complete
            );
            drop(f);
        });
    }
    #[test]
    fn task6_retained_v1_audit_input_same_work_unknown_late_and_busy() {
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = V1AuditInputsFrame::new(transformed(original).frame);
            assert!(f.start(false));
            let work = f.genesis.owner.fee.local.readonly.loan().unwrap().2;
            let remaining = 16 * MIB - work.metadata_used();
            work.metadata(remaining).unwrap();
            assert!(
                !f.finish()
                    && f.fields.rows.iter().all(Vec::is_empty)
                    && f.reads.iter().all(|r| !r.started)
            );
            let primary = f
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .first
                .as_ref()
                .unwrap() as *const GlobalSchemaV1Error;
            let used = f
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .unwrap()
                .2
                .metadata_used();
            assert!(!f.finish());
            assert_eq!(
                f.genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .first
                    .as_ref()
                    .unwrap() as *const GlobalSchemaV1Error,
                primary
            );
            assert_eq!(
                f.genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .loan()
                    .unwrap()
                    .2
                    .metadata_used(),
                used
            );
            drop(f);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = V1AuditInputsFrame::new(transformed(original).frame);
            assert!(f.start(false));
            let actual = f.read_fixed(V1AuditInputQuery::Accounts);
            actual.as_ref().unwrap();
            let pointer = f.fields.rows[0][0].text(3).unwrap().as_ptr();
            assert!(f.reads[0].eof && f.reads[0].scopes_ended && f.returns[0].is_none());
            let first = f.close_and_tail().unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == "additive V1/audit close before inputs returned")
            );
            f.fail(first);
            f.returns[0] = Some(actual); // Retain this exact already reached owning return.
            assert_eq!(f.fields.rows[0][0].text(3).unwrap().as_ptr(), pointer);
            assert!(
                matches!(f.returns[0], Some(Ok(()))) && f.reads[1..].iter().all(|r| !r.started)
            );
            let used = f
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .unwrap()
                .2
                .metadata_used();
            assert!(!f.finish());
            assert_eq!(
                f.genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .loan()
                    .unwrap()
                    .2
                    .metadata_used(),
                used
            );
            assert!(!f.genesis.owner.fee.local.readonly.facts[1].original_tail_validated);
            drop(f);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = V1AuditInputsFrame::new(transformed(original).frame);
            assert!(f.start(false));
            assert!(f.advance_reads());
            f.validate_fields().unwrap();
            f.genesis.owner.fee.local.readonly.prepare_busy_vm();
            let first = f.close_and_tail().unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSqlite { operation: "close additive readonly", source }
                if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy))
            );
            f.fail(first);
            assert!(
                f.genesis.owner.fee.local.readonly.reader.is_some()
                    && f.fields.rows.iter().all(|r| !r.is_empty())
            );
            let used = f
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .unwrap()
                .2
                .metadata_used();
            assert!(!f.finish());
            assert_eq!(
                f.genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .loan()
                    .unwrap()
                    .2
                    .metadata_used(),
                used
            );
            assert!(!f.genesis.owner.fee.local.readonly.facts[1].original_tail_validated);
            assert!(f.genesis.owner.fee.local.readonly.finalize_busy_once());
            f.genesis.owner.fee.local.readonly.cleanup_reader_once();
            assert!(matches!(
                f.genesis.owner.fee.local.readonly.cleanup_close,
                Some(Ok(()))
            ));
            assert!(
                f.fields.rows.iter().all(|r| !r.is_empty())
                    && !f.genesis.owner.fee.local.readonly.facts[1].original_tail_validated
            );
            drop(f);
        });
    }
}

// Ordinary borrowed links only. The complete same owner remains retained;
// canonical content hashes, Fact/economic replay and audit validity are not checked.
#[derive(PartialEq, Eq)]
enum RawV1AuditLinksPhase {
    Fresh,
    InputsChecked,
    AuditStarted,
    AuditChecked,
    V1Started,
    LinksChecked,
    Complete,
    Refused,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum RawV1AuditGate {
    Audit,
    V1,
}
impl RawV1AuditGate {
    const ALL: [Self; 2] = [Self::Audit, Self::V1];
    fn slot(self) -> usize {
        match self {
            Self::Audit => 0,
            Self::V1 => 1,
        }
    }
    fn predecessor(self) -> RawV1AuditLinksPhase {
        match self {
            Self::Audit => RawV1AuditLinksPhase::InputsChecked,
            Self::V1 => RawV1AuditLinksPhase::AuditChecked,
        }
    }
    fn started(self) -> RawV1AuditLinksPhase {
        match self {
            Self::Audit => RawV1AuditLinksPhase::AuditStarted,
            Self::V1 => RawV1AuditLinksPhase::V1Started,
        }
    }
    fn checked(self) -> RawV1AuditLinksPhase {
        match self {
            Self::Audit => RawV1AuditLinksPhase::AuditChecked,
            Self::V1 => RawV1AuditLinksPhase::LinksChecked,
        }
    }
}
#[derive(Default)]
struct RawV1AuditGateFacts {
    started: bool,
    charged: bool,
    callee_reached: bool,
    callee_returned: Option<bool>,
    checked_rows: usize,
    checked_heads: usize,
    tail_row: Option<usize>,
    returned: Option<bool>,
}
#[derive(PartialEq, Eq)]
enum RawV1AuditContentHashes {
    NotChecked,
    AuditOnlyChecked,
    AuditAndV1EventProjectionChecked,
    AuditAndV1EventProjectionAndManifestChecked,
}
struct RawV1AuditLinksFrame {
    input: V1AuditInputsFrame,
    phase: RawV1AuditLinksPhase,
    gates: [RawV1AuditGateFacts; 2],
    returns: [Option<StorageResult<()>>; 2],
    unaccepted_return: Option<StorageResult<()>>,
    content_hashes: RawV1AuditContentHashes,
}
pub(super) struct AdditiveStorageLocalRawV1AuditLinksChecked {
    frame: RawV1AuditLinksFrame,
}
pub(super) struct AdditiveStorageRawV1AuditLinksHeld {
    frame: RawV1AuditLinksFrame,
}
impl AdditiveStorageRawV1AuditLinksHeld {
    pub(super) fn first_error(&self) -> &GlobalSchemaV1Error {
        self.frame
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
    pub(super) fn into_raw_v1_audit_links(
        self,
    ) -> std::result::Result<
        AdditiveStorageLocalRawV1AuditLinksChecked,
        AdditiveStorageRawV1AuditLinksHeld,
    > {
        RawV1AuditLinksFrame::new(self.frame).run(false)
    }
}
impl AdditiveStorageLocalRawV1AuditLinksChecked {
    pub(super) fn create_or_resume(
        source: rows::AdditiveRowsTargetSource,
    ) -> std::result::Result<Self, AdditiveStorageRawV1AuditLinksHeld> {
        raw_v1_audit_links_cold_frame(source).run(true)
    }
}
fn raw_v1_audit_links_cold_frame(source: rows::AdditiveRowsTargetSource) -> RawV1AuditLinksFrame {
    let base = AdditiveStorageCopied {
        source,
        managed: None,
        directory: None,
        anchor: None,
        fresh: false,
        original: None,
        rows: None,
        records: std::array::from_fn(|_| None),
        pending: None,
        target: None,
        target_node: None,
        copied: None,
        census_files: std::array::from_fn(|_| None),
        codec: AdditiveRecordCodecState::new(),
        copy_issued: false,
        rejected_copy_return: None,
        copy_return_failed: false,
        copy_return_error: None,
        copy_origin_return_error: None,
    };
    RawV1AuditLinksFrame::new(TransformFrame::new(base))
}

#[path = "global_schema_audit_content_v1.rs"]
mod audit_content;
impl RawV1AuditLinksFrame {
    fn new(transform: TransformFrame) -> Self {
        Self {
            input: V1AuditInputsFrame::new(transform),
            phase: RawV1AuditLinksPhase::Fresh,
            gates: std::array::from_fn(|_| RawV1AuditGateFacts::default()),
            returns: std::array::from_fn(|_| None),
            unaccepted_return: None,
            content_hashes: RawV1AuditContentHashes::NotChecked,
        }
    }
    fn first(&self) -> bool {
        self.input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .transform
            .first
            .is_some()
    }
    fn fail(&mut self, first: GlobalSchemaV1Error) {
        self.input.fail(first);
        self.phase = RawV1AuditLinksPhase::Refused;
    }
    fn prepare(&mut self, cold: bool) -> bool {
        if self.first() {
            return false;
        }
        if self.phase != RawV1AuditLinksPhase::Fresh {
            self.fail(storage_fail("additive raw links start phase differs"));
            return false;
        }
        if !self.input.start(cold) || !self.input.advance_reads() {
            self.phase = RawV1AuditLinksPhase::Refused;
            return false;
        }
        match self.input.validate_fields() {
            Ok(()) => {
                self.phase = RawV1AuditLinksPhase::InputsChecked;
                true
            }
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
    fn run(
        mut self,
        cold: bool,
    ) -> std::result::Result<
        AdditiveStorageLocalRawV1AuditLinksChecked,
        AdditiveStorageRawV1AuditLinksHeld,
    > {
        if !self.prepare(cold) || !self.finish() {
            return Err(AdditiveStorageRawV1AuditLinksHeld { frame: self });
        }
        Ok(AdditiveStorageLocalRawV1AuditLinksChecked { frame: self })
    }
    fn begin_gate(&mut self, gate: RawV1AuditGate) -> bool {
        if self.first() {
            return false;
        }
        let i = gate.slot();
        if self.phase != gate.predecessor()
            || self.gates[i].started
            || self.returns[i].is_some()
            || self.input.phase != V1AuditInputPhase::InputsChecked
            || !self.input.all_returns()
            || self.input.rosters_returned != Some(true)
            || self.input.genesis.owner.fee.local.readonly.active != Some(1)
        {
            self.fail(storage_fail(
                "additive raw links gate lacks actual input returns",
            ));
            return false;
        }
        let amount = match raw_v1_audit_scan_reservation(&self.input.fields, gate) {
            Ok(amount) => amount,
            Err(first) => {
                self.fail(first);
                return false;
            }
        };
        let charged = self
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .loan()
            .and_then(|(_, _, work, _)| work.metadata(amount));
        if let Err(first) = charged {
            self.fail(first);
            return false;
        }
        self.gates[i].charged = true;
        self.gates[i].started = true;
        self.phase = gate.started();
        true
    }
    fn evaluate_gate(&mut self, gate: RawV1AuditGate) -> Option<StorageResult<()>> {
        let i = gate.slot();
        if self.first()
            || self.phase != gate.started()
            || !self.gates[i].started
            || !self.gates[i].charged
            || self.gates[i].callee_reached
            || self.gates[i].returned.is_some()
            || self.returns[i].is_some()
        {
            return None;
        }
        self.gates[i].callee_reached = true;
        let clear =
            self.input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .and_then(|(_, _, work, _)| {
                    work.require_replay_clear()
                        .map_err(GlobalSchemaV1Error::ReplayTerminal)
                });
        let actual = clear.and_then(|()| match gate {
            RawV1AuditGate::Audit => {
                raw_audit_predecessor_links(&self.input.fields, &mut self.gates[i])
            }
            RawV1AuditGate::V1 => raw_v1_event_head_links(&self.input.fields, &mut self.gates[i]),
        });
        self.gates[i].callee_returned = Some(actual.is_ok());
        Some(actual)
    }
    // The actual owned callee return moves into this frame before any first-error
    // inspection. Rejected duplicates return the SAME payload to their caller.
    fn retain_return(
        &mut self,
        gate: RawV1AuditGate,
        actual: StorageResult<()>,
    ) -> std::result::Result<(), StorageResult<()>> {
        let i = gate.slot();
        if !self.gates[i].started
            || !self.gates[i].charged
            || !self.gates[i].callee_reached
            || self.gates[i].callee_returned != Some(actual.is_ok())
            || self.gates[i].returned.is_some()
            || self.returns[i].is_some()
            || !(self.phase == gate.started() || self.phase == RawV1AuditLinksPhase::Refused)
        {
            return Err(actual);
        }
        self.returns[i] = Some(actual);
        self.gates[i].returned = Some(self.returns[i].as_ref().unwrap().is_ok());
        if self.first() {
            return Ok(());
        } // Late Err stays owned; no successor or replaced first.
        if self.gates[i].returned == Some(false) {
            let first = self.returns[i].take().unwrap().unwrap_err();
            self.fail(first);
        } else {
            self.phase = gate.checked();
        }
        Ok(())
    }
    fn advance_links(&mut self) -> bool {
        if self.first() {
            return false;
        }
        for gate in RawV1AuditGate::ALL {
            if !self.begin_gate(gate) {
                return false;
            }
            let Some(actual) = self.evaluate_gate(gate) else {
                self.fail(storage_fail("additive raw links result not observed"));
                return false;
            };
            if let Err(actual) = self.retain_return(gate, actual) {
                self.unaccepted_return = Some(actual); // Whole owns even an internally inconsistent return.
                self.fail(storage_fail("additive raw links return already retained"));
                return false;
            }
            if self.first() {
                return false;
            }
        }
        true
    }
    fn all_returns(&self) -> bool {
        self.gates.iter().all(|f| {
            f.started
                && f.charged
                && f.callee_reached
                && f.callee_returned == Some(true)
                && f.returned == Some(true)
        }) && self.returns.iter().all(|r| matches!(r, Some(Ok(()))))
            && self.unaccepted_return.is_none()
    }
    fn close_and_tail(&mut self) -> StorageResult<()> {
        if self.first() {
            return Err(storage_fail("additive raw links close after first error"));
        }
        if self.phase != RawV1AuditLinksPhase::LinksChecked
            || !self.all_returns()
            || self.content_hashes != RawV1AuditContentHashes::NotChecked
        {
            return Err(storage_fail(
                "additive raw links close before actual results",
            ));
        }
        self.input.close_and_tail()?;
        self.phase = RawV1AuditLinksPhase::Complete;
        Ok(())
    }
    fn finish(&mut self) -> bool {
        if self.first() {
            return false;
        }
        if !self.advance_links() {
            return false;
        }
        match self.close_and_tail() {
            Ok(()) => true,
            Err(first) => {
                self.fail(first);
                false
            }
        }
    }
}
// Fixed ordinary borrowed scan work, not a serializer/SDK/payment witness.
fn raw_v1_audit_scan_reservation(
    fields: &V1AuditInputFields,
    gate: RawV1AuditGate,
) -> StorageResult<u64> {
    let slots: &[usize] = match gate {
        RawV1AuditGate::Audit => &[3, 4],
        RawV1AuditGate::V1 => &[0, 1, 2],
    };
    let mut count = 0u64;
    for &slot in slots {
        count = count
            .checked_add(
                u64::try_from(fields.rows[slot].len())
                    .map_err(|_| storage_fail("additive raw links count overflow"))?,
            )
            .ok_or_else(|| storage_fail("additive raw links count overflow"))?;
    }
    count
        .checked_mul(64)
        .and_then(|n| n.checked_add(256))
        .ok_or_else(|| storage_fail("additive raw links scan overflow"))
}
fn raw_audit_predecessor_links(
    fields: &V1AuditInputFields,
    facts: &mut RawV1AuditGateFacts,
) -> StorageResult<()> {
    let audits = &fields.rows[3];
    let chain = &fields.rows[4];
    if audits.len() != chain.len() {
        return Err(storage_fail("additive raw audit length differs"));
    }
    let mut previous = super::super::order_audit::AUDIT_CHAIN_GENESIS;
    for (index, (audit, evidence)) in audits.iter().zip(chain).enumerate() {
        if !super::super::order_audit::raw_order_audit_link_matches(
            audit.integer(0)?,
            evidence.integer(0)?,
            evidence.text(1)?,
            previous,
        ) {
            return Err(storage_fail("additive raw audit predecessor differs"));
        }
        previous = evidence.text(2)?;
        facts.checked_rows += 1;
        facts.tail_row = Some(index);
    }
    Ok(())
}
fn raw_v1_event_head_links(
    fields: &V1AuditInputFields,
    facts: &mut RawV1AuditGateFacts,
) -> StorageResult<()> {
    let events = &fields.rows[1];
    let heads = &fields.rows[2];
    let mut index = 0;
    for head in heads {
        let account = head.text(0)?;
        let mut version = 0i64;
        let mut previous = None;
        while index < events.len() && events[index].text(0)? == account {
            let event = &events[index];
            let expected = version
                .checked_add(1)
                .ok_or_else(|| storage_fail("additive raw V1 sequence overflow"))?;
            if !crate::trading::paper_ledger::raw_v1_event_link_matches(
                event.integer(1)?,
                expected,
                event.text(3)?,
                previous,
            ) {
                return Err(storage_fail("additive raw V1 event link differs"));
            }
            version = expected;
            previous = Some(event.text(4)?);
            facts.checked_rows += 1;
            facts.tail_row = Some(index);
            index += 1;
        }
        let last_hash =
            previous.ok_or_else(|| storage_fail("additive raw V1 head lacks events"))?;
        if !crate::trading::paper_ledger::raw_v1_head_link_matches(
            head.integer(1)?,
            head.text(2)?,
            version,
            last_hash,
        ) {
            return Err(storage_fail("additive raw V1 head link differs"));
        }
        facts.checked_heads += 1;
    }
    if index != events.len() {
        return Err(storage_fail("additive raw V1 trailing event differs"));
    }
    Ok(())
}

#[cfg(test)]
mod retained_semantic_validation_tests {
    use super::*;
    fn transformed(
        original: rows::VerifiedUnapprovedOriginalRowsBackup,
    ) -> AdditiveStorageTransformed {
        let copied =
            match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
                Ok(owner) => owner,
                Err(held) => panic!("raw links real Copied: {}", held.first_error()),
            };
        match copied.into_transformed() {
            Ok(owner) => owner,
            Err(held) => panic!("raw links real WAL: {}", held.first_error()),
        }
    }
    fn checked(f: &mut RawV1AuditLinksFrame) {
        assert!(f.phase == RawV1AuditLinksPhase::Complete && f.all_returns());
        assert!(f.content_hashes == RawV1AuditContentHashes::NotChecked);
        assert!(
            f.input.phase == V1AuditInputPhase::Complete
                && f.input.genesis.phase == GenesisInputPhase::Complete
        );
        assert!(f.input.all_returns() && f.input.fields.rows.iter().all(|r| !r.is_empty()));
        assert_eq!(f.input.fields.rows[3].len(), 1); // Genuine current fixture has one audit, not two.
        assert_eq!(f.gates[0].checked_rows, 1);
        assert_eq!(f.gates[0].tail_row, Some(0));
        assert!(
            f.gates[1].checked_rows > f.gates[1].checked_heads
                && f.gates[1].checked_heads == f.input.fields.rows[2].len()
        );
        assert!(
            f.input.genesis.owner.fee.local.readonly.reader.is_none()
                && f.input.genesis.owner.fee.local.readonly.active.is_none()
        );
        assert!(f
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .facts
            .iter()
            .all(|r| r.closed && r.original_tail_validated));
        assert_eq!(
            f.input.genesis.owner.fee.local.readonly.loan().unwrap().3,
            2
        );
    }
    #[test]
    fn task6_retained_semantic_links_nonempty_same_owner_and_cold() {
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = transformed(original);
                let fd = owner.frame.base.target().unwrap().as_raw_fd();
                let mut local = match owner.into_raw_v1_audit_links() {
                    Ok(owner) => owner,
                    Err(held) => panic!("raw links warm: {}", held.first_error()),
                };
                checked(&mut local.frame);
                let base = &local
                    .frame
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .base;
                assert_eq!(base.target().unwrap().as_raw_fd(), fd);
                let saved = (
                    base.target_node.unwrap(),
                    base.records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                    local.frame.input.fields.rows.clone(),
                );
                drop(local);
                saved // All old leases/owners end before a fresh genuine source is acquired.
            },
            |(node, records, fields), original| {
                let mut local = match AdditiveStorageLocalRawV1AuditLinksChecked::create_or_resume(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("raw links cold6: {}", held.first_error()),
                };
                checked(&mut local.frame);
                assert_eq!(local.frame.input.fields.rows, fields);
                assert!(local
                    .frame
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .begin_return
                    .is_none());
                let base = &local
                    .frame
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .base;
                assert_eq!(base.target_node, Some(node));
                assert_eq!(
                    base.records
                        .iter()
                        .flatten()
                        .map(|r| (r.node, r.bytes.clone()))
                        .collect::<Vec<_>>(),
                    records
                );
                drop(local);
            },
        );
        super::super::tests::task6_with_cold_rows_backup_fixture_for_test(
            |original| {
                let owner = transformed(original);
                let base = &owner.frame.base;
                assert_eq!(base.records.iter().flatten().count(), 5);
                let node = base.target_node.unwrap();
                drop(owner);
                node
            },
            |node, original| {
                let mut local = match AdditiveStorageLocalRawV1AuditLinksChecked::create_or_resume(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("raw links cold5: {}", held.first_error()),
                };
                checked(&mut local.frame);
                assert_eq!(
                    local
                        .frame
                        .input
                        .genesis
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .target_node,
                    Some(node)
                );
                assert!(local
                    .frame
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .begin_return
                    .is_none());
                drop(local);
            },
        );
    }
    fn gate_connection() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE paper_ledger_account(account_id,epoch_id,manifest_hash,manifest_bytes);
            CREATE TABLE paper_ledger_event(account_id,seq,command_id,previous_hash,event_hash,payload,business_plan_id,intent_hash,is_terminal,paper_trade_id,order_audit_id);
            CREATE TABLE paper_ledger_head(account_id,version,event_hash,projection_bytes,projection_hash);
            CREATE TABLE order_audit(id,business_order_id,source,decision_basis,side,code,requested_price,execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at);
            CREATE TABLE order_audit_chain(order_audit_id,previous_hash,record_hash);
            INSERT INTO paper_ledger_account VALUES('a','epoch','manifest','{}');
            INSERT INTO paper_ledger_event VALUES('a',1,'seed','PAPER_LEDGER_GENESIS_V1','event1','{}',NULL,NULL,0,NULL,NULL),
                ('a',2,'order','event1','event2','{}','plan','intent',1,1,1);
            INSERT INTO paper_ledger_head VALUES('a',2,'event2','{}','projection');
            INSERT INTO order_audit VALUES(1,'order1','source','basis','buy','code',10.0,10.0,100,NULL,'Filled',NULL,'time'),
                (2,'order2','source','basis','buy','code',20.0,NULL,200,NULL,'Rejected','reason','time');
            INSERT INTO order_audit_chain VALUES(1,'BR086_ORDER_AUDIT_GENESIS_V1','record1'),(2,'record1','record2');").unwrap();
        c
    }
    fn read_gate(c: &Connection, work: &mut target::TargetWork) -> V1AuditInputFields {
        let mut fields = V1AuditInputFields::default();
        let mut facts: [V1AuditInputReadFacts; 5] =
            std::array::from_fn(|_| V1AuditInputReadFacts::default());
        for query in V1AuditInputQuery::ALL {
            V1AuditInputReadLoan {
                connection: c,
                work: &mut *work,
                rows: &mut fields.rows[query.slot()],
                facts: &mut facts[query.slot()],
            }
            .read(query)
            .unwrap();
        }
        let old = [OwnerLinkageRow {
            account_id: Some("a".into()),
            epoch_id: Some("epoch".into()),
            manifest_hash: Some("manifest".into()),
            ..OwnerLinkageRow::default()
        }];
        work.metadata(1024).unwrap();
        v1_audit_input_rosters(&old, &fields).unwrap();
        assert!(facts
            .iter()
            .all(|r| r.eof && r.scopes_ended && r.returned == Some(true)));
        fields
    }
    #[test]
    fn task6_retained_semantic_links_exact_rejections_and_hash_not_checked() {
        // Fixed real SQL material feeds the SAME production borrowed gates.
        // This two-audit/empty-chain control is low permission, not an owner issuer.
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut source = original.into_additive_target_source().unwrap();
            for case in [
                "plain",
                "first_audit",
                "next_audit",
                "seq_gap",
                "first_event",
                "next_event",
                "head_version",
                "head_hash",
                "changed_payload",
                "empty_audit",
            ] {
                let c = gate_connection();
                c.execute_batch(match case {
                    "first_audit" => "UPDATE order_audit_chain SET previous_hash='bad' WHERE order_audit_id=1;",
                    "next_audit" => "UPDATE order_audit_chain SET previous_hash='bad' WHERE order_audit_id=2;",
                    "seq_gap" => "UPDATE paper_ledger_event SET seq=5 WHERE seq=2;",
                    "first_event" => "UPDATE paper_ledger_event SET previous_hash='bad' WHERE seq=1;",
                    "next_event" => "UPDATE paper_ledger_event SET previous_hash='bad' WHERE seq=2;",
                    "head_version" => "UPDATE paper_ledger_head SET version=3;",
                    "head_hash" => "UPDATE paper_ledger_head SET event_hash='bad';",
                    "changed_payload" => "UPDATE paper_ledger_event SET payload='not canonical JSON'; UPDATE order_audit SET decision_basis='different'; UPDATE paper_ledger_head SET projection_bytes='unchecked';",
                    "empty_audit" => "DELETE FROM order_audit_chain; DELETE FROM order_audit;", _ => "",
                }).unwrap();
                let work = source.storage_parts().unwrap().2;
                let fields = read_gate(&c, work);
                let mut audit = RawV1AuditGateFacts::default();
                let mut v1 = RawV1AuditGateFacts::default();
                work.metadata(
                    raw_v1_audit_scan_reservation(&fields, RawV1AuditGate::Audit).unwrap(),
                )
                .unwrap();
                let audit_return = raw_audit_predecessor_links(&fields, &mut audit);
                let actual = match audit_return {
                    Err(first) => Err(first),
                    Ok(()) => {
                        work.metadata(
                            raw_v1_audit_scan_reservation(&fields, RawV1AuditGate::V1).unwrap(),
                        )
                        .unwrap();
                        raw_v1_event_head_links(&fields, &mut v1)
                    }
                };
                if matches!(case, "plain" | "changed_payload" | "empty_audit") {
                    actual.unwrap();
                    assert_eq!(v1.checked_rows, 2);
                    assert_eq!(v1.checked_heads, 1);
                    if case == "empty_audit" {
                        assert_eq!(audit.checked_rows, 0);
                        assert!(audit.tail_row.is_none());
                    } else {
                        assert_eq!(audit.checked_rows, 2);
                        assert_eq!(audit.tail_row, Some(1));
                    }
                    if case == "changed_payload" {
                        assert_eq!(fields.rows[1][0].text(5).unwrap(), "not canonical JSON");
                        assert_eq!(fields.rows[3][0].text(3).unwrap(), "different");
                        assert_eq!(fields.rows[2][0].text(3).unwrap(), "unchecked");
                    } // A real malformed payload passes LINKS only; no content-hash validation ran.
                } else {
                    let expected = match case {
                        "first_audit" | "next_audit" => "additive raw audit predecessor differs",
                        "head_version" | "head_hash" => "additive raw V1 head link differs",
                        _ => "additive raw V1 event link differs",
                    };
                    assert!(
                        matches!(actual.unwrap_err(), GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == expected)
                    );
                    if case == "next_audit" {
                        assert_eq!(audit.checked_rows, 1);
                        assert_eq!(audit.tail_row, Some(0));
                        assert_eq!(v1.checked_rows, 0);
                    }
                    if case == "next_event" || case == "seq_gap" {
                        assert_eq!(v1.checked_rows, 1);
                    }
                }
                c.close().unwrap();
            }
            drop(source);
        });
        for original_drift in [false, true] {
            super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
                let mut f = RawV1AuditLinksFrame::new(transformed(original).frame);
                assert!(f.prepare(false) && f.advance_links());
                let target_node = f
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .base
                    .target_node;
                let original_path = f
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .loan()
                    .unwrap()
                    .0
                    .with_namespace(|ns| Ok(ns.database_parent.path.join(&ns.database_leaf)))
                    .unwrap();
                let original_file = if original_drift {
                    Some(
                        std::fs::OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open(original_path)
                            .unwrap(),
                    )
                } else {
                    None
                };
                let mut byte = [0];
                {
                    let file = original_file.as_ref().unwrap_or_else(|| {
                        f.input
                            .genesis
                            .owner
                            .fee
                            .local
                            .readonly
                            .transform
                            .base
                            .target()
                            .unwrap()
                    });
                    file.read_exact_at(&mut byte, 100).unwrap();
                    file.write_all_at(&[byte[0] ^ 1], 100).unwrap();
                    file.sync_all().unwrap();
                }
                let actual = f.close_and_tail();
                {
                    let file = original_file.as_ref().unwrap_or_else(|| {
                        f.input
                            .genesis
                            .owner
                            .fee
                            .local
                            .readonly
                            .transform
                            .base
                            .target()
                            .unwrap()
                    });
                    file.write_all_at(&byte, 100).unwrap();
                    file.sync_all().unwrap();
                }
                drop(original_file); // Fixture cleanup only; no production or cfg target File clone.
                let expected = if original_drift {
                    "rows original main bytes changed"
                } else {
                    "additive readonly retained target bytes changed"
                };
                let first = actual.unwrap_err();
                assert!(
                    matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == expected)
                );
                f.fail(first);
                assert!(f.input.genesis.owner.fee.local.readonly.reader.is_none());
                assert_eq!(
                    f.input
                        .genesis
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .base
                        .target_node,
                    target_node
                );
                assert_eq!(
                    f.input.genesis.owner.fee.local.readonly.facts[1].original_tail_validated,
                    !original_drift
                );
                let primary = f
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .first
                    .as_ref()
                    .unwrap() as *const GlobalSchemaV1Error;
                let used = f
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .loan()
                    .unwrap()
                    .2
                    .metadata_used();
                assert!(!f.finish());
                assert_eq!(
                    f.input
                        .genesis
                        .owner
                        .fee
                        .local
                        .readonly
                        .loan()
                        .unwrap()
                        .2
                        .metadata_used(),
                    used
                );
                assert_eq!(
                    f.input
                        .genesis
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .first
                        .as_ref()
                        .unwrap() as *const GlobalSchemaV1Error,
                    primary
                );
                assert!(
                    f.input.fields.rows.iter().all(|r| !r.is_empty())
                        && f.content_hashes == RawV1AuditContentHashes::NotChecked
                );
                drop(f);
            });
        }
    }
    #[test]
    fn task6_retained_semantic_links_same_work_unknown_late_and_busy() {
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = RawV1AuditLinksFrame::new(transformed(original).frame);
            let used = f
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .base
                .source
                .storage_parts()
                .unwrap()
                .2
                .metadata_used();
            f.fail(storage_fail("TEST_CODE first before raw links admission"));
            let primary = f
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .first
                .as_ref()
                .unwrap() as *const GlobalSchemaV1Error;
            assert!(!f.prepare(false) && !f.finish() && !f.begin_gate(RawV1AuditGate::Audit));
            assert!(
                f.input.reads.iter().all(|r| !r.started)
                    && f.input.returns.iter().all(Option::is_none)
            );
            assert!(
                f.gates.iter().all(|g| !g.started && !g.callee_reached)
                    && f.returns.iter().all(Option::is_none)
            );
            assert_eq!(
                f.input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .base
                    .source
                    .storage_parts()
                    .unwrap()
                    .2
                    .metadata_used(),
                used
            );
            assert_eq!(
                f.input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .first
                    .as_ref()
                    .unwrap() as *const GlobalSchemaV1Error,
                primary
            );
            drop(f);
        });
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = RawV1AuditLinksFrame::new(transformed(original).frame);
            assert!(f.prepare(false));
            let work = f.input.genesis.owner.fee.local.readonly.loan().unwrap().2;
            let remaining = 16 * MIB - work.metadata_used();
            work.metadata(remaining).unwrap();
            assert!(
                !f.finish()
                    && f.gates.iter().all(|g| !g.started && !g.charged)
                    && f.returns.iter().all(Option::is_none)
            );
            assert!(
                matches!(f.input.genesis.owner.fee.local.readonly.transform.first.as_ref().unwrap(),
                GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == "target metadata work exceeded before allocation")
            );
            let primary = f
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .first
                .as_ref()
                .unwrap() as *const GlobalSchemaV1Error;
            let used = f
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .unwrap()
                .2
                .metadata_used();
            assert!(!f.finish());
            assert_eq!(
                f.input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .loan()
                    .unwrap()
                    .2
                    .metadata_used(),
                used
            );
            assert_eq!(
                f.input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .first
                    .as_ref()
                    .unwrap() as *const GlobalSchemaV1Error,
                primary
            );
            drop(f);
        });
        for late_error in [false, true] {
            super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
                let mut f = RawV1AuditLinksFrame::new(transformed(original).frame);
                assert!(f.prepare(false));
                if late_error {
                    // Held-byte mutation exercises callee Err custody only, not a SQL/issuer witness.
                    f.input.fields.rows[4][0].cells[1] =
                        Some(V1AuditInputCell::Text("TEST_CODE_bad_previous".into()));
                }
                let pointer = f.input.fields.rows[4][0].text(2).unwrap().as_ptr();
                assert!(f.begin_gate(RawV1AuditGate::Audit));
                assert!(f.retain_return(RawV1AuditGate::Audit, Ok(())).is_err()); // No callee return yet.
                let actual = f.evaluate_gate(RawV1AuditGate::Audit).unwrap();
                assert!(f.evaluate_gate(RawV1AuditGate::Audit).is_none()); // Once reached, no second scan/debit.
                let contradiction = if late_error {
                    Ok(())
                } else {
                    Err(storage_fail("TEST_CODE contradictory return"))
                };
                assert!(f
                    .retain_return(RawV1AuditGate::Audit, contradiction)
                    .is_err());
                assert_eq!(actual.is_err(), late_error);
                assert!(f.gates[0].returned.is_none() && f.returns[0].is_none());
                let first = f.close_and_tail().unwrap_err();
                assert!(
                    matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == "additive raw links close before actual results")
                );
                f.fail(first);
                let primary = f
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .first
                    .as_ref()
                    .unwrap() as *const GlobalSchemaV1Error;
                f.retain_return(RawV1AuditGate::Audit, actual).unwrap();
                assert_eq!(f.gates[0].returned, Some(!late_error));
                assert_eq!(f.returns[0].as_ref().unwrap().is_err(), late_error);
                assert_eq!(f.input.fields.rows[4][0].text(2).unwrap().as_ptr(), pointer);
                let duplicate = storage_fail("TEST_CODE duplicate owned return");
                let detail_ptr = match &duplicate {
                    GlobalSchemaV1Error::SelectionSnapshotChanged { detail } => detail.as_ptr(),
                    _ => unreachable!(),
                };
                let same = f
                    .retain_return(RawV1AuditGate::Audit, Err(duplicate))
                    .unwrap_err();
                assert!(
                    matches!(&same, Err(GlobalSchemaV1Error::SelectionSnapshotChanged { detail }) if detail.as_ptr() == detail_ptr)
                );
                f.unaccepted_return = Some(same); // Exact refused payload remains inside the same whole.
                let used = f
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .loan()
                    .unwrap()
                    .2
                    .metadata_used();
                assert!(
                    !f.begin_gate(RawV1AuditGate::V1)
                        && f.evaluate_gate(RawV1AuditGate::Audit).is_none()
                        && !f.finish()
                );
                assert_eq!(
                    f.input
                        .genesis
                        .owner
                        .fee
                        .local
                        .readonly
                        .loan()
                        .unwrap()
                        .2
                        .metadata_used(),
                    used
                );
                assert_eq!(
                    f.input
                        .genesis
                        .owner
                        .fee
                        .local
                        .readonly
                        .transform
                        .first
                        .as_ref()
                        .unwrap() as *const GlobalSchemaV1Error,
                    primary
                );
                assert!(
                    !f.gates[1].started
                        && !f.input.genesis.owner.fee.local.readonly.facts[1]
                            .original_tail_validated
                );
                drop(f);
            });
        }
        super::super::tests::task6_with_actual_rows_backup_for_test(|original| {
            let mut f = RawV1AuditLinksFrame::new(transformed(original).frame);
            assert!(f.prepare(false) && f.advance_links());
            f.input.genesis.owner.fee.local.readonly.prepare_busy_vm();
            let first = f.close_and_tail().unwrap_err();
            assert!(
                matches!(&first, GlobalSchemaV1Error::SelectionSqlite { operation: "close additive readonly", source }
                if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy))
            );
            f.fail(first);
            assert!(f.input.genesis.owner.fee.local.readonly.reader.is_some());
            let used = f
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .unwrap()
                .2
                .metadata_used();
            assert!(!f.finish());
            assert_eq!(
                f.input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .loan()
                    .unwrap()
                    .2
                    .metadata_used(),
                used
            );
            assert!(!f.input.genesis.owner.fee.local.readonly.facts[1].original_tail_validated);
            assert!(f
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .finalize_busy_once());
            f.input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .cleanup_reader_once();
            assert!(matches!(
                f.input.genesis.owner.fee.local.readonly.cleanup_close,
                Some(Ok(()))
            ));
            assert!(f.input.fields.rows.iter().all(|r| !r.is_empty()) && f.all_returns());
            assert!(
                f.phase == RawV1AuditLinksPhase::Refused
                    && !f.input.genesis.owner.fee.local.readonly.facts[1].original_tail_validated
            );
            drop(f);
        });
    }
}
