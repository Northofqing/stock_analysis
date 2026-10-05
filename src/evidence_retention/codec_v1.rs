//! Closed borrowed JSON preflight. No serde tree or escaped-string scratch is
//! allocated until this pass has checked every node and reserved the DTO cost.
use super::*;
use serde::Serialize;
use std::io::{self, Write};
pub(super) const MIB: usize = 1024 * 1024;
pub(super) const DRAFT_LIMIT: usize = 3 * MIB;
pub(super) const RECEIPT_LIMIT: usize = 64 * 1024;
pub(super) const ROOT_LIMIT: usize = 256 * 1024;
pub(super) struct Work {
    owned: usize,
    scanned: usize,
    nodes: usize,
}
impl Work {
    pub(super) fn new() -> Self {
        Self {
            owned: 0,
            scanned: 0,
            nodes: 0,
        }
    }
    pub(super) fn own(&mut self, n: usize) -> Result<(), ValueError> {
        self.owned = self
            .owned
            .checked_add(n)
            .ok_or(ValueError::AllocationLimit)?;
        if self.owned > 8 * MIB {
            Err(ValueError::AllocationLimit)
        } else {
            Ok(())
        }
    }
    pub(super) fn scan(&mut self, n: usize) -> Result<(), ValueError> {
        self.scanned = self.scanned.checked_add(n).ok_or(ValueError::InputLimit)?;
        if self.scanned > 32 * MIB {
            Err(ValueError::InputLimit)
        } else {
            Ok(())
        }
    }
    pub(super) fn node(&mut self) -> Result<(), ValueError> {
        self.nodes = self.nodes.checked_add(1).ok_or(ValueError::NodeLimit)?;
        if self.nodes > 8192 {
            Err(ValueError::NodeLimit)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy)]
pub(super) enum Shape {
    Draft,
    Receipt,
    Root,
    Time,
    Coverage,
    Entry,
    Gates,
}
#[derive(Clone, Copy)]
enum Ty {
    Text(usize),
    Num,
    Bool,
    Obj(Shape),
    Array(Shape, usize),
    Strings,
    OptText(usize),
    OptNum,
    OptTime,
}
const DRAFT: &[(&str, Ty)] = &[
    ("schema", Ty::Text(64)),
    ("schema_version", Ty::Num),
    ("trust", Ty::Text(16)),
    ("owner_domain", Ty::Text(32)),
    ("owner_schema_claim", Ty::Text(128)),
    ("logical_slot_claim", Ty::Text(256)),
    ("business_day_claim", Ty::Text(10)),
    ("window_start_claim", Ty::Obj(Shape::Time)),
    ("window_end_exclusive_claim", Ty::Obj(Shape::Time)),
    ("claimed_record_count", Ty::OptNum),
    ("source_chain_before_claim", Ty::OptText(64)),
    ("source_chain_after_claim", Ty::OptText(64)),
    ("artifact_sha256_claim", Ty::OptText(64)),
    ("activation_id_claim", Ty::OptText(256)),
    ("body_encoding", Ty::Text(8)),
    ("body_length", Ty::Num),
    ("body_sha256", Ty::Text(64)),
    ("body_hex", Ty::Text(2 * MIB)),
];
const RECEIPT: &[(&str, Ty)] = &[
    ("schema", Ty::Text(64)),
    ("schema_version", Ty::Num),
    ("trust", Ty::Text(16)),
    ("package_id", Ty::Text(128)),
    ("storage_authority_claim", Ty::Text(256)),
    ("container_claim", Ty::Text(256)),
    ("object_key_claim", Ty::Text(1024)),
    ("version_id_claim", Ty::OptText(2048)),
    ("retention_mode_claim", Ty::Text(16)),
    ("clock_evidence", Ty::Text(16)),
    ("confirmation_upper_bound_claim", Ty::OptTime),
    ("retain_until_claim", Ty::OptTime),
    ("head_content_length_claim", Ty::OptNum),
    ("get_content_length_claim", Ty::OptNum),
    ("get_sha256_claim", Ty::OptText(64)),
    ("readback_complete_claim", Ty::Bool),
    ("request_id_claims", Ty::Strings),
];
const ROOT: &[(&str, Ty)] = &[
    ("schema", Ty::Text(64)),
    ("schema_version", Ty::Num),
    ("trust", Ty::Text(16)),
    ("coverage_state", Ty::Text(16)),
    ("signature_state", Ty::Text(16)),
    ("authority_gates", Ty::Obj(Shape::Gates)),
    ("business_day_claim", Ty::Text(10)),
    ("revision", Ty::Num),
    ("previous_day_root_id_claim", Ty::OptText(128)),
    ("previous_revision_root_id_claim", Ty::OptText(128)),
    ("coverage", Ty::Array(Shape::Coverage, 4)),
    ("entries", Ty::Array(Shape::Entry, 128)),
];
const TIME: &[(&str, Ty)] = &[("unix_seconds", Ty::Num), ("nanosecond", Ty::Num)];
const COVERAGE: &[(&str, Ty)] = &[
    ("owner_domain", Ty::Text(32)),
    ("state", Ty::Text(32)),
    ("package_count", Ty::Num),
];
const ENTRY: &[(&str, Ty)] = &[
    ("owner_domain", Ty::Text(32)),
    ("logical_slot_claim", Ty::Text(256)),
    ("package_id", Ty::Text(128)),
    ("package_canonical_sha256", Ty::Text(64)),
    ("package_canonical_length", Ty::Num),
    ("receipt_id", Ty::OptText(128)),
    ("receipt_claim_consistency", Ty::Text(32)),
];
const GATES: &[(&str, Ty)] = &[
    ("owner_seal", Ty::Text(32)),
    ("remote_retention", Ty::Text(32)),
    ("signer", Ty::Text(32)),
    ("restore", Ty::Text(32)),
];
fn fields(s: Shape) -> &'static [(&'static str, Ty)] {
    match s {
        Shape::Draft => DRAFT,
        Shape::Receipt => RECEIPT,
        Shape::Root => ROOT,
        Shape::Time => TIME,
        Shape::Coverage => COVERAGE,
        Shape::Entry => ENTRY,
        Shape::Gates => GATES,
    }
}
fn size(s: Shape) -> usize {
    match s {
        Shape::Draft => std::mem::size_of::<DraftWire>(),
        Shape::Receipt => std::mem::size_of::<ReceiptWire>(),
        Shape::Root => std::mem::size_of::<RootWire>(),
        Shape::Time => std::mem::size_of::<UtcInstantClaim>(),
        Shape::Coverage => std::mem::size_of::<Coverage>(),
        Shape::Entry => std::mem::size_of::<Entry>(),
        Shape::Gates => std::mem::size_of::<Gates>(),
    }
}
struct Scan<'a, 'w> {
    b: &'a [u8],
    p: usize,
    work: &'w mut Work,
    reserve: usize,
    arrays: [usize; 3],
    array_count: usize,
}
impl Scan<'_, '_> {
    fn reserve(&mut self, n: usize) -> Result<(), ValueError> {
        self.reserve = self
            .reserve
            .checked_add(n)
            .ok_or(ValueError::AllocationLimit)?;
        if self.reserve > 8 * MIB {
            Err(ValueError::AllocationLimit)
        } else {
            Ok(())
        }
    }
    fn ws(&mut self) {
        while self.b.get(self.p).is_some_and(u8::is_ascii_whitespace) {
            self.p += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), ValueError> {
        self.ws();
        if self.b.get(self.p) == Some(&c) {
            self.p += 1;
            Ok(())
        } else {
            Err(ValueError::WrongType)
        }
    }
    fn word(&mut self, w: &[u8]) -> bool {
        if self.b.get(self.p..self.p + w.len()) == Some(w) {
            self.p += w.len();
            true
        } else {
            false
        }
    }
    fn string(&mut self, cap: usize, key: bool) -> Result<(usize, usize, usize), ValueError> {
        self.eat(b'"')?;
        let start = self.p;
        let mut len = 0usize;
        loop {
            let c = *self.b.get(self.p).ok_or(ValueError::MalformedJson)?;
            if c == b'"' {
                let end = self.p;
                self.p += 1;
                return Ok((start, end, len));
            }
            if c < 32 {
                return Err(ValueError::MalformedJson);
            }
            self.p += 1;
            let n = if c == b'\\' {
                if key {
                    return Err(ValueError::NonCanonical);
                }
                let e = *self.b.get(self.p).ok_or(ValueError::MalformedJson)?;
                self.p += 1;
                match e {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => 1,
                    b'u' => {
                        let a = self.hex4()?;
                        let cp = if (0xd800..=0xdbff).contains(&a) {
                            if !self.word(b"\\u") {
                                return Err(ValueError::MalformedJson);
                            }
                            let z = self.hex4()?;
                            if !(0xdc00..=0xdfff).contains(&z) {
                                return Err(ValueError::MalformedJson);
                            }
                            0x10000 + ((a - 0xd800) << 10) + (z - 0xdc00)
                        } else {
                            a
                        };
                        char::from_u32(cp)
                            .ok_or(ValueError::MalformedJson)?
                            .len_utf8()
                    }
                    _ => return Err(ValueError::MalformedJson),
                }
            } else {
                1
            };
            len = len.checked_add(n).ok_or(ValueError::InputLimit)?;
            if len > cap {
                return Err(ValueError::InvalidScalar);
            }
        }
    }
    fn hex4(&mut self) -> Result<u32, ValueError> {
        let mut n = 0;
        for _ in 0..4 {
            let b = *self.b.get(self.p).ok_or(ValueError::MalformedJson)?;
            self.p += 1;
            n = n * 16
                + match b {
                    b'0'..=b'9' => (b - b'0') as u32,
                    b'a'..=b'f' => (b - b'a' + 10) as u32,
                    b'A'..=b'F' => (b - b'A' + 10) as u32,
                    _ => return Err(ValueError::MalformedJson),
                };
        }
        Ok(n)
    }
    fn number(&mut self) -> Result<(), ValueError> {
        let start = self.p;
        while self.b.get(self.p).is_some_and(u8::is_ascii_digit) {
            self.p += 1;
            if self.p - start > 20 {
                return Err(ValueError::InvalidScalar);
            }
        }
        if start == self.p {
            return Err(ValueError::WrongType);
        }
        let raw = &self.b[start..self.p];
        if raw.len() > 1 && raw[0] == b'0' {
            return Err(ValueError::NonCanonical);
        }
        std::str::from_utf8(raw)
            .map_err(|_| ValueError::MalformedJson)?
            .parse::<u64>()
            .map_err(|_| ValueError::InvalidScalar)?;
        Ok(())
    }
    fn value(&mut self, t: Ty, depth: usize) -> Result<(), ValueError> {
        if depth > 8 {
            return Err(ValueError::DepthLimit);
        }
        self.ws();
        self.work.node()?;
        let t = match t {
            Ty::OptText(n) => {
                if self.word(b"null") {
                    return Ok(());
                }
                Ty::Text(n)
            }
            Ty::OptNum => {
                if self.word(b"null") {
                    return Ok(());
                }
                Ty::Num
            }
            Ty::OptTime => {
                if self.word(b"null") {
                    return Ok(());
                }
                Ty::Obj(Shape::Time)
            }
            t => t,
        };
        match t {
            Ty::Text(cap) => {
                let (_, _, n) = self.string(cap, false)?;
                self.reserve(n + std::mem::size_of::<String>())
            }
            Ty::Num => self.number(),
            Ty::Bool => {
                if self.word(b"true") || self.word(b"false") {
                    Ok(())
                } else {
                    Err(ValueError::WrongType)
                }
            }
            Ty::Obj(s) => self.object(s, depth),
            Ty::Array(s, max) => self.array(Some(s), max, depth),
            Ty::Strings => self.array(None, 4, depth),
            _ => unreachable!(),
        }
    }
    fn object(&mut self, s: Shape, depth: usize) -> Result<(), ValueError> {
        self.eat(b'{')?;
        self.reserve(size(s))?;
        let fs = fields(s);
        let mut seen = 0u32;
        let mut count = 0;
        self.ws();
        if self.b.get(self.p) == Some(&b'}') {
            return Err(ValueError::MissingField);
        }
        loop {
            self.work.node()?;
            let (a, z, _) = self.string(64, true)?;
            let i = fs
                .iter()
                .position(|(k, _)| k.as_bytes() == &self.b[a..z])
                .ok_or(ValueError::UnknownField)?;
            if seen & (1 << i) != 0 {
                return Err(ValueError::DuplicateField);
            }
            if i != count {
                return Err(ValueError::NonCanonical);
            }
            seen |= 1 << i;
            count += 1;
            self.eat(b':')?;
            self.value(fs[i].1, depth + 1)?;
            self.ws();
            if self.b.get(self.p) == Some(&b'}') {
                self.p += 1;
                break;
            }
            self.eat(b',')?;
        }
        if count != fs.len() {
            Err(ValueError::MissingField)
        } else {
            Ok(())
        }
    }
    fn array(&mut self, s: Option<Shape>, max: usize, depth: usize) -> Result<(), ValueError> {
        let index = self.array_count;
        self.array_count += 1;
        if index >= 3 {
            return Err(ValueError::InputLimit);
        }
        self.eat(b'[')?;
        self.reserve(std::mem::size_of::<Vec<u8>>())?;
        self.ws();
        let mut n = 0;
        if self.b.get(self.p) == Some(&b']') {
            self.p += 1;
            return Ok(());
        }
        loop {
            if n == max {
                return Err(ValueError::InputLimit);
            }
            n += 1;
            self.work.node()?;
            self.value(s.map(Ty::Obj).unwrap_or(Ty::Text(256)), depth + 1)?;
            self.ws();
            if self.b.get(self.p) == Some(&b']') {
                self.p += 1;
                break;
            }
            self.eat(b',')?;
        }
        self.arrays[index] = n;
        Ok(())
    }
}
pub(super) struct Reservation {
    owned: usize,
    arrays: [usize; 3],
}
pub(super) fn preflight(
    bytes: &[u8],
    s: Shape,
    limit: usize,
    w: &mut Work,
) -> Result<Reservation, ValueError> {
    if bytes.is_empty() || bytes.len() > limit {
        return Err(ValueError::InputLimit);
    }
    w.scan(bytes.len().checked_mul(2).ok_or(ValueError::InputLimit)?)?;
    std::str::from_utf8(bytes).map_err(|_| ValueError::MalformedJson)?;
    let mut p = Scan {
        b: bytes,
        p: 0,
        work: w,
        reserve: 0,
        arrays: [0; 3],
        array_count: 0,
    };
    p.value(Ty::Obj(s), 1)?;
    p.ws();
    if p.p != bytes.len() {
        return Err(ValueError::MalformedJson);
    }
    Ok(Reservation {
        owned: p.reserve,
        arrays: p.arrays,
    })
}
#[cfg(test)]
thread_local! {pub(super) static OWNED_HITS:std::cell::Cell<usize>=const{std::cell::Cell::new(0)};}
// The second pass uses explicit exact capacities. It does not ask serde_json to
// create strings, arrays, Content or Value, including escaped-string scratch.
pub(super) struct Owned<'a> {
    b: &'a [u8],
    p: usize,
    arrays: [usize; 3],
    array_next: usize,
}
impl Owned<'_> {
    fn ws(&mut self) {
        while self.b.get(self.p).is_some_and(u8::is_ascii_whitespace) {
            self.p += 1;
        }
    }
    fn eat(&mut self, b: u8) -> Result<(), ValueError> {
        self.ws();
        if self.b.get(self.p) != Some(&b) {
            return Err(ValueError::MalformedJson);
        }
        self.p += 1;
        Ok(())
    }
    fn span(&mut self) -> Result<(usize, usize, usize), ValueError> {
        self.eat(b'"')?;
        let start = self.p;
        let mut decoded = 0;
        while self.b[self.p] != b'"' {
            if self.b[self.p] == b'\\' {
                self.p += 1;
                match self.b[self.p] {
                    b'u' => {
                        self.p += 1;
                        let a = self.hex4();
                        let cp = if (0xd800..=0xdbff).contains(&a) {
                            self.p += 2;
                            let z = self.hex4();
                            0x10000 + ((a - 0xd800) << 10) + (z - 0xdc00)
                        } else {
                            a
                        };
                        decoded += char::from_u32(cp)
                            .ok_or(ValueError::MalformedJson)?
                            .len_utf8();
                    }
                    _ => {
                        self.p += 1;
                        decoded += 1;
                    }
                }
            } else {
                self.p += 1;
                decoded += 1;
            }
        }
        let end = self.p;
        self.p += 1;
        Ok((start, end, decoded))
    }
    fn hex4(&mut self) -> u32 {
        let mut n = 0;
        for _ in 0..4 {
            let c = self.b[self.p];
            self.p += 1;
            n = n * 16
                + match c {
                    b'0'..=b'9' => (c - b'0') as u32,
                    b'a'..=b'f' => (c - b'a' + 10) as u32,
                    _ => (c - b'A' + 10) as u32,
                };
        }
        n
    }
    fn string(&mut self) -> Result<String, ValueError> {
        let (start, end, n) = self.span()?;
        let after = self.p;
        self.p = start;
        let mut out = String::with_capacity(n);
        while self.p < end {
            let begin = self.p;
            while self.p < end && self.b[self.p] != b'\\' {
                self.p += 1;
            }
            out.push_str(
                std::str::from_utf8(&self.b[begin..self.p])
                    .map_err(|_| ValueError::MalformedJson)?,
            );
            if self.p == end {
                break;
            }
            self.p += 1;
            let c = self.b[self.p];
            self.p += 1;
            let ch = match c {
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'b' => '\x08',
                b'f' => '\x0c',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'u' => {
                    let a = self.hex4();
                    let cp = if (0xd800..=0xdbff).contains(&a) {
                        self.p += 2;
                        let z = self.hex4();
                        0x10000 + ((a - 0xd800) << 10) + (z - 0xdc00)
                    } else {
                        a
                    };
                    char::from_u32(cp).ok_or(ValueError::MalformedJson)?
                }
                _ => return Err(ValueError::MalformedJson),
            };
            out.push(ch);
        }
        self.p = after;
        debug_assert_eq!(out.len(), n);
        Ok(out)
    }
    fn tag<T: ClosedTag>(&mut self) -> Result<T, ValueError> {
        let (a, z, _) = self.span()?;
        let raw = std::str::from_utf8(&self.b[a..z]).map_err(|_| ValueError::MalformedJson)?;
        T::from_tag(raw).ok_or(ValueError::InvalidScalar)
    }
    fn field(&mut self) -> Result<(), ValueError> {
        self.ws();
        if self.b.get(self.p) == Some(&b',') {
            self.p += 1;
        }
        self.span()?;
        self.eat(b':')
    }
    fn num(&mut self) -> Result<u64, ValueError> {
        self.ws();
        let start = self.p;
        while self.b.get(self.p).is_some_and(u8::is_ascii_digit) {
            self.p += 1;
        }
        std::str::from_utf8(&self.b[start..self.p])
            .map_err(|_| ValueError::MalformedJson)?
            .parse()
            .map_err(|_| ValueError::InvalidScalar)
    }
    fn boolean(&mut self) -> Result<bool, ValueError> {
        self.ws();
        let yes = self.b[self.p] == b't';
        self.p += if yes { 4 } else { 5 };
        Ok(yes)
    }
    fn optional<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, ValueError>,
    ) -> Result<Option<T>, ValueError> {
        self.ws();
        if self.b.get(self.p..self.p + 4) == Some(b"null") {
            self.p += 4;
            Ok(None)
        } else {
            read(self).map(Some)
        }
    }
    fn time(&mut self) -> Result<UtcInstantClaim, ValueError> {
        self.eat(b'{')?;
        self.field()?;
        let unix_seconds = self.num()?;
        self.field()?;
        let nanosecond = self
            .num()?
            .try_into()
            .map_err(|_| ValueError::InvalidScalar)?;
        self.eat(b'}')?;
        Ok(UtcInstantClaim {
            unix_seconds,
            nanosecond,
        })
    }
    fn array<T>(
        &mut self,
        mut read: impl FnMut(&mut Self) -> Result<T, ValueError>,
    ) -> Result<Vec<T>, ValueError> {
        let n = self.arrays[self.array_next];
        self.array_next += 1;
        self.eat(b'[')?;
        let mut v = Vec::with_capacity(n);
        for i in 0..n {
            if i > 0 {
                self.eat(b',')?;
            }
            v.push(read(self)?);
        }
        self.eat(b']')?;
        Ok(v)
    }
    fn coverage(&mut self) -> Result<Coverage, ValueError> {
        self.eat(b'{')?;
        self.field()?;
        let owner_domain = self.tag()?;
        self.field()?;
        let state = self.tag()?;
        self.field()?;
        let package_count = self.num()?;
        self.eat(b'}')?;
        Ok(Coverage {
            owner_domain,
            state,
            package_count,
        })
    }
    fn entry(&mut self) -> Result<Entry, ValueError> {
        self.eat(b'{')?;
        self.field()?;
        let owner_domain = self.tag()?;
        self.field()?;
        let logical_slot_claim = self.string()?;
        self.field()?;
        let package_id = self.string()?;
        self.field()?;
        let package_canonical_sha256 = self.string()?;
        self.field()?;
        let package_canonical_length = self.num()?;
        self.field()?;
        let receipt_id = self.optional(Self::string)?;
        self.field()?;
        let receipt_claim_consistency = self.tag()?;
        self.eat(b'}')?;
        Ok(Entry {
            owner_domain,
            logical_slot_claim,
            package_id,
            package_canonical_sha256,
            package_canonical_length,
            receipt_id,
            receipt_claim_consistency,
        })
    }
    fn gates(&mut self) -> Result<Gates, ValueError> {
        self.eat(b'{')?;
        self.field()?;
        let owner_seal = self.tag()?;
        self.field()?;
        let remote_retention = self.tag()?;
        self.field()?;
        let signer = self.tag()?;
        self.field()?;
        let restore = self.tag()?;
        self.eat(b'}')?;
        Ok(Gates {
            owner_seal,
            remote_retention,
            signer,
            restore,
        })
    }
}
trait ClosedTag: Sized {
    fn from_tag(s: &str) -> Option<Self>;
}
macro_rules! tags {($t:ty,$($v:ident),+)=>{impl ClosedTag for $t {fn from_tag(s:&str)->Option<Self>{match s{$(stringify!($v)=>Some(Self::$v),)+_=>None}}}};}
tags!(
    OwnerDomain,
    Data,
    InvestmentDecision,
    PaperLedger,
    Attribution
);
tags!(TrustState, Unverified);
tags!(CoverageState, Incomplete);
tags!(SignatureState, Unsigned);
tags!(
    RetentionClaim,
    Compliance,
    Locked,
    Governance,
    Unlocked,
    Unknown
);
tags!(
    CoverageClaim,
    NoMaterialProvided,
    UnverifiedMaterialProvided
);
tags!(
    ReceiptConsistency,
    NoReceipt,
    ConsistentClaims,
    InconsistentClaims
);
tags!(NotObserved, NotObserved);
tags!(NotConfigured, NotConfigured);
pub(super) trait ClosedDecode: Sized {
    fn read(p: &mut Owned<'_>) -> Result<Self, ValueError>;
}
macro_rules! read_field {
    ($p:ident,$method:ident) => {{
        $p.field()?;
        $p.$method()?
    }};
    ($p:ident,optional $method:ident) => {{
        $p.field()?;
        $p.optional(Owned::$method)?
    }};
    ($p:ident,array $method:ident) => {{
        $p.field()?;
        $p.array(Owned::$method)?
    }};
}
impl ClosedDecode for DraftWire {
    fn read(p: &mut Owned<'_>) -> Result<Self, ValueError> {
        p.eat(b'{')?;
        let d = Self {
            schema: read_field!(p, string),
            schema_version: read_field!(p, num)
                .try_into()
                .map_err(|_| ValueError::InvalidScalar)?,
            trust: read_field!(p, tag),
            owner_domain: read_field!(p, tag),
            owner_schema_claim: read_field!(p, string),
            logical_slot_claim: read_field!(p, string),
            business_day_claim: read_field!(p, string),
            window_start_claim: read_field!(p, time),
            window_end_exclusive_claim: read_field!(p, time),
            claimed_record_count: read_field!(p,optional num),
            source_chain_before_claim: read_field!(p,optional string),
            source_chain_after_claim: read_field!(p,optional string),
            artifact_sha256_claim: read_field!(p,optional string),
            activation_id_claim: read_field!(p,optional string),
            body_encoding: read_field!(p, string),
            body_length: read_field!(p, num),
            body_sha256: read_field!(p, string),
            body_hex: read_field!(p, string),
        };
        p.eat(b'}')?;
        Ok(d)
    }
}
impl ClosedDecode for ReceiptWire {
    fn read(p: &mut Owned<'_>) -> Result<Self, ValueError> {
        p.eat(b'{')?;
        let d = Self {
            schema: read_field!(p, string),
            schema_version: read_field!(p, num)
                .try_into()
                .map_err(|_| ValueError::InvalidScalar)?,
            trust: read_field!(p, tag),
            package_id: read_field!(p, string),
            storage_authority_claim: read_field!(p, string),
            container_claim: read_field!(p, string),
            object_key_claim: read_field!(p, string),
            version_id_claim: read_field!(p,optional string),
            retention_mode_claim: read_field!(p, tag),
            clock_evidence: read_field!(p, tag),
            confirmation_upper_bound_claim: read_field!(p,optional time),
            retain_until_claim: read_field!(p,optional time),
            head_content_length_claim: read_field!(p,optional num),
            get_content_length_claim: read_field!(p,optional num),
            get_sha256_claim: read_field!(p,optional string),
            readback_complete_claim: read_field!(p, boolean),
            request_id_claims: read_field!(p,array string),
        };
        p.eat(b'}')?;
        Ok(d)
    }
}
impl ClosedDecode for RootWire {
    fn read(p: &mut Owned<'_>) -> Result<Self, ValueError> {
        p.eat(b'{')?;
        let d = Self {
            schema: read_field!(p, string),
            schema_version: read_field!(p, num)
                .try_into()
                .map_err(|_| ValueError::InvalidScalar)?,
            trust: read_field!(p, tag),
            coverage_state: read_field!(p, tag),
            signature_state: read_field!(p, tag),
            authority_gates: read_field!(p, gates),
            business_day_claim: read_field!(p, string),
            revision: read_field!(p, num),
            previous_day_root_id_claim: read_field!(p,optional string),
            previous_revision_root_id_claim: read_field!(p,optional string),
            coverage: read_field!(p,array coverage),
            entries: read_field!(p,array entry),
        };
        p.eat(b'}')?;
        Ok(d)
    }
}
pub(super) fn decode<T: ClosedDecode>(
    b: &[u8],
    s: Shape,
    limit: usize,
    w: &mut Work,
) -> Result<T, ValueError> {
    let r = preflight(b, s, limit, w)?;
    w.own(r.owned)?;
    w.scan(b.len().checked_mul(2).ok_or(ValueError::InputLimit)?)?;
    #[cfg(test)]
    OWNED_HITS.with(|x| x.set(x.get() + 1));
    T::read(&mut Owned {
        b,
        p: 0,
        arrays: r.arrays,
        array_next: 0,
    })
}
// All callers serialize closed DTOs whose maximum encoded size is bounded by
// their schema limit. Reserve that entire pass before invoking serde's writer.
// Writers discard after an error, never allocate an io/serde error payload.
struct Counter {
    n: usize,
    limit: usize,
    failed: bool,
}
impl Write for Counter {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if !self.failed {
            match self.n.checked_add(b.len()) {
                Some(n) if n <= self.limit => self.n = n,
                _ => self.failed = true,
            }
        }
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Output {
    b: Vec<u8>,
    limit: usize,
    failed: bool,
}
impl Write for Output {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if !self.failed {
            if b.len() > self.limit - self.b.len() {
                self.failed = true
            } else {
                self.b.extend_from_slice(b)
            }
        }
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode<T: Serialize>(
    v: &T,
    limit: usize,
    w: &mut Work,
) -> Result<Vec<u8>, ValueError> {
    w.scan(limit)?;
    let mut c = Counter {
        n: 0,
        limit,
        failed: false,
    };
    serde_json::to_writer(&mut c, v).map_err(|_| ValueError::InvalidScalar)?;
    if c.failed {
        return Err(ValueError::InputLimit);
    }
    let n = c.n;
    w.own(n)?;
    w.scan(limit)?;
    let mut out = Output {
        b: Vec::with_capacity(n),
        limit: n,
        failed: false,
    };
    serde_json::to_writer(&mut out, v).map_err(|_| ValueError::InvalidScalar)?;
    if out.failed {
        return Err(ValueError::InputLimit);
    }
    Ok(out.b)
}
struct Compare<'a> {
    b: &'a [u8],
    p: usize,
    failed: bool,
}
impl Write for Compare<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if !self.failed {
            if self.b.get(self.p..self.p + b.len()) != Some(b) {
                self.failed = true
            } else {
                self.p += b.len()
            }
        }
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn canonical<T: Serialize>(v: &T, b: &[u8], w: &mut Work) -> Result<(), ValueError> {
    // DRAFT_LIMIT covers each of the three closed schemas, including escaping.
    w.scan(DRAFT_LIMIT)?;
    let mut c = Compare {
        b,
        p: 0,
        failed: false,
    };
    serde_json::to_writer(&mut c, v).map_err(|_| ValueError::InvalidScalar)?;
    if c.failed || c.p != b.len() {
        return Err(ValueError::NonCanonical);
    }
    Ok(())
}
// Only the fixed shared parser calls this after decoding and validating this
// exact input into DraftWire. Every mandatory field (including both Time
// objects) is present, with no skipped/flattened/custom Serialize field. Compact
// JSON punctuation, decimal integers and necessary string escapes cannot be
// longer than their representations in that same preflighted JSON input. Thus
// the entire serializer pass is covered by b.len(), not a caller credit. Keep
// the generic canonical() and its schema-wide reservation unchanged.
pub(super) fn canonical_preflighted_draft(
    d: &super::DraftWire,
    b: &[u8],
    w: &mut Work,
) -> Result<(), ValueError> {
    if b.is_empty() || b.len() > DRAFT_LIMIT {
        return Err(ValueError::InputLimit);
    }
    // Failure stops before the serializer or any canonical comparison starts.
    // Work remains cumulative, including an attempted over-limit reservation.
    w.scan(b.len())?;
    let mut c = Compare { b, p: 0, failed: false };
    serde_json::to_writer(&mut c, d).map_err(|_| ValueError::InvalidScalar)?;
    if c.failed || c.p != b.len() {
        return Err(ValueError::NonCanonical);
    }
    Ok(())
}
pub(super) fn copy(b: &[u8], w: &mut Work) -> Result<Vec<u8>, ValueError> {
    w.own(b.len())?;
    w.scan(b.len())?;
    Ok(b.to_vec())
}
pub(super) fn text(s: &str, w: &mut Work) -> Result<String, ValueError> {
    w.own(s.len() + std::mem::size_of::<String>())?;
    w.scan(s.len())?;
    Ok(s.to_owned())
}
pub(super) fn digest(b: &[u8], w: &mut Work) -> Result<String, ValueError> {
    use sha2::{Digest, Sha256};
    w.scan(b.len())?;
    w.own(64)?;
    Ok(hex::encode(Sha256::digest(b)))
}
pub(super) fn id(
    prefix: &str,
    domain: &[u8],
    b: &[u8],
    w: &mut Work,
) -> Result<String, ValueError> {
    use sha2::{Digest, Sha256};
    w.scan(b.len() + domain.len())?;
    w.own(prefix.len() + 64)?;
    let mut h = Sha256::new();
    h.update(domain);
    h.update(b);
    let mut out = String::with_capacity(prefix.len() + 64);
    out.push_str(prefix);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in h.finalize() {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 15) as usize] as char);
    }
    Ok(out)
}
