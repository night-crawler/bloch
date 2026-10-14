use std::{borrow::Cow, hint::cold_path};

use crate::mark::BoolView;
use crate::zc;
use crate::{
    interval,
    mark::{
        DateTime, DateTime64, Decimal32, Decimal64, Decimal128, Decimal256, Enum8, Enum16,
        FixedString, Interval, Mark, Time64,
    },
    slice::ByteView,
};
pub use chrono_tz::Tz;

pub type Offsets<'a> = ByteView<'a, zc::U64>;

pub trait OffsetIndexPair {
    fn offset_indices(&self, index: usize) -> crate::Result<Option<(usize, usize)>>;
    /// # Safety
    /// `index < self.len()`.
    unsafe fn offset_indices_unchecked(&self, index: usize) -> crate::Result<(usize, usize)>;
    fn last_or_default(&self) -> crate::Result<usize>;
}

impl OffsetIndexPair for [zc::U64] {
    #[inline(always)]
    fn offset_indices(&self, index: usize) -> crate::Result<Option<(usize, usize)>> {
        let Some(end) = self.get(index) else {
            return Ok(None);
        };
        let end = cast_offset(end.get())?;
        let start = if index == 0 {
            0
        } else {
            // SAFETY: the successful `get` above proves `index < self.len()`.
            cast_offset(unsafe { self.get_unchecked(index - 1) }.get())?
        };
        Ok(Some((start, end)))
    }

    #[inline(always)]
    unsafe fn offset_indices_unchecked(&self, index: usize) -> crate::Result<(usize, usize)> {
        let end = cast_offset(unsafe { self.get_unchecked(index) }.get())?;
        let start = if index == 0 {
            0
        } else {
            cast_offset(unsafe { self.get_unchecked(index - 1) }.get())?
        };
        Ok((start, end))
    }

    fn last_or_default(&self) -> crate::Result<usize> {
        match self.last() {
            Some(last) => cast_offset(last.get()),
            None => Ok(0),
        }
    }
}

impl OffsetIndexPair for Offsets<'_> {
    #[inline(always)]
    fn offset_indices(&self, index: usize) -> crate::Result<Option<(usize, usize)>> {
        self.as_slice().offset_indices(index)
    }

    #[inline(always)]
    unsafe fn offset_indices_unchecked(&self, index: usize) -> crate::Result<(usize, usize)> {
        unsafe { self.as_slice().offset_indices_unchecked(index) }
    }

    fn last_or_default(&self) -> crate::Result<usize> {
        self.as_slice().last_or_default()
    }
}

#[inline(always)]
fn cast_offset(value: u64) -> crate::Result<usize> {
    match usize::try_from(value) {
        Ok(value) => Ok(value),
        Err(_) => {
            cold_path();
            Err(crate::Error::Overflow(value.to_string()))
        }
    }
}

#[derive(Debug)]
pub struct MapHeader<'a> {
    pub(crate) key: TypeHeader<'a>,
    pub(crate) value: TypeHeader<'a>,
}

#[derive(Debug)]
pub struct DynamicHeader<'a> {
    pub(crate) types: Vec<Type<'a>>,
    pub(crate) headers: Vec<TypeHeader<'a>>,
}

#[derive(Debug)]
pub struct JsonHeader<'a> {
    pub(crate) paths: Vec<Cow<'a, str>>,
    pub(crate) col_headers: Vec<JsonColumnHeader<'a>>,
}

#[derive(Debug)]
pub enum TypeHeader<'a> {
    Empty,
    Tuple(Vec<TypeHeader<'a>>),
    Json(Box<JsonHeader<'a>>),
    Map(Box<MapHeader<'a>>),
    Variant(Vec<TypeHeader<'a>>),
    Array(Box<TypeHeader<'a>>),
    Dynamic(Box<DynamicHeader<'a>>),
    Nested(Vec<TypeHeader<'a>>),
}

impl<'a> TypeHeader<'a> {
    pub(crate) fn into_array(self) -> TypeHeader<'a> {
        match self {
            TypeHeader::Array(inner) => *inner,
            e => unreachable!("bug: unexpected type header: {e:?}"),
        }
    }

    pub(crate) fn into_tuple(self) -> Vec<TypeHeader<'a>> {
        match self {
            TypeHeader::Tuple(t) => t,
            e => unreachable!("bug: unexpected type header: {e:?}"),
        }
    }

    pub(crate) fn into_map(self) -> MapHeader<'a> {
        match self {
            TypeHeader::Map(map) => *map,
            e => unreachable!("bug: unexpected type header: {e:?}"),
        }
    }

    pub(crate) fn into_variant(self) -> Vec<TypeHeader<'a>> {
        match self {
            TypeHeader::Variant(variants) => variants,
            e => unreachable!("bug: unexpected type header: {e:?}"),
        }
    }

    pub(crate) fn into_json(self) -> JsonHeader<'a> {
        match self {
            TypeHeader::Json(json) => *json,
            e => unreachable!("bug: unexpected type header: {e:?}"),
        }
    }

    pub(crate) fn into_dynamic(self) -> DynamicHeader<'a> {
        match self {
            TypeHeader::Dynamic(d) => *d,
            e => unreachable!("bug: unexpected type header: {e:?}"),
        }
    }

    pub(crate) fn into_nested(self) -> Vec<TypeHeader<'a>> {
        match self {
            TypeHeader::Nested(n) => n,
            e => unreachable!("Unexpected type header: {e:?}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type<'a> {
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    Int128,
    Int256,

    UInt8,
    UInt16,
    UInt32,
    UInt64,
    UInt128,
    UInt256,

    Float32,
    Float64,

    BFloat16,

    Decimal32(u8),
    Decimal64(u8),
    Decimal128(u8),
    Decimal256(u8),

    String,
    FixedString(usize),

    Uuid,

    Date,
    Date32,
    DateTime(Tz),
    DateTime64(u8, Tz),
    /// Signed seconds as `Int32` (`DataTypeTime : DataTypeNumberBase<Int32>`); no timezone.
    /// ClickHouse saturates only the text form at `±999:59:59`; stored values may exceed it.
    Time,
    /// Signed `Int64` ticks of `10^-precision` seconds
    /// (`DataTypeTime64 : DataTypeDecimalBase<Time64>`); no timezone.
    Time64(u8),
    Interval(interval::Kind),

    Ipv4,
    Ipv6,

    /// Point is represented by its X and Y coordinates, stored as a Tuple(Float64, Float64).
    Point,

    /// Ring is a simple polygon without holes stored as an array of points: Array(Point).
    Ring,

    /// Polygon is a polygon with holes stored as an array of rings: Array(Ring).
    /// First element of outer array is the outer shape of polygon and all the following
    /// elements are holes.
    Polygon,

    /// MultiPolygon consists of multiple polygons and is stored as an array of polygons:
    /// Array(Polygon).
    MultiPolygon,

    /// LineString is a line stored as an array of points: Array(Point).
    LineString,

    /// MultiLineString is multiple lines stored as an array of LineString: Array(LineString).
    MultiLineString,

    /// MultiPoint is a set of points stored as Array(Point).
    MultiPoint,

    /// Geometry is a Variant over every geo type with a fixed discriminator order
    /// (`registerDataTypeDomainGeo`): LineString, MultiLineString, MultiPolygon, Point, Polygon,
    /// Ring, MultiPoint. New geo types are appended, never sorted in.
    Geometry,

    Enum8(Vec<(Cow<'a, str>, i8)>),
    Enum16(Vec<(Cow<'a, str>, i16)>),

    LowCardinality(Box<Type<'a>>),

    Array(Box<Type<'a>>),

    Tuple(Vec<Type<'a>>),

    Nullable(Box<Type<'a>>),

    Map(Box<Type<'a>>, Box<Type<'a>>),

    Variant(Vec<Type<'a>>),

    Nested(Vec<Field<'a>>),
    NamedTuple(Vec<Field<'a>>),

    Dynamic,
    Json(Vec<Field<'a>>),

    Nothing,

    /// From `src/Columns/ColumnDynamic.h`:
    ///
    /// > When new values are inserted into Dynamic column, the internal Variant type and
    /// > column are extended if the inserted value has new type. When the limit on number of
    /// > dynamic types is exceeded, all values with new types are inserted into special
    /// > shared variant with type String that contains values and their types in binary
    /// > format.
    /// >
    /// > When max_dynamic_types = 0, we will have only shared variant and insert all values
    /// > into it.
    ///
    /// From `src/Columns/ColumnDynamic.cpp`:
    ///
    /// > Shared variant will contain String values but we cannot use usual String type
    /// > because we can have regular variant with type String. To solve it, we use String
    /// > type with custom name for shared variant.
    SharedVariant,
}

#[expect(clippy::multiple_inherent_impl)]
impl<'a> Type<'a> {
    pub(crate) const fn is_nullable(&self) -> bool {
        matches!(self, Type::Nullable(_))
    }
    pub(crate) fn strip_null(&self) -> &Type<'a> {
        match self {
            Type::Nullable(inner) => inner,
            _ => self,
        }
    }

    /// The `Variant` a `Geometry` column is serialized as. The element order is the
    /// discriminator order, see [`Type::Geometry`].
    pub(crate) fn geometry_variant() -> Type<'a> {
        Type::Variant(vec![
            Type::LineString,
            Type::MultiLineString,
            Type::MultiPolygon,
            Type::Point,
            Type::Polygon,
            Type::Ring,
            Type::MultiPoint,
        ])
    }

    pub(crate) const fn size(&self) -> Option<usize> {
        #[expect(clippy::match_same_arms)]
        match self {
            Self::Bool => Some(1),
            Self::Int8 => Some(1),
            Self::Int16 => Some(2),
            Self::Int32 => Some(4),
            Self::Int64 => Some(8),
            Self::Int128 => Some(16),
            Self::Int256 => Some(32),
            Self::UInt8 => Some(1),
            Self::UInt16 => Some(2),
            Self::UInt32 => Some(4),
            Self::UInt64 => Some(8),
            Self::UInt128 => Some(16),
            Self::UInt256 => Some(32),

            Self::Float32 => Some(4),
            Self::Float64 => Some(8),
            Self::BFloat16 => Some(2),

            Self::Uuid => Some(16),

            Self::Decimal32(_) => Some(4),
            Self::Decimal64(_) => Some(8),
            Self::Decimal128(_) => Some(16),
            Self::Decimal256(_) => Some(32),

            Self::FixedString(size) => Some(*size),

            Self::Ipv4 => Some(4),
            Self::Ipv6 => Some(16),

            Self::Date => Some(2),
            Self::Date32 => Some(4),
            Self::DateTime(_) => Some(4),
            Self::DateTime64(_, _) => Some(8),
            Self::Time => Some(4),
            Self::Time64(_) => Some(8),
            Self::Interval(_) => Some(8),
            Self::Enum8(_) => Some(1),
            Self::Enum16(_) => Some(2),
            Self::Nothing => Some(1),

            // Point is represented by its X and Y coordinates, stored as a Tuple(Float64, Float64).
            Self::Point => None,

            // For completeness, everything below is variable in size
            Self::Ring => None,
            Self::Polygon => None,
            Self::MultiPolygon => None,
            Self::LineString => None,
            Self::MultiLineString => None,
            Self::MultiPoint => None,
            Self::Geometry => None,
            Self::Map(_, _) => None,

            Self::Array(_) => None,

            // we can calculate the size for the tuple of fixed size types, but still we'll need
            // to parse nested columns later, so it's not worth it
            Self::Tuple(_) => None,
            Self::NamedTuple(_) => None,

            // TODO: is it always variable?
            Self::Variant(_) => None,
            Self::Dynamic => None,
            Self::Json(_) => None,

            Self::Nullable(_) => None,
            Self::LowCardinality(_) => None,
            Self::String => None,
            Self::Nested(_) => None,
            Self::SharedVariant => None,
        }
    }

    pub(crate) fn into_fixed_size_marker(self, data: &'a [u8]) -> crate::Result<Mark<'a>> {
        let mark = match self {
            Type::Bool => Mark::Bool(BoolView { data }),
            Type::Int8 => Mark::Int8(ByteView::try_from(data)?),
            Type::Int16 => Mark::Int16(ByteView::try_from(data)?),
            Type::Int32 => Mark::Int32(ByteView::try_from(data)?),
            Type::Int64 => Mark::Int64(ByteView::try_from(data)?),
            Type::Int128 => Mark::Int128(ByteView::try_from(data)?),
            Type::Int256 => Mark::Int256(ByteView::try_from(data)?),
            Type::UInt8 => Mark::UInt8(ByteView::try_from(data)?),
            Type::UInt16 => Mark::UInt16(ByteView::try_from(data)?),
            Type::UInt32 => Mark::UInt32(ByteView::try_from(data)?),
            Type::UInt64 => Mark::UInt64(ByteView::try_from(data)?),
            Type::UInt128 => Mark::UInt128(ByteView::try_from(data)?),
            Type::UInt256 => Mark::UInt256(ByteView::try_from(data)?),
            Type::Float32 => Mark::Float32(ByteView::try_from(data)?),
            Type::Float64 => Mark::Float64(ByteView::try_from(data)?),
            Type::BFloat16 => Mark::BFloat16(ByteView::try_from(data)?),
            Type::Decimal32(scale) => Mark::Decimal32(Decimal32 {
                scale,
                data: ByteView::try_from(data)?,
            }),
            Type::Decimal64(scale) => Mark::Decimal64(Decimal64 {
                scale,
                data: ByteView::try_from(data)?,
            }),
            Type::Decimal128(scale) => Mark::Decimal128(Decimal128 {
                scale,
                data: ByteView::try_from(data)?,
            }),
            Type::Decimal256(scale) => Mark::Decimal256(Decimal256 {
                scale,
                data: ByteView::try_from(data)?,
            }),
            Type::FixedString(size) => Mark::FixedString(FixedString { size, data }),
            Type::Uuid => Mark::Uuid(ByteView::try_from(data)?),
            Type::Date => Mark::Date(ByteView::try_from(data)?),
            Type::Date32 => Mark::Date32(ByteView::try_from(data)?),
            Type::DateTime(tz) => Mark::DateTime(DateTime {
                tz,
                data: ByteView::try_from(data)?,
            }),
            Type::DateTime64(precision, tz) => Mark::DateTime64(DateTime64 {
                precision,
                tz,
                data: ByteView::try_from(data)?,
            }),
            Type::Time => Mark::Time(ByteView::try_from(data)?),
            Type::Time64(precision) => Mark::Time64(Time64 {
                precision,
                data: ByteView::try_from(data)?,
            }),
            Type::Interval(kind) => Mark::Interval(Interval {
                kind,
                data: ByteView::try_from(data)?,
            }),
            Type::Ipv4 => Mark::Ipv4(ByteView::try_from(data)?),
            Type::Ipv6 => Mark::Ipv6(ByteView::try_from(data)?),

            Type::Enum8(variants) => Mark::Enum8(Enum8 {
                variants: variants.into_boxed_slice(),
                data: ByteView::try_from(data)?,
            }),
            Type::Enum16(variants) => Mark::Enum16(Enum16 {
                variants: variants.into_boxed_slice(),
                data: ByteView::try_from(data)?,
            }),
            Type::Nothing => Mark::Nothing(data.len()),

            _ => {
                cold_path();
                return Err(crate::Error::NotImplemented(format!(
                    "fixed-size marker conversion for {self:?}"
                )));
            }
        };

        Ok(mark)
    }
}

/// A dynamic path is a whole `Dynamic` column: `SerializationObject` delegates its prefix and data
/// to `SerializationDynamic`.
#[derive(Debug)]
pub enum JsonColumnHeader<'a> {
    Typed {
        typ: Type<'a>,
        header: TypeHeader<'a>,
    },
    Dynamic(DynamicHeader<'a>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Field<'a> {
    pub(crate) name: Cow<'a, str>,
    pub(crate) typ: Type<'a>,
}
