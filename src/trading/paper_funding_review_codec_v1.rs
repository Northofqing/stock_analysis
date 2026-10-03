//! Closed borrowed preflight, exact owned proposal/record decoder and bounded writer.
use super::{Binding, Error, FundingMismatchV1, Outcome, Proposal, Record};
use crate::trading::paper_book_v2_budget_v1::{
    BudgetRecord, CashPartitions, InitialLotAllocation, LotDisposition, ProfitPolicy,
};
use crate::trading::paper_ledger::{Lot, Projection};
use chrono::NaiveDate;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Write;
pub(super) const MIB: usize = 1024 * 1024;
pub(super) const PROPOSAL_LIMIT: usize = 128 * 1024;
pub(super) const GENESIS_LIMIT: usize = 512 * 1024;
pub(super) const REVIEW_LIMIT: usize = 768 * 1024;
pub(super) struct Work {
    owned: usize,
    scan: usize,
    nodes: usize,
    failed: bool,
}
impl Work {
    pub(super) fn new() -> Self {
        Self {
            owned: 0,
            scan: 0,
            nodes: 0,
            failed: false,
        }
    }
    fn debit(&mut self, n: usize, kind: u8) -> Result<(), Error> {
        if self.failed {
            return Err(Error::WorkBudget);
        }
        let (current, limit, error) = match kind {
            0 => (self.owned, 8 * MIB, Error::OwnedBudget),
            1 => (self.scan, 32 * MIB, Error::WorkBudget),
            _ => (self.nodes, 16384, Error::Nodes),
        };
        let Some(next) = current.checked_add(n).filter(|v| *v <= limit) else {
            self.failed = true;
            return Err(error);
        };
        match kind {
            0 => self.owned = next,
            1 => self.scan = next,
            _ => self.nodes = next,
        };
        Ok(())
    }
    pub(super) fn finish<T>(&mut self, result: Result<T, Error>) -> Result<T, Error> {
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    pub(super) fn own(&mut self, n: usize) -> Result<(), Error> {
        self.debit(n, 0)
    }
    pub(super) fn scan(&mut self, n: usize) -> Result<(), Error> {
        self.debit(n, 1)
    }
}
#[derive(Clone, Copy)]
enum Shape {
    Proposal,
    Budget,
    Allocation,
    Binding,
    Record,
    Cash,
    Projection,
    Lot,
    Mark,
}
#[derive(Clone, Copy)]
enum Ty {
    Text(usize),
    Literal(&'static str),
    Tags(&'static [&'static str]),
    One,
    Hash,
    Int,
    Uint,
    OptText,
    OptInt,
    Obj(Shape),
    Array(Shape, usize),
    Map(Shape, usize),
    Closes,
    Outcome,
    OptCash,
}
const PROPOSAL: &[(&str, Ty)] = &[
    ("schema", Ty::Literal("paper-funding-proposal-v1")),
    ("version", Ty::One),
    ("account_id", Ty::Text(256)),
    ("epoch_id", Ty::Text(256)),
    ("cutover_id", Ty::Text(256)),
    ("genesis_manifest_hash", Ty::Hash),
    ("genesis_event_hash", Ty::Hash),
    ("genesis_projection_hash", Ty::Hash),
    ("fee_policy_instance_id", Ty::Text(256)),
    ("budget", Ty::Obj(Shape::Budget)),
];
const BUDGET: &[(&str, Ty)] = &[
    ("version", Ty::Literal("paper-parent-budget/v1")),
    ("family_id", Ty::Text(256)),
    ("effective_from", Ty::Text(10)),
    ("effective_through", Ty::Text(10)),
    ("authorized_budget_micro_cny", Ty::Int),
    ("initial_strategy_cash_micro_cny", Ty::Int),
    ("concentration_bps", Ty::Uint),
    ("chain_exposure_bps", Ty::Uint),
    ("cash_floor_bps", Ty::Uint),
    ("max_order_exposure_micro_cny", Ty::Int),
    ("original_seed_reference", Ty::Text(256)),
    ("review_reference", Ty::Text(256)),
    (
        "profit_policy",
        Ty::Literal("ReinvestWithinFixedAuthorizedBudget"),
    ),
    ("initial_lots", Ty::Array(Shape::Allocation, 256)),
];
const ALLOCATION: &[(&str, Ty)] = &[
    ("lot_id", Ty::Text(256)),
    ("original_quantity", Ty::Uint),
    (
        "disposition",
        Ty::Tags(&["AllocatedToStrategy", "UnassignedReadOnly"]),
    ),
    ("chain_id", Ty::OptText),
];
const BINDING: &[(&str, Ty)] = &[
    ("account_id", Ty::Text(256)),
    ("epoch_id", Ty::Text(256)),
    ("cutover_id", Ty::Text(256)),
    ("genesis_manifest_hash", Ty::Hash),
    ("v1_epoch_id", Ty::Text(256)),
    ("v1_manifest_hash", Ty::Hash),
    ("v1_head_version", Ty::Int),
    ("v1_head_hash", Ty::Hash),
    ("v1_projection_hash", Ty::Hash),
    ("genesis_version", Ty::Int),
    ("genesis_event_hash", Ty::Hash),
    ("genesis_projection_hash", Ty::Hash),
    ("fee_policy_instance_id", Ty::Text(256)),
];
const RECORD: &[(&str, Ty)] = &[
    ("schema", Ty::Literal("paper-funding-review-v1")),
    ("version", Ty::One),
    ("proposal_id", Ty::Text(256)),
    ("proposal", Ty::Obj(Shape::Proposal)),
    ("actual_binding", Ty::Obj(Shape::Binding)),
    ("outcome", Ty::Outcome),
    ("initial_cash", Ty::OptCash),
    ("authority_state", Ty::Literal("HistoricalObservationOnly")),
    ("approval_state", Ty::Literal("NotIssued")),
];
const CASH: &[(&str, Ty)] = &[
    ("account_cash", Ty::Int),
    ("strategy_cash", Ty::Int),
    ("unassigned_cash", Ty::Int),
];
const PROJECTION: &[(&str, Ty)] = &[
    ("cash", Ty::Int),
    ("lots", Ty::Array(Shape::Lot, 256)),
    ("marks", Ty::Map(Shape::Mark, 256)),
    ("fees", Ty::Int),
    ("realized_pnl", Ty::Int),
    ("seed_equity", Ty::Int),
    ("as_of", Ty::Text(64)),
    ("closes", Ty::Closes),
];
const LOT: &[(&str, Ty)] = &[
    ("lot_id", Ty::Text(256)),
    ("code", Ty::Text(256)),
    ("name", Ty::Text(256)),
    ("quantity", Ty::Uint),
    ("basis_price", Ty::Int),
    ("buy_fee_remaining", Ty::Int),
    ("acquired_on", Ty::Text(10)),
    ("sellable_from", Ty::Text(10)),
    ("reported_cost", Ty::OptInt),
];
const MARK: &[(&str, Ty)] = &[
    ("code", Ty::Text(256)),
    ("price", Ty::Int),
    ("observed_at", Ty::Text(64)),
    ("source", Ty::Text(256)),
];
fn fields(s: Shape) -> &'static [(&'static str, Ty)] {
    match s {
        Shape::Proposal => PROPOSAL,
        Shape::Budget => BUDGET,
        Shape::Allocation => ALLOCATION,
        Shape::Binding => BINDING,
        Shape::Record => RECORD,
        Shape::Cash => CASH,
        Shape::Projection => PROJECTION,
        Shape::Lot => LOT,
        Shape::Mark => MARK,
    }
}
#[derive(Default)]
struct Stats {
    strings: usize,
    allocations: usize,
    lots: usize,
    marks: usize,
    closes: usize,
}
struct Scan<'a, 'w> {
    b: &'a [u8],
    p: usize,
    w: &'w mut Work,
    stats: Stats,
}
impl Scan<'_, '_> {
    fn eat(&mut self, c: u8) -> Result<(), Error> {
        if self.b.get(self.p) != Some(&c) {
            return Err(Error::Schema);
        }
        self.p += 1;
        Ok(())
    }
    fn word(&mut self, s: &[u8]) -> bool {
        if self.b.get(self.p..self.p + s.len()) == Some(s) {
            self.p += s.len();
            true
        } else {
            false
        }
    }
    fn hex4(&mut self) -> Result<u32, Error> {
        let mut v = 0;
        for _ in 0..4 {
            let c = *self.b.get(self.p).ok_or(Error::Schema)?;
            self.p += 1;
            v = v * 16
                + match c {
                    b'0'..=b'9' => (c - b'0') as u32,
                    b'a'..=b'f' => (c - b'a' + 10) as u32,
                    b'A'..=b'F' => (c - b'A' + 10) as u32,
                    _ => return Err(Error::Schema),
                }
        }
        Ok(v)
    }
    fn string(&mut self, cap: usize, key: bool) -> Result<(usize, usize), Error> {
        self.eat(b'"')?;
        let start = self.p;
        let mut len = 0usize;
        loop {
            let c = *self.b.get(self.p).ok_or(Error::Schema)?;
            if c == b'"' {
                let end = self.p;
                self.p += 1;
                self.stats.strings = self
                    .stats
                    .strings
                    .checked_add(len)
                    .ok_or(Error::OwnedBudget)?;
                return Ok((start, end));
            }
            if c < 32 {
                return Err(Error::Schema);
            }
            self.p += 1;
            let n = if c == b'\\' {
                if key {
                    return Err(Error::NonCanonical);
                }
                let e = *self.b.get(self.p).ok_or(Error::Schema)?;
                self.p += 1;
                match e {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => 1,
                    b'u' => {
                        let a = self.hex4()?;
                        let cp = if (0xd800..=0xdbff).contains(&a) {
                            if !self.word(b"\\u") {
                                return Err(Error::Schema);
                            }
                            let z = self.hex4()?;
                            if !(0xdc00..=0xdfff).contains(&z) {
                                return Err(Error::Schema);
                            }
                            0x10000 + ((a - 0xd800) << 10) + (z - 0xdc00)
                        } else {
                            a
                        };
                        char::from_u32(cp).ok_or(Error::Schema)?.len_utf8()
                    }
                    _ => return Err(Error::Schema),
                }
            } else {
                1
            };
            len = len.checked_add(n).ok_or(Error::InputTooLarge)?;
            if len > cap {
                return Err(Error::InputTooLarge);
            }
        }
    }
    fn number(&mut self, unsigned: bool) -> Result<(), Error> {
        let start = self.p;
        if !unsigned && self.b.get(self.p) == Some(&b'-') {
            self.p += 1;
        }
        let digits = self.p;
        while self.b.get(self.p).is_some_and(u8::is_ascii_digit) {
            self.p += 1;
            if self.p - start > 20 {
                return Err(Error::Schema);
            }
        }
        if self.p == digits
            || (self.p - digits > 1 && self.b[digits] == b'0')
            || (digits > start && self.b[digits] == b'0')
        {
            return Err(Error::Schema);
        }
        let raw = std::str::from_utf8(&self.b[start..self.p]).map_err(|_| Error::Schema)?;
        if unsigned {
            raw.parse::<u32>().map_err(|_| Error::Schema)?;
        } else {
            raw.parse::<i64>().map_err(|_| Error::Schema)?;
        }
        Ok(())
    }
    fn object(&mut self, s: Shape, d: usize) -> Result<(), Error> {
        self.eat(b'{')?;
        for (i, (name, ty)) in fields(s).iter().enumerate() {
            if i > 0 {
                self.eat(b',')?;
            }
            self.w.debit(1, 2)?;
            let (a, z) = self.string(64, true)?;
            if &self.b[a..z] != name.as_bytes() {
                return Err(Error::NonCanonical);
            }
            self.eat(b':')?;
            self.value(*ty, d + 1)?;
        }
        if matches!(s, Shape::Projection) && self.b.get(self.p) == Some(&b',') {
            self.p += 1;
            self.w.debit(2, 2)?;
            let (a, z) = self.string(64, true)?;
            if &self.b[a..z] != b"economic_unavailable" {
                return Err(Error::Schema);
            }
            self.eat(b':')?;
            if !self.word(b"null") {
                return Err(Error::UnavailableGenesis);
            }
        }
        self.eat(b'}')
    }
    fn array(&mut self, s: Shape, max: usize, d: usize) -> Result<(), Error> {
        self.eat(b'[')?;
        let mut n = 0;
        if self.b.get(self.p) != Some(&b']') {
            loop {
                if n == max {
                    return Err(Error::InputTooLarge);
                }
                if n > 0 {
                    self.eat(b',')?;
                }
                self.value(Ty::Obj(s), d + 1)?;
                n += 1;
                if self.b.get(self.p) == Some(&b']') {
                    break;
                }
            }
        }
        self.eat(b']')?;
        match s {
            Shape::Allocation => self.stats.allocations = n,
            Shape::Lot => self.stats.lots = n,
            _ => {}
        }
        Ok(())
    }
    fn map(&mut self, ty: Ty, max: usize, d: usize, closes: bool) -> Result<(), Error> {
        self.eat(b'{')?;
        let mut n = 0;
        let mut prev = None;
        // Worst case fixed-length comparisons are reserved before the first key.
        self.w
            .scan(max.checked_mul(512).ok_or(Error::WorkBudget)?)?;
        if self.b.get(self.p) != Some(&b'}') {
            loop {
                if n == max {
                    return Err(Error::InputTooLarge);
                }
                if n > 0 {
                    self.eat(b',')?;
                }
                self.w.debit(1, 2)?;
                let (a, z) = self.string(if closes { 10 } else { 256 }, true)?;
                if prev.is_some_and(|(x, y)| &self.b[x..y] >= &self.b[a..z]) {
                    return Err(Error::NonCanonical);
                }
                prev = Some((a, z));
                self.eat(b':')?;
                self.value(ty, d + 1)?;
                n += 1;
                if self.b.get(self.p) == Some(&b'}') {
                    break;
                }
            }
        }
        self.eat(b'}')?;
        if closes {
            self.stats.closes = n
        } else {
            self.stats.marks = n
        }
        Ok(())
    }
    fn value(&mut self, t: Ty, d: usize) -> Result<(), Error> {
        if d > 8 {
            return Err(Error::Depth);
        }
        self.w.debit(1, 2)?;
        match t {
            Ty::Text(n) => {
                self.string(n, false)?;
                Ok(())
            }
            Ty::Literal(lit) => {
                let (a, z) = self.string(lit.len(), true)?;
                if &self.b[a..z] != lit.as_bytes() {
                    return Err(Error::Schema);
                }
                Ok(())
            }
            Ty::Tags(tags) => {
                let (a, z) = self.string(256, true)?;
                if !tags.iter().any(|t| t.as_bytes() == &self.b[a..z]) {
                    return Err(Error::Schema);
                }
                Ok(())
            }
            Ty::One => {
                if self.word(b"1") {
                    Ok(())
                } else {
                    Err(Error::Schema)
                }
            }
            Ty::Hash => {
                let (a, z) = self.string(64, true)?;
                if z - a != 64
                    || !self.b[a..z]
                        .iter()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
                {
                    return Err(Error::Schema);
                }
                Ok(())
            }
            Ty::Int => self.number(false),
            Ty::Uint => self.number(true),
            Ty::OptText => {
                if self.word(b"null") {
                    Ok(())
                } else {
                    self.value(Ty::Text(256), d)
                }
            }
            Ty::OptInt => {
                if self.word(b"null") {
                    Ok(())
                } else {
                    self.number(false)
                }
            }
            Ty::Obj(s) => self.object(s, d),
            Ty::Array(s, n) => self.array(s, n, d),
            Ty::Map(s, n) => self.map(Ty::Obj(s), n, d, false),
            Ty::Closes => self.map(Ty::Int, 64, d, true),
            Ty::OptCash => {
                if self.word(b"null") {
                    Ok(())
                } else {
                    self.value(Ty::Obj(Shape::Cash), d)
                }
            }
            Ty::Outcome => {
                if self.b.get(self.p) == Some(&b'"') {
                    let (a, z) = self.string(32, true)?;
                    if &self.b[a..z] != b"ConsistentProposal" {
                        return Err(Error::Schema);
                    }
                } else {
                    self.eat(b'{')?;
                    self.w.debit(2, 2)?;
                    let (a, z) = self.string(32, true)?;
                    if &self.b[a..z] != b"InconsistentProposal" {
                        return Err(Error::Schema);
                    }
                    self.eat(b':')?;
                    let (a, z) = self.string(32, true)?;
                    if ![
                        "Account",
                        "Epoch",
                        "Cutover",
                        "GenesisManifest",
                        "GenesisEvent",
                        "GenesisProjection",
                        "FeePolicy",
                        "SeedReference",
                        "AllocationAgainstGenesis",
                    ]
                    .iter()
                    .any(|v| v.as_bytes() == &self.b[a..z])
                    {
                        return Err(Error::Schema);
                    }
                    self.eat(b'}')?;
                }
                Ok(())
            }
        }
    }
}
fn preflight(b: &[u8], shape: Shape, limit: usize, w: &mut Work) -> Result<Stats, Error> {
    let result = (|| {
        if b.is_empty() || b.len() > limit {
            return Err(Error::InputTooLarge);
        }
        w.scan(b.len().checked_mul(2).ok_or(Error::WorkBudget)?)?;
        std::str::from_utf8(b).map_err(|_| Error::Schema)?;
        let mut s = Scan {
            b,
            p: 0,
            w,
            stats: Stats::default(),
        };
        s.value(Ty::Obj(shape), 0)?;
        if s.p != b.len() {
            return Err(Error::NonCanonical);
        }
        Ok(s.stats)
    })();
    if result.is_err() {
        w.failed = true;
    }
    result
}

macro_rules! field {
    ($p:ident,$m:ident) => {{
        $p.field()?;
        $p.$m()?
    }};
}
struct Owned<'a> {
    b: &'a [u8],
    p: usize,
    allocations: usize,
}
impl Owned<'_> {
    fn ws(&mut self) {
        while self.b.get(self.p).is_some_and(u8::is_ascii_whitespace) {
            self.p += 1;
        }
    }
    fn eat(&mut self, b: u8) -> Result<(), Error> {
        self.ws();
        if self.b.get(self.p) != Some(&b) {
            return Err(Error::Schema);
        }
        self.p += 1;
        Ok(())
    }
    fn span(&mut self) -> Result<(usize, usize, usize), Error> {
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
                        decoded += char::from_u32(cp).ok_or(Error::Schema)?.len_utf8();
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
    fn string(&mut self) -> Result<String, Error> {
        let (start, end, n) = self.span()?;
        let after = self.p;
        self.p = start;
        let mut out = String::with_capacity(n);
        while self.p < end {
            let begin = self.p;
            while self.p < end && self.b[self.p] != b'\\' {
                self.p += 1;
            }
            out.push_str(std::str::from_utf8(&self.b[begin..self.p]).map_err(|_| Error::Schema)?);
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
                    char::from_u32(cp).ok_or(Error::Schema)?
                }
                _ => return Err(Error::Schema),
            };
            out.push(ch);
        }
        self.p = after;
        debug_assert_eq!(out.len(), n);
        Ok(out)
    }
    fn field(&mut self) -> Result<(), Error> {
        self.ws();
        if self.b.get(self.p) == Some(&b',') {
            self.p += 1;
        }
        self.span()?;
        self.eat(b':')
    }
    fn num(&mut self) -> Result<u64, Error> {
        self.ws();
        let start = self.p;
        while self.b.get(self.p).is_some_and(u8::is_ascii_digit) {
            self.p += 1;
        }
        std::str::from_utf8(&self.b[start..self.p])
            .map_err(|_| Error::Schema)?
            .parse()
            .map_err(|_| Error::Schema)
    }
    fn boolean(&mut self) -> Result<bool, Error> {
        self.ws();
        let yes = self.b[self.p] == b't';
        self.p += if yes { 4 } else { 5 };
        Ok(yes)
    }
    fn int(&mut self) -> Result<i64, Error> {
        let start = self.p;
        if self.b[self.p] == b'-' {
            self.p += 1;
        }
        while self.b.get(self.p).is_some_and(u8::is_ascii_digit) {
            self.p += 1;
        }
        std::str::from_utf8(&self.b[start..self.p])
            .map_err(|_| Error::Schema)?
            .parse()
            .map_err(|_| Error::Schema)
    }
    fn uint(&mut self) -> Result<u32, Error> {
        u32::try_from(self.num()?).map_err(|_| Error::Schema)
    }
    fn date(&mut self) -> Result<NaiveDate, Error> {
        let s = self.string()?;
        if s.len() != 10
            || s.as_bytes().get(4) != Some(&b'-')
            || s.as_bytes().get(7) != Some(&b'-')
            || !s
                .bytes()
                .enumerate()
                .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
            || &s[..4] == "0000"
        {
            return Err(Error::Schema);
        }
        NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(|_| Error::Schema)
    }
    fn opt_string(&mut self) -> Result<Option<String>, Error> {
        if self.b.get(self.p..self.p + 4) == Some(b"null") {
            self.p += 4;
            Ok(None)
        } else {
            self.string().map(Some)
        }
    }
    fn allocation(&mut self) -> Result<InitialLotAllocation, Error> {
        self.eat(b'{')?;
        let a = InitialLotAllocation {
            lot_id: field!(self, string),
            original_quantity: field!(self, uint),
            disposition: {
                self.field()?;
                match self.string()?.as_str() {
                    "AllocatedToStrategy" => LotDisposition::AllocatedToStrategy,
                    "UnassignedReadOnly" => LotDisposition::UnassignedReadOnly,
                    _ => return Err(Error::Schema),
                }
            },
            chain_id: field!(self, opt_string),
        };
        self.eat(b'}')?;
        Ok(a)
    }
    fn allocations(&mut self) -> Result<Vec<InitialLotAllocation>, Error> {
        self.eat(b'[')?;
        let mut out = Vec::with_capacity(self.allocations);
        for i in 0..self.allocations {
            if i > 0 {
                self.eat(b',')?;
            }
            out.push(self.allocation()?);
        }
        self.eat(b']')?;
        Ok(out)
    }
    fn budget(&mut self) -> Result<BudgetRecord, Error> {
        self.eat(b'{')?;
        let b = BudgetRecord {
            version: field!(self, string),
            family_id: field!(self, string),
            effective_from: field!(self, date),
            effective_through: field!(self, date),
            authorized_budget_micro_cny: field!(self, int),
            initial_strategy_cash_micro_cny: field!(self, int),
            concentration_bps: field!(self, uint),
            chain_exposure_bps: field!(self, uint),
            cash_floor_bps: field!(self, uint),
            max_order_exposure_micro_cny: field!(self, int),
            original_seed_reference: field!(self, string),
            review_reference: field!(self, string),
            profit_policy: {
                self.field()?;
                if self.string()? != "ReinvestWithinFixedAuthorizedBudget" {
                    return Err(Error::Schema);
                }
                ProfitPolicy::ReinvestWithinFixedAuthorizedBudget
            },
            initial_lots: field!(self, allocations),
        };
        self.eat(b'}')?;
        Ok(b)
    }
    fn proposal(&mut self) -> Result<Proposal, Error> {
        self.eat(b'{')?;
        let p = Proposal {
            schema: field!(self, string),
            version: field!(self, uint),
            account_id: field!(self, string),
            epoch_id: field!(self, string),
            cutover_id: field!(self, string),
            genesis_manifest_hash: field!(self, string),
            genesis_event_hash: field!(self, string),
            genesis_projection_hash: field!(self, string),
            fee_policy_instance_id: field!(self, string),
            budget: field!(self, budget),
        };
        self.eat(b'}')?;
        Ok(p)
    }
    fn binding(&mut self) -> Result<Binding, Error> {
        self.eat(b'{')?;
        let b = Binding {
            account_id: field!(self, string),
            epoch_id: field!(self, string),
            cutover_id: field!(self, string),
            genesis_manifest_hash: field!(self, string),
            v1_epoch_id: field!(self, string),
            v1_manifest_hash: field!(self, string),
            v1_head_version: field!(self, int),
            v1_head_hash: field!(self, string),
            v1_projection_hash: field!(self, string),
            genesis_version: field!(self, int),
            genesis_event_hash: field!(self, string),
            genesis_projection_hash: field!(self, string),
            fee_policy_instance_id: field!(self, string),
        };
        self.eat(b'}')?;
        Ok(b)
    }
    fn outcome(&mut self) -> Result<Outcome, Error> {
        if self.b[self.p] == b'"' {
            if self.string()? != "ConsistentProposal" {
                return Err(Error::Schema);
            }
            return Ok(Outcome::ConsistentProposal);
        }
        self.eat(b'{')?;
        self.field()?;
        let reason = match self.string()?.as_str() {
            "Account" => FundingMismatchV1::Account,
            "Epoch" => FundingMismatchV1::Epoch,
            "Cutover" => FundingMismatchV1::Cutover,
            "GenesisManifest" => FundingMismatchV1::GenesisManifest,
            "GenesisEvent" => FundingMismatchV1::GenesisEvent,
            "GenesisProjection" => FundingMismatchV1::GenesisProjection,
            "FeePolicy" => FundingMismatchV1::FeePolicy,
            "SeedReference" => FundingMismatchV1::SeedReference,
            "AllocationAgainstGenesis" => FundingMismatchV1::AllocationAgainstGenesis,
            _ => return Err(Error::Schema),
        };
        self.eat(b'}')?;
        Ok(Outcome::InconsistentProposal(reason))
    }
    fn cash(&mut self) -> Result<Option<CashPartitions>, Error> {
        if self.b.get(self.p..self.p + 4) == Some(b"null") {
            self.p += 4;
            return Ok(None);
        }
        self.eat(b'{')?;
        let c = CashPartitions {
            account_cash: field!(self, int),
            strategy_cash: field!(self, int),
            unassigned_cash: field!(self, int),
        };
        self.eat(b'}')?;
        Ok(Some(c))
    }
    fn record(&mut self) -> Result<Record, Error> {
        self.eat(b'{')?;
        let r = Record {
            schema: field!(self, string),
            version: field!(self, uint),
            proposal_id: field!(self, string),
            proposal: field!(self, proposal),
            actual_binding: field!(self, binding),
            outcome: field!(self, outcome),
            initial_cash: field!(self, cash),
            authority_state: field!(self, string),
            approval_state: field!(self, string),
        };
        self.eat(b'}')?;
        Ok(r)
    }
}
fn manual<'a>(b: &'a [u8], shape: Shape, limit: usize, w: &mut Work) -> Result<Owned<'a>, Error> {
    let stats = preflight(b, shape, limit, w)?;
    let fixed = match shape {
        Shape::Proposal => std::mem::size_of::<Proposal>(),
        Shape::Record => std::mem::size_of::<Record>(),
        _ => return Err(Error::Schema),
    };
    w.own(
        fixed
            .checked_add(stats.strings)
            .and_then(|v| {
                v.checked_add(
                    stats
                        .allocations
                        .checked_mul(std::mem::size_of::<InitialLotAllocation>())?,
                )
            })
            .ok_or(Error::OwnedBudget)?,
    )?;
    // Span counting, exact copy/unescape and primitive parsing precede no later charge.
    w.scan(b.len().checked_mul(4).ok_or(Error::WorkBudget)?)?;
    #[cfg(test)]
    super::probe(super::Probe::Owned);
    Ok(Owned {
        b,
        p: 0,
        allocations: stats.allocations,
    })
}
pub(super) fn proposal(b: &[u8], w: &mut Work) -> Result<Proposal, Error> {
    let r = manual(b, Shape::Proposal, PROPOSAL_LIMIT, w).and_then(|mut p| p.proposal());
    if r.is_err() {
        w.failed = true;
    }
    r
}
pub(super) fn record(b: &[u8], w: &mut Work) -> Result<Record, Error> {
    let r = manual(b, Shape::Record, REVIEW_LIMIT, w).and_then(|mut p| p.record());
    if r.is_err() {
        w.failed = true;
    }
    r
}
fn projection_inner(b: &[u8], w: &mut Work) -> Result<Projection, Error> {
    let stats = preflight(b, Shape::Projection, GENESIS_LIMIT, w)?;
    let cap = if stats.lots == 0 {
        0
    } else {
        stats
            .lots
            .checked_next_power_of_two()
            .ok_or(Error::OwnedBudget)?
            .max(4)
    };
    // Finite logical reservations, not a std BTreeMap layout or allocator/RSS pin.
    if std::mem::size_of::<Lot>() > 256
        || std::mem::size_of::<crate::trading::paper_ledger::Mark>() > 128
    {
        return Err(Error::OwnedBudget);
    }
    let n = b
        .len()
        .checked_mul(8)
        .and_then(|v| v.checked_add(cap.checked_mul(std::mem::size_of::<Lot>())?))
        .and_then(|v| v.checked_add(stats.marks.checked_mul(2048)?))
        .and_then(|v| v.checked_add(stats.closes.checked_mul(512)?))
        .and_then(|v| v.checked_add(std::mem::size_of::<Projection>()))
        .ok_or(Error::OwnedBudget)?;
    w.own(n)?;
    w.scan(b.len().checked_mul(2).ok_or(Error::WorkBudget)?)?;
    #[cfg(test)]
    super::probe(super::Probe::Owned);
    let p: Projection = serde_json::from_slice(b).map_err(|_| Error::Schema)?;
    canonical(&p, b, GENESIS_LIMIT, w)?;
    Ok(p)
}
struct Count {
    n: usize,
    max: usize,
}
impl Write for Count {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.n = self
            .n
            .checked_add(b.len())
            .filter(|v| *v <= self.max)
            .ok_or_else(|| std::io::Error::other("bounded funding encoding"))?;
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct Exact {
    b: Vec<u8>,
    cap: usize,
}
impl Write for Exact {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self
            .b
            .len()
            .checked_add(b.len())
            .is_none_or(|n| n > self.cap)
        {
            return Err(std::io::Error::other("funding count differs"));
        }
        self.b.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode_inner<T: Serialize>(v: &T, limit: usize, w: &mut Work) -> Result<Vec<u8>, Error> {
    w.scan(limit)?;
    let mut count = Count { n: 0, max: limit };
    serde_json::to_writer(&mut count, v).map_err(|_| Error::InternalEncoding)?;
    w.own(count.n)?;
    w.scan(limit)?;
    let mut out = Exact {
        b: Vec::with_capacity(count.n),
        cap: count.n,
    };
    serde_json::to_writer(&mut out, v).map_err(|_| Error::InternalEncoding)?;
    if out.b.len() != count.n {
        return Err(Error::InternalEncoding);
    }
    Ok(out.b)
}
fn canonical_inner<T: Serialize>(v: &T, b: &[u8], limit: usize, w: &mut Work) -> Result<(), Error> {
    let encoded = encode(v, limit, w)?;
    w.scan(
        encoded
            .len()
            .checked_add(b.len())
            .ok_or(Error::WorkBudget)?,
    )?;
    if encoded != b {
        return Err(Error::NonCanonical);
    }
    Ok(())
}
fn id_inner(prefix: &str, domain: &[u8], b: &[u8], w: &mut Work) -> Result<String, Error> {
    w.scan(domain.len().checked_add(b.len()).ok_or(Error::WorkBudget)?)?;
    w.own(prefix.len() + 64)?;
    let mut h = Sha256::new();
    h.update(domain);
    h.update(b);
    let digest = h.finalize();
    let mut out = String::with_capacity(prefix.len() + 64);
    out.push_str(prefix);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 15) as usize] as char);
    }
    Ok(out)
}

pub(super) fn projection(b: &[u8], w: &mut Work) -> Result<Projection, Error> {
    let r = projection_inner(b, w);
    if r.is_err() {
        w.failed = true;
    }
    r
}

pub(super) fn encode<T: Serialize>(v: &T, limit: usize, w: &mut Work) -> Result<Vec<u8>, Error> {
    let r = encode_inner(v, limit, w);
    if r.is_err() {
        w.failed = true;
    }
    r
}

pub(super) fn canonical<T: Serialize>(
    v: &T,
    b: &[u8],
    limit: usize,
    w: &mut Work,
) -> Result<(), Error> {
    let r = canonical_inner(v, b, limit, w);
    if r.is_err() {
        w.failed = true;
    }
    r
}

pub(super) fn id(prefix: &str, domain: &[u8], b: &[u8], w: &mut Work) -> Result<String, Error> {
    let result = id_inner(prefix, domain, b, w);
    w.finish(result)
}
