use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use chrono::{NaiveDate, TimeDelta, Utc};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use rustc_hash::FxBuildHasher;

use super::{
    DefaultWriter, FixedWidth, FixedWidthColumn, FromReader, ValueWriter as _, write_quoted,
};
use crate::{Error, Result, conv, interval, mark::Mark, reader};

fn uuid_bytes(value: uuid::Uuid) -> [u8; 16] {
    let (hi, lo) = value.as_u64_pair();
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&hi.to_le_bytes());
    bytes[8..].copy_from_slice(&lo.to_le_bytes());
    bytes
}

macro_rules! scalars {
    ($($name:ident: $item:ty => $type_name:literal, $width:literal, |$v:ident| $bytes:expr;)+) => {$(
        #[doc = concat!("The `", $type_name, "` type.")]
        #[derive(Default, Clone, Copy)]
        pub struct $name;

        impl FixedWidth for $name {
            type Value<'a> = $item;

            fn type_name(&self) -> Cow<'static, str> {
                $type_name.into()
            }

            fn width(&self) -> usize {
                $width
            }

            fn write(&self, $v: Self::Value<'_>, data: &mut Vec<u8>) -> Result<()> {
                let bytes: [u8; $width] = $bytes;
                data.extend_from_slice(&bytes);
                Ok(())
            }
        }

        impl<'a> DefaultWriter<'a> for reader::$name<'a> {
            type Writer = FixedWidthColumn<$name>;
        }

        impl<'a> FromReader<'a, reader::$name<'a>> for FixedWidthColumn<$name> {
            fn from_reader(_reader: &reader::$name<'a>) -> Self {
                Self::default()
            }

            fn write_item(&mut self, item: $item) -> Result<()> {
                self.write(item)
            }
        }
    )+};
}

scalars! {
    I8: i8 => "Int8", 1, |v| v.to_le_bytes();
    I16: i16 => "Int16", 2, |v| v.to_le_bytes();
    I32: i32 => "Int32", 4, |v| v.to_le_bytes();
    I64: i64 => "Int64", 8, |v| v.to_le_bytes();
    I128: i128 => "Int128", 16, |v| v.to_le_bytes();
    I256: &'a crate::I256 => "Int256", 32, |v| *v.as_le_bytes();
    U8: u8 => "UInt8", 1, |v| v.to_le_bytes();
    U16: u16 => "UInt16", 2, |v| v.to_le_bytes();
    U32: u32 => "UInt32", 4, |v| v.to_le_bytes();
    U64: u64 => "UInt64", 8, |v| v.to_le_bytes();
    U128: u128 => "UInt128", 16, |v| v.to_le_bytes();
    U256: &'a crate::U256 => "UInt256", 32, |v| *v.as_le_bytes();
    F32: f32 => "Float32", 4, |v| v.to_le_bytes();
    F64: f64 => "Float64", 8, |v| v.to_le_bytes();
    Bf16: half::bf16 => "BFloat16", 2, |v| v.to_le_bytes();
    Bool: bool => "Bool", 1, |v| [u8::from(v)];
    Uuid: uuid::Uuid => "UUID", 16, |v| uuid_bytes(v);
    Ipv4: Ipv4Addr => "IPv4", 4, |v| u32::from(v).to_le_bytes();
    Ipv6: Ipv6Addr => "IPv6", 16, |v| v.octets();
    Date: NaiveDate => "Date", 2, |v| conv::days16(v)?.to_le_bytes();
    Date32: NaiveDate => "Date32", 4, |v| conv::days32(v)?.to_le_bytes();
    Time: TimeDelta => "Time", 4, |v| i32::try_from(v.num_seconds())?.to_le_bytes();
}

#[derive(Default, Clone, Copy)]
pub struct Nothing;

impl FixedWidth for Nothing {
    type Value<'a> = ();

    fn type_name(&self) -> Cow<'static, str> {
        "Nothing".into()
    }

    fn width(&self) -> usize {
        1
    }

    fn push_default(&self, data: &mut Vec<u8>) {
        data.push(b'0');
    }

    fn write(&self, (): (), data: &mut Vec<u8>) -> Result<()> {
        self.push_default(data);
        Ok(())
    }
}

impl DefaultWriter<'_> for reader::Nothing {
    type Writer = FixedWidthColumn<Nothing>;
}

impl FromReader<'_, reader::Nothing> for FixedWidthColumn<Nothing> {
    fn from_reader(_reader: &reader::Nothing) -> Self {
        Self::default()
    }

    fn write_item(&mut self, (): ()) -> Result<()> {
        self.write(())
    }
}

#[derive(Clone, Copy)]
pub struct Usize {
    // TODO: smaller size?
    pub width: usize,
}

impl FixedWidth for Usize {
    type Value<'a> = usize;

    fn type_name(&self) -> Cow<'static, str> {
        match self.width {
            1 => "UInt8",
            2 => "UInt16",
            4 => "UInt32",
            _ => "UInt64",
        }
        .into()
    }

    fn width(&self) -> usize {
        self.width
    }

    fn write(&self, value: usize, data: &mut Vec<u8>) -> Result<()> {
        let width = self.width;
        let wide = u64::try_from(value)?;
        if width < 8 && wide >> (8 * width) != 0 {
            return Err(Error::ValueOutOfRange(
                "usize",
                "column width",
                value.to_string(),
            ));
        }
        data.extend_from_slice(&wide.to_le_bytes()[..width]);
        Ok(())
    }
}

impl<'a> DefaultWriter<'a> for reader::Usize<'a> {
    type Writer = FixedWidthColumn<Usize>;
}

impl<'a> FromReader<'a, reader::Usize<'a>> for FixedWidthColumn<Usize> {
    fn from_reader(reader: &reader::Usize<'a>) -> Self {
        let width = match reader.0 {
            Mark::UInt8(_) => 1,
            Mark::UInt16(_) => 2,
            Mark::UInt32(_) => 4,
            _ => 8,
        };
        Self::new(Usize { width })
    }

    fn write_item(&mut self, item: usize) -> Result<()> {
        self.write(item)
    }
}

macro_rules! enums {
    ($($name:ident: $int:ty => $type_name:literal;)+) => {$(
        #[doc = concat!("The `", $type_name, "(...)` type with its declared variants.")]
        pub struct $name {
            variants: Vec<(Arc<str>, $int)>,
            ids: HashMap<Arc<str>, $int, FxBuildHasher>,
        }

        impl $name {
            pub fn new(variants: &[(&str, $int)]) -> Self {
                let variants = variants
                    .iter()
                    .map(|&(name, id)| (Arc::<str>::from(name), id))
                    .collect::<Vec<_>>();
                let ids = variants
                    .iter()
                    .map(|(name, id)| (Arc::clone(name), *id))
                    .collect();
                Self {
                    variants,
                    ids,
                }
            }
        }

        impl FixedWidth for $name {
            type Value<'a> = &'a str;

            fn type_name(&self) -> Cow<'static, str> {
                let mut out = String::from(concat!($type_name, "("));
                for (index, (name, id)) in self.variants.iter().enumerate() {
                    if index != 0 {
                        out.push_str(", ");
                    }
                    write_quoted(name, &mut out);
                    let _ = write!(out, " = {id}");
                }
                out.push(')');
                out.into()
            }

            fn width(&self) -> usize {
                size_of::<$int>()
            }

            fn push_default(&self, data: &mut Vec<u8>) {
                // ClickHouse's default is the first declared variant.
                let id = self.variants.first().map_or(0, |(_, id)| *id);
                data.extend_from_slice(&id.to_le_bytes());
            }

            fn write(&self, value: &str, data: &mut Vec<u8>) -> Result<()> {
                let Some(id) = self.ids.get(value) else {
                    return Err(Error::ValueOutOfRange(
                        $type_name,
                        "declared variant",
                        value.to_owned(),
                    ));
                };
                data.extend_from_slice(&id.to_le_bytes());
                Ok(())
            }
        }

        impl<'a> DefaultWriter<'a> for reader::$name<'a> {
            type Writer = FixedWidthColumn<$name>;
        }

        impl<'a> FromReader<'a, reader::$name<'a>> for FixedWidthColumn<$name> {
            fn from_reader(reader: &reader::$name<'a>) -> Self {
                Self::new($name::new(&reader.0.variants))
            }

            fn write_item(&mut self, item: &'a str) -> Result<()> {
                self.write(item)
            }
        }
    )+};
}

enums! {
    Enum8: i8 => "Enum8";
    Enum16: i16 => "Enum16";
}

fn quoted_tz(tz: Tz) -> String {
    let mut out = String::new();
    write_quoted(tz.name(), &mut out);
    out
}

#[derive(Default, Clone, Copy)]
pub struct DateTime {
    pub tz: Option<Tz>,
}

impl FixedWidth for DateTime {
    type Value<'a> = chrono::DateTime<Utc>;

    fn type_name(&self) -> Cow<'static, str> {
        match self.tz {
            Some(tz) => format!("DateTime({})", quoted_tz(tz)).into(),
            None => "DateTime".into(),
        }
    }

    fn width(&self) -> usize {
        4
    }

    fn write(&self, value: chrono::DateTime<Utc>, data: &mut Vec<u8>) -> Result<()> {
        let Ok(seconds) = u32::try_from(value.timestamp()) else {
            return Err(Error::ValueOutOfRange(
                "DateTime<Utc>",
                "DateTime",
                value.to_string(),
            ));
        };
        data.extend_from_slice(&seconds.to_le_bytes());
        Ok(())
    }
}

impl<'a> DefaultWriter<'a> for reader::DateTime<'a> {
    type Writer = FixedWidthColumn<DateTime>;
}

impl<'a> FromReader<'a, reader::DateTime<'a>> for FixedWidthColumn<DateTime> {
    fn from_reader(reader: &reader::DateTime<'a>) -> Self {
        let tz = reader.mark.explicit_tz.then_some(reader.mark.tz);
        Self::new(DateTime { tz })
    }

    fn write_item(&mut self, item: chrono::DateTime<Tz>) -> Result<()> {
        self.write(item.to_utc())
    }
}

#[derive(Default, Clone, Copy)]
pub struct DateTime64 {
    pub precision: u8,
    pub tz: Option<Tz>,
}

impl FixedWidth for DateTime64 {
    type Value<'a> = chrono::DateTime<Utc>;

    fn type_name(&self) -> Cow<'static, str> {
        match self.tz {
            Some(tz) => format!("DateTime64({}, {})", self.precision, quoted_tz(tz)).into(),
            None => format!("DateTime64({})", self.precision).into(),
        }
    }

    fn width(&self) -> usize {
        8
    }

    fn write(&self, value: chrono::DateTime<Utc>, data: &mut Vec<u8>) -> Result<()> {
        let total_nanos = i128::from(value.timestamp()) * 1_000_000_000
            + i128::from(value.timestamp_subsec_nanos());
        let ticks = conv::ticks(total_nanos, self.precision, "DateTime64")?;
        data.extend_from_slice(&ticks.to_le_bytes());
        Ok(())
    }
}

impl<'a> DefaultWriter<'a> for reader::DateTime64<'a> {
    type Writer = FixedWidthColumn<DateTime64>;
}

impl<'a> FromReader<'a, reader::DateTime64<'a>> for FixedWidthColumn<DateTime64> {
    fn from_reader(reader: &reader::DateTime64<'a>) -> Self {
        let precision = reader.mark.precision;
        let tz = reader.mark.explicit_tz.then_some(reader.mark.tz);
        Self::new(DateTime64 { precision, tz })
    }

    fn write_item(&mut self, item: chrono::DateTime<Tz>) -> Result<()> {
        self.write(item.to_utc())
    }
}

#[derive(Default, Clone, Copy)]
pub struct Time64 {
    pub precision: u8,
}

impl FixedWidth for Time64 {
    type Value<'a> = TimeDelta;

    fn type_name(&self) -> Cow<'static, str> {
        format!("Time64({})", self.precision).into()
    }

    fn width(&self) -> usize {
        8
    }

    fn write(&self, value: TimeDelta, data: &mut Vec<u8>) -> Result<()> {
        let ticks = conv::ticks(conv::total_nanos(value), self.precision, "Time64")?;
        data.extend_from_slice(&ticks.to_le_bytes());
        Ok(())
    }
}

impl<'a> DefaultWriter<'a> for reader::Time64<'a> {
    type Writer = FixedWidthColumn<Time64>;
}

impl<'a> FromReader<'a, reader::Time64<'a>> for FixedWidthColumn<Time64> {
    fn from_reader(reader: &reader::Time64<'a>) -> Self {
        let precision = reader.0.precision;
        Self::new(Time64 { precision })
    }

    fn write_item(&mut self, item: TimeDelta) -> Result<()> {
        self.write(item)
    }
}

#[derive(Clone, Copy)]
pub struct Interval {
    pub kind: interval::Kind,
}

impl FixedWidth for Interval {
    type Value<'a> = TimeDelta;

    fn type_name(&self) -> Cow<'static, str> {
        format!("Interval{}", self.kind.as_str()).into()
    }

    fn width(&self) -> usize {
        8
    }

    fn write(&self, value: TimeDelta, data: &mut Vec<u8>) -> Result<()> {
        let Some(unit) = self.kind.unit_nanos() else {
            return Err(Error::MismatchedType(
                self.kind.as_str(),
                "fixed-length Interval",
            ));
        };
        let total = conv::total_nanos(value);
        let unit = i128::from(unit);
        if total % unit != 0 {
            return Err(Error::ValueOutOfRange(
                "TimeDelta",
                "whole Interval units",
                value.to_string(),
            ));
        }
        let count = i64::try_from(total / unit)?;
        data.extend_from_slice(&count.to_le_bytes());
        Ok(())
    }
}

impl<'a> DefaultWriter<'a> for reader::Interval<'a> {
    type Writer = FixedWidthColumn<Interval>;
}

impl<'a> FromReader<'a, reader::Interval<'a>> for FixedWidthColumn<Interval> {
    fn from_reader(reader: &reader::Interval<'a>) -> Self {
        let kind = reader.0.kind;
        Self::new(Interval { kind })
    }

    fn write_item(&mut self, item: TimeDelta) -> Result<()> {
        self.write(item)
    }
}

macro_rules! decimals {
    ($($name:ident: $int:ty => $precision:literal;)+) => {$(
        #[doc = concat!("The `Decimal(", $precision, ", scale)` type.")]
        #[derive(Default, Clone, Copy)]
        pub struct $name {
            pub scale: u8,
        }

        impl FixedWidth for $name {
            type Value<'a> = Decimal;

            fn type_name(&self) -> Cow<'static, str> {
                format!("Decimal({}, {})", $precision, self.scale).into()
            }

            fn width(&self) -> usize {
                size_of::<$int>()
            }

            fn write(&self, mut value: Decimal, data: &mut Vec<u8>) -> Result<()> {
                let scale = u32::from(self.scale);
                value.rescale(scale);
                let mantissa = <$int>::try_from(value.mantissa()).ok();
                let Some(mantissa) = mantissa.filter(|_| value.scale() == scale) else {
                    return Err(Error::ValueOutOfRange(
                        "Decimal",
                        stringify!($name),
                        value.to_string(),
                    ));
                };
                data.extend_from_slice(&mantissa.to_le_bytes());
                Ok(())
            }
        }

        impl<'a> DefaultWriter<'a> for reader::$name<'a> {
            type Writer = FixedWidthColumn<$name>;
        }

        impl<'a> FromReader<'a, reader::$name<'a>> for FixedWidthColumn<$name> {
            fn from_reader(reader: &reader::$name<'a>) -> Self {
                let scale = reader.0.scale;
                Self::new($name { scale })
            }

            fn write_item(&mut self, item: Decimal) -> Result<()> {
                self.write(item)
            }
        }
    )+};
}

decimals! {
    Decimal32: i32 => 9;
    Decimal64: i64 => 18;
    Decimal128: i128 => 38;
}
