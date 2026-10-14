use std::hint::cold_path;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::ops::Range;

use chrono::{NaiveDate, TimeDelta};
use chrono_tz::Tz;
use rust_decimal::Decimal;

use super::{ReadSlice, Readable, TryRead};
use crate::error::Error;
use crate::mark::{
    DateTime as DateTimeMark, DateTime64 as DateTime64Mark, Decimal32 as Decimal32Mark,
    Decimal64 as Decimal64Mark, Decimal128 as Decimal128Mark, Enum8 as Enum8Mark,
    Enum16 as Enum16Mark, Interval as IntervalMark, Mark, Time64 as Time64Mark,
};
use crate::{Bf16Data, Date16Data, Date32Data, Ipv4Data, Ipv6Data, UuidData, zc};

macro_rules! col_view {
    ($($name:ident, $variant:ident, $elem:ty, $item:ty, |$v:ident| $conv:expr;)+) => {
        $(
            #[derive(Clone, Copy)]
            pub struct $name<'a>(pub &'a [$elem]);

            impl<'a> TryFrom<&'a Mark<'a>> for $name<'a> {
                type Error = Error;


                fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
                    match value {
                        Mark::$variant(v) => Ok(Self(v.as_slice())),
                        other => {
                            cold_path();
                            Err(Error::MismatchedType(other.as_str(), Self::NAME))
                        }
                    }
                }
            }

            impl<'a> TryRead<'a> for $name<'a> {
                type Item = $item;
                const NAME: &'static str = stringify!($variant);

                #[inline(always)]
                fn len(&self) -> usize {
                    self.0.len()
                }

                #[inline(always)]
                unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
                    let $v = unsafe { self.0.get_unchecked(idx) };
                    Ok($conv)
                }
            }

            impl<'a> ReadSlice<'a> for $name<'a> {
                type Elem = $elem;


                #[inline(always)]
                fn try_read_slice(
                    &self,
                    range: Range<usize>,
                ) -> crate::Result<&'a [Self::Elem]> {
                    let Some(slice) = self.0.get(range.clone()) else {
                        cold_path();
                        return Err(Error::RangeOutOfBounds(range, stringify!($variant)));
                    };
                    Ok(slice)
                }
            }
        )+
    };
}

col_view! {
    I8, Int8, i8, i8, |v| *v;
    I16, Int16, zc::I16, i16, |v| v.get();
    I32, Int32, zc::I32, i32, |v| v.get();
    I64, Int64, zc::I64, i64, |v| v.get();
    I128, Int128, zc::I128, i128, |v| v.get();
    I256, Int256, crate::I256, &'a crate::I256, |v| v;
    U8, UInt8, u8, u8, |v| *v;
    U16, UInt16, zc::U16, u16, |v| v.get();
    U32, UInt32, zc::U32, u32, |v| v.get();
    U64, UInt64, zc::U64, u64, |v| v.get();
    U128, UInt128, zc::U128, u128, |v| v.get();
    U256, UInt256, crate::U256, &'a crate::U256, |v| v;
    F32, Float32, zc::F32, f32, |v| v.get();
    F64, Float64, zc::F64, f64, |v| v.get();
    Bf16, BFloat16, Bf16Data, half::bf16, |v| half::bf16::from(*v);
    Uuid, Uuid, UuidData, uuid::Uuid, |v| uuid::Uuid::from(*v);
    Ipv4, Ipv4, Ipv4Data, Ipv4Addr, |v| Ipv4Addr::from(*v);
    Ipv6, Ipv6, Ipv6Data, Ipv6Addr, |v| Ipv6Addr::from(*v);
    Date, Date, Date16Data, NaiveDate, |v| NaiveDate::from(*v);
    Date32, Date32, Date32Data, NaiveDate, |v| NaiveDate::try_from(*v)?;
    Time, Time, zc::I32, TimeDelta, |v| TimeDelta::seconds(i64::from(v.get()));
}

#[derive(Clone, Copy)]
pub struct Usize<'a>(pub(crate) &'a Mark<'a>);

impl<'a> Usize<'a> {
    pub fn new(value: &'a Mark<'a>) -> crate::Result<Self> {
        Self::try_from(value)
    }
}

impl<'a> TryRead<'a> for Usize<'a> {
    type Item = usize;

    const NAME: &'static str = "UInt8/16/32/64";

    fn len(&self) -> usize {
        self.0.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        unsafe {
            match self.0 {
                Mark::UInt8(v) => Ok(usize::from(*v.as_slice().get_unchecked(idx))),
                Mark::UInt16(v) => Ok(v.as_slice().get_unchecked(idx).get() as usize),
                Mark::UInt32(v) => Ok(v.as_slice().get_unchecked(idx).get() as usize),
                Mark::UInt64(v) => Ok(usize::try_from(v.as_slice().get_unchecked(idx).get())?),
                // Construction accepted only the four variants above.
                _ => std::hint::unreachable_unchecked(),
            }
        }
    }
}

impl<'a> TryFrom<&'a Mark<'a>> for Usize<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::UInt8(_) | Mark::UInt16(_) | Mark::UInt32(_) | Mark::UInt64(_) => {
                Ok(Usize(value))
            }
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

/// `Nothing` is the type of `[]` and `NULL`, seen as `Array(Nothing)` and `Nullable(Nothing)`.
#[derive(Clone, Copy)]
pub struct Nothing(pub usize);

impl<'a> TryFrom<&'a Mark<'a>> for Nothing {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::Nothing(len) => Ok(Self(*len)),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl TryRead<'_> for Nothing {
    type Item = ();

    const NAME: &'static str = "Nothing";

    #[inline(always)]
    fn len(&self) -> usize {
        self.0
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, _idx: usize) -> crate::Result<Self::Item> {
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct Bool<'a>(pub &'a [u8]);

impl<'a> TryFrom<&'a Mark<'a>> for Bool<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::Bool(v) => Ok(Self(v.data)),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a> TryRead<'a> for Bool<'a> {
    type Item = bool;

    const NAME: &'static str = "Bool";

    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        Ok(unsafe { *self.0.get_unchecked(idx) } != 0)
    }
}

impl<'a> ReadSlice<'a> for Bool<'a> {
    type Elem = u8;

    #[inline(always)]
    fn try_read_slice(&self, range: Range<usize>) -> crate::Result<&'a [Self::Elem]> {
        let Some(slice) = self.0.get(range.clone()) else {
            cold_path();
            return Err(Error::RangeOutOfBounds(range, "Bool"));
        };
        Ok(slice)
    }
}

#[derive(Clone, Copy)]
pub struct Enum8<'a>(pub &'a Enum8Mark<'a>);

impl<'a> TryFrom<&'a Mark<'a>> for Enum8<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::Enum8(e) => Ok(Self(e)),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a> TryRead<'a> for Enum8<'a> {
    type Item = &'a str;

    const NAME: &'static str = "Enum8";

    fn len(&self) -> usize {
        self.0.data.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        self.name(unsafe { *self.0.data.as_slice().get_unchecked(idx) })
    }
}

impl<'a> Enum8<'a> {
    fn name(&self, variant: i8) -> crate::Result<&'a str> {
        let Ok(pos) = self
            .0
            .variants
            .binary_search_by_key(&variant, |(_, id)| *id)
        else {
            cold_path();
            return Err(Error::CorruptedData(format!(
                "invalid Enum8 discriminant: {variant}"
            )));
        };
        Ok(self.0.variants[pos].0.as_ref())
    }
}

#[derive(Clone, Copy)]
pub struct Enum16<'a>(pub &'a Enum16Mark<'a>);

impl<'a> TryFrom<&'a Mark<'a>> for Enum16<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::Enum16(e) => Ok(Self(e)),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a> TryRead<'a> for Enum16<'a> {
    type Item = &'a str;

    const NAME: &'static str = "Enum16";

    fn len(&self) -> usize {
        self.0.data.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        self.name(unsafe { self.0.data.as_slice().get_unchecked(idx) }.get())
    }
}

impl<'a> Enum16<'a> {
    fn name(&self, variant: i16) -> crate::Result<&'a str> {
        let Ok(pos) = self
            .0
            .variants
            .binary_search_by_key(&variant, |(_, id)| *id)
        else {
            cold_path();
            return Err(Error::CorruptedData(format!(
                "invalid Enum16 discriminant: {variant}"
            )));
        };
        Ok(self.0.variants[pos].0.as_ref())
    }
}

#[derive(Clone, Copy)]
pub struct DateTime<'a> {
    mark: &'a DateTimeMark<'a>,
    // Cached only when offset is stable and doesn't mess with DST stuff.
    cached_offset: Option<chrono_tz::TzOffset>,
}

impl<'a> TryFrom<&'a Mark<'a>> for DateTime<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::DateTime(dt) => Ok(Self {
                mark: dt,
                cached_offset: crate::conv::utc_alias_offset(dt.tz),
            }),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a> TryRead<'a> for DateTime<'a> {
    type Item = chrono::DateTime<Tz>;

    const NAME: &'static str = "DateTime";

    #[inline(always)]
    fn len(&self) -> usize {
        self.mark.data.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        Ok(self.resolve(
            unsafe { self.mark.data.as_slice().get_unchecked(idx) }
                .0
                .get(),
        ))
    }
}

impl DateTime<'_> {
    #[inline(always)]
    fn resolve(&self, seconds: u32) -> chrono::DateTime<Tz> {
        crate::conv::datetime32_resolved(seconds, self.mark.tz, self.cached_offset)
    }
}

#[derive(Clone, Copy)]
pub struct DateTime64<'a> {
    mark: &'a DateTime64Mark<'a>,
    // Cached only when offset is stable and doesn't mess with DST stuff.
    cached_offset: Option<chrono_tz::TzOffset>,
}

impl<'a> TryFrom<&'a Mark<'a>> for DateTime64<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::DateTime64(dt) => Ok(Self {
                mark: dt,
                cached_offset: crate::conv::utc_alias_offset(dt.tz),
            }),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a> TryRead<'a> for DateTime64<'a> {
    type Item = chrono::DateTime<Tz>;

    const NAME: &'static str = "DateTime64";

    fn len(&self) -> usize {
        self.mark.data.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        self.resolve(
            unsafe { self.mark.data.as_slice().get_unchecked(idx) }
                .0
                .get(),
        )
    }
}

impl DateTime64<'_> {
    #[inline(always)]
    fn resolve(&self, ticks: i64) -> crate::Result<chrono::DateTime<Tz>> {
        crate::conv::datetime64_resolved(
            ticks,
            self.mark.precision,
            self.mark.tz,
            self.cached_offset,
        )
    }
}

macro_rules! col_decimal {
    ($($name:ident, $variant:ident, $mark:ty, |$v:ident, $scale:ident| $conv:expr;)+) => {
        $(
            #[derive(Clone, Copy)]
            pub struct $name<'a>(pub &'a $mark);

            impl<'a> TryFrom<&'a Mark<'a>> for $name<'a> {
                type Error = Error;


                fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
                    match value {
                        Mark::$variant(d) => Ok(Self(d)),
                        other => {
                            cold_path();
                            Err(Error::MismatchedType(other.as_str(), Self::NAME))
                        }
                    }
                }
            }

            impl<'a> TryRead<'a> for $name<'a> {
                type Item = Decimal;

                const NAME: &'static str = stringify!($variant);

                fn len(&self) -> usize {
                    self.0.data.len()
                }

                unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
                    let $v = unsafe { self.0.data.as_slice().get_unchecked(idx) };
                    let $scale = self.0.scale;
                    $conv
                }
            }
        )+
    };
}

col_decimal! {
    Decimal32, Decimal32, Decimal32Mark<'a>, |v, s| Ok(v.with_scale(s));
    Decimal64, Decimal64, Decimal64Mark<'a>, |v, s| Ok(v.with_scale(s));
    Decimal128, Decimal128, Decimal128Mark<'a>, |v, s| v.with_scale(s);
}

#[derive(Clone, Copy)]
pub struct Time64<'a>(pub &'a Time64Mark<'a>);

impl<'a> TryFrom<&'a Mark<'a>> for Time64<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::Time64(t) => Ok(Self(t)),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a> TryRead<'a> for Time64<'a> {
    type Item = TimeDelta;

    const NAME: &'static str = "Time64";

    #[inline(always)]
    fn len(&self) -> usize {
        self.0.data.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        let ticks = unsafe { self.0.data.as_slice().get_unchecked(idx) }.get();
        crate::conv::time64(ticks, self.0.precision)
    }
}

#[derive(Clone, Copy)]
pub struct Interval<'a>(pub &'a IntervalMark<'a>);

impl<'a> TryFrom<&'a Mark<'a>> for Interval<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::Interval(i) if i.kind.is_fixed_length() => Ok(Self(i)),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a> TryRead<'a> for Interval<'a> {
    type Item = TimeDelta;

    const NAME: &'static str = "TimeDelta";

    #[inline(always)]
    fn len(&self) -> usize {
        self.0.data.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        let count = unsafe { self.0.data.as_slice().get_unchecked(idx) }.get();
        self.0.kind.to_time_delta(count)
    }
}

/// It's an escape hatch for runtime-typed columns like [`Mark::Variant`], [`Mark::Dynamic`],
/// or [`Mark::Json`].
///
/// Accepts any [`Mark`] and yields [`Value`]s.
///
/// If you don't want the Value explicitly for some reason, use normal columns.
#[derive(Clone, Copy)]
pub struct Value<'a>(pub &'a Mark<'a>);

impl<'a> TryFrom<&'a Mark<'a>> for Value<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        Ok(Self(value))
    }
}

impl<'a> TryRead<'a> for Value<'a> {
    type Item = crate::value::Value<'a>;

    const NAME: &'static str = "Value";

    fn len(&self) -> usize {
        self.0.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        // `Mark::get` dispatches on the runtime type and bounds-checks on its own; there is no
        // unchecked form.
        let Some(value) = self.0.get(idx)? else {
            cold_path();
            return Err(Error::IndexOutOfBounds(idx, self.0.as_str()));
        };
        Ok(value)
    }
}

/// Implements canonical default readers for scalar rust types; see [`Readable`].
macro_rules! readable {
    ($($item:ty => $reader:ty;)+) => {
        $(
            impl<'a> Readable<'a> for $item {
                type Reader = $reader;
            }
        )+
    };
}

readable! {
    i8 => I8<'a>;
    i16 => I16<'a>;
    i32 => I32<'a>;
    i64 => I64<'a>;
    i128 => I128<'a>;
    &'a crate::I256 => I256<'a>;
    u8 => U8<'a>;
    u16 => U16<'a>;
    u32 => U32<'a>;
    u64 => U64<'a>;
    u128 => U128<'a>;
    &'a crate::U256 => U256<'a>;
    f32 => F32<'a>;
    f64 => F64<'a>;
    half::bf16 => Bf16<'a>;
    bool => Bool<'a>;
    uuid::Uuid => Uuid<'a>;
    Ipv4Addr => Ipv4<'a>;
    Ipv6Addr => Ipv6<'a>;
    NaiveDate => Date<'a>;
    chrono::DateTime<Tz> => DateTime<'a>;
    TimeDelta => Interval<'a>;
    crate::value::Value<'a> => Value<'a>;
    () => Nothing;
}
