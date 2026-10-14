use crate::zc;
use crate::{
    Bf16Data, Date16Data, Date32Data, DateTime32Data, DateTime64Data, Decimal32Data, Decimal64Data,
    Decimal128Data, Decimal256Data, I256, Ipv4Data, Ipv6Data, TinyRange, U256, UuidData,
    error::Error,
    interval, mark,
    types::{OffsetIndexPair as _, Offsets},
};
use bstr::BStr;
use chrono_tz::Tz;
use core::{any::type_name, convert::TryFrom, hint::cold_path, marker::PhantomData};
use half::bf16;
use std::{borrow::Cow, net::Ipv4Addr, ops::Range};

macro_rules! impl_try_from_value {
    ($variant:ident, $ty:ty) => {
        impl<'a> TryFrom<Value<'a>> for $ty {
            type Error = Error;

            fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
                match value {
                    Value::$variant(v) => Ok(v),
                    other => Err(other.mismatched_type(stringify!($ty))),
                }
            }
        }
    };
}

macro_rules! impl_try_from_value_slice {
    ($variant:ident, $ty:ty) => {
        impl<'a> TryFrom<Value<'a>> for $ty {
            type Error = Error;

            fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
                match value {
                    Value::$variant(v) => Ok(v),
                    other => Err(other.mismatched_type(stringify!($ty))),
                }
            }
        }
    };
}

mod scalar;
mod string;

pub use scalar::*;
pub use string::*;

#[derive(Debug, Clone)]
pub enum Value<'a> {
    /// A NULL row of `Nullable`, `Variant`, `Dynamic` or `LowCardinality(Nullable(..))`, or a
    /// value of type `Nothing`. Never an empty array: those are typed slices.
    Empty,
    Bool(bool),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    Int128(&'a zc::I128),
    Int256(&'a I256),
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    UInt128(&'a zc::U128),
    UInt256(&'a U256),
    Float32(f32),
    Float64(f64),
    BFloat16(bf16),
    Decimal32(usize, &'a mark::Decimal32<'a>),
    Decimal64(usize, &'a mark::Decimal64<'a>),
    Decimal128(usize, &'a mark::Decimal128<'a>),
    Decimal256(usize, &'a mark::Decimal256<'a>),
    String(&'a BStr),
    FixedString(&'a BStr),
    Uuid(&'a UuidData),
    Date(chrono::NaiveDate),
    Date32(chrono::NaiveDate),
    DateTime(usize, &'a mark::DateTime<'a>),
    DateTime64(usize, &'a mark::DateTime64<'a>),
    Time(chrono::TimeDelta),
    Time64(usize, &'a mark::Time64<'a>),
    Interval(usize, &'a mark::Interval<'a>),
    Ipv4(Ipv4Addr),
    Ipv6(&'a Ipv6Data),

    /// A slice of `Array(Nothing)` (CH: [])
    NothingSlice,
    StringSlice(&'a [&'a BStr]),
    BoolSlice(&'a [u8]),
    Int8Slice(&'a [i8]),
    Int16Slice(&'a [zc::I16]),
    Int32Slice(&'a [zc::I32]),
    Int64Slice(&'a [zc::I64]),
    Int128Slice(&'a [zc::I128]),
    Int256Slice(&'a [I256]),
    UInt8Slice(&'a [u8]),
    UInt16Slice(&'a [zc::U16]),
    UInt32Slice(&'a [zc::U32]),
    UInt64Slice(&'a [zc::U64]),
    UInt128Slice(&'a [zc::U128]),
    UInt256Slice(&'a [U256]),
    Float32Slice(&'a [zc::F32]),
    Float64Slice(&'a [zc::F64]),
    BFloat16Slice(&'a [Bf16Data]),

    Decimal32Slice {
        scale: u8,
        slice: &'a [Decimal32Data],
    },
    Decimal64Slice {
        scale: u8,
        slice: &'a [Decimal64Data],
    },
    Decimal128Slice {
        scale: u8,
        slice: &'a [Decimal128Data],
    },
    Decimal256Slice {
        scale: u8,
        slice: &'a [Decimal256Data],
    },

    UuidSlice(&'a [UuidData]),
    Date16Slice(&'a [Date16Data]),
    Date32Slice(&'a [Date32Data]),
    DateTime32Slice {
        tz: Tz,
        slice: &'a [DateTime32Data],
    },
    DateTime64Slice {
        tz: Tz,
        precision: u8,
        slice: &'a [DateTime64Data],
    },
    TimeSlice(&'a [zc::I32]),
    Time64Slice {
        precision: u8,
        slice: &'a [zc::I64],
    },
    IntervalSlice {
        kind: interval::Kind,
        slice: &'a [zc::I64],
    },

    Ipv4Slice(&'a [Ipv4Data]),
    Ipv6Slice(&'a [Ipv6Data]),

    LowCardinalitySlice {
        range: TinyRange,
        mark: &'a mark::lc::LowCardinality<'a>,
    },

    ArraySlice {
        mark: &'a mark::Array<'a>,
        range: TinyRange,
    },

    Tuple {
        index: usize,
        mark: &'a mark::Tuple<'a>,
    },
    Map {
        mark: &'a mark::Map<'a>,
        index: usize,
    },
    MapSlice {
        mark: &'a mark::Map<'a>,
        range: TinyRange,
    },
    TupleSlice {
        mark: &'a mark::Tuple<'a>,
        range: TinyRange,
    },
    NullableSlice {
        mark: &'a mark::Nullable<'a>,
        range: TinyRange,
    },
    Nested {
        mark: &'a mark::Nested<'a>,
        index: usize,
    },
    NestedSlice {
        mark: &'a mark::Nested<'a>,
        range: TinyRange,
    },
    NamedTuple {
        mark: &'a mark::NamedTuple<'a>,
        index: usize,
    },
    NamedTupleSlice {
        mark: &'a mark::NamedTuple<'a>,
        range: TinyRange,
    },
    FixedStringSlice {
        mark: &'a mark::FixedString<'a>,
        range: TinyRange,
    },
    Enum8Slice {
        mark: &'a mark::Enum8<'a>,
        range: TinyRange,
    },
    Enum16Slice {
        mark: &'a mark::Enum16<'a>,
        range: TinyRange,
    },
    Json {
        mark: &'a mark::Json<'a>,
        index: usize,
    },
    JsonSlice {
        mark: &'a mark::Json<'a>,
        range: TinyRange,
    },
    VariantSlice {
        mark: &'a mark::Variant<'a>,
        range: TinyRange,
    },
    DynamicSlice {
        mark: &'a mark::Dynamic<'a>,
        range: TinyRange,
    },
}

#[inline(always)]
fn short_type_name<T: ?Sized>() -> &'static str {
    let name = type_name::<T>();
    let outer = name.split_once('<').map_or(name, |(outer, _)| outer);
    outer.rsplit("::").next().unwrap_or(outer)
}

impl Value<'_> {
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Value::Empty => "Empty",
            Value::NothingSlice => "NothingSlice",
            Value::Bool(_) => "Bool",
            Value::Int8(_) => "Int8",
            Value::Int16(_) => "Int16",
            Value::Int32(_) => "Int32",
            Value::Int64(_) => "Int64",
            Value::Int128(_) => "Int128",
            Value::Int256(_) => "Int256",
            Value::UInt8(_) => "UInt8",
            Value::UInt16(_) => "UInt16",
            Value::UInt32(_) => "UInt32",
            Value::UInt64(_) => "UInt64",
            Value::UInt128(_) => "UInt128",
            Value::UInt256(_) => "UInt256",
            Value::Float32(_) => "Float32",
            Value::Float64(_) => "Float64",
            Value::BFloat16(_) => "BFloat16",
            Value::Decimal32(_, _) => "Decimal32",
            Value::Decimal64(_, _) => "Decimal64",
            Value::Decimal128(_, _) => "Decimal128",
            Value::Decimal256(_, _) => "Decimal256",
            Value::String(_) => "String",
            Value::FixedString(_) => "FixedString",
            Value::Uuid(_) => "Uuid",
            Value::Date(_) => "Date",
            Value::Date32(_) => "Date32",
            Value::DateTime(_, _) => "DateTime",
            Value::DateTime64(_, _) => "DateTime64",
            Value::Time(_) => "Time",
            Value::Time64(_, _) => "Time64",
            Value::Interval(_, i) => i.kind.as_str(),
            Value::Ipv4(_) => "Ipv4",
            Value::Ipv6(_) => "Ipv6",
            Value::StringSlice(_) => "StringSlice",
            Value::Int8Slice(_) => "Int8Slice",
            Value::Int16Slice(_) => "Int16Slice",
            Value::Int32Slice(_) => "Int32Slice",
            Value::Int64Slice(_) => "Int64Slice",
            Value::Int128Slice(_) => "Int128Slice",
            Value::Int256Slice(_) => "Int256Slice",
            Value::UInt8Slice(_) => "UInt8Slice",
            Value::UInt16Slice(_) => "UInt16Slice",
            Value::UInt32Slice(_) => "UInt32Slice",
            Value::UInt64Slice(_) => "UInt64Slice",
            Value::UInt128Slice(_) => "UInt128Slice",
            Value::UInt256Slice(_) => "UInt256Slice",
            Value::Float32Slice(_) => "Float32Slice",
            Value::Float64Slice(_) => "Float64Slice",
            Value::LowCardinalitySlice { .. } => "LowCardinalitySlice",
            Value::ArraySlice { .. } => "ArraySlice",
            Value::Tuple { .. } => "Tuple",
            Value::Map { .. } => "Map",
            Value::MapSlice { .. } => "MapSlice",
            Value::TupleSlice { .. } => "TupleSlice",
            Value::BoolSlice(_) => "BoolSlice",
            Value::UuidSlice(_) => "UuidSlice",
            Value::Date16Slice(_) => "Date16Slice",
            Value::Date32Slice(_) => "Date32Slice",
            Value::DateTime32Slice { .. } => "DateTime32Slice",
            Value::DateTime64Slice { .. } => "DateTime64Slice",
            Value::TimeSlice(_) => "TimeSlice",
            Value::Time64Slice { .. } => "Time64Slice",
            Value::IntervalSlice { .. } => "IntervalSlice",
            Value::Ipv4Slice(_) => "Ipv4Slice",
            Value::Ipv6Slice(_) => "Ipv6Slice",
            Value::NullableSlice { .. } => "NullableSlice",
            Value::Decimal32Slice { .. } => "Decimal32Slice",
            Value::Decimal64Slice { .. } => "Decimal64Slice",
            Value::Decimal128Slice { .. } => "Decimal128Slice",
            Value::Decimal256Slice { .. } => "Decimal256Slice",
            Value::Nested { .. } => "Nested",
            Value::NestedSlice { .. } => "NestedSlice",
            Value::NamedTuple { .. } => "NamedTuple",
            Value::NamedTupleSlice { .. } => "NamedTupleSlice",
            Value::FixedStringSlice { .. } => "FixedStringSlice",
            Value::Enum8Slice { .. } => "Enum8Slice",
            Value::Enum16Slice { .. } => "Enum16Slice",
            Value::BFloat16Slice(_) => "BFloat16Slice",
            Value::Json { .. } => "Json",
            Value::JsonSlice { .. } => "JsonSlice",
            Value::VariantSlice { .. } => "VariantSlice",
            Value::DynamicSlice { .. } => "DynamicSlice",
        }
    }

    #[cold]
    #[inline(never)]
    const fn mismatched_type(&self, expected: &'static str) -> Error {
        Error::MismatchedType(self.as_str(), expected)
    }
}

pub struct LowCardinalitySliceIterator<'a> {
    pub(crate) lc: &'a mark::lc::LowCardinality<'a>,
    pub(crate) indices: mark::lc::IndicesIter<'a>,
}

impl<'a> TryFrom<Value<'a>> for LowCardinalitySliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::LowCardinalitySlice { range, mark } => mark.slice(range.into()),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a> Iterator for LowCardinalitySliceIterator<'a> {
    type Item = Result<Value<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(match self.indices.next()? {
            Ok(index) => self.lc.value(index),
            Err(error) => {
                cold_path();
                Err(error)
            }
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }
}

impl ExactSizeIterator for LowCardinalitySliceIterator<'_> {}

pub struct ArraySliceIterator<'a, T> {
    mark: &'a mark::Array<'a>,
    range: Range<usize>,
    _phantom: PhantomData<T>,
}

impl<'a, T> TryFrom<Value<'a>> for ArraySliceIterator<'a, T> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::ArraySlice { mark, range } => Ok(Self {
                mark,
                range: range.into(),
                _phantom: PhantomData,
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a, T> Iterator for ArraySliceIterator<'a, T>
where
    T: TryFrom<Value<'a>, Error = Error>,
{
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let slice_idx = self.range.next()?;
        let (start, end) = match self.mark.offsets.offset_indices(slice_idx) {
            Ok(Some(indices)) => indices,
            Ok(None) => {
                cold_path();
                return Some(Err(Error::IndexOutOfBounds(slice_idx, "ArraySlice")));
            }
            Err(error) => return Some(Err(error)),
        };
        let value = match self.mark.values.slice(start..end) {
            Ok(value) => value,
            Err(error) => return Some(Err(error)),
        };
        Some(T::try_from(value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<'a, T> ExactSizeIterator for ArraySliceIterator<'a, T> where
    T: TryFrom<Value<'a>, Error = Error>
{
}

impl<'a, T> TryFrom<Value<'a>> for Option<T>
where
    T: TryFrom<Value<'a>, Error = Error>,
{
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Empty => Ok(None),
            other => T::try_from(other).map(Some),
        }
    }
}

macro_rules! impl_try_from_tuple {
    ($len:literal, $( $idx:tt => $T:ident ),+ $(,)?) => {
        impl<'a, $( $T , )+> core::convert::TryFrom<Value<'a>> for ( $( $T , )+ )
        where
            $( $T : core::convert::TryFrom<Value<'a>, Error = Error>, )+
        {
            type Error = Error;


            fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
                let (index, tuple_mark) = match value {
                    Value::Tuple { index, mark } => (index, mark),
                    Value::NamedTuple { index, mark } => {
                        let mark::Mark::Tuple(tuple) = mark.tuple.as_ref() else {
                            cold_path();
                            return Err(Error::MismatchedType("non-Tuple", concat!("Tuple", stringify!($len))));
                        };
                        (index, tuple)
                    }
                    other => {
                        return Err(
                            other.mismatched_type(concat!("Tuple", stringify!($len))),
                        );
                    }
                };

                let values = &tuple_mark.values;
                if values.len() != $len {
                    cold_path();
                    return Err(Error::MismatchedType(
                        concat!("Tuple with ", stringify!($len), " elements"),
                        concat!("Tuple", stringify!($len)),
                    ));
                }

                Ok((
                    $(
                        {
                            let Some(field_val) = values[$idx].get(index)? else {
                                cold_path();
                                return Err(Error::IndexOutOfBounds(
                                    index,
                                    concat!("Tuple", stringify!($len)),
                                ));
                            };
                            <$T>::try_from(field_val)?
                        },
                    )+
                ))
            }
        }
    };
}

impl_try_from_tuple!(1, 0 => A);
impl_try_from_tuple!(2, 0 => A, 1 => B);
impl_try_from_tuple!(3, 0 => A, 1 => B, 2 => C);
impl_try_from_tuple!(4, 0 => A, 1 => B, 2 => C, 3 => D);
impl_try_from_tuple!(5, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E);
impl_try_from_tuple!(6, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F);
impl_try_from_tuple!(7, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G);
impl_try_from_tuple!(8, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H);
impl_try_from_tuple!(9, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I);
impl_try_from_tuple!(10, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I, 9 => J);

pub struct MapIterator<'a, K, V> {
    pub(crate) keys: &'a mark::Mark<'a>,
    pub(crate) values: &'a mark::Mark<'a>,
    pub(crate) range: Range<usize>,
    pub(crate) _marker: PhantomData<(K, V)>,
}

impl<'a, K, V> TryFrom<Value<'a>> for MapIterator<'a, K, V>
where
    K: TryFrom<Value<'a>, Error = Error>,
    V: TryFrom<Value<'a>, Error = Error>,
{
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Map { mark, index } => {
                // Resolve (start, end) for the requested row in the Map column
                let Some((start, end)) = mark.offsets.offset_indices(index)? else {
                    return Err(Error::IndexOutOfBounds(index, "Map"));
                };

                Ok(Self {
                    keys: &mark.keys,
                    values: &mark.values,
                    range: start..end,
                    _marker: PhantomData,
                })
            }
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a, K, V> Iterator for MapIterator<'a, K, V>
where
    K: TryFrom<Value<'a>, Error = Error>,
    V: TryFrom<Value<'a>, Error = Error>,
{
    type Item = Result<(K, V), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let idx = self.range.next()?;

        let raw_key = match self.keys.get(idx) {
            Ok(Some(value)) => value,
            Ok(None) => {
                cold_path();
                return Some(Err(Error::IndexOutOfBounds(idx, "Map key")));
            }
            Err(error) => return Some(Err(error)),
        };
        let raw_value = match self.values.get(idx) {
            Ok(Some(value)) => value,
            Ok(None) => {
                cold_path();
                return Some(Err(Error::IndexOutOfBounds(idx, "Map value")));
            }
            Err(error) => return Some(Err(error)),
        };

        let key = match K::try_from(raw_key) {
            Ok(key) => key,
            Err(error) => return Some(Err(error)),
        };
        let value = match V::try_from(raw_value) {
            Ok(value) => value,
            Err(error) => return Some(Err(error)),
        };
        Some(Ok((key, value)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<'a, K, V> ExactSizeIterator for MapIterator<'a, K, V>
where
    K: TryFrom<Value<'a>, Error = Error>,
    V: TryFrom<Value<'a>, Error = Error>,
{
}

pub struct MapSliceIterator<'a, K, V> {
    pub(crate) offsets: &'a Offsets<'a>,
    pub(crate) keys: &'a mark::Mark<'a>,
    pub(crate) values: &'a mark::Mark<'a>,
    pub(crate) range: Range<usize>,
    pub(crate) _marker: PhantomData<(K, V)>,
}

impl<'a, K, V> TryFrom<Value<'a>> for MapSliceIterator<'a, K, V>
where
    K: TryFrom<Value<'a>, Error = Error>,
    V: TryFrom<Value<'a>, Error = Error>,
{
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::MapSlice { mark, range } => Ok(Self {
                offsets: &mark.offsets,
                keys: &mark.keys,
                values: &mark.values,
                range: range.into(),
                _marker: PhantomData,
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a, K, V> Iterator for MapSliceIterator<'a, K, V>
where
    K: TryFrom<Value<'a>, Error = Error>,
    V: TryFrom<Value<'a>, Error = Error>,
{
    type Item = Result<MapIterator<'a, K, V>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let slice_idx = self.range.next()?;
        let (start, end) = match self.offsets.offset_indices(slice_idx) {
            Ok(Some(indices)) => indices,
            Ok(None) => {
                cold_path();
                return Some(Err(Error::IndexOutOfBounds(slice_idx, "MapSlice")));
            }
            Err(error) => return Some(Err(error)),
        };

        Some(Ok(MapIterator {
            keys: self.keys,
            values: self.values,
            range: start..end,
            _marker: PhantomData,
        }))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<'a, K, V> ExactSizeIterator for MapSliceIterator<'a, K, V>
where
    K: TryFrom<Value<'a>, Error = Error>,
    V: TryFrom<Value<'a>, Error = Error>,
{
}

pub struct TupleSliceIterator<'a> {
    mark: &'a mark::Tuple<'a>,
    range: Range<usize>,
}

impl<'a> TryFrom<Value<'a>> for TupleSliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::TupleSlice { mark, range } => Ok(Self {
                mark,
                range: range.into(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a> Iterator for TupleSliceIterator<'a> {
    type Item = Value<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let row_idx = self.range.next()?;
        Some(Value::Tuple {
            index: row_idx,
            mark: self.mark,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl ExactSizeIterator for TupleSliceIterator<'_> {}

pub struct NullableSliceIterator<'a> {
    mark: &'a mark::Nullable<'a>,
    range: Range<usize>,
}

impl<'a> TryFrom<Value<'a>> for NullableSliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::NullableSlice { mark, range } => Ok(Self {
                mark,
                range: range.into(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a> Iterator for NullableSliceIterator<'a> {
    type Item = Result<Value<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.range.next()?;
        if self.mark.is_null(index)? {
            return Some(Ok(Value::Empty));
        }
        self.mark.data.get(index).transpose()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl ExactSizeIterator for NullableSliceIterator<'_> {}

pub struct NestedIterator<'a> {
    col_names: &'a [Cow<'a, str>],
    tuple_slice: TupleSliceIterator<'a>,
}

impl<'a> TryFrom<Value<'a>> for NestedIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Nested { mark, index } => {
                let Some(value) = mark.array_of_tuples.get(index)? else {
                    cold_path();
                    return Err(Error::IndexOutOfBounds(index, "Nested"));
                };
                let tuple_slice: TupleSliceIterator = value.try_into()?;
                Ok(Self {
                    col_names: &mark.col_names,
                    tuple_slice,
                })
            }
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a> Iterator for NestedIterator<'a> {
    type Item = NestedItemsIterator<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.tuple_slice.next()?;
        let Value::Tuple { index: row, mark } = value else {
            return None;
        };

        let mark_iter = mark.values.iter().zip(self.col_names);

        Some(NestedItemsIterator {
            mark_ter: mark_iter,
            row,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.tuple_slice.size_hint()
    }
}

impl ExactSizeIterator for NestedIterator<'_> {}

pub struct NestedItemsIterator<'a> {
    mark_ter:
        std::iter::Zip<std::slice::Iter<'a, mark::Mark<'a>>, std::slice::Iter<'a, Cow<'a, str>>>,
    row: usize,
}

impl<'a> Iterator for NestedItemsIterator<'a> {
    type Item = Result<(&'a str, Value<'a>), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let (mark, col_name) = self.mark_ter.next()?;
        mark.get(self.row)
            .transpose()
            .map(|result| result.map(|value| (col_name.as_ref(), value)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.mark_ter.size_hint()
    }
}

impl ExactSizeIterator for NestedItemsIterator<'_> {}

pub struct NamedTupleIterator<'a> {
    col_names: &'a [Cow<'a, str>],
    mark: &'a mark::Tuple<'a>,
    row: usize,
}

impl<'a> TryFrom<Value<'a>> for NamedTupleIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::NamedTuple { mark, index } => {
                let mark::Mark::Tuple(tuple) = mark.tuple.as_ref() else {
                    cold_path();
                    return Err(Error::MismatchedType(
                        "non-Tuple",
                        short_type_name::<Self>(),
                    ));
                };
                Ok(Self {
                    col_names: &mark.col_names,
                    mark: tuple,
                    row: index,
                })
            }
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a> Iterator for NamedTupleIterator<'a> {
    type Item = Result<(&'a str, Value<'a>), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let (col_name, values) = self.col_names.split_first()?;
        self.col_names = values;
        let mark = self
            .mark
            .values
            .get(self.mark.values.len() - self.col_names.len() - 1)?;
        mark.get(self.row)
            .transpose()
            .map(|result| result.map(|value| (col_name.as_ref(), value)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.col_names.len(), Some(self.col_names.len()))
    }
}

impl ExactSizeIterator for NamedTupleIterator<'_> {}

pub struct NamedTupleSliceIterator<'a> {
    mark: &'a mark::NamedTuple<'a>,
    range: Range<usize>,
}

impl<'a> TryFrom<Value<'a>> for NamedTupleSliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::NamedTupleSlice { mark, range } => Ok(Self {
                mark,
                range: range.into(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a> Iterator for NamedTupleSliceIterator<'a> {
    type Item = Result<NamedTupleIterator<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.range.next()?;
        let mark::Mark::Tuple(tuple) = self.mark.tuple.as_ref() else {
            cold_path();
            return Some(Err(Error::MismatchedType(
                "non-Tuple",
                short_type_name::<Self>(),
            )));
        };
        Some(Ok(NamedTupleIterator {
            col_names: &self.mark.col_names,
            mark: tuple,
            row: index,
        }))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl ExactSizeIterator for NamedTupleSliceIterator<'_> {}

pub struct NestedSliceIterator<'a> {
    col_names: &'a [Cow<'a, str>],
    array_of_tuples: &'a mark::Mark<'a>,
    range: Range<usize>,
}

impl<'a> TryFrom<Value<'a>> for NestedSliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::NestedSlice { mark, range } => Ok(Self {
                col_names: &mark.col_names,
                array_of_tuples: &mark.array_of_tuples,
                range: range.into(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a> Iterator for NestedSliceIterator<'a> {
    type Item = Result<NestedIterator<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let slice_idx = self.range.next()?;
        let val = match self.array_of_tuples.get(slice_idx) {
            Ok(Some(value)) => value,
            Ok(None) => {
                cold_path();
                return Some(Err(Error::IndexOutOfBounds(slice_idx, "NestedSlice")));
            }
            Err(error) => return Some(Err(error)),
        };

        let tuple_slice: TupleSliceIterator = match val.try_into() {
            Ok(value) => value,
            Err(error) => {
                cold_path();
                return Some(Err(error));
            }
        };

        Some(Ok(NestedIterator {
            col_names: self.col_names,
            tuple_slice,
        }))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl ExactSizeIterator for NestedSliceIterator<'_> {}

pub struct VariantSliceIterator<'a> {
    mark: &'a mark::Variant<'a>,
    range: Range<usize>,
}

impl<'a> TryFrom<Value<'a>> for VariantSliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::VariantSlice { mark, range } => Ok(Self {
                mark,
                range: range.into(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a> Iterator for VariantSliceIterator<'a> {
    type Item = Result<Value<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.range.next()?;
        self.mark.get(index).transpose()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl ExactSizeIterator for VariantSliceIterator<'_> {}

pub struct DynamicSliceIterator<'a> {
    mark: &'a mark::Dynamic<'a>,
    range: Range<usize>,
}

impl<'a> TryFrom<Value<'a>> for DynamicSliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::DynamicSlice { mark, range } => Ok(Self {
                mark,
                range: range.into(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a> Iterator for DynamicSliceIterator<'a> {
    type Item = Result<Value<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.range.next()?;
        self.mark.get(index).transpose()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl ExactSizeIterator for DynamicSliceIterator<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mark::{Mark, NamedTuple};

    #[test]
    fn value_conversions_report_source_and_target_types() {
        assert!(matches!(
            bool::try_from(Value::UInt8(1)),
            Err(Error::MismatchedType("UInt8", "bool"))
        ));
        assert!(matches!(
            <&str>::try_from(Value::UInt8(1)),
            Err(Error::MismatchedType("UInt8", "&str"))
        ));
        assert!(matches!(
            TupleSliceIterator::try_from(Value::UInt8(1)),
            Err(Error::MismatchedType("UInt8", "TupleSliceIterator"))
        ));
        assert!(matches!(
            MapSliceIterator::<bool, bool>::try_from(Value::UInt8(1)),
            Err(Error::MismatchedType("UInt8", "MapSliceIterator"))
        ));

        let named_tuple = NamedTuple {
            col_names: Box::new([]),
            tuple: Box::new(Mark::Empty),
        };
        assert!(matches!(
            NamedTupleIterator::try_from(Value::NamedTuple {
                mark: &named_tuple,
                index: 0,
            }),
            Err(Error::MismatchedType("non-Tuple", "NamedTupleIterator"))
        ));

        let mut slice = NamedTupleSliceIterator::try_from(Value::NamedTupleSlice {
            mark: &named_tuple,
            range: TinyRange {
                start: 0,
                length: 1,
            },
        })
        .expect("NamedTupleSlice must convert before reading its malformed tuple");
        assert!(matches!(
            slice.next(),
            Some(Err(Error::MismatchedType(
                "non-Tuple",
                "NamedTupleSliceIterator"
            )))
        ));
    }
}
