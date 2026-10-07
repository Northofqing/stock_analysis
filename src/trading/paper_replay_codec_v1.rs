//! Finite paid DTO decode/copy/serialization. Historical Deserialize remains unchanged.
#![allow(dead_code)]
use super::paper_replay_shapes_v1::{
    self as shapes, Body, Field, JsonPlan, Origin, RootKind, Scalar, Shape, Span, Variant,
};
pub(crate) use crate::database::global_schema_v1::replay_work::{
    CodecMechanics, ReplayCodecFailureKind as K, ReplayMemory, ReplayTerminalFailure,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{
    de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor},
    Deserializer, Serialize,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, io, marker::PhantomData};

// Implementations exist only in the reviewed owner modules and the closed
// primitive/container compositions below. No blanket DeserializeOwned seam.
pub(crate) mod sealed {
    pub(crate) trait Value {}
    pub(crate) trait Root {}
    pub(crate) trait Element {}
    pub(crate) trait Entry {}
}
pub(crate) trait ArrayElement: sealed::Element {}
pub(crate) trait MapEntry: sealed::Entry {}
pub(crate) trait Value: sealed::Value + Serialize + Sized {
    const SHAPE: Shape;
    const OPTIONAL: bool = false;
    fn missing() -> Option<Self> {
        None
    }
    fn read<'de, D: Deserializer<'de>>(
        de: D,
        input: Input<'de, '_, '_, '_>,
    ) -> Result<Self, D::Error>;
    fn paid_copy(&self, work: &mut CodecMechanics<'_, '_>) -> Result<Self, ReplayTerminalFailure>;
}
pub(crate) trait Root: Value + sealed::Root {
    const ROOT: RootKind;
    const CANONICAL: bool;
}
pub(crate) struct Input<'de, 'w, 'loan, 'pool> {
    pub(crate) bytes: &'de [u8],
    pub(crate) span: Span,
    pub(crate) origin: Origin,
    pub(crate) work: &'w mut CodecMechanics<'loan, 'pool>,
}
impl<'de, 'w, 'loan, 'pool> Input<'de, 'w, 'loan, 'pool> {
    pub(crate) fn child(&mut self, span: Span, origin: Origin) -> Input<'de, '_, 'loan, 'pool> {
        Input {
            bytes: self.bytes,
            span,
            origin,
            work: self.work,
        }
    }
    pub(crate) fn error<E: de::Error>(&mut self, kind: K, literal: &'static str) -> E {
        self.work.refuse(kind, Some(self.span.start as u64));
        E::custom(literal)
    }
}
pub(crate) struct Seed<'de, 'w, 'loan, 'pool, T> {
    input: Input<'de, 'w, 'loan, 'pool>,
    marker: PhantomData<T>,
}
impl<'de, 'w, 'loan, 'pool, T> Seed<'de, 'w, 'loan, 'pool, T> {
    pub(crate) fn new(input: Input<'de, 'w, 'loan, 'pool>) -> Self {
        Self {
            input,
            marker: PhantomData,
        }
    }
}
impl<'de, T: Value> DeserializeSeed<'de> for Seed<'de, '_, '_, '_, T> {
    type Value = T;
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<T, D::Error> {
        let Input {
            bytes,
            span,
            origin,
            work,
        } = self.input;
        #[cfg(test)]
        {
            work.hits.active_seeds += 1;
            work.hits.max_active_seeds = work.hits.max_active_seeds.max(work.hits.active_seeds);
        }
        let result = T::read(
            de,
            Input {
                bytes,
                span,
                origin,
                work: &mut *work,
            },
        );
        #[cfg(test)]
        {
            work.hits.active_seeds -= 1;
        }
        result
    }
}
pub(crate) fn terminal<E: de::Error>(_: ReplayTerminalFailure) -> E {
    E::custom("codec terminal")
}
pub(crate) fn expected(f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str("codec type")
}

pub(crate) struct KeySeed {
    pub(crate) names: &'static [&'static str],
}
impl<'de> DeserializeSeed<'de> for KeySeed {
    type Value = &'static str;
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        de.deserialize_identifier(self)
    }
}
impl<'de> Visitor<'de> for KeySeed {
    type Value = &'static str;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expected(f)
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(self
            .names
            .iter()
            .copied()
            .find(|s| *s == value)
            .unwrap_or(""))
    }
}
pub(crate) fn span_error<E: de::Error>() -> E {
    E::custom("codec plan mismatch")
}

struct StringVisitor<'de, 'w, 'loan, 'pool>(Input<'de, 'w, 'loan, 'pool>);
impl<'de> Visitor<'de> for StringVisitor<'de, '_, '_, '_> {
    type Value = String;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expected(f)
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
        self.0.work.string(value).map_err(terminal)
    }
}
impl sealed::Value for String {}
impl Value for String {
    const SHAPE: Shape = Shape::Scalar(Scalar::String);
    fn read<'de, D: Deserializer<'de>>(
        de: D,
        input: Input<'de, '_, '_, '_>,
    ) -> Result<Self, D::Error> {
        de.deserialize_str(StringVisitor(input))
    }
    fn paid_copy(&self, w: &mut CodecMechanics<'_, '_>) -> Result<Self, ReplayTerminalFailure> {
        w.string(self)
    }
}
impl sealed::Element for String {}
impl ArrayElement for String {}
impl sealed::Element for u8 {}
impl ArrayElement for u8 {}

trait Number: Copy {
    fn signed(v: i64) -> Option<Self>;
    fn unsigned(v: u64) -> Option<Self>;
    fn float(v: f64) -> Option<Self>;
}
macro_rules! integer_value {($($t:ty),+)=>{$(impl Number for $t{fn signed(v:i64)->Option<Self>{Self::try_from(v).ok()}fn unsigned(v:u64)->Option<Self>{Self::try_from(v).ok()}fn float(_:f64)->Option<Self>{None}}
impl sealed::Value for $t{} impl Value for $t{const SHAPE:Shape=Shape::Scalar(Scalar::Integer);fn read<'de,D:Deserializer<'de>>(de:D,input:Input<'de,'_,'_,'_>)->Result<Self,D::Error>{read_number::<D,Self>(de,input,false)}fn paid_copy(&self,w:&mut CodecMechanics<'_,'_>)->Result<Self,ReplayTerminalFailure>{w.finish()?;Ok(*self)}})+};}
integer_value!(i64, i32, u32, u8);
impl Number for f64 {
    fn signed(v: i64) -> Option<Self> {
        Some(v as f64)
    }
    fn unsigned(v: u64) -> Option<Self> {
        Some(v as f64)
    }
    fn float(v: f64) -> Option<Self> {
        Some(v)
    }
}
impl sealed::Value for f64 {}
impl Value for f64 {
    const SHAPE: Shape = Shape::Scalar(Scalar::Float);
    fn read<'de, D: Deserializer<'de>>(
        de: D,
        input: Input<'de, '_, '_, '_>,
    ) -> Result<Self, D::Error> {
        read_number::<D, Self>(de, input, true)
    }
    fn paid_copy(&self, w: &mut CodecMechanics<'_, '_>) -> Result<Self, ReplayTerminalFailure> {
        w.finish()?;
        Ok(*self)
    }
}
// Numeric parser failures retain their known leaf span, while a later generic
// fallback reports no invented descendant position.
fn read_number<'de, D: Deserializer<'de>, T: Number>(
    de: D,
    input: Input<'de, '_, '_, '_>,
    float: bool,
) -> Result<T, D::Error> {
    let Input {
        bytes,
        span,
        origin,
        work,
    } = input;
    let visitor = NumberVisitor::<T> {
        input: Input {
            bytes,
            span,
            origin,
            work: &mut *work,
        },
        marker: PhantomData,
    };
    let result = if float {
        de.deserialize_f64(visitor)
    } else {
        de.deserialize_i64(visitor)
    };
    if result.is_err() {
        work.refuse(
            if float {
                K::FloatRange
            } else {
                K::IntegerRange
            },
            Some(span.start as u64),
        );
    }
    result
}
struct NumberVisitor<'de, 'w, 'loan, 'pool, T> {
    input: Input<'de, 'w, 'loan, 'pool>,
    marker: PhantomData<T>,
}
impl<'de, T: Number> Visitor<'de> for NumberVisitor<'de, '_, '_, '_, T> {
    type Value = T;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expected(f)
    }
    fn visit_i64<E: de::Error>(mut self, v: i64) -> Result<T, E> {
        T::signed(v).ok_or_else(|| self.input.error(K::IntegerRange, "codec integer range"))
    }
    fn visit_u64<E: de::Error>(mut self, v: u64) -> Result<T, E> {
        T::unsigned(v).ok_or_else(|| self.input.error(K::IntegerRange, "codec integer range"))
    }
    fn visit_f64<E: de::Error>(mut self, v: f64) -> Result<T, E> {
        T::float(v).ok_or_else(|| self.input.error(K::IntegerRange, "codec integer range"))
    }
}
struct BoolVisitor;
impl<'de> Visitor<'de> for BoolVisitor {
    type Value = bool;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expected(f)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<bool, E> {
        Ok(v)
    }
}
impl sealed::Value for bool {}
impl Value for bool {
    const SHAPE: Shape = Shape::Scalar(Scalar::Bool);
    fn read<'de, D: Deserializer<'de>>(de: D, _: Input<'de, '_, '_, '_>) -> Result<Self, D::Error> {
        de.deserialize_bool(BoolVisitor)
    }
    fn paid_copy(&self, w: &mut CodecMechanics<'_, '_>) -> Result<Self, ReplayTerminalFailure> {
        w.finish()?;
        Ok(*self)
    }
}
macro_rules! date_value {
    ($t:ty,$shape:ident,$failure:ident) => {
        impl sealed::Value for $t {}
        impl Value for $t {
            const SHAPE: Shape = Shape::Scalar(Scalar::$shape);
            fn read<'de, D: Deserializer<'de>>(
                de: D,
                input: Input<'de, '_, '_, '_>,
            ) -> Result<Self, D::Error> {
                struct DateVisitor<'de, 'w, 'loan, 'pool>(Input<'de, 'w, 'loan, 'pool>);
                impl<'de> Visitor<'de> for DateVisitor<'de, '_, '_, '_> {
                    type Value = $t;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        expected(f)
                    }
                    fn visit_str<E: de::Error>(mut self, v: &str) -> Result<Self::Value, E> {
                        v.parse::<$t>().map_err(|e| {
                            self.0
                                .work
                                .refuse(K::$failure, Some(self.0.span.start as u64));
                            E::custom(e)
                        })
                    }
                }
                de.deserialize_str(DateVisitor(input))
            }
            fn paid_copy(
                &self,
                w: &mut CodecMechanics<'_, '_>,
            ) -> Result<Self, ReplayTerminalFailure> {
                w.finish()?;
                Ok(*self)
            }
        }
    };
}
date_value!(NaiveDate, Date, InvalidDate);
date_value!(DateTime<Utc>, DateTime, InvalidDateTime);

impl<T: Value> sealed::Value for Option<T> {}
impl<T: Value> Value for Option<T> {
    const SHAPE: Shape = Shape::Option(&T::SHAPE);
    const OPTIONAL: bool = true;
    fn missing() -> Option<Self> {
        Some(None)
    }
    fn read<'de, D: Deserializer<'de>>(
        de: D,
        input: Input<'de, '_, '_, '_>,
    ) -> Result<Self, D::Error> {
        struct Opt<'de, 'w, 'loan, 'pool, T>(Input<'de, 'w, 'loan, 'pool>, PhantomData<T>);
        impl<'de, T: Value> Visitor<'de> for Opt<'de, '_, '_, '_, T> {
            type Value = Option<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                expected(f)
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_some<D: Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
                T::read(de, self.0).map(Some)
            }
        }
        de.deserialize_option(Opt::<T>(input, PhantomData))
    }
    fn paid_copy(&self, w: &mut CodecMechanics<'_, '_>) -> Result<Self, ReplayTerminalFailure> {
        w.finish()?;
        self.as_ref().map(|v| v.paid_copy(w)).transpose()
    }
}
impl<T: Value + ArrayElement> sealed::Value for Vec<T> {}
impl<T: Value + ArrayElement> Value for Vec<T> {
    const SHAPE: Shape = Shape::Seq(&T::SHAPE);
    fn read<'de, D: Deserializer<'de>>(
        de: D,
        input: Input<'de, '_, '_, '_>,
    ) -> Result<Self, D::Error> {
        struct List<'de, 'w, 'loan, 'pool, T>(Input<'de, 'w, 'loan, 'pool>, PhantomData<T>);
        impl<'de, T: Value + ArrayElement> Visitor<'de> for List<'de, '_, '_, '_, T> {
            type Value = Vec<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                expected(f)
            }
            fn visit_seq<A: SeqAccess<'de>>(mut self, mut seq: A) -> Result<Self::Value, A::Error> {
                let count = self.0.span.count(self.0.bytes);
                let mut out = self.0.work.vector::<T>(count).map_err(terminal)?;
                let origin = self.0.origin;
                for (_, span) in self.0.span.children(self.0.bytes) {
                    let v = seq
                        .next_element_seed(Seed::<T>::new(self.0.child(span, origin)))?
                        .ok_or_else(span_error)?;
                    out.push(v);
                }
                Ok(out)
            }
        }
        de.deserialize_seq(List::<T>(input, PhantomData))
    }
    fn paid_copy(&self, w: &mut CodecMechanics<'_, '_>) -> Result<Self, ReplayTerminalFailure> {
        let mut out = w.vector::<T>(self.len())?;
        for v in self {
            out.push(v.paid_copy(w)?);
        }
        Ok(out)
    }
}
impl<K: Value + Ord, V: Value> sealed::Value for BTreeMap<K, V> where (K, V): MapEntry {}
impl<K: Value + Ord, V: Value> Value for BTreeMap<K, V>
where
    (K, V): MapEntry,
{
    const SHAPE: Shape = Shape::Map(&K::SHAPE, &V::SHAPE);
    fn read<'de, D: Deserializer<'de>>(
        de: D,
        input: Input<'de, '_, '_, '_>,
    ) -> Result<Self, D::Error> {
        struct Map<'de, 'w, 'loan, 'pool, K, V>(Input<'de, 'w, 'loan, 'pool>, PhantomData<(K, V)>);
        impl<'de, K: Value + Ord, V: Value> Visitor<'de> for Map<'de, '_, '_, '_, K, V>
        where
            (K, V): MapEntry,
        {
            type Value = BTreeMap<K, V>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                expected(f)
            }
            fn visit_map<A: MapAccess<'de>>(mut self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = BTreeMap::new();
                let origin = self.0.origin;
                for (key, value) in self.0.span.children(self.0.bytes) {
                    let key = map
                        .next_key_seed(Seed::<K>::new(self.0.child(key.unwrap(), origin)))?
                        .ok_or_else(span_error)?;
                    let value = map.next_value_seed(Seed::<V>::new(self.0.child(value, origin)))?;
                    self.0.work.insert(&mut out, key, value).map_err(terminal)?;
                }
                Ok(out)
            }
        }
        de.deserialize_map(Map::<K, V>(input, PhantomData))
    }
    fn paid_copy(&self, w: &mut CodecMechanics<'_, '_>) -> Result<Self, ReplayTerminalFailure> {
        w.finish()?;
        let mut out = BTreeMap::new();
        for (k, v) in self {
            let k = k.paid_copy(w)?;
            let v = v.paid_copy(w)?;
            w.insert(&mut out, k, v)?;
        }
        Ok(out)
    }
}
macro_rules! tuple_value {
    ($a:ty,$b:ty) => {
        impl sealed::Value for ($a, $b) {}
        impl Value for ($a, $b) {
            const SHAPE: Shape = Shape::Tuple(&<$a as Value>::SHAPE, &<$b as Value>::SHAPE);
            fn read<'de, D: Deserializer<'de>>(
                de: D,
                input: Input<'de, '_, '_, '_>,
            ) -> Result<Self, D::Error> {
                struct Pair<'de, 'w, 'loan, 'pool>(Input<'de, 'w, 'loan, 'pool>);
                impl<'de> Visitor<'de> for Pair<'de, '_, '_, '_> {
                    type Value = ($a, $b);
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        expected(f)
                    }
                    fn visit_seq<A: SeqAccess<'de>>(
                        mut self,
                        mut seq: A,
                    ) -> Result<Self::Value, A::Error> {
                        let mut spans = self.0.span.children(self.0.bytes);
                        let origin = self.0.origin;
                        let a = seq
                            .next_element_seed(Seed::<$a>::new(
                                self.0.child(spans.next().ok_or_else(span_error)?.1, origin),
                            ))?
                            .ok_or_else(span_error)?;
                        let b = seq
                            .next_element_seed(Seed::<$b>::new(
                                self.0.child(spans.next().ok_or_else(span_error)?.1, origin),
                            ))?
                            .ok_or_else(span_error)?;
                        Ok((a, b))
                    }
                }
                de.deserialize_tuple(2, Pair(input))
            }
            fn paid_copy(
                &self,
                w: &mut CodecMechanics<'_, '_>,
            ) -> Result<Self, ReplayTerminalFailure> {
                Ok((self.0.paid_copy(w)?, self.1.paid_copy(w)?))
            }
        }
    };
}
tuple_value!(i64, String);
tuple_value!(String, Option<String>);

pub(crate) struct UnitPayload<'de, 'w, 'loan, 'pool>(pub(crate) Input<'de, 'w, 'loan, 'pool>);
struct FixedUnit;
impl<'de> Visitor<'de> for FixedUnit {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expected(f)
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}
struct RejectKey;
impl<'de> DeserializeSeed<'de> for RejectKey {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, _: D) -> Result<(), D::Error> {
        Err(span_error())
    }
}
struct EmptyUnit<'w, 'loan, 'pool>(&'w mut CodecMechanics<'loan, 'pool>);
impl<'de> Visitor<'de> for EmptyUnit<'_, '_, '_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expected(f)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        #[cfg(test)]
        {
            self.0.hits.unit_maps += 1;
            self.0.fixture_unit_failure().map_err(terminal)?;
        }
        if map.next_key_seed(RejectKey)?.is_none() {
            Ok(())
        } else {
            Err(span_error())
        }
    }
}
impl<'de> DeserializeSeed<'de> for UnitPayload<'de, '_, '_, '_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        if self.0.origin == Origin::Buffered && self.0.bytes[self.0.span.start] == b'{' {
            de.deserialize_map(EmptyUnit(self.0.work))
        } else {
            de.deserialize_unit(FixedUnit)
        }
    }
}
pub(crate) fn adjacent_unit<'de, D: Deserializer<'de>>(de: D) -> Result<(), D::Error> {
    de.deserialize_any(FixedUnit)
}

pub(crate) fn decode_closed<T: Root>(
    bytes: &[u8],
    memory: &mut ReplayMemory<'_, '_>,
) -> Result<T, ReplayTerminalFailure> {
    decode_core::<T>(bytes, &mut memory.mechanics()?)
}
pub(crate) fn decode_core<T: Root>(
    bytes: &[u8],
    work: &mut CodecMechanics<'_, '_>,
) -> Result<T, ReplayTerminalFailure> {
    let plan = JsonPlan::prepare(bytes, T::ROOT, &T::SHAPE, work)?;
    let mut de = serde_json::Deserializer::from_slice(plan.bytes);
    #[cfg(test)]
    {
        work.hits.decoder += 1;
    } // Actual primary root materializer invocation, after preflight.

    let result = T::read(
        &mut de,
        Input {
            bytes: plan.bytes,
            span: plan.span,
            origin: Origin::Direct,
            work,
        },
    );
    let value = result.map_err(|_| {
        work.finish()
            .err()
            .unwrap_or_else(|| work.refuse(K::UnexpectedType, None))
    })?;
    de.end().map_err(|_| work.refuse(K::MalformedJson, None))?;
    work.finish()?;
    Ok(value)
}
pub(crate) fn copy_closed<T: Value>(
    value: &T,
    memory: &mut ReplayMemory<'_, '_>,
) -> Result<T, ReplayTerminalFailure> {
    value.paid_copy(&mut memory.mechanics()?)
}
struct Counter(usize);
impl io::Write for Counter {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(b.len())
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Output<'a> {
    bytes: &'a mut Vec<u8>,
    limit: usize,
}
impl io::Write for Output<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if b.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::from(io::ErrorKind::OutOfMemory));
        }
        self.bytes.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encoded<T: Serialize>(
    value: &T,
    work: &mut CodecMechanics<'_, '_>,
) -> Result<Vec<u8>, ReplayTerminalFailure> {
    work.serializer_escrow()?;
    let mut count = Counter(0);
    serde_json::to_writer(&mut count, value).map_err(|_| work.refuse(K::RecordExtent, None))?;
    let mut bytes = work.output(count.0)?;
    serde_json::to_writer(
        Output {
            bytes: &mut bytes,
            limit: count.0,
        },
        value,
    )
    .map_err(|_| work.refuse(K::PlanMismatch, None))?;
    Ok(bytes)
}
pub(crate) fn encode_core<T: Root>(
    value: &T,
    work: &mut CodecMechanics<'_, '_>,
) -> Result<Vec<u8>, ReplayTerminalFailure> {
    let bytes = encoded(value, work)?;
    if matches!(
        T::ROOT,
        RootKind::ExecutionManifest | RootKind::ExecutionFact | RootKind::ExecutionProjection
    ) && bytes.len() > 32 * 1024 * 1024
    {
        return Err(work.refuse(K::RecordExtent, None));
    }
    Ok(bytes)
}
pub(crate) fn canonical_equal<T: Root>(
    value: &T,
    original: &[u8],
    memory: &mut ReplayMemory<'_, '_>,
) -> Result<bool, ReplayTerminalFailure> {
    Ok(encode_core(value, &mut memory.mechanics()?)? == original)
}
pub(crate) fn decode_record<T: Root>(
    bytes: &[u8],
    memory: &mut ReplayMemory<'_, '_>,
) -> Result<T, ReplayTerminalFailure> {
    decode_record_core::<T>(bytes, &mut memory.mechanics()?)
}
fn decode_record_core<T: Root>(
    bytes: &[u8],
    work: &mut CodecMechanics<'_, '_>,
) -> Result<T, ReplayTerminalFailure> {
    let value = decode_core::<T>(bytes, work)?;
    if T::CANONICAL && encode_core(&value, work)? != bytes {
        return Err(work.refuse(K::Noncanonical, None));
    }
    Ok(value)
}

// Exact domain forms from the immutable owners; no caller-supplied domain or serializer callback.
pub(crate) enum HashInput<'a> {
    SeedBinding(&'a super::paper_ledger::SeedManifest),
    ExecutionManifest(&'a super::paper_book_v2_execution::ExecutionManifest),
    Raw(&'a [u8]),
    Cutover(&'a [u8]),
    V1Event {
        account: &'a str,
        seq: i64,
        command: &'a str,
        previous: &'a str,
        payload: &'a str,
    },
    GenesisEvent {
        account: &'a str,
        command: &'a str,
        previous: &'a str,
        payload: &'a [u8],
    },
    ExecutionEvent {
        account: &'a str,
        seq: i64,
        command: &'a str,
        previous: &'a str,
        payload: &'a [u8],
    },
}
pub(crate) fn digest_closed(
    input: HashInput<'_>,
    memory: &mut ReplayMemory<'_, '_>,
) -> Result<String, ReplayTerminalFailure> {
    digest_core(input, &mut memory.mechanics()?)
}
fn digest_core(
    input: HashInput<'_>,
    work: &mut CodecMechanics<'_, '_>,
) -> Result<String, ReplayTerminalFailure> {
    let mut hash = Sha256::new();
    match input {
        HashInput::SeedBinding(value) => hash.update(encoded(
            &(
                1,
                super::paper_ledger::MONEY_MODEL,
                super::paper_ledger::FEE_MODEL,
                value,
            ),
            work,
        )?),
        HashInput::ExecutionManifest(value) => {
            hash.update(b"paper-parent-execution-manifest/v1\n");
            hash.update(encoded(value, work)?);
        }
        HashInput::Raw(bytes) => hash.update(bytes),
        HashInput::Cutover(bytes) => {
            hash.update(b"paper-book-v2-cutover-manifest/v1\n");
            hash.update(bytes);
        }
        HashInput::V1Event {
            account,
            seq,
            command,
            previous,
            payload,
        } => hash.update(encoded(
            &("PAPER_EVENT_V1", account, seq, command, previous, payload),
            work,
        )?),
        HashInput::GenesisEvent {
            account,
            command,
            previous,
            payload,
        } => {
            hash.update(b"paper-book-v2-genesis-event/v1\n");
            hash.update(encoded(
                &(account, 1_i64, command, previous, payload),
                work,
            )?);
        }
        HashInput::ExecutionEvent {
            account,
            seq,
            command,
            previous,
            payload,
        } => {
            hash.update(b"paper-parent-event/v1\n");
            hash.update(encoded(&(account, seq, command, previous, payload), work)?);
        }
    }
    work.hex_digest(&hash.finalize().into())
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum CodecFixtureCase {
    Unit(UnitFamily, UnitRepresentation, UnitOrigin),
    Boundary(BoundaryCase),
    Compatibility(CompatCase),
    OptionPresence(OptionField, OptionForm),
    ReachedLeaf(ReachedLeafCase, UnitOrigin),
    AdjacentUnit(AdjacentUnitEffect, AdjacentUnitForm, UnitOrigin),
    SeedFault(SeedFaultCase),
    Branch(BranchCase),
    OrderedScratch,
    UnitScratch(UnitWireCase),
    GenericControl(GenericControlCase),

    AllRoots,
    Variants,
    BufferedUnits,
    UnknownContent,
    ScalarBoundaries,
    Copies,
    CanonicalHashes,
    Scratch,
    Cumulative,
    Qualification,
    RejectBufferedIgnored,
    RejectDuplicate,
    RejectDirectUnitMap,
    RejectAdjacentUnitMap,
    RejectSequence,
    RejectNumeric,
    RejectDate,
    RejectTypedDepth,
    RejectOutputBudget,
    RejectNoncanonical,
    RejectUnknownCommand,
}
#[cfg(test)]
#[path = "paper_replay_codec_v1_tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) fn run_fixture(case: CodecFixtureCase, work: &mut CodecMechanics<'_, '_>) {
    tests::run(case, work)
}

// This macro emits owner-local visitors, not a DeserializeOwned or allocator callback.
macro_rules! record_read {
 ($de:ident, $input:ident, $ty:ty, $array:literal, {$($field:ident : $ft:ty => $default:literal),* $(,)?}, $construct:expr) => {{
  struct RecordVisitor<'de,'w,'loan,'pool>(c::Input<'de,'w,'loan,'pool>);
  impl<'de> serde::de::Visitor<'de> for RecordVisitor<'de,'_,'_,'_> {
   type Value=$ty;
   fn expecting(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{c::expected(f)}
   fn visit_map<A:serde::de::MapAccess<'de>>(mut self,mut map:A)->Result<Self::Value,A::Error>{
    $(let mut $field:Option<$ft>=None;)*
    let mut spans=self.0.span.children(self.0.bytes); let origin=self.0.origin;
    while let Some(key)=map.next_key_seed(c::KeySeed{names:&[$(stringify!($field)),*]})? {
     let span=spans.next().ok_or_else(c::span_error)?.1;
     match key {$(stringify!($field)=>{if $field.is_some(){return Err(self.0.error(c::K::DuplicateField,"codec duplicate"));}
      $field=Some(map.next_value_seed(c::Seed::<$ft>::new(self.0.child(span,origin)))?);},)*
      _=>{let _:serde::de::IgnoredAny=map.next_value()?;}}
    }
    $(let $field=$field.or_else(<$ft as c::Value>::missing).ok_or_else(||self.0.error(c::K::MissingField,"codec missing"))?;)*
    Ok($construct)
   }
   fn visit_seq<A:serde::de::SeqAccess<'de>>(mut self,mut seq:A)->Result<Self::Value,A::Error>{
    if !$array{return Err(self.0.error(c::K::UnexpectedType,"codec type"));}
    let mut spans=self.0.span.children(self.0.bytes); let origin=self.0.origin;
    $(let $field: $ft=if let Some((_,span))=spans.next(){seq.next_element_seed(c::Seed::<$ft>::new(self.0.child(span,origin)))?.ok_or_else(c::span_error)?}
      else if $default{<$ft as c::Value>::missing().ok_or_else(||self.0.error(c::K::MissingField,"codec missing"))?}
      else{return Err(self.0.error(c::K::MissingField,"codec missing"));};)*
    Ok($construct)
   }
  }
  if $array{$de.deserialize_struct("codec",&[$(stringify!($field)),*],RecordVisitor($input))}else{$de.deserialize_any(RecordVisitor($input))}
 }};
}
pub(crate) use record_read;
pub(crate) struct TagSeed<'de, 'w, 'loan, 'pool> {
    pub(crate) names: &'static [&'static str],
    pub(crate) input: Input<'de, 'w, 'loan, 'pool>,
}
impl<'de> DeserializeSeed<'de> for TagSeed<'de, '_, '_, '_> {
    type Value = &'static str;
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        de.deserialize_enum("codec", self.names, self)
    }
}
impl<'de> Visitor<'de> for TagSeed<'de, '_, '_, '_> {
    type Value = &'static str;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        expected(f)
    }
    fn visit_enum<A: EnumAccess<'de>>(mut self, a: A) -> Result<Self::Value, A::Error> {
        let (tag, value) = a.variant_seed(KeySeed { names: self.names })?;
        if self.input.bytes[self.input.span.start] == b'{' {
            let span = self
                .input
                .span
                .children(self.input.bytes)
                .next()
                .ok_or_else(span_error)?
                .1;
            let origin = self.input.origin;
            value.newtype_variant_seed(UnitPayload(self.input.child(span, origin)))?;
        } else {
            value.unit_variant()?;
        }
        Ok(tag)
    }
}

#[cfg(test)]
pub(crate) fn exercise_root<T: Root + serde::de::DeserializeOwned>(
    case: CodecFixtureCase,
    work: &mut CodecMechanics<'_, '_>,
) {
    tests::exercise_root::<T>(case, work)
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum BoundaryCase {
    FramesExact,
    FramesShort,
    Escrow8Exact,
    Escrow8Short,
    Escrow4Exact,
    Escrow4Short,
    ScratchExact,
    ScratchShort,
    VectorExact,
    VectorShort,
    MapExact,
    MapShort,
    OutputExact,
    OutputShort,
    LengthOverflow,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeedFaultCase {
    UnitMap,
    StringAfterUnit,
    Vector,
    Map,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum BranchCase {
    BufferedOtherDisposition,
    V1Order,
    V1Adjudication,
    V1Snapshot,
    ExecutionSubmit,
    ExecutionFill,
    ExecutionEvaluate,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum UnitWireCase {
    Bare,
    Null,
    EmptyMap,
    EscapedMap,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum GenericControlCase {
    Finite,
    Shallow,
    InvalidUtf8,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum CompatCase {
    BoolFalse,
    BoolWrong,
    I64Min,
    I64Max,
    I64Under,
    I64Over,
    U32Zero,
    U32Max,
    U32Over,
    U32Negative,
    I32Min,
    I32Max,
    I32Under,
    I32Over,
    U8Max,
    U8Over,
    U8Negative,
    FloatNegativeZero,
    FloatSubnormal,
    FloatExponent,
    FloatOverflow,
    FloatNonfiniteSerialize,
    OptionsNone,
    OptionalDateNone,
    FloatOptionNone,
    DateRelaxed,
    DateInvalid,
    DateTimeOffset,
    DuplicateDecodedMap,
    IgnoredUtf8,
    TypedUtf8,
    UnitDirectNull,
    UnitBufferedNull,
    UnitDirectMap,
    UnitBufferedMap,
    UnitDirectNonempty,
    UnitDirectArray,
    UnitBufferedNonempty,
    UnitBufferedArray,
    AdjacentAbsent,
    AdjacentNull,
    AdjacentArray,
    AdjacentArrayMissing,
    AdjacentArrayExtra,
    AdjacentNumericTag,
    AdjacentMapTag,
    AdjacentContentArray,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum UnitFamily {
    ProfitPolicy,
    LotDisposition,
    Side,
    TimeInForce,
    ParentStatus,
    NoFillReason,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum UnitRepresentation {
    Bare,
    Null,
    EmptyMap,
    NonemptyMap,
    Array,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum UnitOrigin {
    Direct,
    Buffered,
}
#[cfg(test)]
pub(crate) fn fixed_snapshot_fixture() -> Vec<u8> {
    tests::snapshot_fixture()
}

// Closed cfg(test) cases only: no caller budget, callback, new root or capability.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum OptionField {
    LotReportedCost,
    SeedSellableFrom,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum OptionForm {
    ObjectAbsent,
    ObjectNull,
    PositionalNull,
    PositionalMissing,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum ReachedLeafCase {
    OpenU8Max,
    OpenU8Over,
    OpenU32Max,
    OpenU32Over,
    OpenI64Min,
    OpenI64Under,
    OpenDateRelaxed,
    OpenDateInvalid,
    OpenNullOption,
    SubmitU32Max,
    SubmitU32Over,
    SubmitI64Max,
    SubmitI64Over,
    SubmitBoolFalse,
    SubmitBoolWrong,
    SubmitDateRelaxed,
    SubmitDateInvalid,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum AdjacentUnitEffect {
    Opened,
    Cancelled,
    Expired,
    Marks,
}
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum AdjacentUnitForm {
    Absent,
    Null,
    EmptyMap,
    Array,
    NonemptyMap,
}

/// A plan and its original work never travel as independently rebindable values.
/// Construction is called only by the private owned paid-row frame.
pub(crate) struct RawScanLoan<'row, 'loan, 'pool> {
    raw: &'row str,
    work: &'row mut super::paper_replay_financial_work_v1::FinancialWork<'loan, 'pool>,
    outcome: Result<shapes::RawPaperArrayPlan, shapes::RawFault>,
}
impl<'row, 'loan, 'pool> RawScanLoan<'row, 'loan, 'pool> {
    pub(super) fn from_paid_frame(
        view: super::paper_replay_financial_work_v1::PaidRawView<'row, 'loan, 'pool>,
    ) -> Self {
        let (raw, work) = view.split();
        Self {
            raw,
            work,
            outcome: shapes::scan_paid_raw_array(raw),
        }
    }
    pub(crate) fn require_decoded(&mut self) -> super::paper_replay_financial_work_v1::Result<()> {
        use super::paper_replay_financial_work_v1::HistoryText;
        self.work.finish()?;
        match self.outcome {
            Ok(_) => Ok(()),
            Err(fault) => {
                let text = self.work.history_text(HistoryText::RawFault {
                    raw: self.raw,
                    fault,
                })?;
                Err(super::paper_ledger::LedgerError::IntegrityFailure(text).into())
            }
        }
    }
    pub(crate) fn len(&mut self) -> super::paper_replay_financial_work_v1::Result<usize> {
        self.require_decoded()?;
        Ok(self.outcome.as_ref().expect("checked outcome").len())
    }
    pub(crate) fn text(
        &mut self,
        index: usize,
    ) -> super::paper_replay_financial_work_v1::Result<Option<String>> {
        use super::paper_replay_financial_work_v1::HistoryText;
        self.require_decoded()?;
        match self.outcome.as_ref().expect("checked outcome").slot(index) {
            Some(shapes::RawSlot::Text(text)) => self
                .work
                .history_text(HistoryText::RawDecoded {
                    raw: self.raw,
                    text,
                })
                .map(Some),
            _ => Ok(None),
        }
    }
    pub(crate) fn text_equal(
        &mut self,
        index: usize,
        expected: &str,
    ) -> super::paper_replay_financial_work_v1::Result<bool> {
        self.require_decoded()?;
        Ok(
            match self.outcome.as_ref().expect("checked outcome").slot(index) {
                Some(shapes::RawSlot::Text(text)) => text.chars(self.raw).eq(expected.chars()),
                _ => false,
            },
        )
    }
    pub(crate) fn number(
        &mut self,
        index: usize,
    ) -> super::paper_replay_financial_work_v1::Result<Option<shapes::RawNumber>> {
        self.require_decoded()?;
        Ok(
            match self.outcome.as_ref().expect("checked outcome").slot(index) {
                Some(shapes::RawSlot::Number(n)) => Some(n),
                _ => None,
            },
        )
    }
    pub(crate) fn is_null(
        &mut self,
        index: usize,
    ) -> super::paper_replay_financial_work_v1::Result<bool> {
        self.require_decoded()?;
        Ok(matches!(
            self.outcome.as_ref().expect("checked outcome").slot(index),
            Some(shapes::RawSlot::Null)
        ))
    }
}

pub(crate) enum HistoryOutput<'a> {
    Event {
        account: &'a str,
        seq: i64,
        command: &'a str,
        previous: &'a str,
        payload: &'a str,
    },
    Audit {
        previous: &'a str,
        row: &'a crate::database::order_audit::CanonicalOrderAuditRow,
    },
    FrozenAll(&'a [crate::database::attribution_epochs::FrozenPaperFill]),
    FrozenFilled(&'a [crate::database::attribution_epochs::FrozenPaperFill]),
    FrozenCarry(&'a [crate::performance::attribution_epoch::LegacyCarryPosition]),
    TerminalBindings(&'a [crate::database::attribution_epochs::TerminalBindingManifestItem]),
    Snapshot(&'a super::paper_ledger::SnapshotRevision),
    ExtraAdjudication(&'a super::paper_ledger::Adjudication),
    Legacy {
        source: &'a str,
        rows: &'a [crate::performance::economic_position::EconomicFillRow],
        unavailable: &'a Option<String>,
    },
}
impl HistoryOutput<'_> {
    fn write<W: io::Write>(&self, out: W) -> serde_json::Result<()> {
        match self {
            Self::Event {
                account,
                seq,
                command,
                previous,
                payload,
            } => serde_json::to_writer(
                out,
                &("PAPER_EVENT_V1", account, seq, command, previous, payload),
            ),
            Self::Audit { row, .. } => serde_json::to_writer(out, row),
            Self::FrozenAll(rows) | Self::FrozenFilled(rows) => serde_json::to_writer(out, rows),
            Self::FrozenCarry(rows) => serde_json::to_writer(out, rows),
            Self::TerminalBindings(rows) => serde_json::to_writer(out, rows),
            Self::Snapshot(revision) => serde_json::to_writer(
                out,
                &(
                    "PaperSnapshotNetFifoV1",
                    revision.target_date,
                    &revision.projection,
                    &revision.metrics,
                    &revision.opening_exclusions,
                    revision.account_realized_pnl,
                ),
            ),
            Self::ExtraAdjudication(request) => serde_json::to_writer(out, request),
            Self::Legacy {
                source,
                rows,
                unavailable,
            } => serde_json::to_writer(out, &("LegacyEconomicV1", source, rows, unavailable)),
        }
    }
    pub(crate) fn historical_bytes(&self) -> serde_json::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.write(&mut bytes)?;
        Ok(bytes)
    }
    pub(crate) fn digest(&self, bytes: &[u8]) -> [u8; 32] {
        let mut hash = Sha256::new();
        let domain: Option<&[u8]> = match self {
            Self::FrozenAll(_) => Some(b"BR255_ATTRIBUTION_ALL_STATUS_PAPER_MANIFEST_V1\0"),
            Self::FrozenFilled(_) => Some(b"BR255_ATTRIBUTION_LEGACY_FILLED_MANIFEST_V1\0"),
            Self::FrozenCarry(_) => Some(b"BR255_ATTRIBUTION_POSITION_PROJECTION_V1\0"),
            Self::TerminalBindings(_) => Some(b"BR255_ATTRIBUTION_TERMINAL_BINDING_MANIFEST_V1\0"),
            _ => None,
        };
        if let Some(domain) = domain {
            hash.update(domain);
            hash.update((bytes.len() as u64).to_be_bytes());
        }
        if let Self::Audit { previous, .. } = self {
            hash.update(b"BR086_ORDER_AUDIT_V1\0");
            hash.update(previous.as_bytes());
            hash.update(b"\0");
        }
        hash.update(bytes);
        hash.finalize().into()
    }
}
pub(crate) fn encode_history_output(
    input: &HistoryOutput<'_>,
    work: &mut CodecMechanics<'_, '_>,
) -> Result<Vec<u8>, ReplayTerminalFailure> {
    work.serializer_escrow()?;
    let mut count = Counter(0);
    input
        .write(&mut count)
        .map_err(|_| work.refuse(K::RecordExtent, None))?;
    let mut bytes = work.output(count.0)?;
    input
        .write(Output {
            bytes: &mut bytes,
            limit: count.0,
        })
        .map_err(|_| work.refuse(K::PlanMismatch, None))?;
    if bytes.len() != count.0 {
        return Err(work.refuse(K::PlanMismatch, None));
    }
    Ok(bytes)
}
pub(crate) fn seed_binding_digest(
    value: &super::paper_ledger::SeedManifest,
    work: &mut CodecMechanics<'_, '_>,
) -> Result<String, ReplayTerminalFailure> {
    digest_core(HashInput::SeedBinding(value), work)
}

impl RawScanLoan<'_, '_, '_> {
    pub(crate) fn current_timestamp(
        &mut self,
        id: i64,
    ) -> super::paper_replay_financial_work_v1::Result<chrono::DateTime<chrono::Utc>> {
        let text = self.text(13)?;
        super::paper_ledger::parse_raw_timestamp_with_work(id, text.as_deref(), self.work)
    }
    pub(crate) fn current_decision_prefix(
        &mut self,
        decision: &str,
    ) -> super::paper_replay_financial_work_v1::Result<bool> {
        let text = self.text(10)?;
        super::paper_ledger::raw_decision_prefix_with_work(text.as_deref(), decision, self.work)
    }
    pub(crate) fn current_contradiction(
        &mut self,
    ) -> super::paper_replay_financial_work_v1::Result<
        super::paper_replay_financial_work_v1::FinancialFailure,
    > {
        super::paper_ledger::raw_contradiction_with_work(self.work)
    }
}
