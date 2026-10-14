use core::{convert::TryFrom, hint::cold_path};
use std::{
    borrow::Cow,
    net::{Ipv4Addr, Ipv6Addr},
    ops::Range,
};

use chrono::TimeDelta;
use chrono_tz::Tz;
use half::bf16;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::{Value, short_type_name};
use crate::{
    Bf16Data, Date16Data, Date32Data, DateTime32Data, DateTime64Data, Decimal32Data, Decimal64Data,
    Decimal128Data, I256, Ipv4Data, Ipv6Data, U256, UuidData, conv,
    error::Error,
    interval::{self, Interval},
    zc,
};

impl_try_from_value_slice!(Int8Slice, &'a [i8]);
impl_try_from_value_slice!(Int16Slice, &'a [zc::I16]);
impl_try_from_value_slice!(Int32Slice, &'a [zc::I32]);
impl_try_from_value_slice!(Int64Slice, &'a [zc::I64]);
impl_try_from_value_slice!(Int128Slice, &'a [zc::I128]);

impl_try_from_value_slice!(UInt8Slice, &'a [u8]);
impl_try_from_value_slice!(UInt16Slice, &'a [zc::U16]);
impl_try_from_value_slice!(UInt32Slice, &'a [zc::U32]);
impl_try_from_value_slice!(UInt64Slice, &'a [zc::U64]);
impl_try_from_value_slice!(UInt128Slice, &'a [zc::U128]);

impl_try_from_value_slice!(Float32Slice, &'a [zc::F32]);
impl_try_from_value_slice!(Float64Slice, &'a [zc::F64]);

impl_try_from_value_slice!(UuidSlice, &'a [UuidData]);
impl_try_from_value_slice!(Date16Slice, &'a [Date16Data]);
impl_try_from_value_slice!(Date32Slice, &'a [Date32Data]);
impl_try_from_value_slice!(Ipv4Slice, &'a [Ipv4Data]);
impl_try_from_value_slice!(Ipv6Slice, &'a [Ipv6Data]);

impl_try_from_value!(Bool, bool);
impl_try_from_value!(Int256, &'a I256);

impl_try_from_value!(UInt256, &'a U256);

impl_try_from_value!(Float64, f64);
impl_try_from_value!(Float32, f32);
impl_try_from_value!(BFloat16, bf16);
impl_try_from_value_slice!(BFloat16Slice, &'a [Bf16Data]);

impl_try_from_value!(Ipv4, Ipv4Addr);

impl<'a> TryFrom<Value<'a>> for Ipv6Addr {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Ipv6(v) => Ok(Ipv6Addr::from(*v)),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl<'a> TryFrom<Value<'a>> for Uuid {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Uuid(uuid_data) => {
                let [hi, lo] = uuid_data.0;
                Ok(Uuid::from_u64_pair(hi.get(), lo.get()))
            }
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl TryFrom<Value<'_>> for chrono::DateTime<Tz> {
    type Error = Error;

    fn try_from(value: Value<'_>) -> Result<Self, Self::Error> {
        match value {
            Value::DateTime(index, d) => {
                let value = d
                    .data
                    .get(index)
                    .expect("bug: we checked the boundary before creating the Value")
                    .with_tz(d.tz);
                Ok(value)
            }
            Value::DateTime64(index, d) => {
                let value = d
                    .data
                    .get(index)
                    .expect("bug: we checked the boundary before creating the Value")
                    .with_tz_and_precision(d.tz, d.precision)?;
                Ok(value)
            }
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl TryFrom<Value<'_>> for chrono::NaiveDate {
    type Error = Error;

    fn try_from(value: Value<'_>) -> Result<Self, Self::Error> {
        match value {
            Value::Date32(dt) | Value::Date(dt) => Ok(dt),
            other => Err(other.mismatched_type("Date/Date32")),
        }
    }
}

macro_rules! impl_try_from_integer_value {
    ($($target:ty),+ $(,)?) => {
        $(
            impl<'a> core::convert::TryFrom<Value<'a>> for $target {
                type Error = Error;


                fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {

                    match value {
                        Value::Int8(v) => match <$target>::try_from(v) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                cold_path();
                                Err(Error::ValueOutOfRange("i8", stringify!($target), v.to_string()))
                            }
                        },
                        Value::Int16(v) => match <$target>::try_from(v) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                cold_path();
                                Err(Error::ValueOutOfRange("i16", stringify!($target), v.to_string()))
                            }
                        },
                        Value::Int32(v) => match <$target>::try_from(v) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                cold_path();
                                Err(Error::ValueOutOfRange("i32", stringify!($target), v.to_string()))
                            }
                        },
                        Value::Int64(v) => match <$target>::try_from(v) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                cold_path();
                                Err(Error::ValueOutOfRange("i64", stringify!($target), v.to_string()))
                            }
                        },
                        Value::Int128(v) => match <$target>::try_from(v.get()) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                cold_path();
                                Err(Error::ValueOutOfRange("i128", stringify!($target), v.to_string()))
                            }
                        },

                        Value::UInt8(v) => match <$target>::try_from(v) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                cold_path();
                                Err(Error::ValueOutOfRange("u8", stringify!($target), v.to_string()))
                            }
                        },
                        Value::UInt16(v) => match <$target>::try_from(v) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                cold_path();
                                Err(Error::ValueOutOfRange("u16", stringify!($target), v.to_string()))
                            }
                        },
                        Value::UInt32(v) => match <$target>::try_from(v) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                cold_path();
                                Err(Error::ValueOutOfRange("u32", stringify!($target), v.to_string()))
                            }
                        },
                        Value::UInt64(v) => match <$target>::try_from(v) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                cold_path();
                                Err(Error::ValueOutOfRange("u64", stringify!($target), v.to_string()))
                            }
                        },
                        Value::UInt128(v) => match <$target>::try_from(v.get()) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                cold_path();
                                Err(Error::ValueOutOfRange("u128", stringify!($target), v.to_string()))
                            }
                        },

                        other => Err(other.mismatched_type(stringify!($target))),
                    }
                }
            }
        )+
    };
}

impl_try_from_integer_value!(
    u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, i128, u128
);

impl<'a> TryFrom<Value<'a>> for Decimal {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Decimal32(index, mark) => Ok(mark.data[index].with_scale(mark.scale)),
            Value::Decimal64(index, mark) => Ok(mark.data[index].with_scale(mark.scale)),
            Value::Decimal128(index, mark) => mark.data[index].with_scale(mark.scale),
            Value::Decimal256(_, _) => {
                cold_path();
                Err(Error::NotImplemented(
                    "Decimal256 is not yet supported".to_owned(),
                ))
            }
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}
pub struct BoolSliceIterator<'a> {
    data: std::slice::Iter<'a, u8>,
}

impl<'a> TryFrom<Value<'a>> for BoolSliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::BoolSlice(data) => Ok(BoolSliceIterator { data: data.iter() }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl Iterator for BoolSliceIterator<'_> {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        self.data.next().map(|&byte| byte != 0)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.data.size_hint()
    }
}

impl ExactSizeIterator for BoolSliceIterator<'_> {}

pub struct DateTime32SliceIterator<'a> {
    tz: Tz,
    cached_offset: Option<chrono_tz::TzOffset>,
    slice: std::slice::Iter<'a, DateTime32Data>,
}

impl<'a> TryFrom<Value<'a>> for DateTime32SliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::DateTime32Slice { tz, slice } => Ok(Self {
                tz,
                cached_offset: conv::utc_alias_offset(tz),
                slice: slice.iter(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl Iterator for DateTime32SliceIterator<'_> {
    type Item = chrono::DateTime<Tz>;

    fn next(&mut self) -> Option<Self::Item> {
        let dt = self.slice.next()?;
        Some(conv::datetime32_resolved(
            dt.0.get(),
            self.tz,
            self.cached_offset,
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slice.size_hint()
    }
}

impl ExactSizeIterator for DateTime32SliceIterator<'_> {}

pub struct DateTime64SliceIterator<'a> {
    tz: Tz,
    cached_offset: Option<chrono_tz::TzOffset>,
    precision: u8,
    slice: std::slice::Iter<'a, DateTime64Data>,
}

impl<'a> TryFrom<Value<'a>> for DateTime64SliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::DateTime64Slice {
                tz,
                precision,
                slice,
            } => Ok(Self {
                tz,
                cached_offset: conv::utc_alias_offset(tz),
                precision,
                slice: slice.iter(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl Iterator for DateTime64SliceIterator<'_> {
    type Item = crate::Result<chrono::DateTime<Tz>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.slice.next().map(|dt| {
            conv::datetime64_resolved(dt.0.get(), self.precision, self.tz, self.cached_offset)
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slice.size_hint()
    }
}

impl ExactSizeIterator for DateTime64SliceIterator<'_> {}

impl TryFrom<Value<'_>> for Interval {
    type Error = Error;

    fn try_from(value: Value<'_>) -> Result<Self, Self::Error> {
        match value {
            Value::Interval(index, i) => Ok(Interval {
                kind: i.kind,
                count: i.data[index].get(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl TryFrom<Value<'_>> for TimeDelta {
    type Error = Error;

    fn try_from(value: Value<'_>) -> Result<Self, Self::Error> {
        match value {
            Value::Interval(index, i) => i.kind.to_time_delta(i.data[index].get()),
            Value::Time(td) => Ok(td),
            Value::Time64(index, t) => conv::time64(t.data[index].get(), t.precision),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

pub struct IntervalSliceIterator<'a> {
    kind: interval::Kind,
    slice: std::slice::Iter<'a, zc::I64>,
}

impl<'a> TryFrom<Value<'a>> for IntervalSliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::IntervalSlice { kind, slice } => Ok(Self {
                kind,
                slice: slice.iter(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl Iterator for IntervalSliceIterator<'_> {
    type Item = crate::Result<TimeDelta>;

    fn next(&mut self) -> Option<Self::Item> {
        self.slice
            .next()
            .map(|count| self.kind.to_time_delta(count.get()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slice.size_hint()
    }
}

impl ExactSizeIterator for IntervalSliceIterator<'_> {}

pub struct TimeSliceIterator<'a> {
    slice: std::slice::Iter<'a, zc::I32>,
}

impl<'a> TryFrom<Value<'a>> for TimeSliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::TimeSlice(slice) => Ok(Self {
                slice: slice.iter(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl Iterator for TimeSliceIterator<'_> {
    type Item = TimeDelta;

    fn next(&mut self) -> Option<Self::Item> {
        self.slice
            .next()
            .map(|secs| TimeDelta::seconds(i64::from(secs.get())))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slice.size_hint()
    }
}

impl ExactSizeIterator for TimeSliceIterator<'_> {}

pub struct Time64SliceIterator<'a> {
    precision: u8,
    slice: std::slice::Iter<'a, zc::I64>,
}

impl<'a> TryFrom<Value<'a>> for Time64SliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Time64Slice { precision, slice } => Ok(Self {
                precision,
                slice: slice.iter(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl Iterator for Time64SliceIterator<'_> {
    type Item = crate::Result<TimeDelta>;

    fn next(&mut self) -> Option<Self::Item> {
        self.slice
            .next()
            .map(|ticks| conv::time64(ticks.get(), self.precision))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slice.size_hint()
    }
}

impl ExactSizeIterator for Time64SliceIterator<'_> {}

pub struct Decimal32SliceIterator<'a> {
    scale: u8,
    slice: std::slice::Iter<'a, Decimal32Data>,
}

impl<'a> TryFrom<Value<'a>> for Decimal32SliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Decimal32Slice { scale, slice } => Ok(Self {
                scale,
                slice: slice.iter(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl Iterator for Decimal32SliceIterator<'_> {
    type Item = Decimal;

    fn next(&mut self) -> Option<Self::Item> {
        self.slice.next().map(|v| v.with_scale(self.scale))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slice.size_hint()
    }
}

impl ExactSizeIterator for Decimal32SliceIterator<'_> {}

pub struct Decimal64SliceIterator<'a> {
    scale: u8,
    slice: std::slice::Iter<'a, Decimal64Data>,
}

impl<'a> TryFrom<Value<'a>> for Decimal64SliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Decimal64Slice { scale, slice } => Ok(Self {
                scale,
                slice: slice.iter(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl Iterator for Decimal64SliceIterator<'_> {
    type Item = Decimal;

    fn next(&mut self) -> Option<Self::Item> {
        self.slice.next().map(|v| v.with_scale(self.scale))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slice.size_hint()
    }
}

impl ExactSizeIterator for Decimal64SliceIterator<'_> {}

pub struct Decimal128SliceIterator<'a> {
    scale: u8,
    slice: std::slice::Iter<'a, Decimal128Data>,
}

impl<'a> TryFrom<Value<'a>> for Decimal128SliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Decimal128Slice { scale, slice } => Ok(Self {
                scale,
                slice: slice.iter(),
            }),
            other => Err(other.mismatched_type(short_type_name::<Self>())),
        }
    }
}

impl Iterator for Decimal128SliceIterator<'_> {
    type Item = crate::Result<Decimal>;

    fn next(&mut self) -> Option<Self::Item> {
        self.slice.next().map(|v| v.with_scale(self.scale))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slice.size_hint()
    }
}

impl ExactSizeIterator for Decimal128SliceIterator<'_> {}
pub struct Enum8SliceIterator<'a> {
    variants: &'a [(Cow<'a, str>, i8)],
    data: std::slice::Iter<'a, i8>,
}

impl<'a> TryFrom<Value<'a>> for Enum8SliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Enum8Slice { mark, range } => {
                let range: Range<usize> = range.into();
                let data = &mark.data[range];
                Ok(Self {
                    variants: &mark.variants,
                    data: data.iter(),
                })
            }
            other => Err(other.mismatched_type("Enum8Iterator")),
        }
    }
}

impl<'a> Iterator for Enum8SliceIterator<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.data.next()?;
        if let Ok(index) = self.variants.binary_search_by_key(value, |(_, id)| *id) {
            return Some(self.variants[index].0.as_ref());
        }

        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.data.size_hint()
    }
}

impl ExactSizeIterator for Enum8SliceIterator<'_> {}

pub struct Enum16SliceIterator<'a> {
    variants: &'a [(Cow<'a, str>, i16)],
    data: std::slice::Iter<'a, zc::I16>,
}

impl<'a> TryFrom<Value<'a>> for Enum16SliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Enum16Slice { mark, range } => {
                let range: Range<usize> = range.into();
                let data = &mark.data[range];
                Ok(Self {
                    variants: &mark.variants,
                    data: data.iter(),
                })
            }
            other => Err(other.mismatched_type("Enum16Iterator")),
        }
    }
}

impl<'a> Iterator for Enum16SliceIterator<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.data.next()?.get();
        if let Ok(index) = self.variants.binary_search_by_key(&value, |(_, id)| *id) {
            return Some(self.variants[index].0.as_ref());
        }

        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.data.size_hint()
    }
}

impl ExactSizeIterator for Enum16SliceIterator<'_> {}

#[cfg(test)]
mod tests {
    use chrono::{Offset as _, TimeZone as _};
    use testresult::TestResult;

    use super::*;

    #[test]
    fn datetime_slice_iterators_preserve_alias_identity() -> TestResult {
        let seconds = [DateTime32Data(zc::U32::new(1_700_000_000))];
        let ticks = [DateTime64Data(zc::I64::new(1_700_000_000_123))];
        let dt32 = DateTime32SliceIterator::try_from(Value::DateTime32Slice {
            tz: Tz::GMT,
            slice: &seconds,
        })?
        .next()
        .unwrap();
        let dt64 = DateTime64SliceIterator::try_from(Value::DateTime64Slice {
            tz: Tz::GMT,
            precision: 3,
            slice: &ticks,
        })?
        .next()
        .unwrap()?;

        for datetime in [dt32, dt64] {
            assert_eq!(datetime.timezone(), Tz::GMT);
            assert_eq!(datetime.format("%Z").to_string(), "GMT");
        }
        assert_eq!(dt32.timestamp(), 1_700_000_000);
        assert_eq!(dt64.timestamp_millis(), 1_700_000_000_123);
        Ok(())
    }

    #[test]
    fn datetime_slice_iterators_preserve_seasonal_offsets() -> TestResult {
        let timestamps = [1, 7].map(|month| {
            chrono::Utc
                .with_ymd_and_hms(2024, month, 15, 12, 0, 0)
                .unwrap()
                .timestamp()
        });
        let seconds = timestamps.map(|timestamp| {
            DateTime32Data(zc::U32::new(
                u32::try_from(timestamp).expect("test timestamp fits in u32"),
            ))
        });
        let ticks =
            timestamps.map(|timestamp| DateTime64Data(zc::I64::new(timestamp * 1_000 + 123)));
        let mut dt32 = DateTime32SliceIterator::try_from(Value::DateTime32Slice {
            tz: Tz::Europe__London,
            slice: &seconds,
        })?;
        let mut dt64 = DateTime64SliceIterator::try_from(Value::DateTime64Slice {
            tz: Tz::Europe__London,
            precision: 3,
            slice: &ticks,
        })?;

        for (timestamp, offset) in timestamps.into_iter().zip([0, 3_600]) {
            let dt32 = dt32.next().unwrap();
            let dt64 = dt64.next().unwrap()?;
            for datetime in [dt32, dt64] {
                assert_eq!(datetime.timezone(), Tz::Europe__London);
                assert_eq!(datetime.offset().fix().local_minus_utc(), offset);
            }
            assert_eq!(dt32.timestamp(), timestamp);
            assert_eq!(dt64.timestamp_millis(), timestamp * 1_000 + 123);
        }
        Ok(())
    }

    #[test]
    fn datetime_slice_iterators_match_naive_resolution_for_every_utc_alias() -> TestResult {
        // Exhaustively checks every alias in `conv::UTC_ALIASES` (the single source of
        // truth `utc_alias_offset` also reads): the cached-offset path must match the
        // uncached `with_timezone` ground truth bit-for-bit across the full DateTime32
        // range plus a negative DateTime64 tick, proving the construction-time cache is
        // never stale.
        let seconds: [u32; 3] = [0, 1_700_000_000, u32::MAX];
        let ticks: [i64; 4] = [-1_700_000_000_123, -1, 0, 1_700_000_000_123];

        for tz in conv::UTC_ALIASES {
            let data32: Vec<DateTime32Data> = seconds
                .iter()
                .map(|&s| DateTime32Data(zc::U32::new(s)))
                .collect();
            let data64: Vec<DateTime64Data> = ticks
                .iter()
                .map(|&t| DateTime64Data(zc::I64::new(t)))
                .collect();

            let cached32 =
                DateTime32SliceIterator::try_from(Value::DateTime32Slice { tz, slice: &data32 })?;
            for (cached, &s) in cached32.zip(seconds.iter()) {
                let naive = conv::datetime32(s).with_timezone(&tz);
                assert_eq!(
                    cached.to_rfc3339(),
                    naive.to_rfc3339(),
                    "{tz:?} DateTime32({s})"
                );
                assert_eq!(
                    cached.timezone(),
                    tz,
                    "{tz:?} DateTime32({s}) zone identity"
                );
            }

            let cached64 = DateTime64SliceIterator::try_from(Value::DateTime64Slice {
                tz,
                precision: 3,
                slice: &data64,
            })?;
            for (cached, &t) in cached64.zip(ticks.iter()) {
                let naive = conv::datetime64(t, 3)?.with_timezone(&tz);
                assert_eq!(
                    cached?.to_rfc3339(),
                    naive.to_rfc3339(),
                    "{tz:?} DateTime64({t})"
                );
            }
        }
        Ok(())
    }
}
