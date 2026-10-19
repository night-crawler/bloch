use std::hint::cold_path;

use super::{Array, ArrayIter, F64, TryRead, Tuple};
use crate::error::Error;
use crate::mark::Mark;
use crate::{FromVariant, IntoVariant};

/// Geo types are stored as tuples and arrays; the newtype keeps the declared type name so a
/// writer can emit it again.
macro_rules! geo {
    ($($name:ident($inner:ty) => $type_name:literal;)+) => {$(
        #[doc = concat!("Reads a `", $type_name, "` column, stored as `", stringify!($inner), "`.")]
        #[derive(Clone, Copy)]
        pub struct $name<'a>(pub $inner);

        impl<'a> TryFrom<&'a Mark<'a>> for $name<'a> {
            type Error = Error;

            fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
                match <$inner>::try_from(value) {
                    Ok(inner) => Ok(Self(inner)),
                    Err(error) => {
                        cold_path();
                        Err(error)
                    }
                }
            }
        }

        impl<'a> TryRead<'a> for $name<'a> {
            type Item = <$inner as TryRead<'a>>::Item;

            const NAME: &'static str = $type_name;

            #[inline(always)]
            fn len(&self) -> usize {
                self.0.len()
            }

            #[inline(always)]
            unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
                unsafe { self.0.try_read_unchecked(idx) }
            }
        }
    )+};
}

geo! {
    Point(Tuple<(F64<'a>, F64<'a>)>) => "Point";
    Ring(Array<'a, Point<'a>>) => "Ring";
    Polygon(Array<'a, Ring<'a>>) => "Polygon";
    MultiPolygon(Array<'a, Polygon<'a>>) => "MultiPolygon";
    LineString(Array<'a, Point<'a>>) => "LineString";
    MultiLineString(Array<'a, LineString<'a>>) => "MultiLineString";
    MultiPoint(Array<'a, Point<'a>>) => "MultiPoint";
}

/// One row of a `Geometry` column, read through [`Variant`](super::Variant) or
/// [`VariantNullable`](super::VariantNullable).
///
/// ClickHouse fixes the discriminator order of `Geometry` (it is not the alphabetical order of a
/// user-declared `Variant(...)`), so this enum is provided instead of being derived per project.
#[expect(
    clippy::large_enum_variant,
    reason = "readers are borrowed views; boxing would allocate per row"
)]
#[derive(FromVariant, IntoVariant)]
#[col(type_name = "Geometry")]
pub enum Geometry<'a> {
    #[col(reader = LineString<'a>)]
    LineString(ArrayIter<'a, Point<'a>>),
    #[col(reader = MultiLineString<'a>)]
    MultiLineString(ArrayIter<'a, LineString<'a>>),
    #[col(reader = MultiPolygon<'a>)]
    MultiPolygon(ArrayIter<'a, Polygon<'a>>),
    #[col(reader = Point<'a>)]
    Point((f64, f64)),
    #[col(reader = Polygon<'a>)]
    Polygon(ArrayIter<'a, Ring<'a>>),
    #[col(reader = Ring<'a>)]
    Ring(ArrayIter<'a, Point<'a>>),
    #[col(reader = MultiPoint<'a>)]
    MultiPoint(ArrayIter<'a, Point<'a>>),
}
