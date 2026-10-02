//! Completion of the original selected occurrences, not row coverage or PIT.
//! Only a fresh guarded SQL + filesystem reader creates the opaque capability.
use super::super::*;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

const MATERIAL: &str = "g5b-physical-day-seal-v1";
const REASON: &str = "AllSelectedPhysicalAccepted";
const MAX_ROWS: usize = 4096;
const MAX_WITNESS: usize = 32 * 1024 * 1024;
const CODEC: &str = "g5b-physical-zstd-utf8hex-v1";
const ARENA_CODEC: &str = "g5b-physical-zstd-byte-arena-v2";
const ZSTD_LEVEL: i32 = 3;
const ZSTD_WINDOW_LOG: u32 = 20;

pub(super) struct WitnessBudget {
    limit: usize,
    used: usize,
    pool: Vec<std::sync::Arc<Vec<u8>>>,
}
impl WitnessBudget {
    pub(super) fn maximum() -> Self {
        Self {
            limit: MAX_WITNESS,
            used: 0,
            pool: Vec::new(),
        }
    }
    fn for_connection(connection: &Connection) -> Self {
        #[cfg(test)]
        let limit = BUDGET_TEST.with(|slot| {
            slot.borrow()
                .as_ref()
                .filter(|v| connection.path() == Some(v.path.as_str()))
                .map(|v| v.limit)
                .unwrap_or(MAX_WITNESS)
        });
        #[cfg(not(test))]
        let limit = {
            let _ = connection;
            MAX_WITNESS
        };
        Self {
            limit,
            used: 0,
            pool: Vec::new(),
        }
    }
    pub(super) fn reserve(&mut self, length: usize) -> Result<()> {
        let next = self
            .used
            .checked_add(length)
            .ok_or_else(|| mismatch("Physical witness budget overflow"))?;
        if next > self.limit {
            return Err(mismatch(&format!("Physical witness byte budget exceeded before copy/encoding (used={}, requested={}, limit={})",self.used,length,self.limit)));
        }
        self.used = next;
        Ok(())
    }
    fn descriptor<T>(&mut self, count: usize) -> Result<()> {
        self.reserve(
            std::mem::size_of::<T>()
                .checked_mul(count)
                .ok_or_else(|| mismatch("Physical descriptor budget overflow"))?,
        )
    }
    pub(super) fn reserve_clone(&mut self, value: &impl Serialize) -> Result<()> {
        struct Counter<'a>(&'a mut WitnessBudget);
        impl std::io::Write for Counter<'_> {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0
                    .reserve(b.len())
                    .map_err(|_| std::io::Error::other("Physical clone budget exceeded"))?;
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        // Canonical metadata is an upper bound on all cloned variable fields.
        // This reserves before clone; no encoded metadata buffer is allocated.
        serde_json::to_writer(Counter(self), value)?;
        Ok(())
    }
    pub(super) fn intern(&mut self, bytes: &[u8]) -> Result<EvidenceBytes> {
        match self.pool.binary_search_by(|v| v.as_slice().cmp(bytes)) {
            Ok(index) => Ok(EvidenceBytes::Shared(std::sync::Arc::clone(
                &self.pool[index],
            ))),
            Err(index) => {
                self.reserve(bytes.len())?;
                self.descriptor::<Vec<u8>>(1)?;
                self.descriptor::<usize>(2)?;
                // Four descriptor slots per insertion conservatively cover Vec
                // capacity growth; byte ownership is allocated exactly once.
                self.descriptor::<std::sync::Arc<Vec<u8>>>(4)?;
                let value = std::sync::Arc::new(bytes.to_vec());
                self.pool.insert(index, std::sync::Arc::clone(&value));
                Ok(EvidenceBytes::Shared(value))
            }
        }
    }
}

/// Owned is the original v1 reader/encoder allocation. Shared is private v2
/// storage only; both serialize to the exact old ByteLeaf when used by v1.
#[derive(Clone, Debug)]
pub(super) enum EvidenceBytes {
    Owned(Vec<u8>),
    Shared(std::sync::Arc<Vec<u8>>),
}
impl std::ops::Deref for EvidenceBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(v) => v,
            Self::Shared(v) => v,
        }
    }
}
impl PartialEq for EvidenceBytes {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl Eq for EvidenceBytes {}
impl Serialize for EvidenceBytes {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        ByteRef(self).serialize(s)
    }
}
impl<'de> Deserialize<'de> for EvidenceBytes {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        byte_leaf::deserialize(d).map(Self::Owned)
    }
}
#[derive(Clone, Debug)]
enum EvidenceText {
    Owned(String),
    Shared(EvidenceBytes),
}
impl EvidenceText {
    fn as_str(&self) -> &str {
        match self {
            Self::Owned(v) => v,
            Self::Shared(v) => {
                std::str::from_utf8(v).expect("only checked UTF8 creates EvidenceText")
            }
        }
    }
}
impl std::ops::Deref for EvidenceText {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl PartialEq for EvidenceText {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for EvidenceText {}
impl Serialize for EvidenceText {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for EvidenceText {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(d).map(Self::Owned)
    }
}
struct BoundedEncoding<'a> {
    budget: &'a mut WitnessBudget,
    bytes: Vec<u8>,
}
impl std::io::Write for BoundedEncoding<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.budget.reserve(bytes.len()).map_err(|error| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
        })?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode_bounded(
    value: &impl Serialize,
    budget: &mut WitnessBudget,
) -> Result<Vec<u8>> {
    let mut output = BoundedEncoding {
        budget,
        bytes: Vec::new(),
    };
    serde_json::to_writer(&mut output, value)?;
    Ok(output.bytes)
}

/// UTF8 bytes retain the exact original string, including nested JSON spelling.
/// Binary bytes have one lowercase-hex representation; valid UTF8 cannot use it.
pub(super) struct ByteRef<'a>(pub(super) &'a [u8]);
impl Serialize for ByteRef<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match std::str::from_utf8(self.0) {
            Ok(value) => serializer.serialize_newtype_variant("ByteLeaf", 0, "Utf8", value),
            Err(_) => serializer.serialize_newtype_variant("ByteLeaf", 1, "Hex", &HexRef(self.0)),
        }
    }
}
struct HexRef<'a>(&'a [u8]);
impl std::fmt::Display for HexRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut block = [0u8; 128];
        for chunk in self.0.chunks(64) {
            for (index, byte) in chunk.iter().enumerate() {
                block[2 * index] = HEX[usize::from(*byte >> 4)];
                block[2 * index + 1] = HEX[usize::from(*byte & 15)];
            }
            f.write_str(
                std::str::from_utf8(&block[..2 * chunk.len()]).map_err(|_| std::fmt::Error)?,
            )?;
        }
        Ok(())
    }
}
impl Serialize for HexRef<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
pub(super) mod byte_leaf {
    use super::*;
    pub(crate) fn serialize<S: serde::Serializer>(
        value: &[u8],
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        ByteRef(value).serialize(serializer)
    }
    pub(crate) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Vec<u8>, D::Error> {
        #[derive(Deserialize)]
        enum Leaf {
            Utf8(String),
            Hex(String),
        }
        // The enclosing compact JSON/frame was reserved in full before parse.
        // Neither decoded leaf can exceed that reserved input's byte length.
        match Leaf::deserialize(deserializer)? {
            Leaf::Utf8(value) if value.len() <= MAX_WITNESS => Ok(value.into_bytes()),
            Leaf::Hex(value)
                if value.len() <= 2 * MAX_WITNESS
                    && value.len() % 2 == 0
                    && value
                        .bytes()
                        .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v)) =>
            {
                let bytes = hex::decode(&value).map_err(serde::de::Error::custom)?;
                if std::str::from_utf8(&bytes).is_ok() {
                    return Err(serde::de::Error::custom(
                        "canonical byte leaf requires Utf8",
                    ));
                }
                Ok(bytes)
            }
            _ => Err(serde::de::Error::custom(
                "canonical byte leaf length/tag/hex differs",
            )),
        }
    }
}

/// Streaming canonical comparison performs no second whole-document copy.
/// Its validation work is still charged to the shared history budget.
struct MatchEncoding<'a, 'b> {
    expected: &'a [u8],
    offset: usize,
    budget: &'b mut WitnessBudget,
}
impl std::io::Write for MatchEncoding<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.budget
            .reserve(bytes.len())
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let end = self
            .offset
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("canonical length overflow"))?;
        if self.expected.get(self.offset..end) != Some(bytes) {
            return Err(std::io::Error::other("Physical canonical bytes differ"));
        }
        self.offset = end;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn match_canonical(
    value: &impl Serialize,
    expected: &[u8],
    budget: &mut WitnessBudget,
) -> Result<()> {
    let mut writer = MatchEncoding {
        expected,
        offset: 0,
        budget,
    };
    serde_json::to_writer(&mut writer, value)?;
    if writer.offset != expected.len() {
        return Err(mismatch("Physical canonical bytes have trailing data"));
    }
    Ok(())
}
/// Reserve the entire row before r.get/ValueRef conversion copies any variable
/// length SQL value. A malformed or oversize row is never partially adopted.
fn reserve_row(row: &rusqlite::Row<'_>, width: usize, budget: &mut WitnessBudget) -> Result<()> {
    use rusqlite::types::ValueRef;
    let mut length = 0usize;
    for index in 0..width {
        length = length
            .checked_add(match row.get_ref(index)? {
                ValueRef::Text(v) | ValueRef::Blob(v) => v.len(),
                _ => 8,
            })
            .ok_or_else(|| mismatch("Physical witness row budget overflow"))?;
    }
    budget.reserve(length)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Cell {
    Null,
    Integer(i64),
    Text(EvidenceText),
    Blob(EvidenceBytes),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    name: String,
    columns: Vec<String>,
    rows: Vec<Vec<Cell>>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    index: usize,
    occurrence: String,
    decision: String,
    attempt: String,
    evidence_sha256: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhysicalSeal {
    #[serde(skip, default = "legacy_storage_version")]
    storage_version: u8,
    version: u8,
    material: String,
    business_date: NaiveDate,
    cohort_identity: String,
    revision: i64,
    reason: String,
    model_binding: model_bundle::SealModelBinding,
    full_archive_identity: String,
    members: Vec<Member>,
    tables: Vec<Table>,
}

fn legacy_storage_version() -> u8 {
    1
}

pub(crate) struct VerifiedG5bPhysicalSeal {
    date: NaiveDate,
    cohort: String,
    revision: i64,
    identity: String,
    sha256: String,
    count: usize,
    current_head_canonical: Vec<u8>,
    seal_canonical: Vec<u8>,
    namespace_identity: (u64, u64),
    database_identity: FileObjectIdentity,
    lock_identity: (u64, u64),
}
impl VerifiedG5bPhysicalSeal {
    pub(crate) fn business_date(&self) -> NaiveDate {
        self.date
    }
    pub(crate) fn cohort_identity(&self) -> &str {
        &self.cohort
    }
    pub(crate) fn revision(&self) -> i64 {
        self.revision
    }
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }
    pub(crate) fn selected_count(&self) -> usize {
        self.count
    }
    pub(crate) fn reason(&self) -> &'static str {
        REASON
    }
}
/// Incomplete is only a routing observation. It grants no completion permission.
pub(crate) enum G5bPhysicalSealAttempt {
    Incomplete,
    Sealed(VerifiedG5bPhysicalSeal),
}

const TABLES: &[&str] = &[
    "g5b_occurrence_owners",
    "g5b_artifact_events",
    "delivery_decisions",
    "delivery_attempts",
    "sink_results",
    "immutable_audit_outbox",
    "delivery_state_events",
    "delivery_attempt_events",
    "daily_budget_reservations",
    "daily_budget_reservation_events",
    "cooldown_reservations",
    "cooldown_reservation_events",
    "business_date_once_claims",
    "delivery_disposition_payloads",
    "task_transition_payloads",
    "manual_resolutions",
    "delivery_correlation_observations",
    "review_terminal_replay_attempts",
    "review_terminal_replay_completions",
];
impl Table {
    fn cell<'a>(&self, row: &'a [Cell], column: &str) -> Result<&'a Cell> {
        row.get(
            self.columns
                .iter()
                .position(|v| v == column)
                .ok_or_else(|| mismatch("Physical witness column missing"))?,
        )
        .ok_or_else(|| mismatch("Physical witness row width differs"))
    }
    fn text<'a>(&self, row: &'a [Cell], column: &str) -> Result<&'a str> {
        match self.cell(row, column)? {
            Cell::Text(v) => Ok(v),
            _ => Err(mismatch("Physical witness text type differs")),
        }
    }
    fn integer(&self, row: &[Cell], column: &str) -> Result<i64> {
        match self.cell(row, column)? {
            Cell::Integer(v) => Ok(*v),
            _ => Err(mismatch("Physical witness integer type differs")),
        }
    }
    fn blob<'a>(&self, row: &'a [Cell], column: &str) -> Result<&'a [u8]> {
        match self.cell(row, column)? {
            Cell::Blob(v) => Ok(v),
            _ => Err(mismatch("Physical witness blob type differs")),
        }
    }
    fn optional_text<'a>(&self, row: &'a [Cell], column: &str) -> Result<Option<&'a str>> {
        match self.cell(row, column)? {
            Cell::Null => Ok(None),
            Cell::Text(v) => Ok(Some(v)),
            _ => Err(mismatch("Physical witness nullable text differs")),
        }
    }
}
fn table<'a>(tables: &'a [Table], name: &str) -> Result<&'a Table> {
    tables
        .iter()
        .find(|v| v.name == name)
        .ok_or_else(|| mismatch("Physical witness table missing"))
}
fn rows_with_budget(
    connection: &Connection,
    date: NaiveDate,
    budget: &mut WitnessBudget,
) -> Result<Vec<Table>> {
    budget.descriptor::<Table>(TABLES.len())?;
    let mut values = Vec::with_capacity(TABLES.len());
    for name in TABLES {
        // All identifiers and predicates are closed constants, never input paths/SQL.
        let predicate = if *name == "delivery_decisions" {
            "business_date=?1 AND push_kind='G5bAttribution'"
        } else if name.starts_with("g5b_") {
            "business_date=?1"
        } else {
            "decision_identity IN (SELECT decision_identity FROM delivery_decisions WHERE business_date=?1 AND push_kind='G5bAttribution')"
        };
        let mut query = connection.prepare(&format!(
            "SELECT * FROM {name} WHERE {predicate} LIMIT {}",
            MAX_ROWS + 1
        ))?;
        let column_count = query.column_count();
        budget.descriptor::<String>(column_count)?;
        budget.descriptor::<&str>(column_count)?;
        let column_names = query.column_names();
        for column in &column_names {
            budget.reserve(column.len())?;
        }
        let columns = column_names
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>();
        let width = columns.len();
        let mut cursor = query.query([date.to_string()])?;
        let mut output = Vec::new();
        while let Some(row) = cursor.next()? {
            if output.len() == MAX_ROWS {
                return Err(mismatch("Physical witness row budget exceeded"));
            }
            budget.descriptor::<Vec<Cell>>(4)?;
            let item = bounded_cells(row, width, budget)?;
            output.push(item);
        }
        sort_rows(&mut output, budget)?;
        budget.reserve(name.len())?;
        // Rows themselves and capacity growth are reserved before push/copy.
        values.push(Table {
            name: name.to_string(),
            columns,
            rows: output,
        });
    }
    Ok(values)
}
fn bounded_cells(
    row: &rusqlite::Row<'_>,
    width: usize,
    budget: &mut WitnessBudget,
) -> Result<Vec<Cell>> {
    budget.descriptor::<Cell>(width)?;
    let mut item = Vec::with_capacity(width);
    for index in 0..width {
        use rusqlite::types::ValueRef;
        item.push(match row.get_ref(index)? {
            ValueRef::Null => Cell::Null,
            ValueRef::Integer(v) => Cell::Integer(v),
            ValueRef::Text(v) => {
                std::str::from_utf8(v).map_err(|_| mismatch("Physical witness invalid UTF8"))?;
                Cell::Text(EvidenceText::Shared(budget.intern(v)?))
            }
            ValueRef::Blob(v) => Cell::Blob(budget.intern(v)?),
            ValueRef::Real(_) => {
                return Err(mismatch("Physical witness unexpected floating SQL value"))
            }
        });
    }
    Ok(item)
}

/// Arrays have the same width. The first unequal original Cell determines the
/// old complete-row canonical JSON order; equal large bodies need no key copy.
/// Fallible in-place heapsort never turns a budget error into a comparator
/// ordering or allocates an unaccounted whole-row scratch/key vector.
fn sort_rows(rows: &mut [Vec<Cell>], budget: &mut WitnessBudget) -> Result<()> {
    fn sift(
        rows: &mut [Vec<Cell>],
        mut root: usize,
        end: usize,
        budget: &mut WitnessBudget,
    ) -> Result<()> {
        loop {
            let left = root
                .checked_mul(2)
                .and_then(|v| v.checked_add(1))
                .ok_or_else(|| mismatch("Physical sort index overflow"))?;
            if left >= end {
                return Ok(());
            }
            let mut child = left;
            if left + 1 < end
                && compare_rows(&rows[left], &rows[left + 1], budget)? == std::cmp::Ordering::Less
            {
                child = left + 1;
            }
            if compare_rows(&rows[root], &rows[child], budget)? != std::cmp::Ordering::Less {
                return Ok(());
            }
            rows.swap(root, child);
            root = child;
        }
    }
    let length = rows.len();
    for root in (0..length / 2).rev() {
        sift(rows, root, length, budget)?;
    }
    for end in (1..length).rev() {
        rows.swap(0, end);
        sift(rows, 0, end, budget)?;
    }
    Ok(())
}

fn compare_rows(a: &[Cell], b: &[Cell], budget: &mut WitnessBudget) -> Result<std::cmp::Ordering> {
    if a.len() != b.len() {
        return Err(mismatch("Physical row comparator width differs"));
    }
    for (a, b) in a.iter().zip(b) {
        if a != b {
            return Ok(encode_bounded(a, budget)?.cmp(&encode_bounded(b, budget)?));
        }
    }
    Ok(std::cmp::Ordering::Equal)
}

fn rows_legacy_with_budget(
    connection: &Connection,
    date: NaiveDate,
    budget: &mut WitnessBudget,
) -> Result<Vec<Table>> {
    let mut values = Vec::new();
    for name in TABLES {
        // All identifiers and predicates are closed constants, never input paths/SQL.
        let predicate = if *name == "delivery_decisions" {
            "business_date=?1 AND push_kind='G5bAttribution'"
        } else if name.starts_with("g5b_") {
            "business_date=?1"
        } else {
            "decision_identity IN (SELECT decision_identity FROM delivery_decisions WHERE business_date=?1 AND push_kind='G5bAttribution')"
        };
        let mut query = connection.prepare(&format!(
            "SELECT * FROM {name} WHERE {predicate} LIMIT {}",
            MAX_ROWS + 1
        ))?;
        let columns = query
            .column_names()
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>();
        let width = columns.len();
        let mut cursor = query.query([date.to_string()])?;
        let mut output = Vec::new();
        while let Some(row) = cursor.next()? {
            if output.len() == MAX_ROWS {
                return Err(mismatch("Physical witness row budget exceeded"));
            }
            let item = bounded_cells_legacy(row, width, budget)?;
            output.push(item);
        }
        let mut keyed = output
            .into_iter()
            .map(|v| Ok((encode_bounded(&v, budget)?, v)))
            .collect::<Result<Vec<_>>>()?;
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        values.push(Table {
            name: name.to_string(),
            columns,
            rows: keyed.into_iter().map(|(_, v)| v).collect(),
        });
    }
    Ok(values)
}
fn bounded_cells_legacy(
    row: &rusqlite::Row<'_>,
    width: usize,
    budget: &mut WitnessBudget,
) -> Result<Vec<Cell>> {
    reserve_row(row, width, budget)?;
    let mut item = Vec::with_capacity(width);
    for index in 0..width {
        use rusqlite::types::ValueRef;
        let raw = row.get_ref(index)?;
        item.push(match raw {
            ValueRef::Null => Cell::Null,
            ValueRef::Integer(v) => Cell::Integer(v),
            ValueRef::Text(v) => Cell::Text(EvidenceText::Owned(
                std::str::from_utf8(v)
                    .map_err(|_| mismatch("Physical witness invalid UTF8"))?
                    .to_owned(),
            )),
            ValueRef::Blob(v) => Cell::Blob(EvidenceBytes::Owned(v.to_vec())),
            ValueRef::Real(_) => {
                return Err(mismatch("Physical witness unexpected floating SQL value"))
            }
        });
    }
    Ok(item)
}

/// Complete closed SQL routing membership, including an unpublished Prepared
/// cohort. No source, file, model or completion permission comes from this list.
fn nonempty_routes(connection: &Connection) -> Result<(Vec<NaiveDate>, Vec<u8>)> {
    let mut query=connection.prepare(&format!("SELECT c.business_date,h.business_date,c.*,h.* FROM g5b_cohorts c LEFT JOIN g5b_day_heads h ON h.business_date=c.business_date WHERE c.selection_kind='NonEmpty' ORDER BY c.business_date LIMIT {}",MAX_ROWS+1))?;
    let columns = query
        .column_names()
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>();
    let width = columns.len();
    let mut cursor = query.query([])?;
    let mut budget = WitnessBudget::for_connection(connection);
    let mut dates = Vec::new();
    let mut output = Vec::new();
    while let Some(row) = cursor.next()? {
        if dates.len() == MAX_ROWS {
            return Err(mismatch("NonEmpty routing row budget exceeded"));
        }
        let item = bounded_cells(row, width, &mut budget)?;
        let (Cell::Text(date), Cell::Text(head_date)) = (&item[0], &item[1]) else {
            return Err(mismatch("NonEmpty routing cohort has no exact day head"));
        };
        let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(codec_error)?;
        if date != head_date
            || parsed.to_string() != date.as_str()
            || dates.last().is_some_and(|v| v >= &parsed)
        {
            return Err(mismatch("NonEmpty routing date membership differs"));
        }
        dates.push(parsed);
        output.push(item);
    }
    let binding = encode_bounded(&(columns, output), &mut budget)?;
    Ok((dates, binding))
}
impl DurableDeliveryCoordinator {
    pub(crate) fn list_g5b_nonempty_cohort_dates(&self) -> Result<Vec<NaiveDate>> {
        let expected = std::cell::RefCell::new(None::<Vec<u8>>);
        let validate = |tx: &Transaction<'_>| {
            let (_, binding) = nonempty_routes(tx)?;
            if expected.borrow().as_deref() != Some(binding.as_slice()) {
                return Err(mismatch(
                    "NonEmpty routing membership changed at final SQL boundary",
                ));
            }
            Ok(())
        };
        self.with_immediate_transaction_validated_sql(
            SchemaVersionPolicy::Runtime,
            None,
            Some(&validate),
            |tx| {
                let (dates, binding) = nonempty_routes(tx)?;
                *expected.borrow_mut() = Some(binding);
                Ok(dates)
            },
        )
    }
}

fn subset(saved: &[Table], current: &[Table], budget: &mut WitnessBudget) -> Result<()> {
    if saved.len() != TABLES.len() || current.len() != TABLES.len() {
        return Err(mismatch("Physical witness table count differs"));
    }
    for ((saved, current), name) in saved.iter().zip(current).zip(TABLES) {
        if saved.name != *name
            || current.name != *name
            || saved.columns != current.columns
            || saved.rows.len() > MAX_ROWS
            || saved
                .rows
                .iter()
                .any(|row| row.len() != saved.columns.len())
        {
            return Err(mismatch("Physical witness schema differs"));
        }
        let mut previous: Option<&[Cell]> = None;
        let mut at = 0;
        for row in &saved.rows {
            if let Some(previous) = previous {
                if compare_rows(previous, row, budget)? != std::cmp::Ordering::Less {
                    return Err(mismatch("Physical historical row repeated or unordered"));
                }
            }
            while at < current.rows.len()
                && compare_rows(&current.rows[at], row, budget)? == std::cmp::Ordering::Less
            {
                at += 1;
            }
            if current.rows.get(at).is_none_or(|current| current != row) {
                return Err(mismatch("Physical historical row disappeared or changed"));
            }
            previous = Some(row);
            at += 1;
        }
        if !matches!(
            *name,
            "sink_results" | "immutable_audit_outbox" | "delivery_attempt_events"
        ) && saved.rows != current.rows
        {
            return Err(mismatch(
                "Physical original business/model row set changed after historical seal",
            ));
        }
    }
    Ok(())
}

fn subset_legacy(saved: &[Table], current: &[Table]) -> Result<()> {
    if saved.len() != TABLES.len() || current.len() != TABLES.len() {
        return Err(mismatch("Physical witness table count differs"));
    }
    for ((saved, current), name) in saved.iter().zip(current).zip(TABLES) {
        if saved.name != *name
            || current.name != *name
            || saved.columns != current.columns
            || saved.rows.len() > MAX_ROWS
            || saved
                .rows
                .iter()
                .any(|row| row.len() != saved.columns.len())
        {
            return Err(mismatch("Physical witness schema differs"));
        }
        let mut previous = None;
        for row in &saved.rows {
            let encoded = canonical_json(row)?;
            if previous.as_ref().is_some_and(|v| v >= &encoded) || !current.rows.contains(row) {
                return Err(mismatch(
                    "Physical historical row disappeared, changed or repeated",
                ));
            }
            previous = Some(encoded);
        }
        if !matches!(
            *name,
            "sink_results" | "immutable_audit_outbox" | "delivery_attempt_events"
        ) && saved.rows != current.rows
        {
            return Err(mismatch(
                "Physical original business/model row set changed after historical seal",
            ));
        }
    }
    Ok(())
}

fn validate_late_extension(
    connection: &Connection,
    saved: &[Table],
    current: &[Table],
    members: &[Member],
) -> Result<()> {
    let old = table(saved, "sink_results")?;
    let now = table(current, "sink_results")?;
    for row in &now.rows {
        if !old.rows.contains(row)
            && (now.integer(row, "authoritative_for_state")? != 0
                || now.integer(row, "late_after_fence")? != 1)
        {
            return Err(mismatch(
                "Physical historical seal gained a new authoritative result",
            ));
        }
    }
    // New observed receipts must be retained even while their real immutable
    // append is pending. Qualify their event/binding without claiming drain.
    validate_raw(current)?;
    let old_events = table(saved, "delivery_attempt_events")?;
    let events = table(current, "delivery_attempt_events")?;
    for row in &events.rows {
        if !old_events.rows.contains(row)
            && !matches!(
                events.text(row, "event_kind")?,
                "SinkResultAuthorityClassified" | "LateReceiptObserved"
            )
        {
            return Err(mismatch(
                "Physical historical completion was reopened by an attempt event",
            ));
        }
    }
    let old_audits = table(saved, "immutable_audit_outbox")?;
    let audits = table(current, "immutable_audit_outbox")?;
    for row in &audits.rows {
        if old_audits.rows.contains(row) {
            continue;
        }
        let node = StoredAuditChainNode {
            audit_identity: audits.text(row, "audit_identity")?.to_owned(),
            decision_identity: audits.text(row, "decision_identity")?.to_owned(),
            attempt_identity: audits
                .optional_text(row, "attempt_identity")?
                .map(str::to_owned),
            audit_kind: audits.text(row, "audit_kind")?.to_owned(),
            predecessor_audit_identity: audits
                .optional_text(row, "predecessor_audit_identity")?
                .map(str::to_owned),
            canonical: audits.blob(row, "audit_canonical")?.to_vec(),
            sha256: audits.text(row, "audit_sha256")?.to_owned(),
            append_state: audits.text(row, "append_state")?.to_owned(),
            immutable_audit_ref: audits
                .optional_text(row, "immutable_audit_ref")?
                .map(str::to_owned),
            created_at: audits.text(row, "created_at")?.to_owned(),
        };
        if !matches!(
            node.audit_kind.as_str(),
            "SinkResultAuthorityClassified" | "LateReceiptObserved" | "DecisionIdentityConflict"
        ) || sha256_hex(&node.canonical) != node.sha256
            || node.audit_identity
                != stable_identity(
                    "delivery-critical-audit-v1",
                    &[
                        &node.decision_identity,
                        node.attempt_identity.as_deref().unwrap_or("NONE"),
                        &node.audit_kind,
                        &node.sha256,
                    ],
                )
        {
            return Err(mismatch(
                "Physical historical extension is not exact allowed raw/conflict evidence",
            ));
        }
        match node.append_state.as_str() {
            "Pending" if node.immutable_audit_ref.is_none() => {}
            "Appended"
                if node
                    .immutable_audit_ref
                    .as_deref()
                    .is_some_and(has_non_ascii_whitespace) => {}
            _ => {
                return Err(mismatch(
                    "Physical historical extension has invalid append acknowledgment",
                ))
            }
        }
        parse_timestamp(&node.created_at)?;
        validate_audit_event(connection, current, &node)?;
    }
    for member in members {
        let mut root = None;
        let mut children = BTreeMap::new();
        let mut ids = BTreeSet::new();
        for row in &audits.rows {
            if audits.text(row, "decision_identity")? != member.decision {
                continue;
            }
            let id = audits.text(row, "audit_identity")?;
            ids.insert(id);
            match audits.optional_text(row, "predecessor_audit_identity")? {
                None => {
                    if root.replace(id).is_some() {
                        return Err(mismatch("Physical late extension has multiple genesis"));
                    }
                }
                Some(parent) => {
                    if children.insert(parent, id).is_some() {
                        return Err(mismatch("Physical late extension fork"));
                    }
                }
            }
        }
        let mut next = Some(root.ok_or_else(|| mismatch("Physical late extension lost genesis"))?);
        let mut seen = BTreeSet::new();
        while let Some(id) = next {
            if !seen.insert(id) {
                return Err(mismatch("Physical late extension cycle"));
            }
            next = children.get(id).copied();
        }
        if seen != ids {
            return Err(mismatch(
                "Physical late extension chain disconnected/cross-owner",
            ));
        }
    }
    Ok(())
}
fn exact_json(bytes: &[u8]) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if canonical_json(&value)? != bytes {
        return Err(mismatch("Physical audit JSON is not canonical"));
    }
    Ok(value)
}
fn same_json(bytes: &[u8], expected: serde_json::Value) -> Result<()> {
    if canonical_json(&expected)? != bytes {
        return Err(mismatch("Physical audit does not bind actual event"));
    }
    Ok(())
}
fn single<'a>(
    tables: &'a [Table],
    name: &str,
    column: &str,
    id: &str,
) -> Result<(&'a Table, &'a [Cell])> {
    let t = table(tables, name)?;
    let matches = t
        .rows
        .iter()
        .filter_map(|r| match t.text(r, column) {
            Ok(v) if v == id => Some(r.as_slice()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(mismatch("Physical audit has no unique actual row"));
    }
    Ok((t, matches[0]))
}

/// Each critical audit is connected to the original prepare genesis and to a
/// real typed event/result. A self-consistent hash alone cannot close a day.
fn validate_audits(connection: &Connection, tables: &[Table], members: &[Member]) -> Result<()> {
    let audits = table(tables, "immutable_audit_outbox")?;
    let mut covered = BTreeSet::new();
    for member in members {
        let nodes = audits
            .rows
            .iter()
            .filter_map(|r| match audits.text(r, "decision_identity") {
                Ok(id) if id == member.decision => Some(r),
                _ => None,
            })
            .collect::<Vec<_>>();
        if nodes.is_empty() {
            return Err(mismatch("Physical decision has no genesis"));
        }
        let mut children = BTreeMap::new();
        let mut root = None;
        for r in &nodes {
            let id = audits.text(r, "audit_identity")?;
            let node = load_sealed_audit_chain_node(connection, id)?;
            if node.decision_identity != member.decision {
                return Err(mismatch("Physical audit cross-owner"));
            }
            parse_timestamp(&node.created_at)?;
            match audits.optional_text(r, "predecessor_audit_identity")? {
                None => {
                    if root.replace(id).is_some() {
                        return Err(mismatch("Physical audit multiple genesis"));
                    }
                }
                Some(parent) => {
                    if children.insert(parent, id).is_some() {
                        return Err(mismatch("Physical audit fork"));
                    }
                }
            }
            validate_audit_event(connection, tables, &node)?;
        }
        let root = root.ok_or_else(|| mismatch("Physical audit missing genesis"))?;
        validate_prepare_genesis_audit(
            connection,
            &load_sealed_audit_chain_node(connection, root)?,
            &member.decision,
        )?;
        let mut next = Some(root);
        let mut seen = BTreeSet::new();
        let mut current_state: Option<String> = None;
        while let Some(id) = next {
            if !seen.insert(id) {
                return Err(mismatch("Physical audit cycle"));
            }
            let (audit, audit_row) =
                single(tables, "immutable_audit_outbox", "audit_identity", id)?;
            if audit.text(audit_row, "audit_kind")? == "DecisionStateChanged" {
                let (events, event) =
                    single(tables, "delivery_state_events", "audit_identity", id)?;
                if events.optional_text(event, "from_state")? != current_state.as_deref() {
                    return Err(mismatch("Physical state history is disconnected"));
                }
                current_state = Some(events.text(event, "to_state")?.to_owned());
            }
            next = children.get(id).copied();
        }
        if current_state.as_deref() != Some("Delivered") {
            return Err(mismatch("Physical audit history did not reach Delivered"));
        }
        if seen.len() != nodes.len() {
            return Err(mismatch("Physical audit chain disconnected"));
        }
        covered.extend(seen.into_iter().map(str::to_owned));
    }
    if covered.len() != audits.rows.len() {
        return Err(mismatch("Physical unrelated audit present"));
    }
    for (name, col) in [
        ("delivery_state_events", "audit_identity"),
        ("delivery_attempt_events", "audit_identity"),
        ("daily_budget_reservation_events", "audit_identity"),
        ("cooldown_reservation_events", "audit_identity"),
    ] {
        let t = table(tables, name)?;
        if t.rows.iter().any(|r| match t.text(r, col) {
            Ok(id) => !covered.contains(id),
            Err(_) => true,
        }) {
            return Err(mismatch("Physical orphan event"));
        }
    }
    Ok(())
}

fn validate_raw(tables: &[Table]) -> Result<()> {
    let t = table(tables, "sink_results")?;
    for row in &t.rows {
        let attempt = t.text(row, "attempt_identity")?;
        let decision = t.text(row, "decision_identity")?;
        let kind = t.text(row, "result_kind")?;
        let bytes = t.blob(row, "result_canonical")?;
        let sha = t.text(row, "result_sha256")?;
        let fence = t.integer(row, "fence_token")?;
        let (a, r) = single(tables, "delivery_attempts", "attempt_identity", attempt)?;
        if a.text(r, "decision_identity")? != decision
            || a.integer(r, "fence_token")? != fence
            || sha256_hex(bytes) != sha
            || t.text(row, "result_event_identity")?
                != stable_identity(
                    "delivery-sink-result-v1",
                    &[attempt, &fence.to_string(), kind, sha],
                )
        {
            return Err(mismatch("Physical raw result identity/binding differs"));
        }
        parse_timestamp(t.text(row, "observed_at")?)?;
        let authoritative = t.integer(row, "authoritative_for_state")?;
        let late = t.integer(row, "late_after_fence")?;
        if !matches!((authoritative, late), (1, 0) | (0, 1))
            || (late == 1)
                != t.optional_text(row, "late_receipt_audit_identity")?
                    .is_some()
        {
            return Err(mismatch("Physical raw authority flags differ"));
        }
        if authoritative == 1 && a.text(r, "state")? != kind {
            return Err(mismatch(
                "Physical authoritative raw differs from original attempt state",
            ));
        }
        single(
            tables,
            "immutable_audit_outbox",
            "audit_identity",
            t.text(row, "authority_audit_identity")?,
        )?;
        if let Some(id) = t.optional_text(row, "late_receipt_audit_identity")? {
            single(tables, "immutable_audit_outbox", "audit_identity", id)?;
        }
        match kind {
            "Accepted" => {
                let value = AcceptedSinkResultCanonical::parse_exact(bytes)?;
                value.receipt.validate()?;
                let receipt = &value.receipt;
                if value.kind != "Accepted"
                    || t.text(row, "channel")? != receipt.channel
                    || t.text(row, "provider")? != receipt.provider
                    || t.text(row, "message_id")? != receipt.message_id
                    || t.optional_text(row, "platform_message_id")?
                        != receipt.platform_message_id.as_deref()
                    || t.text(row, "accepted_at")? != timestamp(receipt.accepted_at)
                    || t.cell(row, "latency_ms")?
                        != &receipt.latency_ms.map(Cell::Integer).unwrap_or(Cell::Null)
                {
                    return Err(mismatch("Physical typed raw Accepted columns differ"));
                }
                if authoritative == 0 {
                    for column in [
                        "frozen_delivery_audit_canonical",
                        "frozen_delivery_audit_sha256",
                        "delivery_audit_ref",
                    ] {
                        if t.cell(row, column)? != &Cell::Null {
                            return Err(mismatch(
                                "Physical late Accepted gained delivery authority",
                            ));
                        }
                    }
                } else {
                    let audit = t.blob(row, "frozen_delivery_audit_canonical")?;
                    if sha256_hex(audit) != t.text(row, "frozen_delivery_audit_sha256")?
                        || !has_non_ascii_whitespace(t.text(row, "delivery_audit_ref")?)
                    {
                        return Err(mismatch(
                            "Physical original Accepted delivery audit undrained",
                        ));
                    }
                    same_json(
                        audit,
                        json!({"decision_identity":decision,"attempt_identity":attempt,"fence_token":fence,"result_sha256":sha,"receipt":receipt}),
                    )?;
                }
            }
            "Rejected" => {
                let value = RejectedSinkResultCanonical::parse_exact(bytes)?;
                if value.kind != "Rejected"
                    || value.rejection.reason_code.trim().is_empty()
                    || value.rejection.evidence.is_empty()
                {
                    return Err(mismatch("Physical raw rejection invalid"));
                }
            }
            "Uncertain" => {
                let value = UncertainSinkResultCanonical::parse_exact(bytes)?;
                if value.kind != "Uncertain"
                    || value.uncertainty.reason_code.trim().is_empty()
                    || value.uncertainty.evidence.is_empty()
                {
                    return Err(mismatch("Physical raw uncertainty invalid"));
                }
            }
            _ => return Err(mismatch("Physical raw result kind unsupported")),
        }
        if kind != "Accepted" {
            for column in [
                "channel",
                "provider",
                "message_id",
                "platform_message_id",
                "accepted_at",
                "latency_ms",
                "frozen_delivery_audit_canonical",
                "frozen_delivery_audit_sha256",
                "delivery_audit_ref",
            ] {
                if t.cell(row, column)? != &Cell::Null {
                    return Err(mismatch(
                        "Physical non-Accepted has receipt authority columns",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Last in the real predecessor chain, not MAX(timestamp) or lexical audit id.
/// Caller timestamps may move backwards; authority is the original append order.
fn last_lease_binding(tables: &[Table], decision: &str, attempt: &str) -> Result<(String, String)> {
    let audits = table(tables, "immutable_audit_outbox")?;
    let mut root = None;
    let mut children = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for row in &audits.rows {
        if audits.text(row, "decision_identity")? != decision {
            continue;
        }
        let id = audits.text(row, "audit_identity")?;
        ids.insert(id);
        match audits.optional_text(row, "predecessor_audit_identity")? {
            None => {
                if root.replace(id).is_some() {
                    return Err(mismatch("Physical lease chain has multiple genesis"));
                }
            }
            Some(parent) => {
                if children.insert(parent, id).is_some() {
                    return Err(mismatch("Physical lease chain fork"));
                }
            }
        }
    }
    let mut next = Some(root.ok_or_else(|| mismatch("Physical lease chain genesis absent"))?);
    let mut seen = BTreeSet::new();
    let mut last = None;
    let mut granted = false;
    while let Some(id) = next {
        if !seen.insert(id) {
            return Err(mismatch("Physical lease chain cycle"));
        }
        let (_, row) = single(tables, "immutable_audit_outbox", "audit_identity", id)?;
        if audits.optional_text(row, "attempt_identity")? == Some(attempt) {
            let kind = audits.text(row, "audit_kind")?;
            if matches!(kind, "LeaseGranted" | "LeaseHeartbeat") {
                if (kind == "LeaseGranted" && granted) || (kind == "LeaseHeartbeat" && !granted) {
                    return Err(mismatch(
                        "Physical lease audit order differs from original grant",
                    ));
                }
                granted = true;
                let value = exact_json(audits.blob(row, "audit_canonical")?)?;
                let expiry = value
                    .get("lease_expires_at")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| mismatch("Physical lease audit expiry missing"))?
                    .to_owned();
                let created = audits.text(row, "created_at")?.to_owned();
                parse_timestamp(&expiry)?;
                parse_timestamp(&created)?;
                last = Some((created, expiry));
            }
        }
        next = children.get(id).copied();
    }
    if seen != ids {
        return Err(mismatch("Physical lease chain is disconnected"));
    }
    last.ok_or_else(|| mismatch("Physical original lease audit missing"))
}

fn validate_attempts(tables: &[Table], members: &[Member]) -> Result<()> {
    let attempts = table(tables, "delivery_attempts")?;
    let raw = table(tables, "sink_results")?;
    let audits = table(tables, "immutable_audit_outbox")?;
    for row in &attempts.rows {
        let id = attempts.text(row, "attempt_identity")?;
        let decision = attempts.text(row, "decision_identity")?;
        let number = attempts.integer(row, "attempt_no")?;
        let fence = attempts.integer(row, "fence_token")?;
        if number <= 0
            || fence <= 0
            || id
                != stable_identity(
                    "delivery-attempt-v1",
                    &[decision, &number.to_string(), &fence.to_string()],
                )
            || !has_non_ascii_whitespace(attempts.text(row, "owner_instance_identity")?)
        {
            return Err(mismatch("Physical original attempt identity invalid"));
        }
        for column in ["started_at", "lease_heartbeat_at", "lease_expires_at"] {
            parse_timestamp(attempts.text(row, column)?)?;
        }
        let grants = audits
            .rows
            .iter()
            .filter(|r| {
                audits.optional_text(r, "attempt_identity").ok() == Some(Some(id))
                    && audits.text(r, "audit_kind").ok() == Some("LeaseGranted")
            })
            .count();
        let originals = raw
            .rows
            .iter()
            .filter(|r| {
                raw.text(r, "attempt_identity").ok() == Some(id)
                    && raw.integer(r, "authoritative_for_state").ok() == Some(1)
            })
            .collect::<Vec<_>>();
        if grants != 1 || originals.len() != 1 {
            return Err(mismatch(
                "Physical original attempt has no unique lease/original result",
            ));
        }
        let (last_heartbeat, last_expiry) = last_lease_binding(tables, decision, id)?;
        if attempts.text(row, "lease_heartbeat_at")? != last_heartbeat
            || attempts.text(row, "lease_expires_at")? != last_expiry
        {
            return Err(mismatch(
                "Physical current lease columns differ from last original lease audit",
            ));
        }
        let original = originals[0];
        if !matches!(attempts.text(row, "state")?, "Accepted" | "Rejected")
            || attempts.optional_text(row, "fence_revoked_at")?.is_some()
        {
            return Err(mismatch(
                "Physical unresolved/recovered attempt is not original authority",
            ));
        }
        let member = members
            .iter()
            .find(|v| v.decision == decision)
            .ok_or_else(|| mismatch("Physical attempt has no selected decision"))?;
        if member.attempt == id {
            let (d, stored) = single(tables, "delivery_decisions", "decision_identity", decision)?;
            if d.integer(stored, "fence_generation")? != fence
                || raw.text(original, "result_kind")? != "Accepted"
            {
                return Err(mismatch(
                    "Physical final attempt fence/Accepted binding differs",
                ));
            }
        }
    }
    Ok(())
}

fn validate_reservations(tables: &[Table], members: &[Member]) -> Result<()> {
    for (name, events_name, column, domain) in [
        (
            "daily_budget_reservations",
            "daily_budget_reservation_events",
            "budget_reservation_identity",
            "delivery-budget-reservation-v1",
        ),
        (
            "cooldown_reservations",
            "cooldown_reservation_events",
            "cooldown_reservation_identity",
            "delivery-cooldown-reservation-v1",
        ),
    ] {
        let budget = name == "daily_budget_reservations";
        let reservations = table(tables, name)?;
        let events = table(tables, events_name)?;
        for row in &reservations.rows {
            let id = reservations.text(row, column)?;
            let decision = reservations.text(row, "decision_identity")?;
            let generation = reservations.integer(row, "reservation_generation")?;
            let (d, owner) = single(tables, "delivery_decisions", "decision_identity", decision)?;
            let envelope = parse_envelope(d.blob(owner, "envelope_canonical")?)?;
            if id != stable_identity(domain, &[decision, &generation.to_string()])
                || reservations.text(row, "business_date")? != envelope.business_date
                || generation <= 0
                || generation > d.integer(owner, "reservation_generation")?
            {
                return Err(mismatch(
                    "Physical reservation identity/date/generation differs",
                ));
            }
            let matching = events
                .rows
                .iter()
                .filter(|r| events.text(r, column).ok() == Some(id))
                .collect::<Vec<_>>();
            let audits = table(tables, "immutable_audit_outbox")?;
            // Chronology comes from the actual critical-audit predecessor chain,
            // not caller timestamps or SQL row ordering.
            let mut order = BTreeMap::new();
            let mut next = audits.rows.iter().find(|r| {
                audits.text(r, "decision_identity").ok() == Some(decision)
                    && audits.optional_text(r, "predecessor_audit_identity").ok() == Some(None)
            });
            let mut position = 0;
            while let Some(node) = next {
                let audit = audits.text(node, "audit_identity")?;
                if order.insert(audit, position).is_some() {
                    return Err(mismatch("Physical reservation audit cycle"));
                }
                position += 1;
                next = audits.rows.iter().find(|r| {
                    audits.text(r, "decision_identity").ok() == Some(decision)
                        && audits.optional_text(r, "predecessor_audit_identity").ok()
                            == Some(Some(audit))
                });
            }
            let mut matching = matching;
            matching.sort_by_key(|r| {
                order
                    .get(events.text(r, "audit_identity").unwrap_or(""))
                    .copied()
                    .unwrap_or(usize::MAX)
            });
            let mut state = None::<String>;
            for event in matching {
                let from = events.optional_text(event, "from_state")?;
                let to = events.text(event, "to_state")?;
                if from != state.as_deref() {
                    return Err(mismatch("Physical reservation history is disconnected"));
                }
                let (a, audit) = single(
                    tables,
                    "immutable_audit_outbox",
                    "audit_identity",
                    events.text(event, "audit_identity")?,
                )?;
                let occurred_at = a.text(audit, "created_at")?;
                let bytes = events.blob(event, "event_canonical")?;
                let expected = if from.is_none() {
                    if to != "Reserved" || reservations.text(row, "reserved_at")? != occurred_at {
                        return Err(mismatch("Physical reservation original reserve differs"));
                    }
                    if budget {
                        json!({"reservation_generation":generation,"slot_no":reservations.integer(row,"slot_no")?})
                    } else {
                        json!({"reservation_generation":generation,"window_mode":reservations.text(row,"window_mode")?})
                    }
                } else if from == Some("Reserved") && to == "Reserved" {
                    let attempt = reservations
                        .optional_text(row, "attempt_identity")?
                        .ok_or_else(|| mismatch("Physical attached reservation has no attempt"))?;
                    let (a, r) = single(tables, "delivery_attempts", "attempt_identity", attempt)?;
                    if a.text(r, "decision_identity")? != decision
                        || a.text(r, "started_at")? != occurred_at
                    {
                        return Err(mismatch("Physical reservation attempt attachment differs"));
                    }
                    json!({"attempt_identity":attempt,"attempt_no":a.integer(r,"attempt_no")?,"reservation_generation":generation})
                } else {
                    if !matches!(from, Some("Reserved" | "Uncertain"))
                        || !matches!(to, "Accepted" | "Uncertain" | "Released")
                    {
                        return Err(mismatch("Physical reservation illegal state history"));
                    }
                    if budget {
                        json!({"reservation_generation":generation,"occurred_at":occurred_at})
                    } else {
                        let blocked = if to == "Accepted" {
                            reservations.optional_text(row, "blocked_until")?
                        } else {
                            None
                        };
                        json!({"reservation_generation":generation,"blocked_until":blocked})
                    }
                };
                same_json(bytes, expected)?;
                state = Some(to.to_owned());
            }
            if state.as_deref() != Some(reservations.text(row, "state")?) {
                return Err(mismatch(
                    "Physical reservation current state differs from original events",
                ));
            }
            let current = d.optional_text(
                owner,
                if budget {
                    "current_budget_reservation_identity"
                } else {
                    "current_cooldown_reservation_identity"
                },
            )?;
            if current == Some(id) {
                let member = members
                    .iter()
                    .find(|v| v.decision == decision)
                    .ok_or_else(|| mismatch("Physical reservation owner unselected"))?;
                if reservations.text(row, "state")? != "Accepted"
                    || reservations.optional_text(row, "attempt_identity")?
                        != Some(member.attempt.as_str())
                    || generation != d.integer(owner, "reservation_generation")?
                {
                    return Err(mismatch(
                        "Physical current reservation lacks original Accepted",
                    ));
                }
            } else if reservations.text(row, "state")? != "Released" {
                return Err(mismatch("Physical old reservation remains active"));
            }
        }
    }
    Ok(())
}

fn members(
    connection: &Connection,
    bundle: &VerifiedG5bModelBundle,
    tables: &[Table],
) -> Result<Option<Vec<Member>>> {
    let owners = table(tables, "g5b_occurrence_owners")?;
    let decisions = table(tables, "delivery_decisions")?;
    if owners.rows.len() != bundle.cohort().selected_count()
        || decisions.rows.len() != owners.rows.len()
    {
        return Ok(None);
    }
    let mut values = Vec::new();
    let mut seen = BTreeSet::new();
    for member in bundle.members() {
        let (owner, row) = single(
            tables,
            "g5b_occurrence_owners",
            "occurrence_identity",
            member.occurrence(),
        )?;
        let id = owner.text(row, "decision_identity")?;
        if !seen.insert(id.to_owned())
            || owner.text(row, "cohort_identity")? != bundle.cohort().identity()
        {
            return Err(mismatch("Physical duplicate/foreign occurrence owner"));
        }
        let stored = load_decision(connection, id)?
            .ok_or_else(|| mismatch("Physical original decision missing"))?;
        let envelope = parse_envelope(&stored.envelope_canonical)?;
        if envelope.push_kind != PushKind::G5bAttribution
            || envelope.business_date
                != bundle.cohort().evidence.encoded().business_date.to_string()
            || envelope.schedule_occurrence_identity != member.occurrence()
            || envelope.canonical_sha256()? != stored.envelope_sha256
            || envelope.task_binding.is_some()
        {
            return Err(mismatch(
                "Physical decision does not bind taskless selected occurrence",
            ));
        }
        super::super::g5b_v2::validate_owner_tx(connection, &envelope)?;
        if stored.state != DecisionState::Delivered {
            return Ok(None);
        }
        let disposition = load_current_disposition_evidence(connection, &stored)?;
        if disposition.disposition != "Accepted" {
            return Ok(None);
        }
        let terminal = build_validated_terminal_evidence(connection, &stored, &envelope, None)?;
        if terminal.disposition != FoundationTerminalDisposition::Accepted {
            return Ok(None);
        }
        values.push(Member {
            index: member.index(),
            occurrence: member.occurrence().to_owned(),
            decision: id.to_owned(),
            attempt: terminal
                .attempt_id
                .ok_or_else(|| mismatch("Physical Accepted attempt missing"))?,
            evidence_sha256: terminal.evidence_sha256,
        });
    }
    if values.is_empty() || values.len() > 3 {
        return Err(mismatch("Physical selected cardinality invalid"));
    }
    Ok(Some(values))
}
fn drained(tables: &[Table]) -> Result<bool> {
    for name in ["immutable_audit_outbox", "delivery_disposition_payloads"] {
        let t = table(tables, name)?;
        for row in &t.rows {
            if t.text(row, "append_state")? != "Appended"
                || t.optional_text(row, "immutable_audit_ref")?
                    .is_none_or(|v| !has_non_ascii_whitespace(v))
            {
                return Ok(false);
            }
        }
    }
    // C's closed original handoff is taskless. A task/review/manual side effect
    // cannot be laundered by a physical receipt or dropped from the snapshot.
    for name in [
        "task_transition_payloads",
        "manual_resolutions",
        "review_terminal_replay_attempts",
        "review_terminal_replay_completions",
        "delivery_correlation_observations",
    ] {
        if !table(tables, name)?.rows.is_empty() {
            return Ok(false);
        }
    }
    let raw = table(tables, "sink_results")?;
    for row in &raw.rows {
        if raw.integer(row, "authoritative_for_state")? == 1
            && raw.text(row, "result_kind")? == "Accepted"
            && raw
                .optional_text(row, "delivery_audit_ref")?
                .is_none_or(|v| !has_non_ascii_whitespace(v))
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn candidate(
    connection: &Connection,
    date: NaiveDate,
    bundle: &VerifiedG5bModelBundle,
) -> Result<Option<PhysicalSeal>> {
    candidate_version(connection, date, bundle, 2)
}
fn candidate_version(
    connection: &Connection,
    date: NaiveDate,
    bundle: &VerifiedG5bModelBundle,
    storage_version: u8,
) -> Result<Option<PhysicalSeal>> {
    if bundle.head_state() != "Clean" {
        return Ok(None);
    }
    let Some(full_archive_identity) =
        crate::monitor::g5b_analysis_v2::full_model_archive_identity_v2(bundle)
            .map_err(codec_error)?
    else {
        return Ok(None);
    };
    let mut budget = WitnessBudget::for_connection(connection);
    let tables = if storage_version == 1 {
        rows_legacy_with_budget(connection, date, &mut budget)?
    } else {
        rows_with_budget(connection, date, &mut budget)?
    };
    if !drained(&tables)? {
        return Ok(None);
    }
    let Some(members) = members(connection, bundle, &tables)? else {
        return Ok(None);
    };
    validate_raw(&tables)?;
    validate_payloads(&tables)?;
    validate_audits(connection, &tables, &members)?;
    validate_attempts(&tables, &members)?;
    validate_reservations(&tables, &members)?;
    let value = PhysicalSeal {
        storage_version,
        version: 1,
        material: MATERIAL.to_owned(),
        business_date: date,
        cohort_identity: bundle.cohort().identity(),
        revision: bundle.revision(),
        reason: REASON.to_owned(),
        model_binding: if storage_version == 1 {
            bundle.seal_model_binding(&mut budget)?
        } else {
            bundle.seal_model_binding_shared(&mut budget)?
        },
        full_archive_identity,
        members,
        tables,
    };
    if storage_version == 1 {
        encode_bounded(&value, &mut budget)?;
    } else {
        encode_arena(&value, &mut budget)?;
    }
    Ok(Some(value))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum ArenaCell {
    Null,
    Integer(i64),
    TextRef(usize),
    BlobRef(usize),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArenaTable {
    name: String,
    columns: Vec<String>,
    rows: Vec<Vec<ArenaCell>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArenaArtifact {
    event_identity: String,
    material: EventMaterial,
    desired_bytes: usize,
    committed: artifact::FileWitness,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArenaModelBinding {
    selection_bytes: usize,
    admission: Admission,
    artifacts: Vec<ArenaArtifact>,
}
#[derive(Serialize)]
struct ArenaPhysical {
    version: u8,
    material: String,
    business_date: NaiveDate,
    cohort_identity: String,
    revision: i64,
    reason: String,
    model_binding: ArenaModelBinding,
    full_archive_identity: String,
    members: Vec<Member>,
    tables: Vec<ArenaTable>,
    arena: Vec<EvidenceBytes>,
}
fn collect_arena_bytes(
    arena: &mut Vec<EvidenceBytes>,
    bytes: &[u8],
    budget: &mut WitnessBudget,
) -> Result<()> {
    if let Err(index) = arena.binary_search_by(|v| (&**v).cmp(bytes)) {
        budget.descriptor::<EvidenceBytes>(4)?;
        arena.insert(index, budget.intern(bytes)?);
    }
    Ok(())
}
fn collect_table_bytes(
    arena: &mut Vec<EvidenceBytes>,
    tables: &[Table],
    budget: &mut WitnessBudget,
) -> Result<()> {
    for table in tables {
        for row in &table.rows {
            for cell in row {
                match cell {
                    Cell::Text(v) => collect_arena_bytes(arena, v.as_bytes(), budget)?,
                    Cell::Blob(v) => collect_arena_bytes(arena, v, budget)?,
                    _ => (),
                }
            }
        }
    }
    Ok(())
}
fn arena_index(arena: &[EvidenceBytes], bytes: &[u8]) -> Result<usize> {
    arena
        .binary_search_by(|v| (&**v).cmp(bytes))
        .map_err(|_| mismatch("Physical arena original bytes absent"))
}
fn arena_tables(
    tables: &[Table],
    arena: &[EvidenceBytes],
    budget: &mut WitnessBudget,
) -> Result<Vec<ArenaTable>> {
    budget.descriptor::<ArenaTable>(tables.len())?;
    let mut result = Vec::with_capacity(tables.len());
    for table in tables {
        budget.reserve_clone(&table.name)?;
        budget.reserve_clone(&table.columns)?;
        budget.descriptor::<String>(table.columns.len())?;
        budget.descriptor::<Vec<ArenaCell>>(table.rows.len())?;
        let mut rows = Vec::with_capacity(table.rows.len());
        for row in &table.rows {
            budget.descriptor::<ArenaCell>(row.len())?;
            let mut cells = Vec::with_capacity(row.len());
            for cell in row {
                cells.push(match cell {
                    Cell::Null => ArenaCell::Null,
                    Cell::Integer(v) => ArenaCell::Integer(*v),
                    Cell::Text(v) => ArenaCell::TextRef(arena_index(arena, v.as_bytes())?),
                    Cell::Blob(v) => ArenaCell::BlobRef(arena_index(arena, v)?),
                });
            }
            rows.push(cells);
        }
        result.push(ArenaTable {
            name: table.name.clone(),
            columns: table.columns.clone(),
            rows,
        });
    }
    Ok(result)
}
fn arena_physical(value: &PhysicalSeal, budget: &mut WitnessBudget) -> Result<ArenaPhysical> {
    let mut arena = Vec::new();
    collect_table_bytes(&mut arena, &value.tables, budget)?;
    collect_arena_bytes(&mut arena, &value.model_binding.selection_bytes, budget)?;
    for item in &value.model_binding.artifacts {
        collect_arena_bytes(&mut arena, &item.desired_bytes, budget)?;
    }
    let tables = arena_tables(&value.tables, &arena, budget)?;
    budget.descriptor::<ArenaArtifact>(value.model_binding.artifacts.len())?;
    let mut artifacts = Vec::with_capacity(value.model_binding.artifacts.len());
    for item in &value.model_binding.artifacts {
        budget.reserve_clone(&item.event_identity)?;
        budget.reserve_clone(&item.material)?;
        budget.reserve_clone(&item.committed)?;
        artifacts.push(ArenaArtifact {
            event_identity: item.event_identity.clone(),
            material: item.material.clone(),
            desired_bytes: arena_index(&arena, &item.desired_bytes)?,
            committed: item.committed.clone(),
        });
    }
    budget.reserve_clone(&value.model_binding.admission)?;
    budget.reserve_clone(&value.members)?;
    budget.descriptor::<Member>(value.members.len())?;
    budget.reserve_clone(&(
        &value.material,
        &value.cohort_identity,
        &value.reason,
        &value.full_archive_identity,
    ))?;
    Ok(ArenaPhysical {
        version: 2,
        material: value.material.clone(),
        business_date: value.business_date,
        cohort_identity: value.cohort_identity.clone(),
        revision: value.revision,
        reason: value.reason.clone(),
        model_binding: ArenaModelBinding {
            selection_bytes: arena_index(&arena, &value.model_binding.selection_bytes)?,
            admission: value.model_binding.admission.clone(),
            artifacts,
        },
        full_archive_identity: value.full_archive_identity.clone(),
        members: value.members.clone(),
        tables,
        arena,
    })
}
fn encode_arena(value: &PhysicalSeal, budget: &mut WitnessBudget) -> Result<Vec<u8>> {
    encode_bounded(&arena_physical(value, budget)?, budget)
}
fn encode_table_arena(tables: &[Table], budget: &mut WitnessBudget) -> Result<Vec<u8>> {
    let mut arena = Vec::new();
    collect_table_bytes(&mut arena, tables, budget)?;
    let tables = arena_tables(tables, &arena, budget)?;
    encode_bounded(&(2u8, arena, tables), budget)
}

/// Metadata deserialization is budgeted BEFORE serde can copy a String or grow
/// a vector/map. Four slots/element conservatively cover geometric Vec growth.
struct BudgetDeserializer<'a, D> {
    inner: D,
    budget: &'a mut WitnessBudget,
}
struct BudgetVisitor<'a, V> {
    inner: V,
    budget: &'a mut WitnessBudget,
}
struct BudgetSeed<'a, S> {
    inner: S,
    budget: &'a mut WitnessBudget,
}
impl<'de, S: serde::de::DeserializeSeed<'de>> serde::de::DeserializeSeed<'de>
    for BudgetSeed<'_, S>
{
    type Value = S::Value;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        self.inner.deserialize(BudgetDeserializer {
            inner: d,
            budget: self.budget,
        })
    }
}
struct ValueSeed<T>(std::marker::PhantomData<T>);
impl<'de, T: Deserialize<'de>> serde::de::DeserializeSeed<'de> for ValueSeed<T> {
    type Value = T;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> std::result::Result<T, D::Error> {
        T::deserialize(d)
    }
}
macro_rules! budget_deser_simple { ($($name:ident),*)=>{$(fn $name<V:serde::de::Visitor<'de>>(self,v:V)->std::result::Result<V::Value,D::Error>{self.inner.$name(BudgetVisitor{inner:v,budget:self.budget})})*}; }
impl<'de, D: serde::Deserializer<'de>> serde::Deserializer<'de> for BudgetDeserializer<'_, D> {
    type Error = D::Error;
    budget_deser_simple!(
        deserialize_any,
        deserialize_bool,
        deserialize_i8,
        deserialize_i16,
        deserialize_i32,
        deserialize_i64,
        deserialize_i128,
        deserialize_u8,
        deserialize_u16,
        deserialize_u32,
        deserialize_u64,
        deserialize_u128,
        deserialize_f32,
        deserialize_f64,
        deserialize_char,
        deserialize_str,
        deserialize_string,
        deserialize_bytes,
        deserialize_byte_buf,
        deserialize_option,
        deserialize_unit,
        deserialize_seq,
        deserialize_map,
        deserialize_identifier,
        deserialize_ignored_any
    );
    fn deserialize_unit_struct<V: serde::de::Visitor<'de>>(
        self,
        n: &'static str,
        v: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.inner.deserialize_unit_struct(
            n,
            BudgetVisitor {
                inner: v,
                budget: self.budget,
            },
        )
    }
    fn deserialize_newtype_struct<V: serde::de::Visitor<'de>>(
        self,
        n: &'static str,
        v: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.inner.deserialize_newtype_struct(
            n,
            BudgetVisitor {
                inner: v,
                budget: self.budget,
            },
        )
    }
    fn deserialize_tuple<V: serde::de::Visitor<'de>>(
        self,
        n: usize,
        v: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.inner.deserialize_tuple(
            n,
            BudgetVisitor {
                inner: v,
                budget: self.budget,
            },
        )
    }
    fn deserialize_tuple_struct<V: serde::de::Visitor<'de>>(
        self,
        n: &'static str,
        l: usize,
        v: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.inner.deserialize_tuple_struct(
            n,
            l,
            BudgetVisitor {
                inner: v,
                budget: self.budget,
            },
        )
    }
    fn deserialize_struct<V: serde::de::Visitor<'de>>(
        self,
        n: &'static str,
        f: &'static [&'static str],
        v: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.inner.deserialize_struct(
            n,
            f,
            BudgetVisitor {
                inner: v,
                budget: self.budget,
            },
        )
    }
    fn deserialize_enum<V: serde::de::Visitor<'de>>(
        self,
        n: &'static str,
        f: &'static [&'static str],
        v: V,
    ) -> std::result::Result<V::Value, D::Error> {
        self.inner.deserialize_enum(
            n,
            f,
            BudgetVisitor {
                inner: v,
                budget: self.budget,
            },
        )
    }
    fn is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }
}
macro_rules! budget_visit_scalar {($($name:ident:$type:ty),*)=>{$(fn $name<E:serde::de::Error>(self,v:$type)->std::result::Result<V::Value,E>{self.inner.$name(v)})*};}
impl<'de, V: serde::de::Visitor<'de>> serde::de::Visitor<'de> for BudgetVisitor<'_, V> {
    type Value = V::Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.inner.expecting(f)
    }
    budget_visit_scalar!(visit_bool:bool,visit_i8:i8,visit_i16:i16,visit_i32:i32,visit_i64:i64,visit_i128:i128,visit_u8:u8,visit_u16:u16,visit_u32:u32,visit_u64:u64,visit_u128:u128,visit_f32:f32,visit_f64:f64,visit_char:char);
    fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<V::Value, E> {
        self.budget
            .reserve(
                v.len()
                    .checked_mul(2)
                    .and_then(|n| n.checked_add(std::mem::size_of::<String>()))
                    .ok_or_else(|| E::custom("Physical string budget overflow"))?,
            )
            .map_err(E::custom)?;
        self.inner.visit_str(v)
    }
    fn visit_borrowed_str<E: serde::de::Error>(
        self,
        v: &'de str,
    ) -> std::result::Result<V::Value, E> {
        self.budget
            .reserve(
                v.len()
                    .checked_mul(2)
                    .and_then(|n| n.checked_add(std::mem::size_of::<String>()))
                    .ok_or_else(|| E::custom("Physical string budget overflow"))?,
            )
            .map_err(E::custom)?;
        self.inner.visit_borrowed_str(v)
    }
    fn visit_string<E: serde::de::Error>(self, _: String) -> std::result::Result<V::Value, E> {
        Err(E::custom("Physical metadata requires slice deserializer"))
    }
    fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> std::result::Result<V::Value, E> {
        self.budget.reserve(v.len()).map_err(E::custom)?;
        self.inner.visit_bytes(v)
    }
    fn visit_borrowed_bytes<E: serde::de::Error>(
        self,
        v: &'de [u8],
    ) -> std::result::Result<V::Value, E> {
        self.budget.reserve(v.len()).map_err(E::custom)?;
        self.inner.visit_borrowed_bytes(v)
    }
    fn visit_byte_buf<E: serde::de::Error>(self, _: Vec<u8>) -> std::result::Result<V::Value, E> {
        Err(E::custom("Physical metadata requires slice deserializer"))
    }
    fn visit_none<E: serde::de::Error>(self) -> std::result::Result<V::Value, E> {
        self.inner.visit_none()
    }
    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<V::Value, E> {
        self.inner.visit_unit()
    }
    fn visit_some<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<V::Value, D::Error> {
        self.inner.visit_some(BudgetDeserializer {
            inner: d,
            budget: self.budget,
        })
    }
    fn visit_newtype_struct<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<V::Value, D::Error> {
        self.inner.visit_newtype_struct(BudgetDeserializer {
            inner: d,
            budget: self.budget,
        })
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(
        self,
        a: A,
    ) -> std::result::Result<V::Value, A::Error> {
        self.inner.visit_seq(BudgetSeq {
            inner: a,
            budget: self.budget,
        })
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        a: A,
    ) -> std::result::Result<V::Value, A::Error> {
        self.inner.visit_map(BudgetMap {
            inner: a,
            budget: self.budget,
        })
    }
    fn visit_enum<A: serde::de::EnumAccess<'de>>(
        self,
        a: A,
    ) -> std::result::Result<V::Value, A::Error> {
        self.inner.visit_enum(BudgetEnum {
            inner: a,
            budget: self.budget,
        })
    }
}
struct BudgetSeq<'a, A> {
    inner: A,
    budget: &'a mut WitnessBudget,
}
impl<'de, A: serde::de::SeqAccess<'de>> serde::de::SeqAccess<'de> for BudgetSeq<'_, A> {
    type Error = A::Error;
    fn next_element_seed<S: serde::de::DeserializeSeed<'de>>(
        &mut self,
        s: S,
    ) -> std::result::Result<Option<S::Value>, A::Error> {
        self.budget
            .descriptor::<S::Value>(4)
            .map_err(serde::de::Error::custom)?;
        self.inner.next_element_seed(BudgetSeed {
            inner: s,
            budget: self.budget,
        })
    }
    fn size_hint(&self) -> Option<usize> {
        None
    }
}
struct BudgetMap<'a, A> {
    inner: A,
    budget: &'a mut WitnessBudget,
}
impl<'de, A: serde::de::MapAccess<'de>> serde::de::MapAccess<'de> for BudgetMap<'_, A> {
    type Error = A::Error;
    fn next_key_seed<S: serde::de::DeserializeSeed<'de>>(
        &mut self,
        s: S,
    ) -> std::result::Result<Option<S::Value>, A::Error> {
        self.budget
            .descriptor::<S::Value>(4)
            .map_err(serde::de::Error::custom)?;
        self.inner.next_key_seed(BudgetSeed {
            inner: s,
            budget: self.budget,
        })
    }
    fn next_value_seed<S: serde::de::DeserializeSeed<'de>>(
        &mut self,
        s: S,
    ) -> std::result::Result<S::Value, A::Error> {
        self.budget
            .descriptor::<S::Value>(4)
            .map_err(serde::de::Error::custom)?;
        self.inner.next_value_seed(BudgetSeed {
            inner: s,
            budget: self.budget,
        })
    }
    fn size_hint(&self) -> Option<usize> {
        None
    }
}
struct BudgetEnum<'a, A> {
    inner: A,
    budget: &'a mut WitnessBudget,
}
struct BudgetVariant<'a, A> {
    inner: A,
    budget: &'a mut WitnessBudget,
}
impl<'de, 'a, A: serde::de::EnumAccess<'de>> serde::de::EnumAccess<'de> for BudgetEnum<'a, A> {
    type Error = A::Error;
    type Variant = BudgetVariant<'a, A::Variant>;
    fn variant_seed<S: serde::de::DeserializeSeed<'de>>(
        self,
        s: S,
    ) -> std::result::Result<(S::Value, Self::Variant), A::Error> {
        let (v, a) = self.inner.variant_seed(BudgetSeed {
            inner: s,
            budget: &mut *self.budget,
        })?;
        Ok((
            v,
            BudgetVariant {
                inner: a,
                budget: self.budget,
            },
        ))
    }
}
impl<'de, A: serde::de::VariantAccess<'de>> serde::de::VariantAccess<'de> for BudgetVariant<'_, A> {
    type Error = A::Error;
    fn unit_variant(self) -> std::result::Result<(), A::Error> {
        self.inner.unit_variant()
    }
    fn newtype_variant_seed<S: serde::de::DeserializeSeed<'de>>(
        self,
        s: S,
    ) -> std::result::Result<S::Value, A::Error> {
        self.inner.newtype_variant_seed(BudgetSeed {
            inner: s,
            budget: self.budget,
        })
    }
    fn tuple_variant<V: serde::de::Visitor<'de>>(
        self,
        n: usize,
        v: V,
    ) -> std::result::Result<V::Value, A::Error> {
        self.inner.tuple_variant(
            n,
            BudgetVisitor {
                inner: v,
                budget: self.budget,
            },
        )
    }
    fn struct_variant<V: serde::de::Visitor<'de>>(
        self,
        f: &'static [&'static str],
        v: V,
    ) -> std::result::Result<V::Value, A::Error> {
        self.inner.struct_variant(
            f,
            BudgetVisitor {
                inner: v,
                budget: self.budget,
            },
        )
    }
}

// Arena bytes have their own seed: decoded original UTF8 is interned directly,
// never copied into an intermediate String/Vec when that body already exists.
struct ArenaBytesSeed<'a>(&'a mut WitnessBudget);
impl<'de> serde::de::DeserializeSeed<'de> for ArenaBytesSeed<'_> {
    type Value = EvidenceBytes;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<EvidenceBytes, D::Error> {
        struct Leaf<'a>(&'a mut WitnessBudget);
        impl<'de> serde::de::Visitor<'de> for Leaf<'_> {
            type Value = EvidenceBytes;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("one exact Utf8/Hex arena leaf")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<EvidenceBytes, A::Error> {
                let tag = a
                    .next_key_seed(BudgetSeed {
                        inner: ValueSeed::<String>(std::marker::PhantomData),
                        budget: &mut *self.0,
                    })?
                    .ok_or_else(|| serde::de::Error::custom("empty arena leaf"))?;
                struct Body<'a> {
                    budget: &'a mut WitnessBudget,
                    hex: bool,
                }
                impl<'de> serde::de::Visitor<'de> for Body<'_> {
                    type Value = EvidenceBytes;
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str("original arena bytes")
                    }
                    fn visit_str<E: serde::de::Error>(
                        self,
                        v: &str,
                    ) -> std::result::Result<EvidenceBytes, E> {
                        if !self.hex {
                            return self.budget.intern(v.as_bytes()).map_err(E::custom);
                        }
                        if v.len() % 2 != 0
                            || !v
                                .bytes()
                                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
                        {
                            return Err(E::custom("arena lowercase hex differs"));
                        }
                        self.budget.reserve(v.len() / 2).map_err(E::custom)?;
                        let bytes = hex::decode(v).map_err(E::custom)?;
                        if std::str::from_utf8(&bytes).is_ok() {
                            return Err(E::custom("arena valid UTF8 requires Utf8"));
                        }
                        self.budget.intern(&bytes).map_err(E::custom)
                    }
                    fn visit_borrowed_str<E: serde::de::Error>(
                        self,
                        v: &'de str,
                    ) -> std::result::Result<EvidenceBytes, E> {
                        self.visit_str(v)
                    }
                }
                struct BodySeed<'a> {
                    budget: &'a mut WitnessBudget,
                    hex: bool,
                }
                impl<'de> serde::de::DeserializeSeed<'de> for BodySeed<'_> {
                    type Value = EvidenceBytes;
                    fn deserialize<D: serde::Deserializer<'de>>(
                        self,
                        d: D,
                    ) -> std::result::Result<EvidenceBytes, D::Error> {
                        d.deserialize_str(Body {
                            budget: self.budget,
                            hex: self.hex,
                        })
                    }
                }
                let hex = match tag.as_str() {
                    "Utf8" => false,
                    "Hex" => true,
                    _ => return Err(serde::de::Error::custom("unknown arena leaf")),
                };
                let value = a.next_value_seed(BodySeed {
                    budget: self.0,
                    hex,
                })?;
                if a.next_key::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("duplicate/extra arena leaf"));
                }
                Ok(value)
            }
        }
        d.deserialize_map(Leaf(self.0))
    }
}
struct ArenaEntriesSeed<'a>(&'a mut WitnessBudget);
impl<'de> serde::de::DeserializeSeed<'de> for ArenaEntriesSeed<'_> {
    type Value = Vec<EvidenceBytes>;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        struct Entries<'a>(&'a mut WitnessBudget);
        impl<'de> serde::de::Visitor<'de> for Entries<'_> {
            type Value = Vec<EvidenceBytes>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("strict sorted unique byte arena")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut result: Vec<EvidenceBytes> = Vec::new();
                loop {
                    self.0
                        .descriptor::<EvidenceBytes>(4)
                        .map_err(serde::de::Error::custom)?;
                    let Some(value) = a.next_element_seed(ArenaBytesSeed(self.0))? else {
                        break;
                    };
                    if result.last().is_some_and(|v| (&**v) >= (&*value)) {
                        return Err(serde::de::Error::custom(
                            "arena repeated/unordered original bytes",
                        ));
                    }
                    result.push(value);
                }
                Ok(result)
            }
        }
        d.deserialize_seq(Entries(self.0))
    }
}
struct ArenaPhysicalSeed<'a>(&'a mut WitnessBudget);
impl<'de> serde::de::DeserializeSeed<'de> for ArenaPhysicalSeed<'_> {
    type Value = ArenaPhysical;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        self.0
            .descriptor::<ArenaPhysical>(1)
            .map_err(serde::de::Error::custom)?;
        struct Root<'a>(&'a mut WitnessBudget);
        impl<'de> serde::de::Visitor<'de> for Root<'_> {
            type Value = ArenaPhysical;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("closed Physical arena v2")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let (
                    mut version,
                    mut material,
                    mut business_date,
                    mut cohort_identity,
                    mut revision,
                    mut reason,
                    mut model_binding,
                    mut full_archive_identity,
                    mut members,
                    mut tables,
                    mut arena,
                ) = (
                    None, None, None, None, None, None, None, None, None, None, None,
                );
                macro_rules! field {
                    ($target:ident,$ty:ty) => {{
                        if $target.is_some() {
                            return Err(serde::de::Error::custom(concat!(
                                "duplicate ",
                                stringify!($target)
                            )));
                        }
                        self.0
                            .descriptor::<$ty>(1)
                            .map_err(serde::de::Error::custom)?;
                        $target = Some(a.next_value_seed(BudgetSeed {
                            inner: ValueSeed::<$ty>(std::marker::PhantomData),
                            budget: self.0,
                        })?);
                    }};
                }
                while let Some(key) = a.next_key_seed(BudgetSeed {
                    inner: ValueSeed::<String>(std::marker::PhantomData),
                    budget: self.0,
                })? {
                    match key.as_str() {
                        "version" => field!(version, u8),
                        "material" => field!(material, String),
                        "business_date" => field!(business_date, NaiveDate),
                        "cohort_identity" => field!(cohort_identity, String),
                        "revision" => field!(revision, i64),
                        "reason" => field!(reason, String),
                        "model_binding" => field!(model_binding, ArenaModelBinding),
                        "full_archive_identity" => field!(full_archive_identity, String),
                        "members" => field!(members, Vec<Member>),
                        "tables" => field!(tables, Vec<ArenaTable>),
                        "arena" => {
                            if arena.is_some() {
                                return Err(serde::de::Error::custom("duplicate arena"));
                            }
                            arena = Some(a.next_value_seed(ArenaEntriesSeed(self.0))?);
                        }
                        _ => return Err(serde::de::Error::custom("unknown Physical arena field")),
                    }
                }
                macro_rules! required {
                    ($v:ident) => {
                        $v.ok_or_else(|| {
                            serde::de::Error::custom(concat!("missing ", stringify!($v)))
                        })?
                    };
                }
                Ok(ArenaPhysical {
                    version: required!(version),
                    material: required!(material),
                    business_date: required!(business_date),
                    cohort_identity: required!(cohort_identity),
                    revision: required!(revision),
                    reason: required!(reason),
                    model_binding: required!(model_binding),
                    full_archive_identity: required!(full_archive_identity),
                    members: required!(members),
                    tables: required!(tables),
                    arena: required!(arena),
                })
            }
        }
        d.deserialize_map(Root(self.0))
    }
}
fn resolve_arena(
    arena: &[EvidenceBytes],
    used: &mut [bool],
    index: usize,
) -> Result<EvidenceBytes> {
    let value = arena
        .get(index)
        .ok_or_else(|| mismatch("Physical arena reference out of range"))?;
    *used
        .get_mut(index)
        .ok_or_else(|| mismatch("Physical arena reference absent"))? = true;
    Ok(value.clone())
}
fn physical_from_arena(value: ArenaPhysical, budget: &mut WitnessBudget) -> Result<PhysicalSeal> {
    if value.version != 2
        || value.material != MATERIAL
        || value.reason != REASON
        || value.revision <= 0
        || value.members.is_empty()
        || value.members.len() > 3
        || value.members.iter().enumerate().any(|(i, v)| v.index != i)
        || value.tables.len() != TABLES.len()
    {
        return Err(mismatch("Physical arena strict shape differs"));
    }
    budget.descriptor::<bool>(value.arena.len())?;
    let mut used = vec![false; value.arena.len()];
    budget.descriptor::<Table>(value.tables.len())?;
    let mut tables = Vec::with_capacity(value.tables.len());
    for (table, name) in value.tables.into_iter().zip(TABLES) {
        if table.name != *name
            || table.rows.len() > MAX_ROWS
            || table.rows.iter().any(|r| r.len() != table.columns.len())
        {
            return Err(mismatch("Physical arena table membership/width differs"));
        }
        budget.descriptor::<Vec<Cell>>(table.rows.len())?;
        let mut rows = Vec::with_capacity(table.rows.len());
        for row in table.rows {
            budget.descriptor::<Cell>(row.len())?;
            let mut cells = Vec::with_capacity(row.len());
            for cell in row {
                cells.push(match cell {
                    ArenaCell::Null => Cell::Null,
                    ArenaCell::Integer(v) => Cell::Integer(v),
                    ArenaCell::BlobRef(i) => Cell::Blob(resolve_arena(&value.arena, &mut used, i)?),
                    ArenaCell::TextRef(i) => {
                        let bytes = resolve_arena(&value.arena, &mut used, i)?;
                        std::str::from_utf8(&bytes)
                            .map_err(|_| mismatch("Physical arena TextRef is not UTF8"))?;
                        Cell::Text(EvidenceText::Shared(bytes))
                    }
                });
            }
            rows.push(cells);
        }
        for adjacent in rows.windows(2) {
            if compare_rows(&adjacent[0], &adjacent[1], budget)? != std::cmp::Ordering::Less {
                return Err(mismatch("Physical arena original rows repeated/unordered"));
            }
        }
        tables.push(Table {
            name: table.name,
            columns: table.columns,
            rows,
        });
    }
    budget.descriptor::<model_bundle::SealModelArtifactBinding>(
        value.model_binding.artifacts.len(),
    )?;
    let mut artifacts = Vec::with_capacity(value.model_binding.artifacts.len());
    for item in value.model_binding.artifacts {
        artifacts.push(model_bundle::SealModelArtifactBinding {
            event_identity: item.event_identity,
            material: item.material,
            desired_bytes: resolve_arena(&value.arena, &mut used, item.desired_bytes)?,
            committed: item.committed,
        });
    }
    let model_binding = model_bundle::SealModelBinding {
        selection_bytes: resolve_arena(
            &value.arena,
            &mut used,
            value.model_binding.selection_bytes,
        )?,
        admission: value.model_binding.admission,
        artifacts,
    };
    if used.iter().any(|v| !v) {
        return Err(mismatch("Physical arena has unused original bytes"));
    }
    Ok(PhysicalSeal {
        storage_version: 2,
        version: 1,
        material: value.material,
        business_date: value.business_date,
        cohort_identity: value.cohort_identity,
        revision: value.revision,
        reason: value.reason,
        model_binding,
        full_archive_identity: value.full_archive_identity,
        members: value.members,
        tables,
    })
}

fn preimage(bytes: &[u8]) -> Vec<u8> {
    let mut v = MATERIAL.as_bytes().to_vec();
    v.push(0);
    v.extend_from_slice(bytes);
    v
}
// v2 compares the entire original domain preimage without a second owned body.
// The original v1 allocation/comparison path remains unchanged.
fn exact_preimage(bytes: &[u8], stored: &[u8]) -> bool {
    stored
        .strip_prefix(MATERIAL.as_bytes())
        .and_then(|tail| tail.strip_prefix(&[0]))
        == Some(bytes)
}
fn preimage_bounded(bytes: &[u8], budget: &mut WitnessBudget) -> Result<Vec<u8>> {
    let length = MATERIAL
        .len()
        .checked_add(1)
        .and_then(|v| v.checked_add(bytes.len()))
        .ok_or_else(|| mismatch("Physical preimage budget overflow"))?;
    budget.reserve(length)?;
    let mut result = Vec::with_capacity(length);
    result.extend_from_slice(MATERIAL.as_bytes());
    result.push(0);
    result.extend_from_slice(bytes);
    Ok(result)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPhysicalSeal {
    version: u8,
    material: String,
    codec: String,
    uncompressed_length: u64,
    uncompressed_sha256: String,
    #[serde(with = "byte_leaf")]
    payload: Vec<u8>,
}
/// Pinned profile: this codec must fail closed after an incompatible engine
/// upgrade. A future encoder requires an explicit new version/migration, never
/// automatic adoption of another compressed representation of the same bytes.
fn compress(bytes: &[u8], budget: &mut WitnessBudget) -> Result<Vec<u8>> {
    use std::io::Write;
    let output = BoundedEncoding {
        budget,
        bytes: Vec::new(),
    };
    let mut encoder = zstd::stream::write::Encoder::new(output, ZSTD_LEVEL).map_err(io_error)?;
    encoder.window_log(ZSTD_WINDOW_LOG).map_err(io_error)?;
    encoder.include_checksum(true).map_err(io_error)?;
    encoder.include_contentsize(true).map_err(io_error)?;
    encoder
        .set_pledged_src_size(Some(bytes.len() as u64))
        .map_err(io_error)?;
    encoder.write_all(bytes).map_err(io_error)?;
    Ok(encoder.finish().map_err(io_error)?.bytes)
}
fn encode_stored(value: &PhysicalSeal, budget: &mut WitnessBudget) -> Result<Vec<u8>> {
    let inner = if value.storage_version == 1 {
        encode_bounded(value, budget)?
    } else {
        encode_arena(value, budget)?
    };
    let payload = compress(&inner, budget)?;
    encode_bounded(
        &StoredPhysicalSeal {
            version: value.storage_version,
            material: MATERIAL.to_owned(),
            codec: if value.storage_version == 1 {
                CODEC
            } else {
                ARENA_CODEC
            }
            .to_owned(),
            uncompressed_length: inner.len() as u64,
            uncompressed_sha256: sha256_hex(&inner),
            payload,
        },
        budget,
    )
}
fn decode_with_budget(bytes: &[u8], budget: &mut WitnessBudget) -> Result<PhysicalSeal> {
    if bytes.len() > MAX_WITNESS {
        return Err(mismatch("Physical stored seal budget exceeded"));
    }
    // Canonical wrapper field order begins with this fixed version token.
    // Reject alternate spelling/order before any unbudgeted dispatch parser.
    let version = if bytes.starts_with(b"{\"version\":1,") {
        1
    } else if bytes.starts_with(b"{\"version\":2,") {
        2
    } else {
        return Err(mismatch(
            "Physical canonical wrapper version prefix differs",
        ));
    };
    let wrapper: StoredPhysicalSeal = if version == 2 {
        // Escaped JSON uses serde's geometric scratch Vec. Reserve its
        // conservative two-input-length bound before the parser can grow it.
        budget.reserve(
            bytes
                .len()
                .checked_mul(2)
                .ok_or_else(|| mismatch("Physical JSON scratch budget overflow"))?,
        )?;
        budget.descriptor::<StoredPhysicalSeal>(1)?;
        let mut d = serde_json::Deserializer::from_slice(bytes);
        let value = StoredPhysicalSeal::deserialize(BudgetDeserializer {
            inner: &mut d,
            budget,
        })?;
        d.end()?;
        value
    } else {
        serde_json::from_slice(bytes)?
    };
    if !matches!(
        (wrapper.version, wrapper.codec.as_str()),
        (1, CODEC) | (2, ARENA_CODEC)
    ) || wrapper.material != MATERIAL
        || wrapper.uncompressed_length == 0
        || wrapper.uncompressed_length > MAX_WITNESS as u64
    {
        return Err(mismatch("Physical stored codec/version/length differs"));
    }
    match_canonical(&wrapper, bytes, budget)?;
    let size = usize::try_from(wrapper.uncompressed_length).map_err(codec_error)?;
    if zstd::zstd_safe::find_frame_compressed_size(&wrapper.payload)
        .map_err(|_| mismatch("Physical compressed frame invalid"))?
        != wrapper.payload.len()
        || zstd::zstd_safe::get_frame_content_size(&wrapper.payload)
            .map_err(|_| mismatch("Physical compressed content size absent/invalid"))?
            != Some(wrapper.uncompressed_length)
    {
        return Err(mismatch(
            "Physical compressed frame has trailing/concatenated data or size mismatch",
        ));
    }
    // One shared history budget reserves the actual declared output before any
    // inflate. The actual stream is additionally bounded to declared+1 bytes.
    budget.reserve(size)?;
    use std::io::Read;
    let mut decoder = zstd::stream::read::Decoder::with_buffer(wrapper.payload.as_slice())
        .map_err(io_error)?
        .single_frame();
    decoder.window_log_max(ZSTD_WINDOW_LOG).map_err(io_error)?;
    let mut inner = Vec::with_capacity(size);
    let mut block = [0u8; 8192];
    let mut decoder = decoder.take(wrapper.uncompressed_length + 1);
    loop {
        let count = decoder.read(&mut block).map_err(io_error)?;
        if count == 0 {
            break;
        }
        if inner.len().checked_add(count).is_none_or(|v| v > size) {
            return Err(mismatch(
                "Physical compressed output exceeds declared length",
            ));
        }
        inner.extend_from_slice(&block[..count]);
    }
    if inner.len() != size || sha256_hex(&inner) != wrapper.uncompressed_sha256 {
        return Err(mismatch("Physical compressed output length/hash differs"));
    }
    let value: PhysicalSeal = if wrapper.version == 1 {
        serde_json::from_slice(&inner)?
    } else {
        // Reserve geometric UTF8-unescape scratch before parsing. Every owned
        // metadata allocation and unique arena body has a separate reservation.
        budget.reserve(
            inner
                .len()
                .checked_mul(2)
                .ok_or_else(|| mismatch("Physical arena scratch budget overflow"))?,
        )?;
        let mut d = serde_json::Deserializer::from_slice(&inner);
        let arena = serde::de::DeserializeSeed::deserialize(ArenaPhysicalSeed(budget), &mut d)?;
        d.end()?;
        match_canonical(&arena, &inner, budget)?;
        physical_from_arena(arena, budget)?
    };
    if value.version != 1
        || value.material != MATERIAL
        || value.reason != REASON
        || value.revision <= 0
        || value.members.is_empty()
        || value.members.len() > 3
        || value.members.iter().enumerate().any(|(i, v)| v.index != i)
        || value.tables.len() != TABLES.len()
        || value.tables.iter().any(|table| {
            table.rows.len() > MAX_ROWS
                || table
                    .rows
                    .iter()
                    .any(|row| row.len() != table.columns.len())
        })
    {
        return Err(mismatch("Physical strict seal codec differs"));
    }
    if wrapper.version == 1 {
        match_canonical(&value, &inner, budget)?;
    }
    if compress(&inner, budget)? != wrapper.payload {
        return Err(mismatch(
            "Physical compressed payload is not the fixed canonical profile",
        ));
    }
    Ok(value)
}
fn decode(bytes: &[u8]) -> Result<PhysicalSeal> {
    decode_with_budget(bytes, &mut WitnessBudget::maximum())
}
fn is_physical(bytes: &[u8]) -> Result<bool> {
    if bytes.len() > MAX_WITNESS {
        return Err(mismatch("Physical dispatch byte budget exceeded"));
    }
    // Prefix checks copy/parse no input and grant no evidence/capability.
    // Only canonical accepted codec prefixes reach their original strict reader.
    if bytes.starts_with(b"{\"version\":1,\"material\":\"g5b-physical-day-seal-v1\",")
        || bytes.starts_with(b"{\"version\":2,\"material\":\"g5b-physical-day-seal-v1\",")
    {
        Ok(true)
    } else if bytes.starts_with(b"{\"version\":1,\"material\":\"g5b-empty-day-seal-v1\",") {
        Ok(false)
    } else {
        Err(mismatch("Physical/Empty canonical dispatch prefix differs"))
    }
}
type SealRow = (String, String, String, i64, Vec<u8>, String, Vec<u8>);
fn bounded_seal_row(row: &rusqlite::Row<'_>, budget: &mut WitnessBudget) -> Result<SealRow> {
    budget.descriptor::<SealRow>(1)?;
    reserve_row(row, 7, budget)?;
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}
fn load_seal(
    connection: &Connection,
    identity: &str,
    budget: &mut WitnessBudget,
) -> Result<Option<SealRow>> {
    let mut query=connection.prepare("SELECT seal_identity,business_date,cohort_identity,revision,seal_canonical,seal_sha256,seal_preimage FROM g5b_day_seals WHERE seal_identity=?1")?;
    let mut cursor = query.query([identity])?;
    let Some(row) = cursor.next()? else {
        return Ok(None);
    };
    let value = bounded_seal_row(row, budget)?;
    if cursor.next()?.is_some() {
        return Err(mismatch("Physical seal identity is not unique"));
    }
    Ok(Some(value))
}
fn current_seal_row(
    connection: &Connection,
    date: NaiveDate,
    budget: &mut WitnessBudget,
) -> Result<Option<SealRow>> {
    let mut query=connection.prepare("SELECT s.seal_identity,s.business_date,s.cohort_identity,s.revision,s.seal_canonical,s.seal_sha256,s.seal_preimage FROM g5b_day_heads h JOIN g5b_day_seals s ON s.seal_identity=h.current_seal_identity WHERE h.business_date=?1")?;
    let mut cursor = query.query([date.to_string()])?;
    let Some(row) = cursor.next()? else {
        return Ok(None);
    };
    Ok(Some(bounded_seal_row(row, budget)?))
}
fn seal_at_revision(
    connection: &Connection,
    date: NaiveDate,
    cohort: &str,
    revision: i64,
    budget: &mut WitnessBudget,
) -> Result<Option<SealRow>> {
    let mut query=connection.prepare("SELECT seal_identity,business_date,cohort_identity,revision,seal_canonical,seal_sha256,seal_preimage FROM g5b_day_seals WHERE business_date=?1 AND cohort_identity=?2 AND revision=?3")?;
    let mut cursor = query.query(params![date.to_string(), cohort, revision])?;
    let Some(row) = cursor.next()? else {
        return Ok(None);
    };
    Ok(Some(bounded_seal_row(row, budget)?))
}

fn seal_rows(connection: &Connection, date: Option<NaiveDate>) -> Result<Vec<SealRow>> {
    seal_rows_with_budget(
        connection,
        date,
        &mut WitnessBudget::for_connection(connection),
    )
}
fn seal_rows_with_budget(
    connection: &Connection,
    date: Option<NaiveDate>,
    budget: &mut WitnessBudget,
) -> Result<Vec<SealRow>> {
    let mut query=connection.prepare("SELECT seal_identity,business_date,cohort_identity,revision,seal_canonical,seal_sha256,seal_preimage FROM g5b_day_seals WHERE (?1 IS NULL OR business_date=?1) ORDER BY revision,seal_identity LIMIT 4097")?;
    let mut cursor = query.query([date.map(|v| v.to_string())])?;
    let mut values = Vec::new();
    while let Some(row) = cursor.next()? {
        if values.len() == MAX_ROWS {
            return Err(mismatch("Physical historical seal row budget exceeded"));
        }
        budget.descriptor::<SealRow>(4)?;
        values.push(bounded_seal_row(row, budget)?);
    }
    Ok(values)
}

fn validate_historical(connection: &Connection, row: &SealRow) -> Result<PhysicalSeal> {
    validate_historical_with_budget(
        connection,
        row,
        &mut WitnessBudget::for_connection(connection),
    )
}
fn validate_historical_with_budget(
    connection: &Connection,
    row: &SealRow,
    budget: &mut WitnessBudget,
) -> Result<PhysicalSeal> {
    let (id, date, cohort, rev, bytes, sha, pre) = row;
    let value = decode_with_budget(bytes, budget)?;
    let bundle = model_bundle::load_bundle(connection, value.business_date)?
        .ok_or_else(|| mismatch("Physical stored cohort absent"))?;
    if value.business_date.to_string() != *date
        || value.cohort_identity != *cohort
        || bundle.cohort().identity() != *cohort
        || value.revision != *rev
        || *rev > bundle.revision()
        || sha256_hex(bytes) != *sha
        || domain_sha256_hex(MATERIAL, bytes) != *id
        || (if value.storage_version == 1 {
            preimage(bytes) != *pre
        } else {
            !exact_preimage(bytes, pre)
        })
        || (if value.storage_version == 1 {
            bundle.seal_model_binding(budget)?
        } else {
            bundle.seal_model_binding_shared(budget)?
        }) != value.model_binding
        || crate::monitor::g5b_analysis_v2::full_model_archive_identity_v2(&bundle)
            .map_err(codec_error)?
            .as_deref()
            != Some(value.full_archive_identity.as_str())
    {
        return Err(mismatch(
            "Physical historical seal original binding differs",
        ));
    }
    let current = if value.storage_version == 1 {
        rows_legacy_with_budget(connection, value.business_date, budget)?
    } else {
        rows_with_budget(connection, value.business_date, budget)?
    };
    if value.storage_version == 1 {
        encode_bounded(&current, budget)?;
        subset_legacy(&value.tables, &current)?;
    } else {
        encode_table_arena(&current, budget)?;
        subset(&value.tables, &current, budget)?;
    }
    validate_late_extension(connection, &value.tables, &current, &value.members)?;
    if !drained(&value.tables)? {
        return Err(mismatch(
            "Physical historical seal contains pending evidence",
        ));
    }
    let actual = members(connection, &bundle, &value.tables)?
        .ok_or_else(|| mismatch("Physical historical original Accepted members absent"))?;
    if actual != value.members {
        return Err(mismatch(
            "Physical historical Accepted member binding differs",
        ));
    }
    validate_raw(&value.tables)?;
    validate_payloads(&value.tables)?;
    validate_audits(connection, &value.tables, &value.members)?;
    validate_attempts(&value.tables, &value.members)?;
    validate_reservations(&value.tables, &value.members)?;
    Ok(value)
}
pub(super) fn validate_seals(connection: &Connection) -> Result<()> {
    let mut budget = WitnessBudget::for_connection(connection);
    validate_seals_with_budget(connection, &mut budget)
}
fn validate_seals_with_budget(connection: &Connection, budget: &mut WitnessBudget) -> Result<()> {
    for row in seal_rows_with_budget(connection, None, budget)? {
        if is_physical(&row.4)? {
            validate_historical_with_budget(connection, &row, budget)?;
        } else {
            empty::validate_seal_row(
                connection, &row.0, &row.1, &row.2, row.3, &row.4, &row.5, &row.6,
            )?;
        }
    }
    // The immutable earliest seal closes all new business/artifact production.
    let late_artifact:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM g5b_artifact_events e WHERE COALESCE(e.commit_revision,e.prepared_revision) > (SELECT MIN(s.revision) FROM g5b_day_seals s WHERE s.business_date=e.business_date))",[],|r|r.get(0))?;
    if late_artifact {
        return Err(mismatch("artifact was opened after historical completion"));
    }
    Ok(())
}
pub(super) fn validate_current_pointer(
    connection: &Connection,
    date: &str,
    revision: i64,
    state: &str,
    cohort: Option<&str>,
    pointer: &str,
) -> Result<()> {
    let row = load_seal(
        connection,
        pointer,
        &mut WitnessBudget::for_connection(connection),
    )?
    .ok_or_else(|| mismatch("Physical current seal row missing"))?;
    if !is_physical(&row.4)? {
        return empty::validate_current_pointer(connection, date, revision, state, cohort, pointer);
    }
    if row.1 != date || row.3 != revision || Some(row.2.as_str()) != cohort || state != "Clean" {
        return Err(mismatch(
            "Physical pointer is not current clean exact revision",
        ));
    }
    let saved = validate_historical(connection, &row)?;
    let bundle = model_bundle::load_bundle(connection, saved.business_date)?
        .ok_or_else(|| mismatch("Physical current bundle absent"))?;
    let current = candidate_version(
        connection,
        saved.business_date,
        &bundle,
        saved.storage_version,
    )?
    .ok_or_else(|| mismatch("Physical current obligations incomplete"))?;
    if current != saved {
        return Err(mismatch("Physical current pointer witness set is stale"));
    }
    Ok(())
}

type HeadRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<Vec<u8>>,
    Option<String>,
);
fn head_with_budget(
    connection: &Connection,
    date: NaiveDate,
    budget: &mut WitnessBudget,
) -> Result<HeadRow> {
    let mut query=connection.prepare("SELECT revision,artifact_state,cohort_identity,current_seal_identity,prospective_canonical,prospective_sha256 FROM g5b_day_heads WHERE business_date=?1")?;
    let mut cursor = query.query([date.to_string()])?;
    let row = cursor
        .next()?
        .ok_or_else(|| mismatch("Physical SQL snapshot day head missing"))?;
    budget.descriptor::<HeadRow>(1)?;
    reserve_row(row, 6, budget)?;
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}
fn sql_snapshot(connection: &Connection, date: NaiveDate) -> Result<Vec<u8>> {
    let mut budget = WitnessBudget::for_connection(connection);
    let head = head_with_budget(connection, date, &mut budget)?;
    let values = rows_with_budget(connection, date, &mut budget)?;
    let seals = seal_rows_with_budget(connection, Some(date), &mut budget)?;
    encode_snapshot(&head, &values, &seals, &mut budget)
}

fn encode_snapshot(
    head: &HeadRow,
    values: &[Table],
    seals: &[SealRow],
    budget: &mut WitnessBudget,
) -> Result<Vec<u8>> {
    let mut arena = Vec::new();
    collect_table_bytes(&mut arena, values, budget)?;
    if let Some(bytes) = &head.4 {
        collect_arena_bytes(&mut arena, bytes, budget)?;
    }
    for row in seals {
        collect_arena_bytes(&mut arena, &row.4, budget)?;
        collect_arena_bytes(&mut arena, &row.6, budget)?;
    }
    let tables = arena_tables(values, &arena, budget)?;
    let prospective = head
        .4
        .as_ref()
        .map(|v| arena_index(&arena, v))
        .transpose()?;
    type SealRef<'a> = (&'a str, &'a str, &'a str, i64, usize, &'a str, usize);
    budget.descriptor::<SealRef<'_>>(seals.len())?;
    let mut seal_refs = Vec::with_capacity(seals.len());
    for row in seals {
        seal_refs.push((
            row.0.as_str(),
            row.1.as_str(),
            row.2.as_str(),
            row.3,
            arena_index(&arena, &row.4)?,
            row.5.as_str(),
            arena_index(&arena, &row.6)?,
        ));
    }
    encode_bounded(
        &(
            2u8,
            &arena,
            (head.0, &head.1, &head.2, &head.3, prospective, &head.5),
            tables,
            seal_refs,
        ),
        budget,
    )
}

fn expect_sql(connection: &Connection, date: NaiveDate, expected: &[u8]) -> Result<()> {
    if sql_snapshot(connection, date)? != expected {
        return Err(mismatch(
            "Physical exact SQL snapshot changed at final transaction boundary",
        ));
    }
    Ok(())
}

fn validate_known_sql(connection: &Connection, known: &VerifiedG5bPhysicalSeal) -> Result<()> {
    let row = load_seal(
        connection,
        &known.identity,
        &mut WitnessBudget::for_connection(connection),
    )?
    .ok_or_else(|| mismatch("Physical known historical seal disappeared"))?;
    if row.1 != known.date.to_string()
        || row.2 != known.cohort
        || row.3 != known.revision
        || row.4 != known.seal_canonical
        || row.5 != known.sha256
    {
        return Err(mismatch(
            "Physical known historical seal bytes/identity changed",
        ));
    }
    let saved = validate_historical(connection, &row)?;
    if saved.members.len() != known.count {
        return Err(mismatch("Physical known member count changed"));
    }
    Ok(())
}

impl G5bDaySession<'_> {
    fn physical_input(
        &self,
        bundle: &VerifiedG5bModelBundle,
        known: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        self.validate()?;
        let prefix = self
            .coordinator
            .g5b_input_log
            .inspect_date_input_prefix_locked_bounded(self.date, &self.fence, MAX_ARTIFACT_BYTES)
            .map_err(codec_error)?;
        bundle
            .cohort()
            .evidence
            .verify_locked_prefix(&prefix)
            .map_err(codec_error)?;
        if let Some(known) = known {
            prefix
                .cutoff_for_captured_head(known)
                .map_err(codec_error)?;
        }
        let current = prefix.current_cutoff().map_err(codec_error)?;
        self.validate()?;
        Ok(current.head_canonical().to_vec())
    }
    fn physical_files(
        &self,
        bundle: &VerifiedG5bModelBundle,
        known: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        let before = self.physical_input(bundle, known)?;
        self.reject_legacy_artifacts()?;
        let namespace = self.fence.namespace_path().map_err(io_error)?;
        let mut allowed = bundle.artifact_filenames();
        for suffix in ["jsonl", "input-head.v1.json", "g5b-day.lock"] {
            allowed.push(format!("{}.{}", self.date.format("%Y%m%d"), suffix));
        }
        for leaf in empty::empty_directory_leaves(namespace)? {
            let leaf = leaf
                .to_str()
                .ok_or_else(|| mismatch("Physical nonUTF8 namespace leaf"))?;
            if leaf.starts_with(".g5b-v2-")
                || (leaf.starts_with(&format!("{}.", self.date.format("%Y%m%d")))
                    && !allowed.iter().any(|v| v == leaf))
            {
                return Err(mismatch("Physical unexplained date artifact"));
            }
        }
        let legacy = match self.coordinator.config.environment {
            crate::durable_delivery::StoreEnvironment::Production => {
                crate::production_root::production_root().join("data/g5b/attempts")
            }
            crate::durable_delivery::StoreEnvironment::Test { .. } => namespace.join("attempts"),
        };
        for leaf in empty::empty_directory_leaves(&legacy)? {
            if leaf
                .to_str()
                .is_none_or(|v| v.starts_with(&format!("{}.", self.date)))
            {
                return Err(mismatch("Physical legacy date residue"));
            }
        }
        bundle.verify_files(self)?;
        #[cfg(test)]
        run_file_fault(self)?;
        // Model/archive files first, final bounded input observation last.
        let after = self.physical_input(bundle, Some(&before))?;
        if after != before {
            return Err(mismatch(
                "Physical input changed during unified observation",
            ));
        }
        self.validate()?;
        Ok(after)
    }
    fn validate_known_session(&self, known: &VerifiedG5bPhysicalSeal) -> Result<()> {
        self.validate()?;
        if known.date != self.date
            || known.namespace_identity != self.namespace_identity
            || known.database_identity != self.coordinator.database_binding()?.objects[0].identity
            || known.lock_identity != self.fence.lock_identity().map_err(io_error)?
        {
            return Err(mismatch(
                "Physical known capability session identity differs",
            ));
        }
        Ok(())
    }
    pub(crate) fn try_seal_physical_cohort(&self) -> Result<G5bPhysicalSealAttempt> {
        self.try_seal_physical_inner(None, 2)
    }
    /// Retain a previously observed incarnation/generation across legitimate
    /// late receipts. Incomplete grants no replacement for the caller's anchor.
    pub(crate) fn try_seal_physical_cohort_known(
        &self,
        known: &VerifiedG5bPhysicalSeal,
    ) -> Result<G5bPhysicalSealAttempt> {
        self.validate_known_session(known)?;
        self.try_seal_physical_inner(Some(known), 2)
    }
    #[cfg(test)]
    pub(crate) fn try_seal_physical_legacy_for_test(&self) -> Result<G5bPhysicalSealAttempt> {
        if !matches!(
            self.coordinator.config.environment,
            crate::durable_delivery::StoreEnvironment::Test { .. }
        ) {
            return Err(mismatch(
                "Legacy Physical fixture writer requires actual Test",
            ));
        }
        self.try_seal_physical_inner(None, 1)
    }
    fn try_seal_physical_inner(
        &self,
        known: Option<&VerifiedG5bPhysicalSeal>,
        storage_version: u8,
    ) -> Result<G5bPhysicalSealAttempt> {
        if let Some(known) = known {
            self.validate_known_session(known)?;
        }
        if let Some(value) = self.read_physical_inner(known)? {
            return Ok(G5bPhysicalSealAttempt::Sealed(value));
        }
        let Some((bundle, value, bytes, seal_preimage, binding)) = self.transaction(|tx| {
            if let Some(known) = known {
                validate_known_sql(tx, known)?;
            }
            let Some(bundle) = model_bundle::load_bundle(tx, self.date)? else {
                return Ok(None);
            };
            if known.is_some_and(|v| v.cohort != bundle.cohort().identity()) {
                return Err(mismatch(
                    "Physical known cohort cannot change during reclose",
                ));
            }
            let Some(value) = (if storage_version == 1 {
                candidate_version(tx, self.date, &bundle, 1)?
            } else {
                candidate(tx, self.date, &bundle)?
            }) else {
                return Ok(None);
            };
            let mut encoding_budget = WitnessBudget::for_connection(tx);
            let bytes = encode_stored(&value, &mut encoding_budget)?;
            let seal_preimage = if value.storage_version == 1 {
                preimage(&bytes)
            } else {
                preimage_bounded(&bytes, &mut encoding_budget)?
            };
            Ok(Some((
                bundle,
                value,
                bytes,
                seal_preimage,
                sql_snapshot(tx, self.date)?,
            )))
        })?
        else {
            return Ok(G5bPhysicalSealAttempt::Incomplete);
        };
        let known_head = known.map(|v| v.current_head_canonical.as_slice());
        self.physical_files(&bundle, known_head)?;
        let id = domain_sha256_hex(MATERIAL, &bytes);
        let expected = std::cell::RefCell::new(None::<Vec<u8>>);
        let validate = || {
            if let Some(known) = known {
                self.validate_known_session(known)?;
            }
            self.physical_files(&bundle, known_head).map(|_| ())
        };
        let validate_sql = |tx: &Transaction<'_>| {
            if let Some(known) = known {
                validate_known_sql(tx, known)?;
            }
            let captured = expected.borrow();
            expect_sql(
                tx,
                self.date,
                captured
                    .as_deref()
                    .ok_or_else(|| mismatch("Physical expected posteffect SQL missing"))?,
            )
        };
        self.coordinator.with_immediate_transaction_validated_sql(SchemaVersionPolicy::Runtime,Some(&validate),Some(&validate_sql),|tx| {
            expect_sql(tx,self.date,&binding)?;
            if let Some(known) = known { validate_known_sql(tx,known)?; }
            let existing=seal_at_revision(tx,self.date,&value.cohort_identity,value.revision,&mut WitnessBudget::for_connection(tx))?;
            if let Some(saved)=existing {
                if saved.0!=id || saved.4!=bytes {return Err(mismatch("Physical exact seal recovery preimage differs"));}
            }
            else {tx.execute("INSERT INTO g5b_day_seals(seal_identity,business_date,cohort_identity,revision,seal_canonical,seal_sha256,seal_preimage) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,self.date.to_string(),value.cohort_identity,value.revision,bytes,sha256_hex(&bytes),seal_preimage])?;}
            let changed=tx.execute("UPDATE g5b_day_heads SET current_seal_identity=?1 WHERE business_date=?2 AND revision=?3 AND cohort_identity=?4 AND artifact_state='Clean' AND current_seal_identity IS NULL",params![id,self.date.to_string(),value.revision,value.cohort_identity])?;
            require_single_cas_update(changed,"Physical seal pointer CAS")?;
            *expected.borrow_mut()=Some(sql_snapshot(tx,self.date)?);Ok(())
        })?;
        // A successful INSERT/COMMIT never returns a capability by itself.
        self.read_physical_inner(known)?
            .map(G5bPhysicalSealAttempt::Sealed)
            .ok_or_else(|| mismatch("Physical committed seal lacks fresh current reader"))
    }
    pub(crate) fn read_physical_seal(&self) -> Result<Option<VerifiedG5bPhysicalSeal>> {
        self.read_physical_inner(None)
    }
    pub(crate) fn refresh_physical_seal(
        &self,
        known: &VerifiedG5bPhysicalSeal,
    ) -> Result<VerifiedG5bPhysicalSeal> {
        self.validate_known_session(known)?;
        let value = self
            .read_physical_inner(Some(known))?
            .ok_or_else(|| mismatch("Physical current seal invalidated"))?;
        if value.identity != known.identity
            || value.sha256 != known.sha256
            || value.revision != known.revision
            || value.cohort != known.cohort
        {
            return Err(mismatch("Physical capability identity changed"));
        }
        Ok(value)
    }
    fn read_physical_inner(
        &self,
        known: Option<&VerifiedG5bPhysicalSeal>,
    ) -> Result<Option<VerifiedG5bPhysicalSeal>> {
        if let Some(known) = known {
            self.validate_known_session(known)?;
        }
        let Some((bundle, current, binding)) = self.transaction(|tx| {
            if let Some(known) = known {
                validate_known_sql(tx, known)?;
            }
            let row = current_seal_row(tx, self.date, &mut WitnessBudget::for_connection(tx))?;
            let current = if let Some((id, _, _, _, bytes, sha, _)) = row {
                if !is_physical(&bytes)? {
                    return Ok(None);
                }
                let value = decode(&bytes)?;
                validate_current_pointer(
                    tx,
                    &self.date.to_string(),
                    value.revision,
                    "Clean",
                    Some(&value.cohort_identity),
                    &id,
                )?;
                Some((value, id, sha, bytes))
            } else {
                if known.is_none() {
                    return Ok(None);
                }
                None
            };
            let bundle = model_bundle::load_bundle(tx, self.date)?
                .ok_or_else(|| mismatch("Physical current actual bundle absent"))?;
            if known.is_some_and(|v| v.cohort != bundle.cohort().identity()) {
                return Err(mismatch("Physical known cohort differs from current model"));
            }
            Ok(Some((bundle, current, sql_snapshot(tx, self.date)?)))
        })?
        else {
            return Ok(None);
        };
        let known_head = known.map(|v| v.current_head_canonical.as_slice());
        let validate = || {
            if let Some(known) = known {
                self.validate_known_session(known)?;
            }
            self.physical_files(&bundle, known_head).map(|_| ())
        };
        let validate_sql = |tx: &Transaction<'_>| {
            if let Some(known) = known {
                validate_known_sql(tx, known)?;
            }
            expect_sql(tx, self.date, &binding)
        };
        validate()?;
        self.coordinator.with_immediate_transaction_validated_sql(
            SchemaVersionPolicy::Runtime,
            Some(&validate),
            Some(&validate_sql),
            |tx| validate_sql(tx),
        )?;
        let current_head_canonical = self.physical_files(&bundle, known_head)?;
        let Some((value, id, sha, seal_canonical)) = current else {
            return Ok(None);
        };
        self.validate()?;
        Ok(Some(VerifiedG5bPhysicalSeal {
            date: self.date,
            cohort: value.cohort_identity,
            revision: value.revision,
            identity: id,
            sha256: sha,
            count: value.members.len(),
            current_head_canonical,
            seal_canonical,
            namespace_identity: self.namespace_identity,
            database_identity: self.coordinator.database_binding()?.objects[0].identity,
            lock_identity: self.fence.lock_identity().map_err(io_error)?,
        }))
    }
}

#[cfg(test)]
#[derive(Clone)]
struct BudgetTestScope {
    path: String,
    limit: usize,
}
#[cfg(test)]
thread_local! {static BUDGET_TEST:std::cell::RefCell<Option<BudgetTestScope>>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
pub(crate) struct PhysicalWitnessBudgetTestGuard {
    previous: Option<BudgetTestScope>,
}
#[cfg(test)]
impl Drop for PhysicalWitnessBudgetTestGuard {
    fn drop(&mut self) {
        BUDGET_TEST.with(|slot| *slot.borrow_mut() = self.previous.take());
    }
}
#[cfg(test)]
impl DurableDeliveryCoordinator {
    pub(crate) fn install_physical_witness_budget_for_test(
        &self,
        limit: usize,
    ) -> Result<PhysicalWitnessBudgetTestGuard> {
        if !matches!(
            self.config.environment,
            crate::durable_delivery::StoreEnvironment::Test { .. }
        ) || limit == 0
            || limit > MAX_WITNESS
        {
            return Err(mismatch(
                "Physical budget hook requires attested Test and a smaller positive limit",
            ));
        }
        let path = self.with_connection(|connection| {
            connection
                .path()
                .map(str::to_owned)
                .ok_or_else(|| mismatch("Physical budget hook requires actual named database"))
        })?;
        let previous = BUDGET_TEST.with(|slot| slot.replace(Some(BudgetTestScope { path, limit })));
        Ok(PhysicalWitnessBudgetTestGuard { previous })
    }
    // These closed Test-only read probes retain database attestation. Bootstrap
    // policy avoids the unrelated global reader consuming the test budget before
    // the exact copied-history/snapshot boundary under test. They return scalar
    // sizes only, never a capability, JSON factory or writable connection.
    fn require_physical_budget_test(&self) -> Result<()> {
        if !matches!(
            self.config.environment,
            crate::durable_delivery::StoreEnvironment::Test { .. }
        ) {
            return Err(mismatch("Physical budget probe requires attested Test"));
        }
        Ok(())
    }
    // Only the scalar probe's schema preflight uses the original global limit.
    // Taking the private slot avoids installing a nested connection hook. The
    // lexical guard restores the caller's exact limit on success, error or panic,
    // before the requested probe constructs its own budget.
    fn require_physical_probe_schema(&self, connection: &Connection) -> Result<()> {
        self.require_physical_budget_test()?;
        let previous = BUDGET_TEST.with(|slot| slot.take());
        let _restore = PhysicalWitnessBudgetTestGuard { previous };
        require_current_schema_version(connection)
    }
    pub(crate) fn physical_history_copy_count_for_test(&self) -> Result<usize> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            Ok(seal_rows(connection, None)?.len())
        })
    }
    // Actual copied rows, including the exact list-growth and tuple charges.
    // Each row has its own budget only for this scalar probe; the original list
    // reader continues to share one budget across every historical row.
    pub(crate) fn physical_history_copy_parts_for_test(&self) -> Result<[usize; 2]> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            let mut query = connection.prepare("SELECT seal_identity,business_date,cohort_identity,revision,seal_canonical,seal_sha256,seal_preimage FROM g5b_day_seals ORDER BY revision,seal_identity LIMIT 4097")?;
            let mut cursor = query.query([])?;
            let (mut count, mut largest, mut total) = (0usize, 0usize, 0usize);
            while let Some(row) = cursor.next()? {
                if count == MAX_ROWS {
                    return Err(mismatch("Physical historical seal row budget exceeded"));
                }
                let mut single = WitnessBudget::for_connection(connection);
                single.descriptor::<SealRow>(4)?;
                let _actual = bounded_seal_row(row, &mut single)?;
                largest = largest.max(single.used);
                total = total.checked_add(single.used)
                    .ok_or_else(|| mismatch("Physical history copy sum overflow"))?;
                count += 1;
            }
            Ok([largest, total])
        })
    }
    pub(crate) fn physical_snapshot_len_for_test(&self, date: NaiveDate) -> Result<usize> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            Ok(sql_snapshot(connection, date)?.len())
        })
    }
    pub(crate) fn physical_snapshot_parts_for_test(&self, date: NaiveDate) -> Result<[usize; 4]> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            let mut h = WitnessBudget::for_connection(connection);
            let head = head_with_budget(connection, date, &mut h)?;
            let mut r = WitnessBudget::for_connection(connection);
            let values = rows_with_budget(connection, date, &mut r)?;
            let mut s = WitnessBudget::for_connection(connection);
            let seals = seal_rows_with_budget(connection, Some(date), &mut s)?;
            let mut e = WitnessBudget::for_connection(connection);
            encode_snapshot(&head, &values, &seals, &mut e)?;
            Ok([h.used, r.used, s.used, e.used])
        })
    }
    /// Scalar probes use real attested Test storage and grant no capability.
    pub(crate) fn physical_codec_decode_for_test(&self, bytes: &[u8]) -> Result<usize> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            Ok(
                decode_with_budget(bytes, &mut WitnessBudget::for_connection(connection))?
                    .members
                    .len(),
            )
        })
    }
    pub(crate) fn physical_codec_sizes_for_test(&self, date: NaiveDate) -> Result<[usize; 3]> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            let row = current_seal_row(
                connection,
                date,
                &mut WitnessBudget::for_connection(connection),
            )?
            .ok_or_else(|| mismatch("actual Test Physical seal absent"))?;
            let value: StoredPhysicalSeal = serde_json::from_slice(&row.4)?;
            Ok([
                usize::try_from(value.uncompressed_length).map_err(codec_error)?,
                value.payload.len(),
                row.4.len(),
            ])
        })
    }
    pub(crate) fn physical_arena_stats_for_test(&self, date: NaiveDate) -> Result<[usize; 6]> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            let mut budget = WitnessBudget::for_connection(connection);
            let row = current_seal_row(connection, date, &mut budget)?
                .ok_or_else(|| mismatch("actual Test Physical seal absent"))?;
            let value = decode_with_budget(&row.4, &mut budget)?;
            let wire = arena_physical(&value, &mut budget)?;
            let unique = wire.arena.iter().try_fold(0usize, |n, v| {
                n.checked_add(v.len())
                    .ok_or_else(|| mismatch("Physical unique byte sum overflow"))
            })?;
            let mut actual_shared_sql_refs = 0usize;
            if value.storage_version == 2 {
                let artifacts = table(&value.tables, "g5b_artifact_events")?;
                let at = artifacts
                    .columns
                    .iter()
                    .position(|v| v == "desired_bytes")
                    .ok_or_else(|| mismatch("actual artifact byte column absent"))?;
                for item in &value.model_binding.artifacts {
                    let EvidenceBytes::Shared(model_body) = &item.desired_bytes else {
                        return Err(mismatch("arena model body was expanded"));
                    };
                    let mut original_rows = 0;
                    for row in &artifacts.rows {
                        let Some(Cell::Blob(sql_body)) = row.get(at) else {
                            return Err(mismatch("actual artifact desired type differs"));
                        };
                        if sql_body == &item.desired_bytes {
                            let EvidenceBytes::Shared(sql_body) = sql_body else {
                                return Err(mismatch("arena SQL body was expanded"));
                            };
                            if !std::sync::Arc::ptr_eq(sql_body, model_body) {
                                return Err(mismatch(
                                    "equal complete artifact bodies do not share",
                                ));
                            }
                            original_rows += 1;
                            actual_shared_sql_refs += 1;
                        }
                    }
                    if original_rows < 2 {
                        return Err(mismatch("actual Prepared/Committed originals absent"));
                    }
                }
            }
            Ok([
                value.storage_version as usize,
                wire.arena.len(),
                unique,
                budget.used,
                value.model_binding.artifacts.len(),
                actual_shared_sql_refs,
            ])
        })
    }
    /// Closed negative probes qualify against the real original SQL snapshot,
    /// returning only a scalar/error, never a physical/live capability.
    pub(crate) fn physical_arena_invalid_sql_for_test(
        &self,
        date: NaiveDate,
        variant: &str,
    ) -> Result<usize> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            let mut budget = WitnessBudget::for_connection(connection);
            let mut row = current_seal_row(connection, date, &mut budget)?
                .ok_or_else(|| mismatch("actual Test Physical seal absent"))?;
            let mut value = decode_with_budget(&row.4, &mut budget)?;
            value.storage_version = 2;
            let table = value
                .tables
                .iter_mut()
                .find(|t| t.name == "g5b_artifact_events")
                .ok_or_else(|| mismatch("actual artifact table absent"))?;
            match variant {
                "text-blob-alias" => {
                    let at = table
                        .columns
                        .iter()
                        .position(|v| v == "desired_bytes")
                        .ok_or_else(|| mismatch("actual desired bytes column absent"))?;
                    let cell = table
                        .rows
                        .first_mut()
                        .and_then(|r| r.get_mut(at))
                        .ok_or_else(|| mismatch("actual artifact row absent"))?;
                    let Cell::Blob(bytes) = cell else {
                        return Err(mismatch("actual desired SQL type differs"));
                    };
                    std::str::from_utf8(bytes).map_err(codec_error)?;
                    *cell = Cell::Text(EvidenceText::Shared(budget.intern(bytes)?));
                }
                "missing-original-row" => {
                    table
                        .rows
                        .pop()
                        .ok_or_else(|| mismatch("actual artifact row absent"))?;
                }
                _ => return Err(mismatch("unknown closed arena SQL fault")),
            }
            row.4 = encode_stored(&value, &mut budget)?;
            row.0 = domain_sha256_hex(MATERIAL, &row.4);
            row.5 = sha256_hex(&row.4);
            row.6 = preimage_bounded(&row.4, &mut budget)?;
            validate_historical_with_budget(connection, &row, &mut budget)
                .map(|value| value.members.len())
        })
    }
    pub(crate) fn physical_history_validation_usage_for_test(&self) -> Result<usize> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            let mut budget = WitnessBudget::for_connection(connection);
            validate_seals_with_budget(connection, &mut budget)?;
            Ok(budget.used)
        })
    }
    pub(crate) fn physical_single_history_validation_usage_for_test(
        &self,
        date: NaiveDate,
    ) -> Result<usize> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            let mut budget = WitnessBudget::for_connection(connection);
            let row = current_seal_row(connection, date, &mut budget)?
                .ok_or_else(|| mismatch("actual Test Physical seal absent"))?;
            validate_historical_with_budget(connection, &row, &mut budget)?;
            Ok(budget.used)
        })
    }
    pub(crate) fn physical_codec_variant_for_test(
        &self,
        date: NaiveDate,
        variant: &str,
    ) -> Result<Vec<u8>> {
        self.require_physical_budget_test()?;
        self.with_connection_core(SchemaVersionPolicy::Bootstrap, false, false, |connection| {
            self.require_physical_probe_schema(connection)?;
            let row = current_seal_row(
                connection,
                date,
                &mut WitnessBudget::for_connection(connection),
            )?
            .ok_or_else(|| mismatch("actual Test Physical seal absent"))?;
            decode_with_budget(&row.4, &mut WitnessBudget::for_connection(connection))?;
            let mut value: StoredPhysicalSeal = serde_json::from_slice(&row.4)?;
            match variant {
                "trailing" => value.payload.push(0),
                "concatenated" => {
                    let second = value.payload.clone();
                    value.payload.extend_from_slice(&second);
                }
                "declared-small" => value.uncompressed_length -= 1,
                "declared-bomb" => value.uncompressed_length = MAX_WITNESS as u64 + 1,
                "frame-bomb" => {
                    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), ZSTD_LEVEL)
                        .map_err(io_error)?;
                    encoder.window_log(ZSTD_WINDOW_LOG).map_err(io_error)?;
                    encoder.include_checksum(true).map_err(io_error)?;
                    encoder
                        .set_pledged_src_size(Some(MAX_WITNESS as u64 + 1))
                        .map_err(io_error)?;
                    let chunk = [b'x'; 8192];
                    for _ in 0..MAX_WITNESS / chunk.len() {
                        std::io::Write::write_all(&mut encoder, &chunk).map_err(io_error)?;
                    }
                    std::io::Write::write_all(&mut encoder, b"x").map_err(io_error)?;
                    value.payload = encoder.finish().map_err(io_error)?;
                }
                "large-window" => {
                    let inner =
                        zstd::stream::decode_all(value.payload.as_slice()).map_err(io_error)?;
                    let descriptor = *value
                        .payload
                        .get(4)
                        .ok_or_else(|| mismatch("closed Test zstd frame header absent"))?;
                    // A pledged small input lets zstd clamp window_log(24) to
                    // the canonical single segment. Mutate only the real frame
                    // header: bit 5 controls single segment; the explicit window
                    // descriptor encodes log2(window) as (byte >> 3) + 10.
                    let window_descriptor = (24u8 - 10) << 3;
                    if descriptor & 0x20 != 0 {
                        if descriptor >> 6 == 0 {
                            return Err(mismatch(
                                "closed Test large-window needs a retained multibyte content size",
                            ));
                        }
                        value.payload[4] = descriptor & !0x20;
                        value.payload.insert(5, window_descriptor);
                    } else {
                        *value
                            .payload
                            .get_mut(5)
                            .ok_or_else(|| mismatch("closed Test zstd window absent"))? =
                            window_descriptor;
                    }
                    if zstd::zstd_safe::find_frame_compressed_size(&value.payload)
                        .map_err(|_| mismatch("closed Test large-window frame invalid"))?
                        != value.payload.len()
                        || zstd::zstd_safe::get_frame_content_size(&value.payload)
                            .map_err(|_| mismatch("closed Test large-window size invalid"))?
                            != Some(value.uncompressed_length)
                        || zstd::stream::decode_all(value.payload.as_slice()).map_err(io_error)?
                            != inner
                    {
                        return Err(mismatch(
                            "closed Test large-window must preserve the complete original content",
                        ));
                    }
                }
                "missing-size" => {
                    let inner =
                        zstd::stream::decode_all(value.payload.as_slice()).map_err(io_error)?;
                    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), ZSTD_LEVEL)
                        .map_err(io_error)?;
                    encoder.window_log(24).map_err(io_error)?;
                    encoder.include_checksum(true).map_err(io_error)?;
                    encoder.include_contentsize(false).map_err(io_error)?;
                    encoder
                        .set_pledged_src_size(Some(inner.len() as u64))
                        .map_err(io_error)?;
                    std::io::Write::write_all(&mut encoder, &inner).map_err(io_error)?;
                    value.payload = encoder.finish().map_err(io_error)?;
                }
                "different-profile" | "different-level" => {
                    let inner =
                        zstd::stream::decode_all(value.payload.as_slice()).map_err(io_error)?;
                    let mut encoder = zstd::stream::write::Encoder::new(
                        Vec::new(),
                        if variant == "different-level" {
                            1
                        } else {
                            ZSTD_LEVEL
                        },
                    )
                    .map_err(io_error)?;
                    encoder.window_log(ZSTD_WINDOW_LOG).map_err(io_error)?;
                    encoder
                        .include_checksum(variant == "different-level")
                        .map_err(io_error)?;
                    encoder.include_contentsize(true).map_err(io_error)?;
                    encoder
                        .set_pledged_src_size(Some(inner.len() as u64))
                        .map_err(io_error)?;
                    std::io::Write::write_all(&mut encoder, &inner).map_err(io_error)?;
                    value.payload = encoder.finish().map_err(io_error)?;
                }
                "inner-trailing-space" => {
                    let mut inner =
                        zstd::stream::decode_all(value.payload.as_slice()).map_err(io_error)?;
                    inner.push(b' ');
                    value.uncompressed_length = inner.len() as u64;
                    value.uncompressed_sha256 = sha256_hex(&inner);
                    value.payload = compress(&inner, &mut WitnessBudget::maximum())?;
                }
                "unknown-codec" => value.codec.push_str("-unknown"),
                "duplicate-codec" => {
                    let mut bytes = canonical_json(&value)?;
                    bytes.pop();
                    bytes.extend_from_slice(b",\"codec\":\"duplicate\"}");
                    return Ok(bytes);
                }
                "unknown-field" => {
                    let mut bytes = canonical_json(&value)?;
                    bytes.pop();
                    bytes.extend_from_slice(b",\"unknown\":1}");
                    return Ok(bytes);
                }
                "wrapper-space" => {
                    let mut bytes = canonical_json(&value)?;
                    bytes.push(b' ');
                    return Ok(bytes);
                }
                _ => return Err(mismatch("unknown closed Test codec fault")),
            }
            canonical_json(&value)
        })
    }
    pub(crate) fn physical_byte_leaf_roundtrip_for_test(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        self.require_physical_budget_test()?;
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Leaf {
            #[serde(with = "byte_leaf")]
            bytes: Vec<u8>,
        }
        let encoded = encode_bounded(&ByteRef(bytes), &mut WitnessBudget::maximum())?;
        let wrapped = [
            b"{\"bytes\":".as_slice(),
            encoded.as_slice(),
            b"}".as_slice(),
        ]
        .concat();
        let decoded: Leaf = serde_json::from_slice(&wrapped)?;
        match_canonical(&decoded, &wrapped, &mut WitnessBudget::maximum())?;
        Ok(decoded.bytes)
    }
    pub(crate) fn physical_byte_leaf_decode_for_test(&self, encoded: &[u8]) -> Result<Vec<u8>> {
        self.require_physical_budget_test()?;
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Leaf {
            #[serde(with = "byte_leaf")]
            bytes: Vec<u8>,
        }
        if encoded.len() > MAX_WITNESS {
            return Err(mismatch("Test byte leaf input over budget"));
        }
        let value: Leaf = serde_json::from_slice(encoded)?;
        match_canonical(&value, encoded, &mut WitnessBudget::maximum())?;
        Ok(value.bytes)
    }
}

#[cfg(test)]
struct FileFault {
    namespace: (u64, u64),
    date: NaiveDate,
    remaining: usize,
    action: Box<dyn FnOnce() -> Result<()>>,
}
#[cfg(test)]
thread_local! {static FILE_FAULT:std::cell::RefCell<Option<FileFault>>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
fn run_file_fault(session: &G5bDaySession<'_>) -> Result<()> {
    let action = FILE_FAULT.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(value) = slot.as_mut() else {
            return None;
        };
        if value.namespace != session.namespace_identity || value.date != session.date {
            return None;
        }
        value.remaining -= 1;
        if value.remaining == 0 {
            slot.take().map(|v| v.action)
        } else {
            None
        }
    });
    if let Some(action) = action {
        action()?;
    }
    Ok(())
}
#[cfg(test)]
impl G5bDaySession<'_> {
    pub(crate) fn install_physical_file_fault_for_test(
        &self,
        remaining: usize,
        action: impl FnOnce() -> Result<()> + 'static,
    ) -> Result<()> {
        if !matches!(
            self.coordinator.config.environment,
            crate::durable_delivery::StoreEnvironment::Test { .. }
        ) || remaining == 0
        {
            return Err(mismatch("Physical file fault requires isolated Test"));
        }
        FILE_FAULT.with(|slot| {
            *slot.borrow_mut() = Some(FileFault {
                namespace: self.namespace_identity,
                date: self.date,
                remaining,
                action: Box::new(action),
            })
        });
        Ok(())
    }
}

fn validate_state_evidence(
    tables: &[Table],
    node: &StoredAuditChainNode,
    t: &Table,
    r: &[Cell],
) -> Result<()> {
    let bytes = t.blob(r, "evidence_canonical")?;
    let value = exact_json(bytes)?;
    let actor = t.text(r, "actor")?;
    let to = t.text(r, "to_state")?;
    let (d, decision) = single(
        tables,
        "delivery_decisions",
        "decision_identity",
        &node.decision_identity,
    )?;
    let envelope_sha = d.text(decision, "envelope_sha256")?;
    match actor {
        "prepare" => same_json(
            bytes,
            json!({"envelope_sha256":envelope_sha,"reservation_generation":if to=="Reserved"{1}else{0}}),
        ),
        "resume-deliverable" | "sink-result" => {
            let id = value
                .get("attempt_identity")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| mismatch("Physical state attempt missing"))?;
            let (a, attempt) = single(tables, "delivery_attempts", "attempt_identity", id)?;
            if a.text(attempt, "decision_identity")? != node.decision_identity {
                return Err(mismatch("Physical state cross-owner attempt"));
            }
            let fence = a.integer(attempt, "fence_token")?;
            if actor == "resume-deliverable" {
                if to != "AttemptInFlight" || a.text(attempt, "started_at")? != node.created_at {
                    return Err(mismatch("Physical state attempt start differs"));
                }
                same_json(
                    bytes,
                    json!({"attempt_identity":id,"attempt_no":a.integer(attempt,"attempt_no")?,"fence_token":fence}),
                )
            } else {
                let sha = value
                    .get("result_sha256")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| mismatch("Physical state raw hash missing"))?;
                let raw = table(tables, "sink_results")?;
                let matching = raw
                    .rows
                    .iter()
                    .filter(|row| {
                        raw.text(row, "result_sha256").ok() == Some(sha)
                            && raw.text(row, "attempt_identity").ok() == Some(id)
                    })
                    .collect::<Vec<_>>();
                if matching.len() != 1 {
                    return Err(mismatch("Physical state raw result ambiguous"));
                }
                let result = matching[0];
                let kind = raw.text(result, "result_kind")?;
                let expected_to = match kind {
                    "Accepted" => "AcceptedAuditPending",
                    "Rejected" => "RejectedAuditPending",
                    "Uncertain" => "UncertainAuditPending",
                    _ => return Err(mismatch("Physical state raw kind unknown")),
                };
                if to != expected_to
                    || raw.integer(result, "authoritative_for_state")? != 1
                    || raw.text(result, "observed_at")? != node.created_at
                {
                    return Err(mismatch("Physical state lacks original raw authority"));
                }
                same_json(
                    bytes,
                    json!({"attempt_identity":id,"result_sha256":sha,"fence_token":fence}),
                )
            }
        }
        "reconcile" => {
            let id = value
                .get("current_disposition_identity")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| mismatch("Physical finalizer disposition missing"))?;
            let (p, payload) = single(
                tables,
                "delivery_disposition_payloads",
                "disposition_identity",
                id,
            )?;
            let disposition = p.text(payload, "disposition")?;
            if p.text(payload, "decision_identity")? != node.decision_identity
                || !matches!(
                    (disposition, to),
                    ("Accepted", "Delivered")
                        | ("Rejected", "RejectedDurable")
                        | ("Uncertain", "UncertainManualReview")
                )
            {
                return Err(mismatch("Physical finalizer has wrong actual disposition"));
            }
            same_json(
                bytes,
                json!({"current_disposition_identity":id,"task_binding_present":false}),
            )
        }
        "authorized-retry" => {
            let generation = value
                .get("reservation_generation")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| mismatch("Physical retry generation absent"))?;
            if to != "Reserved"
                || generation <= 1
                || generation > d.integer(decision, "reservation_generation")?
            {
                return Err(mismatch("Physical retry generation differs"));
            }
            let from = t.optional_text(r, "from_state")?;
            if from != Some("RejectedDurable") {
                return Err(mismatch("Physical retry did not follow original rejection"));
            }
            same_json(
                bytes,
                json!({"reservation_generation":generation,"envelope_sha256":envelope_sha}),
            )
        }
        "attempt-preflight" => {
            let hash = sha256_hex(bytes);
            let p = table(tables, "delivery_disposition_payloads")?;
            let mut matches = 0;
            for row in &p.rows {
                let payload = DeliveryDispositionCanonical::parse_exact(
                    p.blob(row, "disposition_canonical")?,
                    "Physical denial",
                )?;
                if payload.decision_identity == node.decision_identity
                    && payload.evidence_sha256 == hash
                    && payload.denial_identity.is_some()
                    && payload.disposition == "Rejected"
                {
                    matches += 1;
                }
            }
            if to != "RejectedAuditPending" || matches != 1 {
                return Err(mismatch(
                    "Physical preflight does not bind original denial payload",
                ));
            }
            Ok(())
        }
        // A recovered uncertain or manually resolved original cannot later
        // become this taskless original physical Accepted decision.
        _ => Err(mismatch("Physical unsupported original state actor")),
    }
}

fn validate_payloads(tables: &[Table]) -> Result<()> {
    let payloads = table(tables, "delivery_disposition_payloads")?;
    for row in &payloads.rows {
        let bytes = payloads.blob(row, "disposition_canonical")?;
        let value = DeliveryDispositionCanonical::parse_exact(bytes, "Physical disposition")?;
        let (d, decision) = single(
            tables,
            "delivery_decisions",
            "decision_identity",
            &value.decision_identity,
        )?;
        let attempt = payloads.optional_text(row, "attempt_identity")?;
        let denial = payloads.optional_text(row, "denial_identity")?;
        let resolution = payloads.optional_text(row, "resolution_identity")?;
        if value.schema_version != 1
            || sha256_hex(bytes) != payloads.text(row, "disposition_sha256")?
            || value.disposition_identity != payloads.text(row, "disposition_identity")?
            || value.decision_identity != payloads.text(row, "decision_identity")?
            || value.envelope_sha256 != d.text(decision, "envelope_sha256")?
            || value.attempt_identity.as_deref() != attempt
            || value.denial_identity.as_deref() != denial
            || value.resolution_identity.as_deref() != resolution
            || value.disposition != payloads.text(row, "disposition")?
            || value.created_at != payloads.text(row, "created_at")?
            || resolution.is_some()
        {
            return Err(mismatch(
                "Physical disposition columns differ from exact original payload",
            ));
        }
        let source = attempt
            .or(denial)
            .ok_or_else(|| mismatch("Physical disposition source absent"))?;
        if value.disposition_identity
            != stable_identity(
                "delivery-disposition-v1",
                &[
                    &value.decision_identity,
                    source,
                    &value.disposition,
                    &value.evidence_sha256,
                ],
            )
        {
            return Err(mismatch("Physical disposition identity differs"));
        }
        if let Some(attempt) = attempt {
            let raw = table(tables, "sink_results")?;
            let matching = raw
                .rows
                .iter()
                .filter(|r| {
                    raw.text(r, "attempt_identity").ok() == Some(attempt)
                        && raw.text(r, "result_sha256").ok() == Some(value.evidence_sha256.as_str())
                })
                .collect::<Vec<_>>();
            if matching.len() != 1 {
                return Err(mismatch("Physical disposition raw evidence missing"));
            }
            let result = matching[0];
            if raw.integer(result, "authoritative_for_state")? != 1
                || raw.text(result, "decision_identity")? != value.decision_identity
                || raw.text(result, "result_kind")? != value.disposition
            {
                return Err(mismatch(
                    "Physical disposition references non-authoritative raw",
                ));
            }
            let (at, retry, manual) = match value.disposition.as_str() {
                "Accepted" => {
                    let v = AcceptedSinkResultCanonical::parse_exact(
                        raw.blob(result, "result_canonical")?,
                    )?;
                    (timestamp(v.receipt.accepted_at), false, false)
                }
                "Rejected" => {
                    let v = RejectedSinkResultCanonical::parse_exact(
                        raw.blob(result, "result_canonical")?,
                    )?;
                    (
                        timestamp(v.rejection.observed_at),
                        v.rejection.retry_authorized,
                        false,
                    )
                }
                "Uncertain" => {
                    let v = UncertainSinkResultCanonical::parse_exact(
                        raw.blob(result, "result_canonical")?,
                    )?;
                    (timestamp(v.uncertainty.observed_at), false, true)
                }
                _ => return Err(mismatch("Physical disposition kind unsupported")),
            };
            if value.created_at != at
                || value.retry_authorized != retry
                || value.manual_action_required != manual
            {
                return Err(mismatch("Physical disposition typed raw semantics differ"));
            }
        } else {
            if value.disposition != "Rejected"
                || value.retry_authorized
                || value.manual_action_required
            {
                return Err(mismatch("Physical denial payload semantics differ"));
            }
            let events = table(tables, "delivery_state_events")?;
            if !events.rows.iter().any(|r| {
                events.text(r, "decision_identity").ok() == Some(value.decision_identity.as_str())
                    && events.text(r, "evidence_sha256").ok()
                        == Some(value.evidence_sha256.as_str())
            }) {
                return Err(mismatch("Physical denial evidence event absent"));
            }
        }
        parse_timestamp(&value.created_at)?;
    }
    Ok(())
}

fn validate_audit_event(
    connection: &Connection,
    tables: &[Table],
    node: &StoredAuditChainNode,
) -> Result<()> {
    let id = &node.audit_identity;
    let bytes = &node.canonical;
    match node.audit_kind.as_str() {
        "DecisionStateChanged" => {
            let (t, r) = single(tables, "delivery_state_events", "audit_identity", id)?;
            let from = t.optional_text(r, "from_state")?;
            let to = t.text(r, "to_state")?;
            let actor = t.text(r, "actor")?;
            let evidence = t.blob(r, "evidence_canonical")?;
            let sha = t.text(r, "evidence_sha256")?;
            let event = t.text(r, "state_event_identity")?;
            if t.text(r, "decision_identity")? != node.decision_identity
                || node.attempt_identity.is_some()
                || sha256_hex(evidence) != sha
                || event
                    != stable_identity(
                        "delivery-state-event-v1",
                        &[
                            &node.decision_identity,
                            from.unwrap_or("NONE"),
                            to,
                            actor,
                            sha,
                        ],
                    )
            {
                return Err(mismatch("Physical state event binding differs"));
            }
            if let Some(from) = from {
                if !legal_transition(DecisionState::parse(from)?, DecisionState::parse(to)?) {
                    return Err(mismatch("Physical illegal state event"));
                }
            }
            same_json(
                bytes,
                json!({"state_event_identity":event,"decision_identity":node.decision_identity,"from_state":from,"to_state":to,"actor":actor,
                "operator_identity_hash":t.optional_text(r,"operator_identity")?.map(|v|sha256_hex(v.as_bytes())),"evidence_sha256":sha,"occurred_at":node.created_at}),
            )?;
            validate_state_evidence(tables, node, t, r)?;
        }
        "BudgetReservationChanged" | "CooldownReservationChanged" => {
            let budget = node.audit_kind == "BudgetReservationChanged";
            let (name, column, domain) = if budget {
                (
                    "daily_budget_reservation_events",
                    "budget_reservation_identity",
                    "delivery-budget-event-v1",
                )
            } else {
                (
                    "cooldown_reservation_events",
                    "cooldown_reservation_identity",
                    "delivery-cooldown-event-v1",
                )
            };
            let (t, r) = single(tables, name, "audit_identity", id)?;
            let reservation = t.text(r, column)?;
            let from = t.optional_text(r, "from_state")?;
            let to = t.text(r, "to_state")?;
            let evidence = t.blob(r, "event_canonical")?;
            let sha = t.text(r, "event_sha256")?;
            if node.attempt_identity.is_some()
                || t.text(r, "decision_identity")? != node.decision_identity
                || sha256_hex(evidence) != sha
                || t.text(r, "event_identity")?
                    != stable_identity(domain, &[reservation, from.unwrap_or("NONE"), to, sha])
            {
                return Err(mismatch("Physical reservation event differs"));
            }
            let (reservation_table, reservation_row) = single(
                tables,
                if budget {
                    "daily_budget_reservations"
                } else {
                    "cooldown_reservations"
                },
                column,
                reservation,
            )?;
            if reservation_table.text(reservation_row, "decision_identity")?
                != node.decision_identity
            {
                return Err(mismatch("Physical cross-owner reservation"));
            }
            let mut expected = json!({"from_state":from,"to_state":to,"event_sha256":sha,"occurred_at":node.created_at});
            expected
                .as_object_mut()
                .unwrap()
                .insert(column.to_owned(), json!(reservation));
            same_json(bytes, expected)?;
            exact_json(evidence)?;
        }
        "LeaseGranted"
        | "LeaseHeartbeat"
        | "FenceRevoked"
        | "RecoveryClassified"
        | "SinkResultAuthorityClassified"
        | "LateReceiptObserved" => {
            single(tables, "delivery_attempt_events", "audit_identity", id)?;
            validate_attempt_event_for_audit(connection, node)?;
            let attempt = node
                .attempt_identity
                .as_deref()
                .ok_or_else(|| mismatch("Physical attempt audit missing attempt"))?;
            let (a, r) = single(tables, "delivery_attempts", "attempt_identity", attempt)?;
            if a.text(r, "decision_identity")? != node.decision_identity {
                return Err(mismatch("Physical cross-owner attempt"));
            }
            let fence = a.integer(r, "fence_token")?;
            match node.audit_kind.as_str() {
                "LeaseGranted" | "LeaseHeartbeat" => {
                    let expires = exact_json(bytes)?
                        .get("lease_expires_at")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| mismatch("Physical lease expiry missing"))?
                        .to_owned();
                    if parse_timestamp(&expires)? <= parse_timestamp(&node.created_at)? {
                        return Err(mismatch("Physical lease expiry precedes observation"));
                    }
                    let mut expected = json!({"lease_expires_at":expires,"fence_token":fence});
                    if node.audit_kind == "LeaseGranted" {
                        if a.text(r, "started_at")? != node.created_at {
                            return Err(mismatch(
                                "Physical lease grant time differs from original attempt",
                            ));
                        }
                        expected.as_object_mut().unwrap().insert(
                            "owner_instance_identity_hash".to_owned(),
                            json!(sha256_hex(a.text(r, "owner_instance_identity")?.as_bytes())),
                        );
                    }
                    same_json(bytes, expected)?;
                }
                "SinkResultAuthorityClassified" | "LateReceiptObserved" => {
                    let column = if node.audit_kind == "LateReceiptObserved" {
                        "late_receipt_audit_identity"
                    } else {
                        "authority_audit_identity"
                    };
                    let (s, r) = single(tables, "sink_results", column, id)?;
                    if s.text(r, "attempt_identity")? != attempt
                        || s.text(r, "decision_identity")? != node.decision_identity
                        || s.integer(r, "fence_token")? != fence
                        || s.text(r, "observed_at")? != node.created_at
                    {
                        return Err(mismatch("Physical raw receipt/audit binding differs"));
                    }
                    let mut expected = json!({"attempt_identity":attempt,"fence_token":fence,"result_sha256":s.text(r,"result_sha256")?});
                    if column == "authority_audit_identity" {
                        expected.as_object_mut().unwrap().insert(
                            "authoritative_for_state".to_owned(),
                            json!(s.integer(r, "authoritative_for_state")? == 1),
                        );
                    } else {
                        expected
                            .as_object_mut()
                            .unwrap()
                            .insert("result_kind".to_owned(), json!(s.text(r, "result_kind")?));
                    }
                    same_json(bytes, expected)?;
                }
                "FenceRevoked" => {
                    let value = exact_json(bytes)?;
                    if value.get("attempt_identity") != Some(&json!(attempt))
                        || value.get("revoked_fence_token") != Some(&json!(fence))
                        || a.optional_text(r, "fence_revoked_at")? != Some(node.created_at.as_str())
                    {
                        return Err(mismatch("Physical revoked fence binding differs"));
                    }
                    let replacement = value
                        .get("replacement_fence_generation")
                        .and_then(serde_json::Value::as_i64)
                        .ok_or_else(|| mismatch("Physical replacement fence absent"))?;
                    if replacement <= fence {
                        return Err(mismatch("Physical fence did not advance"));
                    }
                    same_json(
                        bytes,
                        json!({"attempt_identity":attempt,"revoked_fence_token":fence,"replacement_fence_generation":replacement,"lease_expires_at":a.text(r,"lease_expires_at")?}),
                    )?;
                }
                "RecoveryClassified" => {
                    let previous = node
                        .predecessor_audit_identity
                        .as_deref()
                        .ok_or_else(|| mismatch("Physical recovery fence predecessor absent"))?;
                    let fence_node = load_sealed_audit_chain_node(connection, previous)?;
                    if fence_node.audit_kind != "FenceRevoked"
                        || fence_node.attempt_identity.as_deref() != Some(attempt)
                        || fence_node.decision_identity != node.decision_identity
                    {
                        return Err(mismatch("Physical recovery fence predecessor differs"));
                    }
                    same_json(
                        bytes,
                        json!({"classification":"Uncertain","automatic_resend":false,"persisted_receipt":false,"fence_evidence_sha256":fence_node.sha256}),
                    )?;
                }
                _ => unreachable!(),
            }
        }
        "BusinessDateOnceClaimed" => {
            let (t, r) = single(tables, "business_date_once_claims", "audit_identity", id)?;
            if node.attempt_identity.is_some()
                || t.text(r, "decision_identity")? != node.decision_identity
                || t.text(r, "claimed_at")? != node.created_at
            {
                return Err(mismatch("Physical business once claim differs"));
            }
            same_json(
                bytes,
                json!({"business_date":t.text(r,"business_date")?,"push_kind":t.text(r,"push_kind")?,"sub_kind":t.text(r,"sub_kind")?,"scope_key":t.text(r,"scope_key")?,"decision_identity":node.decision_identity,"policy_version":t.integer(r,"policy_version")?}),
            )?;
        }
        "DecisionIdentityConflict" => {
            let (t, r) = single(
                tables,
                "delivery_decisions",
                "decision_identity",
                &node.decision_identity,
            )?;
            let value = exact_json(bytes)?;
            let incoming = value
                .get("incoming_envelope_sha256")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| mismatch("Physical incoming conflict hash missing"))?;
            if incoming.len() != 64
                || !incoming
                    .bytes()
                    .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
                || node.attempt_identity.is_some()
            {
                return Err(mismatch("Physical conflict hash invalid"));
            }
            same_json(
                bytes,
                json!({"decision_identity":node.decision_identity,"stored_envelope_sha256":t.text(r,"envelope_sha256")?,"incoming_envelope_sha256":incoming}),
            )?;
        }
        // Original G5b model envelopes have no task/review replay binding. These
        // audits cannot be silently ignored or grant physical completion.
        _ => return Err(mismatch("Physical unsupported task/review audit")),
    }
    Ok(())
}

#[cfg(test)]
mod arena_codec_tests {
    use super::*;
    // Synthetic codec material only: no actual source/file/Accepted witness and
    // no VerifiedG5bPhysicalSeal can be produced by any function in this module.
    fn material() -> PhysicalSeal {
        let date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let selection = b"same body".to_vec();
        let mut tables = TABLES
            .iter()
            .map(|name| Table {
                name: (*name).into(),
                columns: vec!["id".into(), "blob".into(), "text".into()],
                rows: Vec::new(),
            })
            .collect::<Vec<_>>();
        tables[0].rows = vec![
            vec![
                Cell::Integer(1),
                Cell::Blob(EvidenceBytes::Owned(selection.clone())),
                Cell::Text(EvidenceText::Owned("same body".into())),
            ],
            vec![
                Cell::Integer(2),
                Cell::Blob(EvidenceBytes::Owned(selection.clone())),
                Cell::Text(EvidenceText::Owned("nul\0quote\"slash\\\n中文".into())),
            ],
            vec![
                Cell::Integer(10),
                Cell::Blob(EvidenceBytes::Owned(vec![0xff, 0])),
                Cell::Text(EvidenceText::Owned(String::new())),
            ],
        ];
        sort_rows(&mut tables[0].rows, &mut WitnessBudget::maximum()).unwrap();
        PhysicalSeal {
            storage_version: 2,
            version: 1,
            material: MATERIAL.into(),
            business_date: date,
            cohort_identity: "synthetic-codec-only".into(),
            revision: 1,
            reason: REASON.into(),
            model_binding: model_bundle::SealModelBinding {
                selection_bytes: EvidenceBytes::Owned(selection),
                admission: Admission {
                    material: "synthetic-codec-only".into(),
                    business_date: date,
                    cohort_identity: "synthetic-codec-only".into(),
                    selection_sha256: "not-qualified".into(),
                    calendar_authority_sha256: "not-qualified".into(),
                    namespace_device: 0,
                    namespace_inode: 0,
                    database_device: 0,
                    database_inode: 0,
                    owner_instance: "TEST_CODE_CODEC".into(),
                    environment: "Test:TEST_CODE_CODEC".into(),
                    observed_at: DateTime::parse_from_rfc3339("2026-09-28T15:10:00+08:00")
                        .unwrap()
                        .with_timezone(&Utc),
                    provider: "synthetic".into(),
                    model: "synthetic".into(),
                },
                artifacts: Vec::new(),
            },
            full_archive_identity: "no-actual-archive".into(),
            members: vec![Member {
                index: 0,
                occurrence: "no-actual-occurrence".into(),
                decision: "no-actual-owner".into(),
                attempt: "no-actual-attempt".into(),
                evidence_sha256: "no-actual-Accepted".into(),
            }],
            tables,
        }
    }
    fn store_inner(inner: &[u8]) -> Vec<u8> {
        let mut budget = WitnessBudget::maximum();
        let payload = compress(inner, &mut budget).unwrap();
        encode_bounded(
            &StoredPhysicalSeal {
                version: 2,
                material: MATERIAL.into(),
                codec: ARENA_CODEC.into(),
                uncompressed_length: inner.len() as u64,
                uncompressed_sha256: sha256_hex(inner),
                payload,
            },
            &mut budget,
        )
        .unwrap()
    }
    #[test]
    fn g5b_physical_seal_byte_arena_roundtrip_preserves_complete_bytes_types_and_shares() {
        let original = material();
        let bytes = encode_stored(&original, &mut WitnessBudget::maximum()).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(
            encode_stored(&decoded, &mut WitnessBudget::maximum()).unwrap(),
            bytes
        );
        let table = &decoded.tables[0];
        let row = table
            .rows
            .iter()
            .find(|r| table.integer(r, "id").unwrap() == 1)
            .unwrap();
        assert_eq!(table.blob(row, "blob").unwrap(), b"same body");
        assert_eq!(table.text(row, "text").unwrap(), "same body");
        let (
            Cell::Blob(EvidenceBytes::Shared(blob)),
            Cell::Text(EvidenceText::Shared(EvidenceBytes::Shared(text))),
        ) = (&row[1], &row[2])
        else {
            panic!("v2 must retain shared full bodies")
        };
        let EvidenceBytes::Shared(selection) = &decoded.model_binding.selection_bytes else {
            panic!("model binding must share original body")
        };
        assert!(std::sync::Arc::ptr_eq(blob, text));
        assert!(std::sync::Arc::ptr_eq(blob, selection));
        let binary = table
            .rows
            .iter()
            .find(|r| table.integer(r, "id").unwrap() == 10)
            .unwrap();
        assert_eq!(table.blob(binary, "blob").unwrap(), &[0xff, 0]);
        assert_eq!(table.text(binary, "text").unwrap(), "");
    }
    #[test]
    fn g5b_physical_seal_byte_arena_old_full_row_canonical_order_is_exact() {
        let mut rows = Vec::new();
        for i in [2, 10, 1, -1, 0, i64::MIN, i64::MAX] {
            for text in ["", "a", "a\0", "\n", "中文", "\"\\"] {
                rows.push(vec![
                    Cell::Integer(i),
                    Cell::Text(EvidenceText::Owned(text.into())),
                    Cell::Blob(EvidenceBytes::Owned(b"same large body".repeat(128))),
                ]);
            }
        }
        let mut oracle = rows.clone();
        oracle.sort_by_key(|row| serde_json::to_vec(row).unwrap());
        sort_rows(&mut rows, &mut WitnessBudget::maximum()).unwrap();
        assert_eq!(rows, oracle);
        for pair in rows.windows(2) {
            assert_eq!(
                compare_rows(&pair[0], &pair[1], &mut WitnessBudget::maximum()).unwrap(),
                serde_json::to_vec(&pair[0])
                    .unwrap()
                    .cmp(&serde_json::to_vec(&pair[1]).unwrap())
            );
        }
    }
    #[test]
    fn g5b_physical_seal_byte_arena_rejects_aliases_unused_and_dangling_refs() {
        for mode in 0..4 {
            let mut wire = arena_physical(&material(), &mut WitnessBudget::maximum()).unwrap();
            match mode {
                0 => wire.arena.insert(0, wire.arena[0].clone()),
                1 => wire.arena.push(EvidenceBytes::Owned(vec![0xff, 0xff])),
                2 => wire.model_binding.selection_bytes = usize::MAX,
                3 => {
                    wire.tables[0].rows[0][1] =
                        ArenaCell::TextRef(arena_index(&wire.arena, &[0xff, 0]).unwrap())
                }
                _ => unreachable!(),
            }
            let bytes = store_inner(&serde_json::to_vec(&wire).unwrap());
            assert!(decode(&bytes).is_err(), "mode {mode}");
        }
    }
    #[test]
    fn g5b_physical_seal_byte_arena_rejects_noncanonical_fields_leaves_and_index_types() {
        let inner =
            String::from_utf8(encode_arena(&material(), &mut WitnessBudget::maximum()).unwrap())
                .unwrap();
        for (from, to) in [
            ("\"Utf8\":\"same body\"", "\"Utf8\":\"\\u0073ame body\""),
            ("\"Utf8\":\"same body\"", "\"Hex\":\"73616d6520626f6479\""),
            ("\"Hex\":\"ff00\"", "\"Hex\":\"FF00\""),
            ("\"Integer\":1}", "\"Real\":1.0}"),
            ("\"BlobRef\":3}", "\"BlobRef\":-1}"),
        ] {
            let changed = inner.replacen(from, to, 1);
            assert_ne!(changed, inner, "fixture token {from}");
            assert!(decode(&store_inner(changed.as_bytes())).is_err(), "{to}");
        }
        let mut extra = inner.clone();
        extra.pop();
        extra.push_str(",\"unknown\":1}");
        assert!(decode(&store_inner(extra.as_bytes())).is_err());
        let mut duplicate = inner.clone();
        duplicate.pop();
        duplicate.push_str(",\"version\":2}");
        assert!(decode(&store_inner(duplicate.as_bytes())).is_err());

        // Prefix dispatch is allocation-free routing only. Both original
        // canonical versions classify, while malformed/escaped/reordered bytes
        // still fail their actual strict decoder and never produce a capability.
        for version in [1, 2] {
            let mut value = material();
            value.storage_version = version;
            let bytes = encode_stored(&value, &mut WitnessBudget::maximum()).unwrap();
            assert!(is_physical(&bytes).unwrap());
            assert_eq!(decode(&bytes).unwrap(), value);
        }
        let original =
            String::from_utf8(encode_stored(&material(), &mut WitnessBudget::maximum()).unwrap())
                .unwrap();
        for (from, to, classified) in [
            ("{\"version\":2,", "{\"version\":3,", None),
            (
                "\"material\":\"g5b-physical-day-seal-v1\"",
                "\"material\":\"\\u00675b-physical-day-seal-v1\"",
                None,
            ),
            (
                "{\"version\":2,\"material\":\"g5b-physical-day-seal-v1\",",
                "{\"material\":\"g5b-physical-day-seal-v1\",\"version\":2,",
                None,
            ),
            (
                "\"material\":\"g5b-physical-day-seal-v1\",",
                "\"material\":\"g5b-physical-day-seal-v1\",\"unknown\":1,",
                Some(true),
            ),
        ] {
            let changed = original.replacen(from, to, 1);
            assert_ne!(changed, original);
            match classified {
                Some(expected) => assert_eq!(is_physical(changed.as_bytes()).unwrap(), expected),
                None => assert!(is_physical(changed.as_bytes()).is_err()),
            }
            assert!(decode(changed.as_bytes()).is_err());
        }
        let truncated = b"{\"version\":2,\"material\":\"g5b-physical-day-seal-v1\",";
        assert!(is_physical(truncated).unwrap());
        assert!(decode(truncated).is_err());
        assert!(!is_physical(b"{\"version\":1,\"material\":\"g5b-empty-day-seal-v1\",").unwrap());
        assert!(is_physical(b"not JSON").is_err());
        assert!(decode(b"not JSON").is_err());
        // Below MAX_WITNESS, an unknown huge material must fail before either
        // owned parser can copy it. This input is synthetic negative material.
        let mut huge_unknown = b"{\"version\":1,\"material\":\"".to_vec();
        huge_unknown.extend(std::iter::repeat_n(b'x', 1024 * 1024));
        huge_unknown.extend_from_slice(b"\",\"business_date\":\"2026-09-28\"}");
        assert!(huge_unknown.len() < MAX_WITNESS);
        let error = is_physical(&huge_unknown).unwrap_err().to_string();
        assert!(
            error.contains("canonical dispatch prefix differs"),
            "{error}"
        );
    }
    #[test]
    fn g5b_physical_seal_byte_arena_rejects_rowid_order_width_and_4097_rows() {
        for mode in 0..4 {
            let mut wire = arena_physical(&material(), &mut WitnessBudget::maximum()).unwrap();
            match mode {
                0 => wire.tables[0].rows.swap(0, 1),
                1 => wire.tables[0].rows[0].pop().map(|_| ()).unwrap(),
                2 => wire.tables.swap(0, 1),
                3 => {
                    let row = wire.tables[0].rows[0].clone();
                    wire.tables[0].rows = vec![row; MAX_ROWS + 1];
                }
                _ => unreachable!(),
            }
            assert!(
                decode(&store_inner(&serde_json::to_vec(&wire).unwrap())).is_err(),
                "mode {mode}"
            );
        }
    }
    #[test]
    fn g5b_physical_seal_byte_arena_intern_and_metadata_fail_before_allocation() {
        let mut budget = WitnessBudget::maximum();
        budget.limit = 0;
        assert!(budget.intern(b"not-owned").is_err());
        assert!(budget.pool.is_empty());
        assert_eq!(budget.used, 0);
        let bytes = encode_stored(&material(), &mut WitnessBudget::maximum()).unwrap();
        let mut full = WitnessBudget::maximum();
        decode_with_budget(&bytes, &mut full).unwrap();
        let mut short = WitnessBudget::maximum();
        short.limit = full.used - 1;
        assert!(decode_with_budget(&bytes, &mut short).is_err());
        let mut d = serde_json::Deserializer::from_slice(b"[1,2,3,4]");
        let mut tiny = WitnessBudget::maximum();
        tiny.limit = 1;
        assert!(Vec::<usize>::deserialize(BudgetDeserializer {
            inner: &mut d,
            budget: &mut tiny
        })
        .is_err());
        assert_eq!(tiny.used, 0);
        let mut d = serde_json::Deserializer::from_slice(b"\"metadata\"");
        let mut tiny = WitnessBudget::maximum();
        tiny.limit = 1;
        assert!(String::deserialize(BudgetDeserializer {
            inner: &mut d,
            budget: &mut tiny
        })
        .is_err());
        assert_eq!(tiny.used, 0);
    }
    #[test]
    fn g5b_physical_seal_byte_arena_v1_exact_codec_and_types_remain_original() {
        let mut old = material();
        old.storage_version = 1;
        let inner = serde_json::to_vec(&old).unwrap();
        assert!(inner.starts_with(b"{\"version\":1,"));
        assert!(!String::from_utf8_lossy(&inner).contains("storage_version"));
        let bytes = encode_stored(&old, &mut WitnessBudget::maximum()).unwrap();
        let value = decode(&bytes).unwrap();
        assert_eq!(value, old);
        assert_eq!(serde_json::to_vec(&value).unwrap(), inner);
        assert_eq!(
            encode_stored(&value, &mut WitnessBudget::maximum()).unwrap(),
            bytes
        );
        assert_eq!(
            serde_json::to_vec(&Cell::Text(EvidenceText::Owned("raw".into()))).unwrap(),
            br#"{"Text":"raw"}"#
        );
        assert_eq!(
            serde_json::to_vec(&Cell::Blob(EvidenceBytes::Owned(b"raw".to_vec()))).unwrap(),
            br#"{"Blob":{"Utf8":"raw"}}"#
        );
    }
}
