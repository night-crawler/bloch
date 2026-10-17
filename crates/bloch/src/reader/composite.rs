use super::{Bytes, FromVariant, ReadSlice, Readable, Str, TrustedStr, TryRead};
use crate::error::Error;
use crate::mark;
use crate::mark::{Mark, Variant as VariantMark};
use crate::types::OffsetIndexPair as _;
use std::hint::cold_path;
use std::ops::Range;

#[derive(Clone, Copy)]
pub struct Nullable<'a, Inner: TryRead<'a>> {
    pub(crate) mark: &'a mark::Nullable<'a>,
    pub(crate) inner: Inner,
}

impl<'a, Inner> TryFrom<&'a Mark<'a>> for Nullable<'a, Inner>
where
    Inner: TryRead<'a> + TryFrom<&'a Mark<'a>> + 'a,
    Error: From<<Inner as TryFrom<&'a Mark<'a>>>::Error>,
{
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        let Mark::Nullable(n) = value else {
            cold_path();
            return Err(Error::MismatchedType(value.as_str(), Self::NAME));
        };
        let inner = Inner::try_from(n.data.as_ref())?;
        if inner.len() < n.len() {
            cold_path();
            return Err(Error::CorruptedData(format!(
                "Nullable mask has {} rows, values have {}",
                n.len(),
                inner.len()
            )));
        }
        Ok(Nullable { mark: n, inner })
    }
}

impl<'a, Inner: TryRead<'a> + 'a> TryRead<'a> for Nullable<'a, Inner> {
    type Item = Option<Inner::Item>;

    const NAME: &'static str = "Nullable";

    #[inline(always)]
    fn len(&self) -> usize {
        self.mark.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        // SAFETY: caller guarantees `idx < mark.len()`; construction checked `<= inner.len()`.
        unsafe {
            if self.mark.is_null_unchecked(idx) {
                return Ok(None);
            }
            Ok(Some(self.inner.try_read_unchecked(idx)?))
        }
    }
}

#[derive(Clone, Copy)]
pub struct Lc<'a, Inner: TryRead<'a>> {
    indices: mark::lc::Indices<'a>,
    pub(crate) dict: Option<Inner>,
}

pub type LcStr<'a> = Lc<'a, Str<'a>>;
pub type LcBytes<'a> = Lc<'a, Bytes<'a>>;
pub type LcTrustedStr<'a> = Lc<'a, TrustedStr<'a>>;

impl<'a, Inner> TryFrom<&'a Mark<'a>> for Lc<'a, Inner>
where
    Inner: TryRead<'a> + TryFrom<&'a Mark<'a>> + 'a,
    Error: From<<Inner as TryFrom<&'a Mark<'a>>>::Error>,
{
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        let lc = value.lc()?;
        lc.not_nullable()?;
        let dict = match lc.keys()? {
            Mark::Empty => None,
            keys => Some(Inner::try_from(keys)?),
        };
        Ok(Lc {
            indices: lc.indices,
            dict,
        })
    }
}

impl<'a, Inner: TryRead<'a> + 'a> TryRead<'a> for Lc<'a, Inner> {
    type Item = Inner::Item;

    const NAME: &'static str = "LowCardinality";

    #[inline(always)]
    fn len(&self) -> usize {
        self.indices.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        let value_index = unsafe { self.indices.get_unchecked(idx) }?;
        self.dict_read(value_index)
    }
}

impl<'a, Inner: TryRead<'a> + 'a> Lc<'a, Inner> {
    /// Dictionary entry `value_index`; `IndexOutOfBounds` past the dictionary.
    #[inline(always)]
    fn dict_read(&self, value_index: usize) -> crate::Result<Inner::Item> {
        // SAFETY: construction guarantees that a missing dictionary has no readable index, so
        // reaching this point proves that the dictionary exists.
        let dict = unsafe { self.dict.as_ref().unwrap_unchecked() };
        // `value_index` is wire data, not an index the constructor bounded: keep the check.
        dict.try_read(value_index)
    }
}

#[derive(Clone, Copy)]
pub struct LcNullable<'a, Inner: TryRead<'a>> {
    indices: mark::lc::Indices<'a>,
    pub(crate) dict: Option<Inner>,
}

pub type LcNullableStr<'a> = LcNullable<'a, Str<'a>>;
pub type LcNullableBytes<'a> = LcNullable<'a, Bytes<'a>>;
pub type LcNullableTrustedStr<'a> = LcNullable<'a, TrustedStr<'a>>;

impl<'a, Inner> TryFrom<&'a Mark<'a>> for LcNullable<'a, Inner>
where
    Inner: TryRead<'a> + TryFrom<&'a Mark<'a>> + 'a,
    Error: From<<Inner as TryFrom<&'a Mark<'a>>>::Error>,
{
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        let lc = value.lc()?;
        if !lc.is_nullable {
            cold_path();
            return Err(Error::MismatchedType(
                "LowCardinality",
                "LowCardinality(Nullable)",
            ));
        }
        let dict = match lc.keys()? {
            Mark::Empty => None,
            keys => Some(Inner::try_from(keys)?),
        };
        Ok(LcNullable {
            indices: lc.indices,
            dict,
        })
    }
}

impl<'a, Inner: TryRead<'a> + 'a> TryRead<'a> for LcNullable<'a, Inner> {
    type Item = Option<Inner::Item>;

    const NAME: &'static str = "LowCardinality";

    #[inline(always)]
    fn len(&self) -> usize {
        self.indices.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        let value_index = unsafe { self.indices.get_unchecked(idx) }?;
        self.dict_read(value_index)
    }
}

impl<'a, Inner: TryRead<'a> + 'a> LcNullable<'a, Inner> {
    /// Dictionary entry `value_index`; `None` for the NULL slot, `IndexOutOfBounds` past the
    /// dictionary.
    #[inline(always)]
    fn dict_read(&self, value_index: usize) -> crate::Result<Option<Inner::Item>> {
        if value_index == 0 {
            return Ok(None);
        }
        // SAFETY: construction guarantees that a missing dictionary has only null indices.
        let dict = unsafe { self.dict.as_ref().unwrap_unchecked() };
        // `value_index` is wire data, not an index the constructor bounded: keep the check.
        Ok(Some(dict.try_read(value_index)?))
    }
}

#[derive(Clone, Copy)]
pub struct Array<'a, Inner: TryRead<'a>> {
    pub(crate) offsets: &'a [crate::zc::U64],
    pub(crate) values: Inner,
    values_len: usize,
}

impl<'a, Inner> TryFrom<&'a Mark<'a>> for Array<'a, Inner>
where
    Inner: TryRead<'a> + TryFrom<&'a Mark<'a>> + 'a,
    Error: From<<Inner as TryFrom<&'a Mark<'a>>>::Error>,
{
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::Array(arr) => {
                let elements = arr.offsets.last_or_default()?;
                let values = Inner::try_from(arr.values.as_ref())?;
                let values_len = values.len();
                if values_len < elements {
                    cold_path();
                    return Err(offsets_exceed_values("Array", elements, values_len));
                }
                Ok(Array {
                    offsets: arr.offsets.as_slice(),
                    values,
                    values_len,
                })
            }
            // `Nested(...)` is stored as an array of tuples.
            Mark::Nested(n) => Self::try_from(n.array_of_tuples.as_ref()),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a, Inner: TryRead<'a> + 'a> TryRead<'a> for Array<'a, Inner> {
    type Item = ArrayIter<'a, Inner>;

    const NAME: &'static str = "Array";

    #[inline(always)]
    fn len(&self) -> usize {
        self.offsets.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        let (s, e) = unsafe { self.offsets.offset_indices_unchecked(idx) }?;
        self.iter_range(s..e)
    }
}

impl<'a, Inner: TryRead<'a> + 'a> Array<'a, Inner> {
    /// Elements `range` of the values; `CorruptedData` when it runs past them.
    #[inline(always)]
    fn iter_range(&self, range: Range<usize>) -> crate::Result<ArrayIter<'a, Inner>> {
        // One check per row here lets `ArrayIter` read every element unchecked.
        if range.end > self.values_len {
            cold_path();
            return Err(offsets_exceed_values("Array", range.end, self.values_len));
        }
        Ok(ArrayIter {
            inner: self.values,
            range,
            _marker: std::marker::PhantomData,
        })
    }
}

#[cold]
#[inline(never)]
fn offsets_exceed_values(kind: &str, end: usize, present: usize) -> Error {
    Error::CorruptedData(format!(
        "{kind} offsets address {end} values, {present} present"
    ))
}

pub struct ArrayIter<'a, Inner: TryRead<'a>> {
    inner: Inner,
    range: Range<usize>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, Inner: TryRead<'a>> ArrayIter<'a, Inner> {
    #[inline(always)]
    pub fn try_collect_vec(self) -> crate::Result<Vec<Inner::Item>> {
        let mut out = Vec::with_capacity(self.range.len());
        for item in self {
            out.push(item?);
        }
        Ok(out)
    }
}

impl<'a, Inner: TryRead<'a>> Iterator for ArrayIter<'a, Inner> {
    type Item = crate::Result<Inner::Item>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let i = self.range.next()?;
        // SAFETY: `Array::iter_range` verified `range.end <= values_len`, so `i < inner.len()`.
        unsafe { Some(self.inner.try_read_unchecked(i)) }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<'a, Inner: TryRead<'a>> ExactSizeIterator for ArrayIter<'a, Inner> {}

impl<'a, Inner> Readable<'a> for ArrayIter<'a, Inner>
where
    Inner: TryRead<'a> + TryFrom<&'a Mark<'a>> + 'a,
    Error: From<<Inner as TryFrom<&'a Mark<'a>>>::Error>,
{
    type Reader = Array<'a, Inner>;
}

impl<'a, Inner: ReadSlice<'a>> ArrayIter<'a, Inner> {
    /// The contiguous backing slice for this array cell.
    #[inline]
    pub fn try_as_slice(&self) -> crate::Result<&'a [Inner::Elem]> {
        self.inner.try_read_slice(self.range.clone())
    }
}

#[derive(Clone, Copy)]
pub struct Map<'a, K: TryRead<'a>, V: TryRead<'a>> {
    pub(crate) offsets: &'a [crate::zc::U64],
    pub(crate) keys: K,
    pub(crate) values: V,
    // `min(keys.len(), values.len())`; every row's range is checked against it once.
    entries_len: usize,
}

impl<'a, K, V> TryFrom<&'a Mark<'a>> for Map<'a, K, V>
where
    K: TryRead<'a> + TryFrom<&'a Mark<'a>> + 'a,
    V: TryRead<'a> + TryFrom<&'a Mark<'a>> + 'a,
    Error: From<<K as TryFrom<&'a Mark<'a>>>::Error> + From<<V as TryFrom<&'a Mark<'a>>>::Error>,
{
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        let Mark::Map(m) = value else {
            cold_path();
            return Err(Error::MismatchedType(value.as_str(), Self::NAME));
        };
        let keys = K::try_from(m.keys.as_ref())?;
        let values = V::try_from(m.values.as_ref())?;
        let entries_len = keys.len().min(values.len());
        let entries = m.offsets.last_or_default()?;
        if entries_len < entries {
            cold_path();
            return Err(offsets_exceed_values("Map", entries, entries_len));
        }
        Ok(Map {
            offsets: m.offsets.as_slice(),
            keys,
            values,
            entries_len,
        })
    }
}

impl<'a, K: TryRead<'a> + 'a, V: TryRead<'a> + 'a> TryRead<'a> for Map<'a, K, V> {
    type Item = MapIter<'a, K, V>;

    const NAME: &'static str = "Map";

    fn len(&self) -> usize {
        self.offsets.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        let (s, e) = unsafe { self.offsets.offset_indices_unchecked(idx) }?;
        self.iter_range(s..e)
    }
}

impl<'a, K: TryRead<'a> + 'a, V: TryRead<'a> + 'a> Map<'a, K, V> {
    /// Entries `range` of the keys and values; `CorruptedData` when it runs past them.
    #[inline(always)]
    fn iter_range(&self, range: Range<usize>) -> crate::Result<MapIter<'a, K, V>> {
        // One check per row here lets `MapIter` read every entry unchecked.
        if range.end > self.entries_len {
            cold_path();
            return Err(offsets_exceed_values("Map", range.end, self.entries_len));
        }
        Ok(MapIter {
            keys: self.keys,
            values: self.values,
            range,
            _marker: std::marker::PhantomData,
        })
    }
}

pub struct MapIter<'a, K: TryRead<'a>, V: TryRead<'a>> {
    keys: K,
    values: V,
    range: Range<usize>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, K: TryRead<'a>, V: TryRead<'a>> Iterator for MapIter<'a, K, V> {
    type Item = crate::Result<(K::Item, V::Item)>;

    fn next(&mut self) -> Option<Self::Item> {
        let i = self.range.next()?;
        // SAFETY: `Map::iter_range` verified `range.end <= min(keys.len(), values.len())`.
        unsafe {
            let key = match self.keys.try_read_unchecked(i) {
                Ok(key) => key,
                Err(error) => return Some(Err(error)),
            };
            let value = match self.values.try_read_unchecked(i) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            Some(Ok((key, value)))
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<'a, K: TryRead<'a>, V: TryRead<'a>> ExactSizeIterator for MapIter<'a, K, V> {}

impl<'a, K, V> Readable<'a> for MapIter<'a, K, V>
where
    K: TryRead<'a> + TryFrom<&'a Mark<'a>> + 'a,
    V: TryRead<'a> + TryFrom<&'a Mark<'a>> + 'a,
    Error: From<<K as TryFrom<&'a Mark<'a>>>::Error> + From<<V as TryFrom<&'a Mark<'a>>>::Error>,
{
    type Reader = Map<'a, K, V>;
}

/// Reads a `Variant(...)` column into a `T` enum implementing [`FromVariant`].
/// Normally, it should be derived via `#[derive(FromVariant)]`.
///
/// This will error on NULLs, use [`VariantNullable`] for nullable columns.
pub struct Variant<'a, T: FromVariant<'a>> {
    mark: &'a VariantMark<'a>,
    pub(crate) readers: T::Readers,
}

impl<'a, T: FromVariant<'a>> Clone for Variant<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T: FromVariant<'a>> Copy for Variant<'a, T> {}

impl<'a, T: FromVariant<'a> + 'a> TryFrom<&'a Mark<'a>> for Variant<'a, T> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::Variant(v) => Ok(Self {
                mark: v,
                readers: T::from_marks(&v.types)?,
            }),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a, T: FromVariant<'a> + 'a> TryRead<'a> for Variant<'a, T> {
    type Item = T;

    const NAME: &'static str = "Variant";

    fn len(&self) -> usize {
        self.mark.discriminators.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        let discriminator = unsafe { *self.mark.discriminators.get_unchecked(idx) };
        self.row(discriminator, idx)
    }
}

impl<'a, T: FromVariant<'a> + 'a> Variant<'a, T> {
    fn row(&self, discriminator: u8, idx: usize) -> crate::Result<T> {
        if discriminator == VariantMark::NULL_DISCRIMINATOR {
            cold_path();
            return Err(Error::MismatchedType(
                "Null",
                "non-null Variant row (use VariantNullable)",
            ));
        }
        self.value(discriminator, idx)
    }

    fn value(&self, discriminator: u8, idx: usize) -> crate::Result<T> {
        // `offsets` and `discriminators` always have the same length.
        T::read(
            &self.readers,
            discriminator as usize,
            self.mark.offsets[idx] as usize,
        )
    }
}

/// Same as [`Variant`] that yields `None` for NULL rows.
pub struct VariantNullable<'a, T: FromVariant<'a>> {
    pub(crate) inner: Variant<'a, T>,
}

impl<'a, T: FromVariant<'a>> Clone for VariantNullable<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T: FromVariant<'a>> Copy for VariantNullable<'a, T> {}

impl<'a, T: FromVariant<'a> + 'a> TryFrom<&'a Mark<'a>> for VariantNullable<'a, T> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            inner: Variant::try_from(value)?,
        })
    }
}

impl<'a, T: FromVariant<'a> + 'a> TryRead<'a> for VariantNullable<'a, T> {
    type Item = Option<T>;

    const NAME: &'static str = "Variant";

    fn len(&self) -> usize {
        self.inner.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        let discriminator = unsafe { *self.inner.mark.discriminators.get_unchecked(idx) };
        self.row(discriminator, idx)
    }
}

impl<'a, T: FromVariant<'a> + 'a> VariantNullable<'a, T> {
    fn row(&self, discriminator: u8, idx: usize) -> crate::Result<Option<T>> {
        if discriminator == VariantMark::NULL_DISCRIMINATOR {
            return Ok(None);
        }
        self.inner.value(discriminator, idx).map(Some)
    }
}

#[derive(Clone, Copy)]
pub struct Tuple<T>(pub T);

macro_rules! impl_col_tuple {
    ($n:literal, $($idx:tt => $t:ident),+) => {
        impl<'a, $($t,)+> TryFrom<&'a Mark<'a>> for Tuple<($($t,)+)>
        where
            $(
                $t: TryRead<'a> + TryFrom<&'a Mark<'a>> + 'a,
                Error: From<<$t as TryFrom<&'a Mark<'a>>>::Error>,
            )+
        {
            type Error = Error;

            fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
                let tuple = match value {
                    Mark::Tuple(t) => t,
                    Mark::NamedTuple(nt) => match nt.tuple.as_ref() {
                        Mark::Tuple(t) => t,
                        other => {
                            cold_path();
                            return Err(Error::MismatchedType(other.as_str(), Self::NAME));
                        }
                    },
                    other => {
                        cold_path();
                        return Err(Error::MismatchedType(other.as_str(), Self::NAME));
                    }
                };

                if tuple.values.len() != $n {
                    cold_path();
                    return Err(Error::MismatchedType("Tuple", "Tuple with matching arity"));
                }

                Ok(Self(($($t::try_from(&tuple.values[$idx])?,)+)))
            }
        }

        impl<'a, $($t: TryRead<'a> + 'a,)+> TryRead<'a> for Tuple<($($t,)+)> {
            type Item = ($($t::Item,)+);

            const NAME: &'static str = "Tuple";

            #[inline(always)]
            fn len(&self) -> usize {
                // The shortest element column: `try_read_unchecked(idx)` is sound for every
                // element only when `idx` is below all of their lengths.
                let len = crate::parse::consts::MAX_NUM_ROWS;
                $( let len = len.min(self.0.$idx.len()); )+
                len
            }

            #[inline(always)]
            unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
                // SAFETY: `idx < len()`, and `len()` is the minimum over the elements.
                unsafe { Ok(($(self.0.$idx.try_read_unchecked(idx)?,)+)) }
            }
        }
    };
}

impl_col_tuple!(1, 0 => A);
impl_col_tuple!(2, 0 => A, 1 => B);
impl_col_tuple!(3, 0 => A, 1 => B, 2 => C);
impl_col_tuple!(4, 0 => A, 1 => B, 2 => C, 3 => D);
impl_col_tuple!(5, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E);
impl_col_tuple!(6, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F);
impl_col_tuple!(7, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G);
impl_col_tuple!(8, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H);
impl_col_tuple!(9, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I);
impl_col_tuple!(10, 0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I, 9 => J);
