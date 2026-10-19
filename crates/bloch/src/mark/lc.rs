use crate::mark::{Mark, checked_slice};
use crate::value::{LowCardinalitySliceIterator, Value};
use crate::{Error, zc};
use bstr::BStr;
use std::hint::cold_path;
use std::ops::Range;

const NO_KEYS: Mark<'static> = Mark::Empty;

#[derive(Debug)]
pub struct LowCardinality<'a> {
    pub is_nullable: bool,
    pub indices: Indices<'a>,
    pub global_dictionary: Option<Box<Mark<'a>>>,
    pub additional_keys: Option<Box<Mark<'a>>>,
}

impl<'a> LowCardinality<'a> {
    #[inline(always)]
    pub(crate) fn keys(&self) -> crate::Result<&Mark<'a>> {
        match self.additional_keys.as_deref() {
            Some(keys) => Ok(keys),
            None if self.indices.is_empty() => Ok(&NO_KEYS),
            // Absent dictionary along indices cannot come from a valid stream
            None => {
                cold_path();
                Err(Error::CorruptedData(
                    "LowCardinality dictionary is missing".to_owned(),
                ))
            }
        }
    }

    #[inline(always)]
    pub fn key(&self, index: usize) -> crate::Result<Option<(&Mark<'a>, usize)>> {
        self.not_nullable()?;
        match self.value_index(index)? {
            Some(value_index) => Ok(Some((self.keys()?, value_index))),
            None => Ok(None),
        }
    }

    #[inline(always)]
    pub fn opt_key(&self, index: usize) -> crate::Result<Option<Option<(&Mark<'a>, usize)>>> {
        match self.value_index(index)? {
            None => Ok(None),
            Some(value_index) if self.is_null(value_index) => Ok(Some(None)),
            Some(value_index) => Ok(Some(Some((self.keys()?, value_index)))),
        }
    }

    #[inline(always)]
    fn str_keys(&self) -> crate::Result<StrKeys<'a, '_>> {
        match self.keys()? {
            Mark::String(keys) => Ok(StrKeys(keys)),
            Mark::Empty => Ok(StrKeys(&[])),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), "String"))
            }
        }
    }

    #[inline(always)]
    pub(crate) const fn is_null(&self, value_index: usize) -> bool {
        // The wire carries the NULL placeholder as the nested type's default (`""` for String).
        // Taking blindly a value by index is wrong.
        // == 0`, identifies it.
        self.is_nullable && value_index == 0
    }

    #[inline(always)]
    pub(crate) const fn not_nullable(&self) -> crate::Result<()> {
        if self.is_nullable {
            cold_path();
            return Err(Error::MismatchedType(
                "LowCardinality(Nullable)",
                "LowCardinality",
            ));
        }
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn value_index(&self, index: usize) -> crate::Result<Option<usize>> {
        self.indices.get(index)
    }

    #[inline(always)]
    pub(crate) fn value(&self, value_index: usize) -> crate::Result<Value<'_>> {
        if self.is_null(value_index) {
            return Ok(Value::Empty);
        }
        match self.keys()?.get(value_index)? {
            Some(value) => Ok(value),
            None => {
                cold_path();
                Err(Error::IndexOutOfBounds(
                    value_index,
                    "LowCardinality dictionary",
                ))
            }
        }
    }

    pub(crate) fn get(&self, index: usize) -> crate::Result<Option<Value<'_>>> {
        match self.value_index(index)? {
            Some(value_index) => self.value(value_index).map(Some),
            None => Ok(None),
        }
    }

    pub(crate) fn slice(
        &self,
        range: Range<usize>,
    ) -> crate::Result<LowCardinalitySliceIterator<'_>> {
        Ok(LowCardinalitySliceIterator {
            lc: self,
            indices: self.indices.iter(range)?,
        })
    }

    #[inline(always)]
    pub(crate) fn get_str(&self, index: usize) -> crate::Result<Option<&'a BStr>> {
        self.not_nullable()?;
        Ok(self.get_opt_str(index)?.flatten())
    }

    /// Outer `None`: index out of range. Inner `None`: NULL
    #[inline(always)]
    pub(crate) fn get_opt_str(&self, index: usize) -> crate::Result<Option<Option<&'a BStr>>> {
        let Some(value_index) = self.value_index(index)? else {
            return Ok(None);
        };
        if self.is_null(value_index) {
            return Ok(Some(None));
        }
        self.str_keys()?
            .get(value_index)
            .map(|value| Some(Some(value)))
    }

    #[inline(always)]
    pub(crate) fn slice_strs(&self, range: Range<usize>) -> crate::Result<StrIter<'a, '_>> {
        self.not_nullable()?;
        Ok(StrIter {
            indices: self.indices.iter(range)?,
            keys: self.str_keys()?,
        })
    }

    #[inline(always)]
    pub(crate) fn slice_opt_strs(&self, range: Range<usize>) -> crate::Result<OptStrIter<'a, '_>> {
        Ok(OptStrIter {
            lc: self,
            indices: self.indices.iter(range)?,
            keys: self.str_keys()?,
        })
    }
}

#[derive(Clone, Copy)]
struct StrKeys<'data: 'keys, 'keys>(&'keys [&'data BStr]);

impl<'data> StrKeys<'data, '_> {
    #[inline(always)]
    fn get(self, index: usize) -> crate::Result<&'data BStr> {
        match self.0.get(index) {
            Some(value) => Ok(value),
            None => {
                cold_path();
                Err(Error::IndexOutOfBounds(index, "LowCardinality dictionary"))
            }
        }
    }
}

/// Iterator over raw string keys of a non-nullable `LowCardinality` column slice.
/// Waiting for: <https://github.com/rust-lang/rust/issues/63063>
pub struct StrIter<'data: 'keys, 'keys> {
    indices: IndicesIter<'data>,
    keys: StrKeys<'data, 'keys>,
}

impl<'data> Iterator for StrIter<'data, '_> {
    type Item = crate::Result<&'data BStr>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        Some(match self.indices.next()? {
            Ok(index) => self.keys.get(index),
            Err(error) => {
                cold_path();
                Err(error)
            }
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }
}

impl ExactSizeIterator for StrIter<'_, '_> {}

/// Iterator over raw string keys of a `LowCardinality` nullable column slice
pub struct OptStrIter<'data: 'keys, 'keys> {
    lc: &'keys LowCardinality<'data>,
    indices: IndicesIter<'data>,
    // It's cached here to skip more matching
    keys: StrKeys<'data, 'keys>,
}

impl<'data> Iterator for OptStrIter<'data, '_> {
    type Item = crate::Result<Option<&'data BStr>>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let index = match self.indices.next()? {
            Ok(index) => index,
            Err(error) => {
                cold_path();
                return Some(Err(error));
            }
        };
        if self.lc.is_null(index) {
            return Some(Ok(None));
        }
        Some(self.keys.get(index).map(Some))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }
}

impl ExactSizeIterator for OptStrIter<'_, '_> {}

#[derive(Clone, Copy, Debug)]
pub enum Indices<'a> {
    U8(&'a [u8]),
    U16(&'a [zc::U16]),
    U32(&'a [zc::U32]),
    U64(&'a [zc::U64]),
}

impl<'a> TryFrom<Mark<'a>> for Indices<'a> {
    type Error = Error;

    fn try_from(mark: Mark<'a>) -> Result<Self, Self::Error> {
        match mark {
            Mark::Empty => Ok(Self::U8(&[])),
            Mark::UInt8(indices) => Ok(Self::U8(indices.as_slice())),
            Mark::UInt16(indices) => Ok(Self::U16(indices.as_slice())),
            Mark::UInt32(indices) => Ok(Self::U32(indices.as_slice())),
            Mark::UInt64(indices) => Ok(Self::U64(indices.as_slice())),
            other => {
                cold_path();
                Err(Error::CorruptedData(format!(
                    "unexpected LowCardinality indices type: {}",
                    other.as_str()
                )))
            }
        }
    }
}

impl<'a> Indices<'a> {
    #[inline(always)]
    pub(crate) fn get(self, index: usize) -> crate::Result<Option<usize>> {
        match self {
            Self::U8(indices) => Ok(indices.get(index).copied().map(usize::from)),
            Self::U16(indices) => Ok(indices.get(index).map(|value| usize::from(value.get()))),
            Self::U32(indices) => Ok(indices.get(index).map(|value| value.get() as usize)),
            Self::U64(indices) => Ok(indices
                .get(index)
                .map(|value| usize::try_from(value.get()))
                .transpose()?),
        }
    }

    /// # Safety
    /// `index < self.len()`.
    #[inline(always)]
    pub(crate) unsafe fn get_unchecked(self, index: usize) -> crate::Result<usize> {
        unsafe {
            match self {
                Self::U8(indices) => Ok(usize::from(*indices.get_unchecked(index))),
                Self::U16(indices) => Ok(usize::from(indices.get_unchecked(index).get())),
                Self::U32(indices) => Ok(indices.get_unchecked(index).get() as usize),
                Self::U64(indices) => match usize::try_from(indices.get_unchecked(index).get()) {
                    Ok(index) => Ok(index),
                    Err(error) => {
                        cold_path();
                        Err(error.into())
                    }
                },
            }
        }
    }

    pub(crate) const fn len(self) -> usize {
        match self {
            Self::U8(indices) => indices.len(),
            Self::U16(indices) => indices.len(),
            Self::U32(indices) => indices.len(),
            Self::U64(indices) => indices.len(),
        }
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub(crate) fn iter(self, range: Range<usize>) -> crate::Result<IndicesIter<'a>> {
        match self {
            Self::U8(indices) => Ok(IndicesIter::U8(
                checked_slice(indices, range, "UInt8")?.iter(),
            )),
            Self::U16(indices) => Ok(IndicesIter::U16(
                checked_slice(indices, range, "UInt16")?.iter(),
            )),
            Self::U32(indices) => Ok(IndicesIter::U32(
                checked_slice(indices, range, "UInt32")?.iter(),
            )),
            Self::U64(indices) => Ok(IndicesIter::U64(
                checked_slice(indices, range, "UInt64")?.iter(),
            )),
        }
    }
}

pub(crate) enum IndicesIter<'a> {
    U8(std::slice::Iter<'a, u8>),
    U16(std::slice::Iter<'a, zc::U16>),
    U32(std::slice::Iter<'a, zc::U32>),
    U64(std::slice::Iter<'a, zc::U64>),
}

impl Iterator for IndicesIter<'_> {
    type Item = crate::Result<usize>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let value = match self {
            Self::U8(iter) => usize::from(*iter.next()?),
            Self::U16(iter) => usize::from(iter.next()?.get()),
            Self::U32(iter) => iter.next()?.get() as usize,
            Self::U64(iter) => {
                let value = iter.next()?.get();
                return match usize::try_from(value) {
                    Ok(value) => Some(Ok(value)),
                    Err(error) => Some(Err(Error::from(error))),
                };
            }
        };
        Some(Ok(value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::U8(iter) => iter.size_hint(),
            Self::U16(iter) => iter.size_hint(),
            Self::U32(iter) => iter.size_hint(),
            Self::U64(iter) => iter.size_hint(),
        }
    }
}

impl ExactSizeIterator for IndicesIter<'_> {}
