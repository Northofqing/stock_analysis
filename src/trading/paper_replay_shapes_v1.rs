//! Borrowed finite DTO grammar/representation plan. No Content/Value or pin issuer.
#![allow(dead_code)]
use crate::database::global_schema_v1::replay_work::{
    CodecMechanics, ReplayCodecFailureKind as K, ReplayTerminalFailure,
};
use serde::de::{Deserializer as _, Visitor};
use std::fmt;

#[derive(Clone, Copy, Debug)]
pub(crate) enum RootKind {
    Seed,
    Binding,
    Projection,
    V1Fact,
    Cutover,
    Genesis,
    ExecutionManifest,
    ExecutionFact,
    ExecutionProjection,
}
impl RootKind {
    pub(crate) fn q(self) -> u64 {
        match self {
            Self::Seed | Self::Projection => 3,
            Self::Binding | Self::Cutover | Self::Genesis => 1,
            Self::V1Fact | Self::ExecutionManifest => 4,
            Self::ExecutionFact => 8,
            Self::ExecutionProjection => 5,
        }
    }
    fn execution(self) -> bool {
        matches!(
            self,
            Self::ExecutionManifest | Self::ExecutionFact | Self::ExecutionProjection
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Direct,
    Buffered,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Scalar {
    String,
    Bool,
    Integer,
    Float,
    Date,
    DateTime,
}
#[derive(Clone, Copy)]
pub(crate) struct Field {
    pub(crate) name: &'static str,
    pub(crate) shape: &'static Shape,
    pub(crate) optional: bool,
    pub(crate) positional_default: bool,
}
#[derive(Clone, Copy)]
pub(crate) enum Body {
    Unit,
    Value(&'static Shape),
    Record(&'static [Field], bool),
}
#[derive(Clone, Copy)]
pub(crate) struct Variant {
    pub(crate) name: &'static str,
    pub(crate) body: Body,
}
#[derive(Clone, Copy)]
pub(crate) enum Shape {
    Scalar(Scalar),
    Option(&'static Shape),
    Seq(&'static Shape),
    Map(&'static Shape, &'static Shape),
    Tuple(&'static Shape, &'static Shape),
    Record(&'static [Field], bool, bool),
    External(&'static [Variant]),
    Adjacent(&'static str, &'static str, &'static [Variant]),
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    pub(crate) start: usize,
    pub(crate) end: usize,
}
#[derive(Clone, Copy, Debug)]
struct Fault {
    kind: K,
    at: usize,
}
type Checked<T> = Result<T, Fault>;
fn fail<T>(kind: K, at: usize) -> Checked<T> {
    Err(Fault { kind, at })
}
fn ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b" \r\n\t".contains(&b[i]) {
        i += 1;
    }
    i
}
fn hex(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u32),
        b'a'..=b'f' => Some((b - b'a' + 10) as u32),
        b'A'..=b'F' => Some((b - b'A' + 10) as u32),
        _ => None,
    }
}
fn quad(b: &[u8], i: usize) -> Checked<u32> {
    let mut n = 0;
    for j in i..i.saturating_add(4) {
        n = n * 16
            + hex(*b.get(j).ok_or(Fault {
                kind: K::MalformedJson,
                at: j,
            })?)
            .ok_or(Fault {
                kind: K::MalformedJson,
                at: j,
            })?;
    }
    Ok(n)
}
fn string_end(b: &[u8], start: usize) -> Checked<usize> {
    let mut i = start + 1;
    while let Some(&c) = b.get(i) {
        match c {
            b'"' => return Ok(i + 1),
            0..=31 => return fail(K::MalformedJson, i),
            b'\\' => {
                i += 1;
                match b.get(i) {
                    Some(b'u') => {
                        quad(b, i + 1)?;
                        i += 4;
                    }
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {}
                    _ => return fail(K::MalformedJson, i),
                }
            }
            _ => {}
        }
        i += 1;
    }
    fail(K::MalformedJson, i)
}
fn number_end(b: &[u8], start: usize) -> Checked<usize> {
    let mut i = start;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            i += 1;
            while matches!(b.get(i), Some(b'0'..=b'9')) {
                i += 1;
            }
        }
        _ => return fail(K::MalformedJson, i),
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        let begin = i;
        while matches!(b.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        if i == begin {
            return fail(K::MalformedJson, i);
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let begin = i;
        while matches!(b.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        if i == begin {
            return fail(K::MalformedJson, i);
        }
    }
    Ok(i)
}
#[derive(Clone, Copy)]
pub(crate) struct ScanFrame {
    kind: u8,
    state: u8,
}
fn lexical_frames(b: &[u8]) -> usize {
    let (mut depth, mut peak, mut quote, mut escaped) = (0usize, 0usize, false, false);
    for &c in b {
        if quote {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                quote = false;
            }
        } else {
            match c {
                b'"' => quote = true,
                b'[' | b'{' => {
                    depth += 1;
                    peak = peak.max(depth);
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    peak.saturating_add(1)
}
fn grammar(b: &[u8], frames: &mut Vec<ScanFrame>, capacity: usize) -> Checked<()> {
    frames.push(ScanFrame {
        kind: b'R',
        state: 0,
    });
    let mut i = 0;
    while !frames.is_empty() {
        i = ws(b, i);
        let f = *frames.last().unwrap();
        let c = b.get(i).copied();
        let mut value = false;
        match (f.kind, f.state) {
            (b'R', 0) => {
                frames.last_mut().unwrap().state = 1;
                value = true;
            }
            (b'R', _) => {
                if i != b.len() {
                    return fail(K::MalformedJson, i);
                }
                frames.pop();
            }
            (b'[', 0) if c == Some(b']') => {
                frames.pop();
                i += 1;
            }
            (b'[', 0 | 2) => {
                frames.last_mut().unwrap().state = 1;
                value = true;
            }
            (b'[', _) => match c {
                Some(b',') => {
                    frames.last_mut().unwrap().state = 2;
                    i += 1;
                }
                Some(b']') => {
                    frames.pop();
                    i += 1;
                }
                _ => return fail(K::MalformedJson, i),
            },
            (b'{', 0) if c == Some(b'}') => {
                frames.pop();
                i += 1;
            }
            (b'{', 0 | 4) => {
                if c != Some(b'"') {
                    return fail(K::MalformedJson, i);
                }
                i = string_end(b, i)?;
                frames.last_mut().unwrap().state = 1;
            }
            (b'{', 1) => {
                if c != Some(b':') {
                    return fail(K::MalformedJson, i);
                }
                i += 1;
                frames.last_mut().unwrap().state = 2;
            }
            (b'{', 2) => {
                frames.last_mut().unwrap().state = 3;
                value = true;
            }
            (b'{', _) => match c {
                Some(b',') => {
                    frames.last_mut().unwrap().state = 4;
                    i += 1;
                }
                Some(b'}') => {
                    frames.pop();
                    i += 1;
                }
                _ => return fail(K::MalformedJson, i),
            },
            _ => return fail(K::PlanMismatch, i),
        }
        if value {
            match c {
                Some(b'[' | b'{') => {
                    if frames.len() == capacity {
                        return fail(K::PlanMismatch, i);
                    }
                    frames.push(ScanFrame {
                        kind: c.unwrap(),
                        state: 0,
                    });
                    i += 1;
                }
                Some(b'"') => i = string_end(b, i)?,
                Some(b'-' | b'0'..=b'9') => i = number_end(b, i)?,
                Some(b'n' | b't' | b'f') => {
                    let word: &[u8] = match c {
                        Some(b'n') => b"null",
                        Some(b't') => b"true",
                        _ => b"false",
                    };
                    if b.get(i..i + word.len()) != Some(word) {
                        return fail(K::MalformedJson, i);
                    }
                    i += word.len();
                }
                _ => return fail(K::MalformedJson, i),
            }
        }
    }
    Ok(())
}
// Called only after whole-input grammar succeeded. Monotonic borrowed rescans;
// no span/token index is allocated or persisted.
pub(crate) fn extent(b: &[u8], at: usize) -> Span {
    let start = ws(b, at);
    let mut i = start;
    let mut depth = 0;
    loop {
        if depth == 0
            && i > start
            && matches!(
                b[i],
                b',' | b':' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t'
            )
        {
            break;
        }
        match b[i] {
            b'"' => {
                i = string_end(b, i).expect("validated string");
                if depth == 0 {
                    break;
                }
                continue;
            }
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    i += 1;
                    break;
                }
            }
            b',' | b':' | b' ' | b'\n' | b'\r' | b'\t' if depth == 0 => break,
            _ => {}
        }
        i += 1;
        if i == b.len() {
            break;
        }
    }
    Span { start, end: i }
}
pub(crate) struct Children<'a> {
    bytes: &'a [u8],
    end: usize,
    next: usize,
    object: bool,
}
impl Span {
    pub(crate) fn children(self, b: &[u8]) -> Children<'_> {
        Children {
            bytes: b,
            end: self.end - 1,
            next: self.start + 1,
            object: b[self.start] == b'{',
        }
    }
    pub(crate) fn bytes(self, b: &[u8]) -> &[u8] {
        &b[self.start..self.end]
    }
    pub(crate) fn count(self, b: &[u8]) -> usize {
        self.children(b).count()
    }
}
impl Iterator for Children<'_> {
    type Item = (Option<Span>, Span);
    fn next(&mut self) -> Option<Self::Item> {
        let b = self.bytes;
        let mut i = ws(b, self.next);
        if i >= self.end {
            return None;
        }
        if b[i] == b',' {
            i = ws(b, i + 1);
        }
        let first = extent(b, i);
        let (key, value) = if self.object {
            (Some(first), extent(b, ws(b, first.end) + 1))
        } else {
            (None, first)
        };
        self.next = value.end;
        Some((key, value))
    }
}
#[derive(Default)]
pub(crate) struct ScratchTrace {
    len: u64,
    capacity: u64,
    requested: u64,
    #[cfg(test)]
    observation: ScratchObservation,
}
#[cfg(test)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScratchObservation {
    pub(crate) requests: [u64; 16],
    pub(crate) offsets: [usize; 16],
    pub(crate) count: usize,
    pub(crate) clears: usize,
    pub(crate) capacity: u64,
    pub(crate) sum: u64,
}
impl ScratchTrace {
    #[cfg(test)]
    pub(crate) fn observation(&self) -> ScratchObservation {
        self.observation
    }
    pub(crate) fn requested(&self) -> u64 {
        self.requested
    }
    fn clear(&mut self) {
        self.len = 0;
        #[cfg(test)]
        {
            self.observation.clears += 1;
        }
    }
    fn reserve(&mut self, n: u64, at: usize) -> Checked<()> {
        let need = self.len.checked_add(n).ok_or(Fault {
            kind: K::RecordExtent,
            at,
        })?;
        if need > self.capacity {
            let cap = self
                .capacity
                .checked_mul(2)
                .ok_or(Fault {
                    kind: K::RecordExtent,
                    at,
                })?
                .max(need)
                .max(8);
            if cap > isize::MAX as u64 {
                return fail(K::RecordExtent, at);
            }
            self.requested = self.requested.checked_add(cap).ok_or(Fault {
                kind: K::RecordExtent,
                at,
            })?;
            self.capacity = cap;
            #[cfg(test)]
            {
                let o = &mut self.observation;
                if o.count < o.requests.len() {
                    o.requests[o.count] = cap;
                    o.offsets[o.count] = at;
                }
                o.count += 1;
                o.capacity = cap;
                o.sum = self.requested;
            }
        }
        Ok(())
    }
    fn extend(&mut self, n: usize, at: usize) -> Checked<()> {
        self.reserve(n as u64, at)?;
        self.len += n as u64;
        Ok(())
    }
}
// Raw segments and escapes are decoded without storing an owned key/tag.
fn text(
    b: &[u8],
    span: Span,
    mut trace: Option<&mut ScratchTrace>,
    mut byte: impl FnMut(u8),
) -> Checked<usize> {
    if b[span.start] != b'"' {
        return fail(K::UnexpectedType, span.start);
    }
    if let Some(t) = trace.as_deref_mut() {
        t.clear();
    }
    let mut i = span.start + 1;
    let mut begin = i;
    let mut length = 0;
    while i < span.end - 1 {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        let segment = &b[begin..i];
        std::str::from_utf8(segment).map_err(|_| Fault {
            kind: K::MalformedJson,
            at: begin,
        })?;
        if let Some(t) = trace.as_deref_mut() {
            t.extend(segment.len(), i)?;
        }
        for &c in segment {
            byte(c);
        }
        length += segment.len();
        i += 1;
        let ch = match b[i] {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let first = quad(b, i + 1)?;
                i += 4;
                let code = if (0xd800..=0xdbff).contains(&first) {
                    if b.get(i + 1..i + 3) != Some(&b"\\u"[..]) {
                        return fail(K::MalformedJson, i);
                    }
                    let second = quad(b, i + 3)?;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return fail(K::MalformedJson, i);
                    }
                    i += 6;
                    0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
                } else {
                    first
                };
                char::from_u32(code).ok_or(Fault {
                    kind: K::MalformedJson,
                    at: i,
                })?
            }
            _ => return fail(K::MalformedJson, i),
        };
        let mut buf = [0; 4];
        let encoded = ch.encode_utf8(&mut buf).as_bytes();
        if let Some(t) = trace.as_deref_mut() {
            t.reserve(if ch.is_ascii() { 1 } else { 4 }, i)?;
            t.len += encoded.len() as u64;
        }
        for &c in encoded {
            byte(c);
        }
        length += encoded.len();
        i += 1;
        begin = i;
    }
    let segment = &b[begin..span.end - 1];
    std::str::from_utf8(segment).map_err(|_| Fault {
        kind: K::MalformedJson,
        at: begin,
    })?;
    if let Some(t) = trace {
        if t.len != 0 {
            t.extend(segment.len(), begin)?;
        }
    }
    for &c in segment {
        byte(c);
    }
    Ok(length + segment.len())
}
pub(crate) fn string_eq(b: &[u8], span: Span, wanted: &str) -> bool {
    let mut i = 0;
    let mut same = true;
    let result = text(b, span, None, |c| {
        same &= wanted.as_bytes().get(i) == Some(&c);
        i += 1;
    });
    result.is_ok() && same && i == wanted.len()
}
fn ignored(b: &[u8], span: Span, trace: &mut ScratchTrace) -> Checked<()> {
    trace.clear();
    let mut i = span.start;
    let mut depth = 0;
    while i < span.end {
        match b[i] {
            b'"' => {
                i = string_end(b, i)?;
                continue;
            }
            b'[' | b'{' => {
                if depth > 0 {
                    trace.extend(1, i)?;
                }
                depth += 1;
            }
            b']' | b'}' => {
                if depth > 1 {
                    trace.len -= 1;
                }
                depth -= 1;
            }
            _ => {}
        }
        i += 1;
    }
    Ok(())
}
struct Numeric;
impl<'de> Visitor<'de> for Numeric {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("codec type")
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
}
fn generic(b: &[u8], span: Span, ancestor: usize) -> Checked<()> {
    let mut i = span.start;
    let mut depth = ancestor;
    while i < span.end {
        match b[i] {
            b'[' | b'{' => {
                depth += 1;
                if depth >= 128 {
                    return fail(K::TypedRecursionLimit, i);
                }
            }
            b']' | b'}' => depth -= 1,
            b'"' => {
                let end = string_end(b, i)?;
                text(b, Span { start: i, end }, None, |_| {})?;
                i = end;
                continue;
            }
            b'-' | b'0'..=b'9' => {
                let end = number_end(b, i)?;
                let mut de = serde_json::Deserializer::from_slice(&b[i..end]);
                de.deserialize_f64(Numeric).map_err(|_| Fault {
                    kind: K::FloatRange,
                    at: i,
                })?;
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    Ok(())
}
fn entered(depth: usize, at: usize) -> Checked<usize> {
    if depth + 1 >= 128 {
        fail(K::TypedRecursionLimit, at)
    } else {
        Ok(depth + 1)
    }
}
fn unit(b: &[u8], span: Span, origin: Origin, depth: usize) -> Checked<()> {
    if span.bytes(b) == b"null" {
        return Ok(());
    }
    if origin == Origin::Buffered && b[span.start] == b'{' && span.count(b) == 0 {
        entered(depth, span.start)?;
        return Ok(());
    }
    fail(K::UnexpectedType, span.start)
}
fn variant<'a>(b: &[u8], tag: Span, variants: &'a [Variant]) -> Checked<&'a Variant> {
    variants
        .iter()
        .find(|v| string_eq(b, tag, v.name))
        .ok_or(Fault {
            kind: K::UnknownVariant,
            at: tag.start,
        })
}
fn record(
    b: &[u8],
    span: Span,
    fields: &[Field],
    deny: bool,
    array: bool,
    origin: Origin,
    depth: usize,
    trace: &mut ScratchTrace,
) -> Checked<()> {
    let depth = entered(depth, span.start)?;
    let mut seen = 0u64;
    if fields.len() > 64 {
        return fail(K::PlanMismatch, span.start);
    }
    match b[span.start] {
        b'{' => {
            for (key, value) in span.children(b) {
                let key = key.unwrap();
                text(b, key, Some(trace), |_| {})?;
                if let Some(index) = fields.iter().position(|f| string_eq(b, key, f.name)) {
                    let bit = 1u64 << index;
                    if seen & bit != 0 {
                        return fail(K::DuplicateField, key.start);
                    }
                    seen |= bit;
                    check(b, value, fields[index].shape, origin, depth, trace)?;
                } else if deny {
                    return fail(K::UnknownField, key.start);
                } else {
                    ignored(b, value, trace)?;
                }
            }
        }
        b'[' if array => {
            let mut count = 0;
            for (_, value) in span.children(b) {
                if count >= fields.len() {
                    return fail(K::SequenceArity, value.start);
                }
                seen |= 1 << count;
                check(b, value, fields[count].shape, origin, depth, trace)?;
                count += 1;
            }
        }
        _ => return fail(K::UnexpectedType, span.start),
    }
    for (i, field) in fields.iter().enumerate() {
        if seen & (1 << i) == 0
            && !(if b[span.start] == b'[' {
                field.positional_default
            } else {
                field.optional
            })
        {
            return fail(K::MissingField, span.end);
        }
    }
    Ok(())
}
fn body(
    b: &[u8],
    span: Span,
    body: Body,
    origin: Origin,
    depth: usize,
    trace: &mut ScratchTrace,
    external: bool,
) -> Checked<()> {
    match body {
        Body::Unit => {
            if external {
                unit(b, span, origin, depth)
            } else if span.bytes(b) == b"null" {
                Ok(())
            } else {
                fail(K::UnexpectedType, span.start)
            }
        }
        Body::Value(shape) => check(b, span, shape, origin, depth, trace),
        Body::Record(fields, deny) => record(b, span, fields, deny, external, origin, depth, trace),
    }
}
// Rescans only the two-field adjacent shell. Descendant origin never resets.
pub(crate) fn adjacent<'a>(
    b: &[u8],
    span: Span,
    tag_name: &str,
    content_name: &str,
    variants: &'a [Variant],
    origin: Origin,
) -> Option<(&'a Variant, Option<Span>, Origin)> {
    if b[span.start] == b'[' {
        let mut c = span.children(b);
        let tag = c.next()?.1;
        let value = c.next()?.1;
        let v = variants.iter().find(|v| string_eq(b, tag, v.name))?;
        return Some((v, Some(value), origin));
    }
    let (mut tag, mut value) = (None, None);
    for (key, child) in span.children(b) {
        let key = key?;
        if string_eq(b, key, tag_name) {
            tag = Some(child);
        } else if string_eq(b, key, content_name) {
            value = Some(child);
        }
    }
    let tag = tag?;
    let selected = if b[tag.start] == b'"' {
        tag
    } else {
        tag.children(b).next()?.0?
    };
    let v = variants.iter().find(|v| string_eq(b, selected, v.name))?;
    let mode = if value.is_some_and(|v| v.start < tag.start) {
        Origin::Buffered
    } else {
        origin
    };
    Some((v, value, mode))
}
fn check(
    b: &[u8],
    span: Span,
    shape: &Shape,
    origin: Origin,
    depth: usize,
    trace: &mut ScratchTrace,
) -> Checked<()> {
    match *shape {
        Shape::Scalar(kind) => match kind {
            Scalar::String | Scalar::Date | Scalar::DateTime => {
                text(b, span, Some(trace), |_| {})?;
                Ok(())
            }
            Scalar::Bool => {
                if matches!(span.bytes(b), b"true" | b"false") {
                    Ok(())
                } else {
                    fail(K::UnexpectedType, span.start)
                }
            }
            Scalar::Integer | Scalar::Float => {
                if matches!(b[span.start], b'-' | b'0'..=b'9') {
                    Ok(())
                } else {
                    fail(K::UnexpectedType, span.start)
                }
            }
        },
        Shape::Option(child) => {
            if span.bytes(b) == b"null" {
                Ok(())
            } else {
                check(b, span, child, origin, depth, trace)
            }
        }
        Shape::Record(fields, deny, array) => {
            record(b, span, fields, deny, array, origin, depth, trace)
        }
        Shape::Seq(child) => {
            let d = entered(depth, span.start)?;
            if b[span.start] != b'[' {
                return fail(K::UnexpectedType, span.start);
            }
            for (_, v) in span.children(b) {
                check(b, v, child, origin, d, trace)?;
            }
            Ok(())
        }
        Shape::Tuple(a, c) => {
            let d = entered(depth, span.start)?;
            if b[span.start] != b'[' || span.count(b) != 2 {
                return fail(K::SequenceArity, span.start);
            }
            let mut it = span.children(b);
            check(b, it.next().unwrap().1, a, origin, d, trace)?;
            check(b, it.next().unwrap().1, c, origin, d, trace)
        }
        Shape::Map(key, value) => {
            let d = entered(depth, span.start)?;
            if b[span.start] != b'{' {
                return fail(K::UnexpectedType, span.start);
            }
            for (k, v) in span.children(b) {
                check(b, k.unwrap(), key, origin, d, trace)?;
                check(b, v, value, origin, d, trace)?;
            }
            Ok(())
        }
        Shape::External(variants) => {
            if b[span.start] == b'"' {
                text(b, span, Some(trace), |_| {})?;
                if !matches!(variant(b, span, variants)?.body, Body::Unit) {
                    return fail(K::UnexpectedType, span.start);
                }
                return Ok(());
            }
            let d = entered(depth, span.start)?;
            if b[span.start] != b'{' || span.count(b) != 1 {
                return fail(K::UnexpectedType, span.start);
            }
            let (key, v) = span.children(b).next().unwrap();
            let key = key.unwrap();
            text(b, key, Some(trace), |_| {})?;
            body(
                b,
                v,
                variant(b, key, variants)?.body,
                origin,
                d,
                trace,
                true,
            )
        }
        Shape::Adjacent(tag_name, content_name, variants) => {
            let d = entered(depth, span.start)?;
            if !matches!(b[span.start], b'{' | b'[') {
                return fail(K::UnexpectedType, span.start);
            }
            if b[span.start] == b'[' {
                if span.count(b) != 2 {
                    return fail(K::SequenceArity, span.start);
                }
                let mut it = span.children(b);
                let tag = it.next().unwrap().1;
                text(b, tag, Some(trace), |_| {})?;
                return body(
                    b,
                    it.next().unwrap().1,
                    variant(b, tag, variants)?.body,
                    origin,
                    d,
                    trace,
                    false,
                );
            }
            let (mut tag, mut content) = (None, None);
            for (key, value) in span.children(b) {
                let key = key.unwrap();
                if string_eq(b, key, tag_name) {
                    if tag.replace(value).is_some() {
                        return fail(K::DuplicateField, key.start);
                    }
                } else if string_eq(b, key, content_name) {
                    if content.replace(value).is_some() {
                        return fail(K::DuplicateField, key.start);
                    }
                } else {
                    return fail(K::UnknownField, key.start);
                }
            }
            let tag = tag.ok_or(Fault {
                kind: K::MissingField,
                at: span.end,
            })?;
            let tag_key = if b[tag.start] == b'"' {
                tag
            } else {
                if b[tag.start] != b'{' || tag.count(b) != 1 {
                    return fail(K::UnexpectedType, tag.start);
                }
                let (k, v) = tag.children(b).next().unwrap();
                unit(b, v, origin, entered(d, tag.start)?)?;
                k.unwrap()
            };
            let selected = variant(b, tag_key, variants)?;
            let mode = if content.is_some_and(|c| c.start < tag.start) {
                Origin::Buffered
            } else {
                origin
            };
            if mode == Origin::Buffered {
                if let Some(c) = content {
                    generic(b, c, d)?;
                }
            }
            for (key, value) in span.children(b) {
                let key = key.unwrap();
                text(b, key, Some(trace), |_| {})?;
                if string_eq(b, key, tag_name) {
                    text(b, tag_key, Some(trace), |_| {})?;
                } else {
                    body(b, value, selected.body, mode, d, trace, false)?;
                }
            }
            if content.is_none() && !matches!(selected.body, Body::Unit) {
                return fail(K::MissingField, span.end);
            }
            Ok(())
        }
    }
}
pub(crate) struct JsonPlan<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) span: Span,
    trace: ScratchTrace,
}
impl<'a> JsonPlan<'a> {
    pub(crate) fn prepare(
        bytes: &'a [u8],
        root: RootKind,
        shape: &Shape,
        work: &mut CodecMechanics<'_, '_>,
    ) -> Result<Self, ReplayTerminalFailure> {
        work.finish()?;
        if root.execution() && bytes.len() > 32 * 1024 * 1024 {
            return Err(work.refuse(K::RecordExtent, None));
        }
        let count = lexical_frames(bytes);
        let mut frames = work.frames(count)?;
        grammar(bytes, &mut frames, count).map_err(|f| work.refuse(f.kind, Some(f.at as u64)))?;
        work.decode_escrow(root)?;
        let span = extent(bytes, 0);
        let mut trace = ScratchTrace::default();
        check(bytes, span, shape, Origin::Direct, 0, &mut trace).map_err(|f| {
            if matches!(f.kind, K::RecordExtent) {
                work.scratch_layout_failure()
            } else {
                work.refuse(f.kind, Some(f.at as u64))
            }
        })?;
        work.decoder_scratch(&trace)?;
        Ok(Self { bytes, span, trace })
    }
}

// A fixed actual root fixture, used for shadow-order and debit boundary evidence.
#[cfg(test)]
pub(crate) fn ordered_scratch_bytes() -> Vec<u8> {
    format!(r#"{{"account_id":"abcdefghi\n","epoch_id":"abcdefghijklmno\u4E2Dy","manifest_hash":"abcdefghijklmnopqrs\n","ignored":{}0{}}}"#,"[".repeat(40),"]".repeat(40)).into_bytes()
}
#[cfg(test)]
pub(crate) fn fixed_scratch_trace() -> ScratchTrace {
    let bytes = ordered_scratch_bytes();
    let mut trace = ScratchTrace::default();
    let span = extent(&bytes, 0);
    check(&bytes,span,&<crate::trading::paper_ledger::AccountBinding as super::paper_replay_codec_v1::Value>::SHAPE,Origin::Direct,0,&mut trace).unwrap();
    trace
}

// The raw-paper array is a separate default/std compatibility grammar. It does
// not add a Deserialize root, materialize Value, or confer source authority.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RawNumber {
    I64(i64),
    U64(u64),
    F64(f64),
}
impl RawNumber {
    pub(super) fn as_f64(self) -> f64 {
        match self {
            Self::I64(v) => v as f64,
            Self::U64(v) => v as f64,
            Self::F64(v) => v,
        }
    }
    pub(super) fn as_u64(self) -> Option<u64> {
        match self {
            Self::U64(v) => Some(v),
            Self::I64(v) => u64::try_from(v).ok(),
            Self::F64(_) => None,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct RawText {
    start: usize,
    end: usize,
    decoded: usize,
}
impl RawText {
    pub(super) fn decoded_len(self) -> usize {
        self.decoded
    }
    pub(super) fn chars(self, raw: &str) -> RawChars<'_> {
        RawChars {
            raw: &raw[self.start..self.end],
            at: 0,
        }
    }
}
// Created only after validation; iteration cannot encounter an invalid escape.
pub(super) struct RawChars<'a> {
    raw: &'a str,
    at: usize,
}
impl Iterator for RawChars<'_> {
    type Item = char;
    fn next(&mut self) -> Option<char> {
        let c = self.raw[self.at..].chars().next()?;
        self.at += c.len_utf8();
        if c != '\\' {
            return Some(c);
        }
        let b = self.raw.as_bytes()[self.at];
        self.at += 1;
        Some(match b {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let mut n = 0;
                for b in &self.raw.as_bytes()[self.at..self.at + 4] {
                    n = 16 * n + hex(*b).expect("validated hex");
                }
                self.at += 4;
                if (0xD800..=0xDBFF).contains(&n) {
                    self.at += 2;
                    let mut low = 0;
                    for b in &self.raw.as_bytes()[self.at..self.at + 4] {
                        low = 16 * low + hex(*b).expect("validated low surrogate");
                    }
                    self.at += 4;
                    n = 0x10000 + ((n - 0xD800) << 10) + low - 0xDC00;
                }
                char::from_u32(n).expect("validated scalar")
            }
            _ => unreachable!("validated escape"),
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) enum RawSlot {
    Null,
    Bool(bool),
    Number(RawNumber),
    Text(RawText),
    Array,
    Object,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum RawSyntax {
    EofList,
    EofObject,
    EofString,
    EofValue,
    Colon,
    ListComma,
    ObjectComma,
    Ident,
    Value,
    Escape,
    Number,
    NumberRange,
    Control,
    Key,
    LoneSurrogate,
    TrailingComma,
    TrailingCharacters,
    EndHex,
    Depth,
}
impl RawSyntax {
    pub(super) fn literal(self) -> &'static str {
        match self {
            Self::EofList => "EOF while parsing a list",
            Self::EofObject => "EOF while parsing an object",
            Self::EofString => "EOF while parsing a string",
            Self::EofValue => "EOF while parsing a value",
            Self::Colon => "expected `:`",
            Self::ListComma => "expected `,` or `]`",
            Self::ObjectComma => "expected `,` or `}`",
            Self::Ident => "expected ident",
            Self::Value => "expected value",
            Self::Escape => "invalid escape",
            Self::Number => "invalid number",
            Self::NumberRange => "number out of range",
            Self::Control => "control character (\\u0000-\\u001F) found while parsing a string",
            Self::Key => "key must be a string",
            Self::LoneSurrogate => "lone leading surrogate in hex escape",
            Self::TrailingComma => "trailing comma",
            Self::TrailingCharacters => "trailing characters",
            Self::EndHex => "unexpected end of hex escape",
            Self::Depth => "recursion limit exceeded",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) enum RawFaultKind {
    Syntax(RawSyntax),
    ExpectedSequence(RawSlot),
}
#[derive(Clone, Copy, Debug)]
pub(super) struct RawFault {
    pub(super) kind: RawFaultKind,
    pub(super) line: usize,
    pub(super) column: usize,
}
pub(super) struct RawPaperArrayPlan {
    slots: [Option<RawSlot>; 15],
    len: usize,
}
impl RawPaperArrayPlan {
    pub(super) fn slot(&self, index: usize) -> Option<RawSlot> {
        self.slots.get(index).copied().flatten()
    }
    pub(super) fn len(&self) -> usize {
        self.len
    }
}
#[derive(Clone, Copy)]
enum RawFrame {
    ArrayFirst,
    ArrayNext,
    ObjectFirst,
    ObjectNext,
}
struct RawScanner<'a> {
    raw: &'a str,
    at: usize,
}
type RawResult<T> = Result<T, RawFault>;
impl RawScanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.raw.as_bytes().get(self.at).copied()
    }
    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.at += 1;
        Some(b)
    }
    fn space(&mut self) -> Option<u8> {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.at += 1;
        }
        self.peek()
    }
    fn position(&self, kind: RawFaultKind, peek: bool) -> RawFault {
        let end = if peek {
            (self.at + 1).min(self.raw.len())
        } else {
            self.at
        };
        let bytes = &self.raw.as_bytes()[..end];
        let start = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        RawFault {
            kind,
            line: 1 + bytes[..start].iter().filter(|b| **b == b'\n').count(),
            column: end - start,
        }
    }
    fn error(&self, kind: RawSyntax) -> RawFault {
        self.position(RawFaultKind::Syntax(kind), false)
    }
    fn peek_error(&self, kind: RawSyntax) -> RawFault {
        self.position(RawFaultKind::Syntax(kind), true)
    }
    fn ident(&mut self, suffix: &[u8]) -> RawResult<()> {
        for expected in suffix {
            match self.next() {
                Some(b) if b == *expected => (),
                Some(_) => return Err(self.error(RawSyntax::Ident)),
                None => return Err(self.error(RawSyntax::EofValue)),
            }
        }
        Ok(())
    }
    fn quad(&mut self) -> RawResult<u32> {
        if self.raw.len() - self.at < 4 {
            self.at = self.raw.len();
            return Err(self.error(RawSyntax::EofString));
        }
        let begin = self.at;
        self.at += 4;
        let mut n = 0;
        for b in &self.raw.as_bytes()[begin..self.at] {
            n = 16 * n + hex(*b).ok_or_else(|| self.error(RawSyntax::Escape))?;
        }
        Ok(n)
    }
    fn string(&mut self) -> RawResult<RawText> {
        self.at += 1;
        let start = self.at;
        let mut decoded = 0;
        loop {
            let b = self
                .next()
                .ok_or_else(|| self.error(RawSyntax::EofString))?;
            match b {
                b'"' => {
                    return Ok(RawText {
                        start,
                        end: self.at - 1,
                        decoded,
                    })
                }
                0..=31 => return Err(self.error(RawSyntax::Control)),
                b'\\' => {
                    let escape = self
                        .next()
                        .ok_or_else(|| self.error(RawSyntax::EofString))?;
                    match escape {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => decoded += 1,
                        b'u' => {
                            let mut n = self.quad()?;
                            if (0xDC00..=0xDFFF).contains(&n) {
                                return Err(self.error(RawSyntax::LoneSurrogate));
                            }
                            if (0xD800..=0xDBFF).contains(&n) {
                                for expected in [b'\\', b'u'] {
                                    match self.next() {
                                        Some(b) if b == expected => (),
                                        Some(_) => return Err(self.error(RawSyntax::EndHex)),
                                        None => return Err(self.error(RawSyntax::EofString)),
                                    }
                                }
                                let low = self.quad()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(self.error(RawSyntax::LoneSurrogate));
                                }
                                n = 0x10000 + ((n - 0xD800) << 10) + low - 0xDC00;
                            }
                            decoded += char::from_u32(n).expect("validated scalar").len_utf8();
                        }
                        _ => return Err(self.error(RawSyntax::Escape)),
                    }
                }
                _ => decoded += 1,
            }
        }
    }
    fn number(&mut self, positive: bool) -> RawResult<RawNumber> {
        let first = self.next().ok_or_else(|| self.error(RawSyntax::EofValue))?;
        let mut n = match first {
            b'0' => {
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.peek_error(RawSyntax::Number));
                }
                0
            }
            b'1'..=b'9' => u64::from(first - b'0'),
            _ => return Err(self.error(RawSyntax::Number)),
        };
        if first != b'0' {
            while let Some(b @ b'0'..=b'9') = self.peek() {
                let Some(next) = n
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(u64::from(b - b'0')))
                else {
                    let mut exponent = 0;
                    while matches!(self.peek(), Some(b'0'..=b'9')) {
                        self.at += 1;
                        exponent += 1;
                    }
                    return match self.peek() {
                        Some(b'.') => self.decimal(positive, n, exponent),
                        Some(b'e' | b'E') => self.exponent(positive, n, exponent),
                        _ => self.float(positive, n, exponent),
                    }
                    .map(RawNumber::F64);
                };
                self.at += 1;
                n = next;
            }
        }
        match self.peek() {
            Some(b'.') => self.decimal(positive, n, 0).map(RawNumber::F64),
            Some(b'e' | b'E') => self.exponent(positive, n, 0).map(RawNumber::F64),
            _ if positive => Ok(RawNumber::U64(n)),
            _ => {
                let negative = (n as i64).wrapping_neg();
                Ok(if negative >= 0 {
                    RawNumber::F64(-(n as f64))
                } else {
                    RawNumber::I64(negative)
                })
            }
        }
    }
    fn decimal(&mut self, positive: bool, mut n: u64, before: i32) -> RawResult<f64> {
        self.at += 1;
        let mut after = 0;
        while let Some(b @ b'0'..=b'9') = self.peek() {
            let Some(next) = n
                .checked_mul(10)
                .and_then(|v| v.checked_add(u64::from(b - b'0')))
            else {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.at += 1;
                }
                return match self.peek() {
                    Some(b'e' | b'E') => self.exponent(positive, n, before + after),
                    _ => self.float(positive, n, before + after),
                };
            };
            self.at += 1;
            n = next;
            after -= 1;
        }
        if after == 0 {
            return Err(self.peek_error(if self.peek().is_some() {
                RawSyntax::Number
            } else {
                RawSyntax::EofValue
            }));
        }
        match self.peek() {
            Some(b'e' | b'E') => self.exponent(positive, n, before + after),
            _ => self.float(positive, n, before + after),
        }
    }
    fn exponent(&mut self, positive: bool, n: u64, before: i32) -> RawResult<f64> {
        self.at += 1;
        let exp_positive = match self.peek() {
            Some(b'+') => {
                self.at += 1;
                true
            }
            Some(b'-') => {
                self.at += 1;
                false
            }
            _ => true,
        };
        let mut exp = match self.next() {
            Some(b @ b'0'..=b'9') => i32::from(b - b'0'),
            Some(_) => return Err(self.error(RawSyntax::Number)),
            None => return Err(self.error(RawSyntax::EofValue)),
        };
        while let Some(b @ b'0'..=b'9') = self.peek() {
            self.at += 1;
            let Some(next) = exp
                .checked_mul(10)
                .and_then(|v| v.checked_add(i32::from(b - b'0')))
            else {
                if n != 0 && exp_positive {
                    return Err(self.error(RawSyntax::NumberRange));
                }
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.at += 1;
                }
                return Ok(if positive { 0.0 } else { -0.0 });
            };
            exp = next;
        }
        self.float(
            positive,
            n,
            if exp_positive {
                before.saturating_add(exp)
            } else {
                before.saturating_sub(exp)
            },
        )
    }
    fn float(&self, positive: bool, n: u64, mut exponent: i32) -> RawResult<f64> {
        let mut value = n as f64;
        loop {
            if let Some(power) = RAW_POW10.get(exponent.wrapping_abs() as usize) {
                if exponent >= 0 {
                    value *= power;
                    if value.is_infinite() {
                        return Err(self.error(RawSyntax::NumberRange));
                    }
                } else {
                    value /= power;
                }
                break;
            }
            if value == 0.0 {
                break;
            }
            if exponent >= 0 {
                return Err(self.error(RawSyntax::NumberRange));
            }
            value /= 1e308;
            exponent += 308;
        }
        Ok(if positive { value } else { -value })
    }
    fn scalar(&mut self) -> RawResult<RawSlot> {
        match self.space() {
            Some(b'n') => {
                self.at += 1;
                self.ident(b"ull")?;
                Ok(RawSlot::Null)
            }
            Some(b't') => {
                self.at += 1;
                self.ident(b"rue")?;
                Ok(RawSlot::Bool(true))
            }
            Some(b'f') => {
                self.at += 1;
                self.ident(b"alse")?;
                Ok(RawSlot::Bool(false))
            }
            Some(b'"') => self.string().map(RawSlot::Text),
            Some(b'-') => {
                self.at += 1;
                self.number(false).map(RawSlot::Number)
            }
            Some(b'0'..=b'9') => self.number(true).map(RawSlot::Number),
            Some(b'[') => Ok(RawSlot::Array),
            Some(b'{') => Ok(RawSlot::Object),
            Some(_) => Err(self.peek_error(RawSyntax::Value)),
            None => Err(self.peek_error(RawSyntax::EofValue)),
        }
    }
}
// The only production caller holds an inseparable paid-owned-row/work view.
// This function is not exported as a decoder that accepts caller qualification.
pub(super) fn scan_paid_raw_array(raw: &str) -> RawResult<RawPaperArrayPlan> {
    let mut scan = RawScanner { raw, at: 0 };
    if scan.space() != Some(b'[') {
        let unexpected = scan.scalar()?;
        return Err(scan.position(RawFaultKind::ExpectedSequence(unexpected), false));
    }
    let mut frames = [RawFrame::ArrayFirst; 127];
    let mut depth = 1;
    scan.at += 1;
    let mut plan = RawPaperArrayPlan {
        slots: [None; 15],
        len: 0,
    };
    loop {
        let state = frames[depth - 1];
        let peek = scan.space();
        let array = matches!(state, RawFrame::ArrayFirst | RawFrame::ArrayNext);
        let close = if array { b']' } else { b'}' };
        if peek == Some(close) {
            scan.at += 1;
            depth -= 1;
            if depth == 0 {
                break;
            }
            continue;
        }
        let Some(mut next) = peek else {
            return Err(scan.peek_error(if array {
                RawSyntax::EofList
            } else {
                RawSyntax::EofObject
            }));
        };
        if matches!(state, RawFrame::ArrayNext | RawFrame::ObjectNext) {
            if next != b',' {
                return Err(scan.peek_error(if array {
                    RawSyntax::ListComma
                } else {
                    RawSyntax::ObjectComma
                }));
            }
            scan.at += 1;
            next = scan
                .space()
                .ok_or_else(|| scan.peek_error(RawSyntax::EofValue))?;
            if next == close {
                return Err(scan.peek_error(RawSyntax::TrailingComma));
            }
        }
        if !array {
            if next != b'"' {
                return Err(scan.peek_error(RawSyntax::Key));
            }
            scan.string()?;
            match scan.space() {
                Some(b':') => scan.at += 1,
                Some(_) => return Err(scan.peek_error(RawSyntax::Colon)),
                None => return Err(scan.peek_error(RawSyntax::EofObject)),
            }
        }
        let value = scan.scalar()?;
        if depth == 1 {
            if plan.len < 15 {
                plan.slots[plan.len] = Some(value);
            }
            plan.len += 1;
        }
        frames[depth - 1] = if array {
            RawFrame::ArrayNext
        } else {
            RawFrame::ObjectNext
        };
        let child = match value {
            RawSlot::Array => Some(RawFrame::ArrayFirst),
            RawSlot::Object => Some(RawFrame::ObjectFirst),
            _ => None,
        };
        if let Some(child) = child {
            // Original remaining_depth starts at128 and rejects its zero entry.
            if depth == 127 {
                return Err(scan.peek_error(RawSyntax::Depth));
            }
            scan.at += 1;
            frames[depth] = child;
            depth += 1;
        }
    }
    if scan.space().is_some() {
        return Err(scan.peek_error(RawSyntax::TrailingCharacters));
    }
    Ok(plan)
}

static RAW_POW10: [f64; 309] = [
    1e000, 1e001, 1e002, 1e003, 1e004, 1e005, 1e006, 1e007, 1e008, 1e009, //
    1e010, 1e011, 1e012, 1e013, 1e014, 1e015, 1e016, 1e017, 1e018, 1e019, //
    1e020, 1e021, 1e022, 1e023, 1e024, 1e025, 1e026, 1e027, 1e028, 1e029, //
    1e030, 1e031, 1e032, 1e033, 1e034, 1e035, 1e036, 1e037, 1e038, 1e039, //
    1e040, 1e041, 1e042, 1e043, 1e044, 1e045, 1e046, 1e047, 1e048, 1e049, //
    1e050, 1e051, 1e052, 1e053, 1e054, 1e055, 1e056, 1e057, 1e058, 1e059, //
    1e060, 1e061, 1e062, 1e063, 1e064, 1e065, 1e066, 1e067, 1e068, 1e069, //
    1e070, 1e071, 1e072, 1e073, 1e074, 1e075, 1e076, 1e077, 1e078, 1e079, //
    1e080, 1e081, 1e082, 1e083, 1e084, 1e085, 1e086, 1e087, 1e088, 1e089, //
    1e090, 1e091, 1e092, 1e093, 1e094, 1e095, 1e096, 1e097, 1e098, 1e099, //
    1e100, 1e101, 1e102, 1e103, 1e104, 1e105, 1e106, 1e107, 1e108, 1e109, //
    1e110, 1e111, 1e112, 1e113, 1e114, 1e115, 1e116, 1e117, 1e118, 1e119, //
    1e120, 1e121, 1e122, 1e123, 1e124, 1e125, 1e126, 1e127, 1e128, 1e129, //
    1e130, 1e131, 1e132, 1e133, 1e134, 1e135, 1e136, 1e137, 1e138, 1e139, //
    1e140, 1e141, 1e142, 1e143, 1e144, 1e145, 1e146, 1e147, 1e148, 1e149, //
    1e150, 1e151, 1e152, 1e153, 1e154, 1e155, 1e156, 1e157, 1e158, 1e159, //
    1e160, 1e161, 1e162, 1e163, 1e164, 1e165, 1e166, 1e167, 1e168, 1e169, //
    1e170, 1e171, 1e172, 1e173, 1e174, 1e175, 1e176, 1e177, 1e178, 1e179, //
    1e180, 1e181, 1e182, 1e183, 1e184, 1e185, 1e186, 1e187, 1e188, 1e189, //
    1e190, 1e191, 1e192, 1e193, 1e194, 1e195, 1e196, 1e197, 1e198, 1e199, //
    1e200, 1e201, 1e202, 1e203, 1e204, 1e205, 1e206, 1e207, 1e208, 1e209, //
    1e210, 1e211, 1e212, 1e213, 1e214, 1e215, 1e216, 1e217, 1e218, 1e219, //
    1e220, 1e221, 1e222, 1e223, 1e224, 1e225, 1e226, 1e227, 1e228, 1e229, //
    1e230, 1e231, 1e232, 1e233, 1e234, 1e235, 1e236, 1e237, 1e238, 1e239, //
    1e240, 1e241, 1e242, 1e243, 1e244, 1e245, 1e246, 1e247, 1e248, 1e249, //
    1e250, 1e251, 1e252, 1e253, 1e254, 1e255, 1e256, 1e257, 1e258, 1e259, //
    1e260, 1e261, 1e262, 1e263, 1e264, 1e265, 1e266, 1e267, 1e268, 1e269, //
    1e270, 1e271, 1e272, 1e273, 1e274, 1e275, 1e276, 1e277, 1e278, 1e279, //
    1e280, 1e281, 1e282, 1e283, 1e284, 1e285, 1e286, 1e287, 1e288, 1e289, //
    1e290, 1e291, 1e292, 1e293, 1e294, 1e295, 1e296, 1e297, 1e298, 1e299, //
    1e300, 1e301, 1e302, 1e303, 1e304, 1e305, 1e306, 1e307, 1e308,
];
