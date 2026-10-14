use std::collections::HashMap;
use std::fmt::Write as _;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use chrono::{NaiveDate, TimeDelta};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use rustc_hash::FxBuildHasher;

use super::{ConstSizeType, ConstSizeWriter, Writable, WriteColumn, write_quoted};
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

        impl ConstSizeType for $name {
            fn write_type_name(&self, out: &mut String) {
                out.push_str($type_name);
            }

            fn value_size(&self) -> usize {
                $width
            }
        }

        impl<'a> Writable<'a> for reader::$name<'a> {
            type Writer = ConstSizeWriter<$name>;
        }

        impl<'a> WriteColumn<'a, reader::$name<'a>> for ConstSizeWriter<$name> {
            fn for_reader(_reader: &reader::$name<'a>) -> Self {
                Self::default()
            }

            fn write(&mut self, $v: $item) -> Result<()> {
                let bytes: [u8; $width] = $bytes;
                self.data.extend_from_slice(&bytes);
                Ok(())
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

impl ConstSizeType for Nothing {
    fn write_type_name(&self, out: &mut String) {
        out.push_str("Nothing");
    }

    fn value_size(&self) -> usize {
        1
    }

    fn write_default(&self, data: &mut Vec<u8>) {
        data.push(b'0');
    }
}

impl Writable<'_> for reader::Nothing {
    type Writer = ConstSizeWriter<Nothing>;
}

impl WriteColumn<'_, reader::Nothing> for ConstSizeWriter<Nothing> {
    fn for_reader(_reader: &reader::Nothing) -> Self {
        Self::default()
    }

    fn write(&mut self, (): ()) -> Result<()> {
        self.ty.write_default(&mut self.data);
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct Usize {
    // TODO: smaller size?
    pub width: usize,
}

impl ConstSizeType for Usize {
    fn write_type_name(&self, out: &mut String) {
        out.push_str(match self.width {
            1 => "UInt8",
            2 => "UInt16",
            4 => "UInt32",
            _ => "UInt64",
        });
    }

    fn value_size(&self) -> usize {
        self.width
    }
}

impl<'a> Writable<'a> for reader::Usize<'a> {
    type Writer = ConstSizeWriter<Usize>;
}

impl<'a> WriteColumn<'a, reader::Usize<'a>> for ConstSizeWriter<Usize> {
    fn for_reader(reader: &reader::Usize<'a>) -> Self {
        let width = match reader.0 {
            Mark::UInt8(_) => 1,
            Mark::UInt16(_) => 2,
            Mark::UInt32(_) => 4,
            _ => 8,
        };
        Self::new(Usize { width })
    }

    fn write(&mut self, item: usize) -> Result<()> {
        let width = self.ty.width;
        let value = u64::try_from(item)?;
        if width < 8 && value >> (8 * width) != 0 {
            return Err(Error::ValueOutOfRange(
                "usize",
                "column width",
                item.to_string(),
            ));
        }
        self.data.extend_from_slice(&value.to_le_bytes()[..width]);
        Ok(())
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

        impl ConstSizeType for $name {
            fn write_type_name(&self, out: &mut String) {
                out.push_str(concat!($type_name, "("));
                for (index, (name, id)) in self.variants.iter().enumerate() {
                    if index != 0 {
                        out.push_str(", ");
                    }
                    write_quoted(name, out);
                    let _ = write!(out, " = {id}");
                }
                out.push(')');
            }

            fn value_size(&self) -> usize {
                size_of::<$int>()
            }

            fn write_default(&self, data: &mut Vec<u8>) {
                // ClickHouse's default is the first declared variant.
                let id = self.variants.first().map_or(0, |(_, id)| *id);
                data.extend_from_slice(&id.to_le_bytes());
            }
        }

        impl<'a> Writable<'a> for reader::$name<'a> {
            type Writer = ConstSizeWriter<$name>;
        }

        impl<'a> WriteColumn<'a, reader::$name<'a>> for ConstSizeWriter<$name> {
            fn for_reader(reader: &reader::$name<'a>) -> Self {
                Self::new($name::new(&reader.0.variants))
            }

            fn write(&mut self, item: &'a str) -> Result<()> {
                let Some(id) = self.ty.ids.get(item) else {
                    return Err(Error::ValueOutOfRange(
                        $type_name,
                        "declared variant",
                        item.to_owned(),
                    ));
                };
                self.data.extend_from_slice(&id.to_le_bytes());
                Ok(())
            }
        }
    )+};
}

enums! {
    Enum8: i8 => "Enum8";
    Enum16: i16 => "Enum16";
}

fn write_tz(tz: Option<Tz>, out: &mut String) {
    if let Some(tz) = tz {
        write_quoted(tz.name(), out);
    }
}

#[derive(Default, Clone, Copy)]
pub struct DateTime {
    pub tz: Option<Tz>,
}

impl ConstSizeType for DateTime {
    fn write_type_name(&self, out: &mut String) {
        out.push_str("DateTime");
        if self.tz.is_some() {
            out.push('(');
            write_tz(self.tz, out);
            out.push(')');
        }
    }

    fn value_size(&self) -> usize {
        4
    }
}

impl<'a> Writable<'a> for reader::DateTime<'a> {
    type Writer = ConstSizeWriter<DateTime>;
}

impl<'a> WriteColumn<'a, reader::DateTime<'a>> for ConstSizeWriter<DateTime> {
    fn for_reader(reader: &reader::DateTime<'a>) -> Self {
        let tz = reader.mark.explicit_tz.then_some(reader.mark.tz);
        Self::new(DateTime { tz })
    }

    fn write(&mut self, item: chrono::DateTime<Tz>) -> Result<()> {
        let Ok(seconds) = u32::try_from(item.timestamp()) else {
            return Err(Error::ValueOutOfRange(
                "DateTime<Tz>",
                "DateTime",
                item.to_string(),
            ));
        };
        self.data.extend_from_slice(&seconds.to_le_bytes());
        Ok(())
    }
}

#[derive(Default, Clone, Copy)]
pub struct DateTime64 {
    pub precision: u8,
    pub tz: Option<Tz>,
}

impl ConstSizeType for DateTime64 {
    fn write_type_name(&self, out: &mut String) {
        let _ = write!(out, "DateTime64({}", self.precision);
        if self.tz.is_some() {
            out.push_str(", ");
            write_tz(self.tz, out);
        }
        out.push(')');
    }

    fn value_size(&self) -> usize {
        8
    }
}

impl<'a> Writable<'a> for reader::DateTime64<'a> {
    type Writer = ConstSizeWriter<DateTime64>;
}

impl<'a> WriteColumn<'a, reader::DateTime64<'a>> for ConstSizeWriter<DateTime64> {
    fn for_reader(reader: &reader::DateTime64<'a>) -> Self {
        let precision = reader.mark.precision;
        let tz = reader.mark.explicit_tz.then_some(reader.mark.tz);
        Self::new(DateTime64 { precision, tz })
    }

    fn write(&mut self, item: chrono::DateTime<Tz>) -> Result<()> {
        let total_nanos = i128::from(item.timestamp()) * 1_000_000_000
            + i128::from(item.timestamp_subsec_nanos());
        let ticks = conv::ticks(total_nanos, self.ty.precision, "DateTime64")?;
        self.data.extend_from_slice(&ticks.to_le_bytes());
        Ok(())
    }
}

#[derive(Default, Clone, Copy)]
pub struct Time64 {
    pub precision: u8,
}

impl ConstSizeType for Time64 {
    fn write_type_name(&self, out: &mut String) {
        let _ = write!(out, "Time64({})", self.precision);
    }

    fn value_size(&self) -> usize {
        8
    }
}

impl<'a> Writable<'a> for reader::Time64<'a> {
    type Writer = ConstSizeWriter<Time64>;
}

impl<'a> WriteColumn<'a, reader::Time64<'a>> for ConstSizeWriter<Time64> {
    fn for_reader(reader: &reader::Time64<'a>) -> Self {
        let precision = reader.0.precision;
        Self::new(Time64 { precision })
    }

    fn write(&mut self, item: TimeDelta) -> Result<()> {
        let ticks = conv::ticks(conv::total_nanos(item), self.ty.precision, "Time64")?;
        self.data.extend_from_slice(&ticks.to_le_bytes());
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct Interval {
    pub kind: interval::Kind,
}

impl ConstSizeType for Interval {
    fn write_type_name(&self, out: &mut String) {
        out.push_str("Interval");
        out.push_str(self.kind.as_str());
    }

    fn value_size(&self) -> usize {
        8
    }
}

impl<'a> Writable<'a> for reader::Interval<'a> {
    type Writer = ConstSizeWriter<Interval>;
}

impl<'a> WriteColumn<'a, reader::Interval<'a>> for ConstSizeWriter<Interval> {
    fn for_reader(reader: &reader::Interval<'a>) -> Self {
        let kind = reader.0.kind;
        Self::new(Interval { kind })
    }

    fn write(&mut self, item: TimeDelta) -> Result<()> {
        let kind = self.ty.kind;
        let Some(unit) = kind.unit_nanos() else {
            return Err(Error::MismatchedType(
                kind.as_str(),
                "fixed-length Interval",
            ));
        };
        let total = conv::total_nanos(item);
        let unit = i128::from(unit);
        if total % unit != 0 {
            return Err(Error::ValueOutOfRange(
                "TimeDelta",
                "whole Interval units",
                item.to_string(),
            ));
        }
        let count = i64::try_from(total / unit)?;
        self.data.extend_from_slice(&count.to_le_bytes());
        Ok(())
    }
}

macro_rules! decimals {
    ($($name:ident: $int:ty => $precision:literal;)+) => {$(
        #[doc = concat!("The `Decimal(", $precision, ", scale)` type.")]
        #[derive(Default, Clone, Copy)]
        pub struct $name {
            pub scale: u8,
        }

        impl ConstSizeType for $name {
            fn write_type_name(&self, out: &mut String) {
                let _ = write!(out, "Decimal({}, {})", $precision, self.scale);
            }

            fn value_size(&self) -> usize {
                size_of::<$int>()
            }
        }

        impl<'a> Writable<'a> for reader::$name<'a> {
            type Writer = ConstSizeWriter<$name>;
        }

        impl<'a> WriteColumn<'a, reader::$name<'a>> for ConstSizeWriter<$name> {
            fn for_reader(reader: &reader::$name<'a>) -> Self {
                let scale = reader.0.scale;
                Self::new($name { scale })
            }

            fn write(&mut self, mut item: Decimal) -> Result<()> {
                let scale = u32::from(self.ty.scale);
                item.rescale(scale);
                let mantissa = <$int>::try_from(item.mantissa()).ok();
                let Some(mantissa) = mantissa.filter(|_| item.scale() == scale) else {
                    return Err(Error::ValueOutOfRange(
                        "Decimal",
                        stringify!($name),
                        item.to_string(),
                    ));
                };
                self.data.extend_from_slice(&mantissa.to_le_bytes());
                Ok(())
            }
        }
    )+};
}

decimals! {
    Decimal32: i32 => 9;
    Decimal64: i64 => 18;
    Decimal128: i128 => 38;
}
