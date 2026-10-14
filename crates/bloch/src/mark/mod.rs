mod json;
pub mod lc;
mod string;

pub(crate) use json::Json;
pub use string::{FixedString, StringView};

use crate::{
    Bf16Data, Date16Data, Date32Data, DateTime32Data, DateTime64Data, Decimal32Data, Decimal64Data,
    Decimal128Data, Decimal256Data, Error, I256, Ipv4Data, Ipv6Data, TinyRange, U256, UuidData,
    interval,
    macros::{define_int_getters, define_ip_getters, define_opt_getters, define_slice_fns},
    slice::ByteView,
    types::{OffsetIndexPair as _, Offsets},
    value::{MapIterator, Value},
    zc,
};
use bstr::BStr;
use chrono::{DateTime as ChronoDateTime, TimeDelta, TimeZone};
use chrono_tz::Tz;
use core::fmt;
use std::{borrow::Cow, fmt::Debug, hint::cold_path, marker::PhantomData, ops::Range};
use uuid::Uuid;

pub enum Mark<'a> {
    Empty,
    // It's enough to know the row count only
    Nothing(usize),
    Bool(BoolView<'a>),
    Int8(ByteView<'a, i8>),
    Int16(ByteView<'a, zc::I16>),
    Int32(ByteView<'a, zc::I32>),
    Int64(ByteView<'a, zc::I64>),
    Int128(ByteView<'a, zc::I128>),
    Int256(ByteView<'a, I256>),
    UInt8(ByteView<'a, u8>),
    UInt16(ByteView<'a, zc::U16>),
    UInt32(ByteView<'a, zc::U32>),
    UInt64(ByteView<'a, zc::U64>),
    UInt128(ByteView<'a, zc::U128>),
    UInt256(ByteView<'a, U256>),
    Float32(ByteView<'a, zc::F32>),
    Float64(ByteView<'a, zc::F64>),
    BFloat16(ByteView<'a, Bf16Data>),
    Decimal32(Decimal32<'a>),
    Decimal64(Decimal64<'a>),
    Decimal128(Decimal128<'a>),
    Decimal256(Decimal256<'a>),
    String(StringView<'a>),
    FixedString(FixedString<'a>),
    Uuid(ByteView<'a, UuidData>),
    Date(ByteView<'a, Date16Data>),
    Date32(ByteView<'a, Date32Data>),
    DateTime(DateTime<'a>),
    DateTime64(DateTime64<'a>),
    Time(ByteView<'a, zc::I32>),
    Time64(Time64<'a>),
    Interval(Interval<'a>),
    Ipv4(ByteView<'a, Ipv4Data>),
    Ipv6(ByteView<'a, Ipv6Data>),

    Enum8(Enum8<'a>),
    Enum16(Enum16<'a>),

    LowCardinality(lc::LowCardinality<'a>),
    Array(Array<'a>),
    Tuple(Tuple<'a>),
    Nullable(Nullable<'a>),
    Map(Map<'a>),
    Variant(Variant<'a>),
    Nested(Nested<'a>),
    NamedTuple(NamedTuple<'a>),
    Dynamic(Dynamic<'a>),

    Json(Json<'a>),
}

impl<'a> Mark<'a> {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Mark::Empty => "Empty",
            Mark::Nothing(_) => "Nothing",
            Mark::Bool(_) => "Bool",
            Mark::Int8(_) => "Int8",
            Mark::Int16(_) => "Int16",
            Mark::Int32(_) => "Int32",
            Mark::Int64(_) => "Int64",
            Mark::Int128(_) => "Int128",
            Mark::Int256(_) => "Int256",
            Mark::UInt8(_) => "UInt8",
            Mark::UInt16(_) => "UInt16",
            Mark::UInt32(_) => "UInt32",
            Mark::UInt64(_) => "UInt64",
            Mark::UInt128(_) => "UInt128",
            Mark::UInt256(_) => "UInt256",
            Mark::Float32(_) => "Float32",
            Mark::Float64(_) => "Float64",
            Mark::BFloat16(_) => "BFloat16",
            Mark::Decimal32(_) => "Decimal32",
            Mark::Decimal64(_) => "Decimal64",
            Mark::Decimal128(_) => "Decimal128",
            Mark::Decimal256(_) => "Decimal256",
            Mark::String(_) => "String",
            Mark::FixedString(_) => "FixedString",
            Mark::Uuid(_) => "Uuid",
            Mark::Date(_) => "Date",
            Mark::Date32(_) => "Date32",
            Mark::DateTime(_) => "DateTime",
            Mark::DateTime64(_) => "DateTime64",
            Mark::Time(_) => "Time",
            Mark::Time64(_) => "Time64",
            Mark::Interval(i) => i.kind.as_str(),
            Mark::Ipv4(_) => "Ipv4",
            Mark::Ipv6(_) => "Ipv6",
            Mark::Enum8(_) => "Enum8",
            Mark::Enum16(_) => "Enum16",
            Mark::LowCardinality(_) => "LowCardinality",
            Mark::Array(_) => "Array",
            Mark::Tuple(_) => "Tuple",
            Mark::Nullable(_) => "Nullable",
            Mark::Map(_) => "Map",
            Mark::Variant(_) => "Variant",
            Mark::Nested(_) => "Nested",
            Mark::NamedTuple(_) => "NamedTuple",
            Mark::Dynamic(_) => "Dynamic",
            Mark::Json(_) => "Json",
        }
    }

    #[inline(always)]
    fn checked_slice<T>(&self, data: &'a [T], range: Range<usize>) -> crate::Result<&'a [T]> {
        let Some(slice) = data.get(range.clone()) else {
            cold_path();
            return Err(Error::RangeOutOfBounds(range, self.as_str()));
        };
        Ok(slice)
    }

    #[inline(always)]
    fn checked_range(&self, len: usize, range: Range<usize>) -> crate::Result<TinyRange> {
        if range.start > range.end || range.end > len {
            cold_path();
            return Err(Error::RangeOutOfBounds(range, self.as_str()));
        }
        range.try_into()
    }

    pub fn len(&self) -> usize {
        match self {
            Mark::Empty => 0,
            Mark::Nothing(len) => *len,
            Mark::Bool(bv) => bv.data.len(),
            Mark::Int8(bv) => bv.len(),
            Mark::Int16(bv) => bv.len(),
            Mark::Int32(bv) | Mark::Time(bv) => bv.len(),
            Mark::Int64(bv) => bv.len(),
            Mark::Int128(bv) => bv.len(),
            Mark::Int256(bv) => bv.len(),
            Mark::UInt8(bv) => bv.len(),
            Mark::UInt16(bv) => bv.len(),
            Mark::UInt32(bv) => bv.len(),
            Mark::UInt64(bv) => bv.len(),
            Mark::UInt128(bv) => bv.len(),
            Mark::UInt256(bv) => bv.len(),
            Mark::Float32(bv) => bv.len(),
            Mark::Float64(bv) => bv.len(),
            Mark::BFloat16(bv) => bv.len(),
            Mark::Decimal32(d) => d.data.len(),
            Mark::Decimal64(d) => d.data.len(),
            Mark::Decimal128(d) => d.data.len(),
            Mark::Decimal256(d) => d.data.len(),
            Mark::String(sv) => sv.data.len(),
            Mark::FixedString(fs) => fs.len(),
            Mark::Uuid(bv) => bv.len(),
            Mark::Date(bv) => bv.len(),
            Mark::Date32(bv) => bv.len(),
            Mark::DateTime(d) => d.data.len(),
            Mark::DateTime64(d) => d.data.len(),
            Mark::Time64(t) => t.data.len(),
            Mark::Interval(i) => i.data.len(),
            Mark::Ipv4(bv) => bv.len(),
            Mark::Ipv6(bv) => bv.len(),
            Mark::Enum8(e) => e.data.len(),
            Mark::Enum16(e) => e.data.len(),
            Mark::LowCardinality(lc) => lc.indices.len(),
            Mark::Array(a) => a.offsets.len(),
            Mark::Tuple(t) => t.len(),
            Mark::Nullable(n) => n.mask.len(),
            Mark::Map(m) => m.offsets.len(),
            Mark::Variant(v) => v.discriminators.len(),
            Mark::Nested(n) => n.array_of_tuples.len(),
            Mark::NamedTuple(n) => n.tuple.len(),
            Mark::Dynamic(d) => d.discriminators.len(),
            Mark::Json(j) => j.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&'a self, index: usize) -> crate::Result<Option<Value<'a>>> {
        match self {
            Mark::Empty => Ok(None),
            Mark::Nothing(len) => Ok((index < *len).then_some(Value::Empty)),
            Mark::Bool(b) => Ok(b.get(index).map(Value::Bool)),
            Mark::Int8(bv) => Ok(bv.get(index).copied().map(Value::Int8)),
            Mark::Int16(bv) => Ok(bv.get(index).map(|v| v.get()).map(Value::Int16)),
            Mark::Int32(bv) => Ok(bv.get(index).map(|v| v.get()).map(Value::Int32)),
            Mark::Int64(bv) => Ok(bv.get(index).map(|v| v.get()).map(Value::Int64)),
            Mark::Int128(bv) => Ok(bv.get(index).map(Value::Int128)),
            Mark::Int256(bv) => Ok(bv.get(index).map(Value::Int256)),
            Mark::UInt8(bv) => Ok(bv.get(index).copied().map(Value::UInt8)),
            Mark::UInt16(bv) => Ok(bv.get(index).map(|v| v.get()).map(Value::UInt16)),
            Mark::UInt32(bv) => Ok(bv.get(index).map(|v| v.get()).map(Value::UInt32)),
            Mark::UInt64(bv) => Ok(bv.get(index).map(|v| v.get()).map(Value::UInt64)),
            Mark::UInt128(bv) => Ok(bv.get(index).map(Value::UInt128)),
            Mark::UInt256(bv) => Ok(bv.get(index).map(Value::UInt256)),
            Mark::Float32(bv) => Ok(bv.get(index).map(|v| v.get()).map(Value::Float32)),
            Mark::Float64(bv) => Ok(bv.get(index).map(|v| v.get()).map(Value::Float64)),
            Mark::BFloat16(bv) => Ok(bv.get(index).copied().map(Into::into).map(Value::BFloat16)),
            Mark::Decimal32(d) => Ok(d.get(index)),
            Mark::Decimal64(d) => Ok(d.get(index)),
            Mark::Decimal128(d) => Ok(d.get(index)),
            Mark::Decimal256(d) => Ok(d.get(index)),
            Mark::String(strings) => Ok(strings.get(index).map(Value::String)),
            Mark::FixedString(fs) => Ok(fs.get(index)),
            Mark::Uuid(bv) => Ok(bv.get(index).map(Value::Uuid)),
            Mark::Date(bv) => Ok(bv.get(index).copied().map(Into::into).map(Value::Date)),
            Mark::Date32(bv) => bv
                .get(index)
                .copied()
                .map(chrono::NaiveDate::try_from)
                .transpose()
                .map(|date| date.map(Value::Date32)),
            Mark::DateTime(d) => Ok(d.get(index)),
            Mark::DateTime64(d) => Ok(d.get(index)),
            Mark::Time(bv) => Ok(bv
                .get(index)
                .map(|v| TimeDelta::seconds(i64::from(v.get())))
                .map(Value::Time)),
            Mark::Time64(t) => Ok(t.get(index)),
            Mark::Interval(i) => Ok(i.get(index)),
            Mark::Ipv4(data) => Ok(data.get(index).copied().map(Into::into).map(Value::Ipv4)),
            Mark::Ipv6(data) => Ok(data.get(index).map(Value::Ipv6)),
            Mark::Enum8(v) => Ok(v.get(index)),
            Mark::Enum16(v) => Ok(v.get(index)),
            Mark::LowCardinality(lc) => lc.get(index),
            Mark::Array(a) => a.get(index),
            Mark::Tuple(t) => Ok(t.get(index)),
            Mark::Nullable(n) => n.get(index),
            Mark::Map(m) => Ok(m.get(index)),
            Mark::Variant(v) => v.get(index),
            Mark::Nested(n) => n.get(index),
            Mark::NamedTuple(n) => n.get(index),
            Mark::Dynamic(d) => d.get(index),
            Mark::Json(j) => Ok(j.get(index)),
        }
    }

    pub fn slice(&'a self, idx: Range<usize>) -> crate::Result<Value<'a>> {
        match self {
            Mark::Empty => {
                if idx.start != 0 || idx.end != 0 {
                    cold_path();
                    return Err(Error::RangeOutOfBounds(idx, self.as_str()));
                }
                Ok(Value::Empty)
            }
            Mark::Nothing(len) => {
                self.checked_range(*len, idx)?;
                Ok(Value::NothingSlice)
            }
            Mark::Bool(bv) => Ok(Value::BoolSlice(self.checked_slice(bv.data, idx)?)),
            Mark::Int8(bv) => Ok(Value::Int8Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Int16(bv) => Ok(Value::Int16Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Int32(bv) => Ok(Value::Int32Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Int64(bv) => Ok(Value::Int64Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Int128(bv) => Ok(Value::Int128Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Int256(bv) => Ok(Value::Int256Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::UInt8(bv) => Ok(Value::UInt8Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::UInt16(bv) => Ok(Value::UInt16Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::UInt32(bv) => Ok(Value::UInt32Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::UInt64(bv) => Ok(Value::UInt64Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::UInt128(bv) => Ok(Value::UInt128Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::UInt256(bv) => Ok(Value::UInt256Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Float32(bv) => Ok(Value::Float32Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Float64(bv) => Ok(Value::Float64Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::BFloat16(bv) => Ok(Value::BFloat16Slice(
                self.checked_slice(bv.as_slice(), idx)?,
            )),
            Mark::Uuid(bv) => Ok(Value::UuidSlice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Date(bv) => Ok(Value::Date16Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Date32(bv) => Ok(Value::Date32Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Ipv4(bv) => Ok(Value::Ipv4Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Ipv6(bv) => Ok(Value::Ipv6Slice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::String(sv) => Ok(Value::StringSlice(self.checked_slice(&sv.data, idx)?)),

            Mark::Decimal32(d) => Ok(Value::Decimal32Slice {
                scale: d.scale,
                slice: self.checked_slice(d.data.as_slice(), idx)?,
            }),
            Mark::Decimal64(d) => Ok(Value::Decimal64Slice {
                scale: d.scale,
                slice: self.checked_slice(d.data.as_slice(), idx)?,
            }),
            Mark::Decimal128(d) => Ok(Value::Decimal128Slice {
                scale: d.scale,
                slice: self.checked_slice(d.data.as_slice(), idx)?,
            }),
            Mark::Decimal256(d) => Ok(Value::Decimal256Slice {
                scale: d.scale,
                slice: self.checked_slice(d.data.as_slice(), idx)?,
            }),
            Mark::FixedString(mark) => Ok(Value::FixedStringSlice {
                mark,
                range: self.checked_range(self.len(), idx)?,
            }),

            Mark::DateTime(d) => Ok(Value::DateTime32Slice {
                tz: d.tz,
                slice: self.checked_slice(d.data.as_slice(), idx)?,
            }),
            Mark::DateTime64(d) => Ok(Value::DateTime64Slice {
                precision: d.precision,
                tz: d.tz,
                slice: self.checked_slice(d.data.as_slice(), idx)?,
            }),
            Mark::Time(bv) => Ok(Value::TimeSlice(self.checked_slice(bv.as_slice(), idx)?)),
            Mark::Time64(t) => Ok(Value::Time64Slice {
                precision: t.precision,
                slice: self.checked_slice(t.data.as_slice(), idx)?,
            }),
            Mark::Interval(i) => Ok(Value::IntervalSlice {
                kind: i.kind,
                slice: self.checked_slice(i.data.as_slice(), idx)?,
            }),
            Mark::Enum8(mark) => Ok(Value::Enum8Slice {
                mark,
                range: self.checked_range(mark.data.len(), idx)?,
            }),
            Mark::Enum16(mark) => Ok(Value::Enum16Slice {
                mark,
                range: self.checked_range(mark.data.len(), idx)?,
            }),
            Mark::LowCardinality(mark) => Ok(Value::LowCardinalitySlice {
                range: self.checked_range(mark.indices.len(), idx)?,
                mark,
            }),
            Mark::Array(mark) => Ok(Value::ArraySlice {
                mark,
                range: self.checked_range(mark.offsets.len(), idx)?,
            }),
            Mark::Tuple(mark) => Ok(Value::TupleSlice {
                mark,
                range: self.checked_range(self.len(), idx)?,
            }),
            Mark::Nullable(mark) => Ok(Value::NullableSlice {
                mark,
                range: self.checked_range(mark.mask.len(), idx)?,
            }),
            Mark::Map(mark) => Ok(Value::MapSlice {
                mark,
                range: self.checked_range(mark.offsets.len(), idx)?,
            }),
            Mark::Nested(mark) => Ok(Value::NestedSlice {
                mark,
                range: self.checked_range(mark.array_of_tuples.len(), idx)?,
            }),
            Mark::NamedTuple(mark) => Ok(Value::NamedTupleSlice {
                mark,
                range: self.checked_range(mark.tuple.len(), idx)?,
            }),
            Mark::Variant(mark) => Ok(Value::VariantSlice {
                mark,
                range: self.checked_range(mark.discriminators.len(), idx)?,
            }),
            Mark::Dynamic(mark) => Ok(Value::DynamicSlice {
                mark,
                range: self.checked_range(mark.discriminators.len(), idx)?,
            }),
            Mark::Json(mark) => Ok(Value::JsonSlice {
                mark,
                range: self.checked_range(mark.len(), idx)?,
            }),
        }
    }

    #[inline(always)]
    pub fn get_str(&self, index: usize) -> crate::Result<Option<&'a BStr>> {
        match self {
            Mark::String(strings) => Ok(strings.get(index)),
            Mark::FixedString(fs) => Ok(fs.get_bstr(index)),
            Mark::LowCardinality(lc) => lc.get_str(index),
            mark => {
                cold_path();
                Err(Error::MismatchedType(mark.as_str(), "&BStr"))
            }
        }
    }

    /// Outer `None`: index out of range. Inner `None`: NULL.
    #[inline(always)]
    pub fn get_opt_str(&self, index: usize) -> crate::Result<Option<Option<&'a BStr>>> {
        let nullable = match self {
            Mark::Nullable(nullable) => nullable,
            Mark::LowCardinality(lc) => return lc.get_opt_str(index),
            // convenience wrapper for non-nullable columns
            mark => {
                return match mark.get_str(index)? {
                    Some(value) => Ok(Some(Some(value))),
                    None => Ok(None),
                };
            }
        };

        match nullable.is_null(index) {
            None => Ok(None),
            Some(true) => Ok(Some(None)),
            Some(false) => match nullable.data.get_str(index)? {
                Some(value) => Ok(Some(Some(value))),
                None => Ok(None),
            },
        }
    }

    #[expect(clippy::needless_pass_by_value)]
    #[inline(always)]
    pub fn get_datetime<T: TimeZone>(
        &self,
        index: usize,
        tz: T,
    ) -> crate::Result<Option<ChronoDateTime<T>>> {
        match self {
            Mark::DateTime(d) => {
                let value = d
                    .data
                    .get(index)
                    .map(|dt| dt.with_tz(d.tz))
                    .map(|dt| dt.with_timezone(&tz));
                Ok(value)
            }
            Mark::DateTime64(d) => {
                let Some(dt) = d.data.get(index) else {
                    return Ok(None);
                };
                let dt = dt.with_tz_and_precision(d.tz, d.precision)?;
                Ok(Some(dt.with_timezone(&tz)))
            }
            _ => {
                cold_path();
                Err(Error::MismatchedType(self.as_str(), "DateTime"))
            }
        }
    }

    #[inline(always)]
    pub(crate) const fn lc(&self) -> crate::Result<&lc::LowCardinality<'a>> {
        match self {
            Mark::LowCardinality(lc) => Ok(lc),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), "LowCardinality"))
            }
        }
    }

    #[inline(always)]
    fn array_elements(&self, index: usize) -> crate::Result<Option<(&Mark<'a>, Range<usize>)>> {
        let array = match self {
            Mark::Array(array) => array,
            Mark::Empty => return Ok(None),
            other => {
                cold_path();
                return Err(Error::MismatchedType(other.as_str(), "Array"));
            }
        };
        Ok(array
            .offsets
            .offset_indices(index)?
            .map(|(start, end)| (array.values.as_ref(), start..end)))
    }

    #[inline(always)]
    pub fn slice_lc_strs(&self, idx: Range<usize>) -> crate::Result<lc::StrIter<'a, '_>> {
        self.lc()?.slice_strs(idx)
    }

    #[inline(always)]
    pub fn slice_lc_opt_strs(&self, idx: Range<usize>) -> crate::Result<lc::OptStrIter<'a, '_>> {
        self.lc()?.slice_opt_strs(idx)
    }

    #[inline(always)]
    pub fn get_array_lc_strs(&self, index: usize) -> crate::Result<Option<lc::StrIter<'a, '_>>> {
        match self.array_elements(index)? {
            Some((values, range)) => values.slice_lc_strs(range).map(Some),
            None => Ok(None),
        }
    }

    #[inline(always)]
    pub fn get_array_lc_opt_strs(
        &self,
        index: usize,
    ) -> crate::Result<Option<lc::OptStrIter<'a, '_>>> {
        match self.array_elements(index)? {
            Some((values, range)) => values.slice_lc_opt_strs(range).map(Some),
            None => Ok(None),
        }
    }

    pub fn get_map<K, V>(&'a self, index: usize) -> crate::Result<Option<MapIterator<'a, K, V>>> {
        let Mark::Map(map) = self else {
            cold_path();
            return Err(Error::MismatchedType(self.as_str(), "Map"));
        };
        let Some((start, end)) = map.offsets.offset_indices(index)? else {
            return Ok(None);
        };

        let it = MapIterator {
            keys: &map.keys,
            values: &map.values,
            range: start..end,
            _marker: PhantomData,
        };

        Ok(Some(it))
    }

    #[inline]
    pub fn get_arr_bool_iter(
        &self,
        index: usize,
    ) -> crate::Result<Option<impl Iterator<Item = bool>>> {
        let Some((values, range)) = self.array_elements(index)? else {
            return Ok(None);
        };
        let slice = match values {
            Mark::Bool(bv) => &bv.data[range],
            other => {
                cold_path();
                return Err(Error::MismatchedType(other.as_str(), "Bool"));
            }
        };
        Ok(Some(slice.iter().copied().map(|b| b != 0)))
    }

    pub fn get_bool(&self, index: usize) -> crate::Result<Option<bool>> {
        match self {
            Mark::Bool(bv) => {
                let value = bv.get(index);
                Ok(value)
            }
            _ => {
                cold_path();
                Err(Error::MismatchedType(self.as_str(), "bool"))
            }
        }
    }

    define_ip_getters!((Ipv4, std::net::Ipv4Addr), (Ipv6, std::net::Ipv6Addr));

    define_int_getters!(
        (Int8, i8, std::convert::identity),
        (Int16, i16, zc::I16::get),
        (Int32, i32, zc::I32::get),
        (Int64, i64, zc::I64::get),
        (Int128, i128, zc::I128::get),
        (UInt8, u8, std::convert::identity),
        (UInt16, u16, zc::U16::get),
        (UInt32, u32, zc::U32::get),
        (UInt64, u64, zc::U64::get),
        (UInt128, u128, zc::U128::get),
        (Float32, f32, zc::F32::get),
        (Float64, f64, zc::F64::get),
        (Uuid, Uuid, Uuid::from),
    );

    define_opt_getters!(
        (Ipv4, std::net::Ipv4Addr),
        (Ipv6, std::net::Ipv6Addr),
        (Uuid, Uuid),
        (i8, i8),
        (i16, i16),
        (i32, i32),
        (i64, i64),
        (i128, i128),
        (u8, u8),
        (u16, u16),
        (u32, u32),
        (u64, u64),
        (u128, u128),
        (f64, f64),
        (f32, f32),
    );

    define_slice_fns!(
        (Int8, i8),
        (Int16, zc::I16),
        (Int32, zc::I32),
        (Int64, zc::I64),
        (Int128, zc::I128),
        (Int256, I256),
        (UInt8, u8),
        (UInt16, zc::U16),
        (UInt32, zc::U32),
        (UInt64, zc::U64),
        (UInt128, zc::U128),
        (UInt256, U256),
        (Float32, zc::F32),
        (Float64, zc::F64),
        (BFloat16, Bf16Data),
        (Uuid, UuidData),
        (Date, Date16Data),
        (Date32, Date32Data),
        (Ipv4, Ipv4Data),
        (Ipv6, Ipv6Data),
    );

    // It borrows the Mark's vec, so can't be done the same way as get_arr_*_slice
    pub fn get_arr_string_slice(&self, index: usize) -> crate::Result<Option<&[&'a BStr]>> {
        let Some((values, range)) = self.array_elements(index)? else {
            return Ok(None);
        };
        match values {
            Mark::String(bv) => Ok(Some(&bv[range])),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), "String"))
            }
        }
    }
}

fn checked_slice<'a, T>(
    data: &'a [T],
    range: Range<usize>,
    kind: &'static str,
) -> crate::Result<&'a [T]> {
    let Some(slice) = data.get(range.clone()) else {
        cold_path();
        return Err(Error::RangeOutOfBounds(range, kind));
    };
    Ok(slice)
}

macro_rules! impl_get {
    ($ty:ident, $variant:ident) => {
        impl<'a> $ty<'a> {
            pub(crate) const fn get(&'a self, index: usize) -> Option<Value<'a>> {
                if self.data.len() <= index {
                    cold_path();
                    None
                } else {
                    Some(Value::$variant(index, self))
                }
            }
        }
    };
}

macro_rules! impl_get_many {
    ($($ty:ident),+ $(,)?) => {
        $( impl_get!($ty, $ty); )+
    };
}

#[derive(Debug)]
pub struct Map<'a> {
    pub(crate) offsets: Offsets<'a>,
    pub(crate) keys: Box<Mark<'a>>,
    pub(crate) values: Box<Mark<'a>>,
}

impl Map<'_> {
    pub(crate) const fn get(&self, index: usize) -> Option<Value<'_>> {
        if index >= self.offsets.len() {
            cold_path();
            return None;
        }
        Some(Value::Map { mark: self, index })
    }
}

#[derive(Debug)]
pub struct Variant<'a> {
    pub(crate) offsets: Box<[u32]>,
    pub(crate) discriminators: &'a [u8],
    pub(crate) types: Box<[Mark<'a>]>,
}

impl Variant<'_> {
    /// Discriminator byte marking a NULL row.
    pub(crate) const NULL_DISCRIMINATOR: u8 = 255;

    /// `Ok(None)` is out of range, `Ok(Some(Value::Empty))` is a NULL row
    pub(crate) fn get(&self, index: usize) -> crate::Result<Option<Value<'_>>> {
        let Some(&discriminator) = self.discriminators.get(index) else {
            return Ok(None);
        };
        if discriminator == Self::NULL_DISCRIMINATOR {
            return Ok(Some(Value::Empty));
        }
        let Some(&in_type_index) = self.offsets.get(index) else {
            return Ok(None);
        };
        let Some(mark) = self.types.get(usize::from(discriminator)) else {
            return Ok(None);
        };
        mark.get(in_type_index as usize)
    }
}

#[derive(Debug)]
pub struct Nested<'a> {
    pub(crate) col_names: Box<[Cow<'a, str>]>,
    pub(crate) array_of_tuples: Box<Mark<'a>>,
}

impl Nested<'_> {
    pub(crate) fn get(&self, index: usize) -> crate::Result<Option<Value<'_>>> {
        // verify the index is present
        if self.array_of_tuples.get(index)?.is_none() {
            return Ok(None);
        }
        Ok(Some(Value::Nested { mark: self, index }))
    }
}

#[derive(Debug)]
pub struct NamedTuple<'a> {
    pub col_names: Box<[Cow<'a, str>]>,
    pub tuple: Box<Mark<'a>>,
}

impl<'a> NamedTuple<'a> {
    pub fn mark(&self, name: &str) -> crate::Result<&Mark<'a>> {
        let Mark::Tuple(tuple) = self.tuple.as_ref() else {
            cold_path();
            return Err(Error::MismatchedType(self.tuple.as_str(), "Tuple"));
        };
        crate::mark_by_name(&self.col_names, &tuple.values, name)
    }

    pub(crate) fn get(&self, index: usize) -> crate::Result<Option<Value<'_>>> {
        if self.tuple.get(index)?.is_none() {
            return Ok(None);
        }
        Ok(Some(Value::NamedTuple { mark: self, index }))
    }
}

#[derive(Debug)]
pub struct Array<'a> {
    pub(crate) offsets: Offsets<'a>,
    pub(crate) values: Box<Mark<'a>>,
}

impl Array<'_> {
    pub(crate) fn get(&self, index: usize) -> crate::Result<Option<Value<'_>>> {
        let Some((start, end)) = self.offsets.offset_indices(index)? else {
            return Ok(None);
        };
        Ok(Some(self.values.slice(start..end)?))
    }
}

#[derive(Debug)]
pub struct Decimal32<'a> {
    pub(crate) scale: u8,
    pub(crate) data: ByteView<'a, Decimal32Data>,
}

#[derive(Debug)]
pub struct Decimal64<'a> {
    pub(crate) scale: u8,
    pub(crate) data: ByteView<'a, Decimal64Data>,
}

#[derive(Debug)]
pub struct Decimal128<'a> {
    pub(crate) scale: u8,
    pub(crate) data: ByteView<'a, Decimal128Data>,
}

#[derive(Debug)]
pub struct Decimal256<'a> {
    pub scale: u8,
    pub data: ByteView<'a, Decimal256Data>,
}

#[derive(Debug)]
pub struct DateTime<'a> {
    pub(crate) tz: Tz,
    pub(crate) data: ByteView<'a, DateTime32Data>,
}

#[derive(Debug)]
pub struct DateTime64<'a> {
    pub(crate) precision: u8,
    pub(crate) tz: Tz,
    pub(crate) data: ByteView<'a, DateTime64Data>,
}

#[derive(Debug)]
pub struct Time64<'a> {
    pub(crate) precision: u8,
    pub(crate) data: ByteView<'a, zc::I64>,
}

#[derive(Debug)]
pub struct Interval<'a> {
    pub(crate) kind: interval::Kind,
    pub(crate) data: ByteView<'a, zc::I64>,
}

impl_get_many!(
    Decimal32, Decimal64, Decimal128, Decimal256, DateTime, DateTime64, Time64, Interval
);

#[derive(Debug)]
pub struct Enum8<'a> {
    pub(crate) variants: Box<[(Cow<'a, str>, i8)]>,
    pub(crate) data: ByteView<'a, i8>,
}

impl Enum8<'_> {
    pub(crate) fn name(&self, index: usize) -> Option<&str> {
        let id = *self.data.get(index)?;
        let slot = self
            .variants
            .binary_search_by_key(&id, |(_, id)| *id)
            .ok()?;
        Some(self.variants[slot].0.as_ref())
    }

    pub(crate) fn get(&self, index: usize) -> Option<Value<'_>> {
        // An undeclared id means broken data, but we trust clickhouse!
        let name = self.name(index)?;
        Some(Value::String(BStr::new(name)))
    }
}

#[derive(Debug)]
pub struct Enum16<'a> {
    pub(crate) variants: Box<[(Cow<'a, str>, i16)]>,
    pub(crate) data: ByteView<'a, zc::I16>,
}

impl Enum16<'_> {
    pub(crate) fn name(&self, index: usize) -> Option<&str> {
        let id = self.data.get(index)?.get();
        let slot = self
            .variants
            .binary_search_by_key(&id, |(_, id)| *id)
            .ok()?;
        Some(self.variants[slot].0.as_ref())
    }

    pub(crate) fn get(&self, index: usize) -> Option<Value<'_>> {
        let name = self.name(index)?;
        Some(Value::String(BStr::new(name)))
    }
}

#[derive(Debug)]
pub struct Dynamic<'a> {
    pub(crate) offsets: Box<[u32]>,
    pub(crate) discriminators: &'a [u8],
    pub(crate) columns: Box<[Mark<'a>]>,
}

impl Dynamic<'_> {
    pub(crate) fn is_null(&self, index: usize) -> bool {
        self.discriminators.get(index) == Some(&Variant::NULL_DISCRIMINATOR)
    }

    /// `Ok(None)` is out of range; `Ok(Some(Value::Empty))` is a NULL row.
    pub(crate) fn get(&self, index: usize) -> crate::Result<Option<Value<'_>>> {
        let Some(&discriminator) = self.discriminators.get(index) else {
            return Ok(None);
        };
        if discriminator == Variant::NULL_DISCRIMINATOR {
            return Ok(Some(Value::Empty));
        }
        let Some(&in_type_index) = self.offsets.get(index) else {
            return Ok(None);
        };
        let Some(mark) = self.columns.get(usize::from(discriminator)) else {
            return Ok(None);
        };
        mark.get(in_type_index as usize)
    }
}

#[derive(Debug)]
pub struct Nullable<'a> {
    pub(crate) mask: &'a [u8],
    pub(crate) data: Box<Mark<'a>>,
}

impl Nullable<'_> {
    pub(crate) const fn len(&self) -> usize {
        self.mask.len()
    }

    #[inline(always)]
    pub(crate) fn is_null(&self, index: usize) -> Option<bool> {
        Some(*self.mask.get(index)? != 0)
    }

    /// # Safety
    /// `index < self.len()`.
    #[inline(always)]
    pub(crate) unsafe fn is_null_unchecked(&self, index: usize) -> bool {
        unsafe { *self.mask.get_unchecked(index) != 0 }
    }

    pub(crate) fn get(&self, index: usize) -> crate::Result<Option<Value<'_>>> {
        match self.is_null(index) {
            None => Ok(None),
            Some(true) => Ok(Some(Value::Empty)),
            Some(false) => self.data.get(index),
        }
    }
}

#[derive(Debug)]
pub struct Tuple<'a> {
    pub values: Box<[Mark<'a>]>,
    pub num_rows: usize,
}

impl Tuple<'_> {
    #[inline]
    pub(crate) const fn len(&self) -> usize {
        self.num_rows
    }

    pub(crate) const fn get(&self, index: usize) -> Option<Value<'_>> {
        if index >= self.len() {
            cold_path();
            return None;
        }
        Some(Value::Tuple { mark: self, index })
    }
}

#[derive(Debug)]
pub struct BoolView<'a> {
    pub(crate) data: &'a [u8],
}

impl BoolView<'_> {
    pub(crate) fn get(&self, index: usize) -> Option<bool> {
        self.data.get(index).map(|&val| val != 0)
    }
}

impl Debug for Mark<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        macro_rules! fmt_mark {
            (named: [$($named:ident),* $(,)?], delegate: [$($delegate:ident),* $(,)?]) => {
                match self {
                    Mark::Empty => f.write_str("Empty"),
                    $( Mark::$named(inner) => f.debug_tuple(self.as_str()).field(inner).finish(), )*
                    $( Mark::$delegate(inner) => Debug::fmt(inner, f), )*
                }
            };
        }
        fmt_mark! {
            named: [
                Nothing, Bool, Int8, Int16, Int32, Int64, Int128, Int256, UInt8, UInt16, UInt32,
                UInt64, UInt128, UInt256, Float32, Float64, BFloat16, Uuid, Date, Date32, Time,
                Ipv4, Ipv6, String,
            ],
            delegate: [
                Decimal32, Decimal64, Decimal128, Decimal256, FixedString, DateTime,
                DateTime64, Time64, Interval, Enum8, Enum16, LowCardinality, Array, Tuple,
                Nullable, Map, Variant, Nested, NamedTuple, Dynamic, Json,
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testresult::TestResult;

    #[test]
    fn mark_accessors_return_errors() -> TestResult {
        let bytes = [1_u8, 2];
        let mark = Mark::UInt8(ByteView::try_from(bytes.as_slice())?);
        let Value::UInt8Slice(slice) = mark.slice(0..bytes.len())? else {
            unreachable!("UInt8 mark returned a non-UInt8 slice");
        };
        assert_eq!(slice, bytes.as_slice());

        assert!(matches!(
            Mark::Empty.slice(0..1),
            Err(Error::RangeOutOfBounds(range, "Empty")) if range == (0..1)
        ));

        assert!(matches!(
            mark.slice(1..3),
            Err(Error::RangeOutOfBounds(range, "UInt8")) if range == (1..3)
        ));

        assert!(matches!(
            lc::Indices::try_from(Mark::String(StringView { data: Box::new([]) })),
            Err(Error::CorruptedData(_))
        ));

        let indices = [0_u8];
        let invalid_dictionary = Mark::LowCardinality(lc::LowCardinality {
            is_nullable: false,
            indices: lc::Indices::U8(&indices),
            global_dictionary: None,
            additional_keys: Some(Box::new(Mark::String(StringView { data: Box::new([]) }))),
        });
        let mut values = invalid_dictionary.slice_lc_strs(0..1)?;
        assert!(matches!(
            values.next(),
            Some(Err(Error::IndexOutOfBounds(0, "LowCardinality dictionary")))
        ));

        // Composite marks bound-check the row range before handing it to a slice iterator.
        let offsets = 1_u64.to_le_bytes();
        let array = Mark::Array(Array {
            offsets: ByteView::try_from(offsets.as_slice())?,
            values: Box::new(Mark::UInt8(ByteView::try_from(bytes.as_slice())?)),
        });
        assert_eq!(array.len(), 1);
        assert!(matches!(array.slice(0..1)?, Value::ArraySlice { .. }));
        assert!(matches!(
            array.slice(0..2),
            Err(Error::RangeOutOfBounds(range, "Array")) if range == (0..2)
        ));
        let reversed = Range { start: 1, end: 0 };
        assert!(matches!(
            array.slice(reversed.clone()),
            Err(Error::RangeOutOfBounds(range, "Array")) if range == reversed
        ));

        let tuple = Mark::Tuple(Tuple {
            values: Box::new([Mark::UInt8(ByteView::try_from(bytes.as_slice())?)]),
            num_rows: 2,
        });
        assert_eq!(tuple.len(), 2);
        assert!(matches!(
            tuple.slice(1..3),
            Err(Error::RangeOutOfBounds(range, "Tuple")) if range == (1..3)
        ));

        let oversized_start = u32::MAX as usize + 1;
        assert!(matches!(
            TinyRange::try_from(oversized_start..oversized_start),
            Err(Error::ValueOutOfRange("usize", "u32", _))
        ));

        Ok(())
    }
}
