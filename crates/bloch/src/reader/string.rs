use std::hint::cold_path;
use std::ops::Range;

use bstr::BStr;

use super::{ReadSlice, Readable, TryRead};
use crate::ByteExt as _;
use crate::error::{Error, decode_utf8};
use crate::mark::{FixedString as FixedStringMark, Mark};

/// Reads raw ClickHouse `String` values without assuming UTF-8.
#[derive(Clone, Copy)]
pub struct Bytes<'a>(pub &'a [&'a BStr]);

impl<'a> TryFrom<&'a Mark<'a>> for Bytes<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::String(strings) => Ok(Self(&strings.data)),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a> TryRead<'a> for Bytes<'a> {
    type Item = &'a BStr;

    const NAME: &'static str = "String";

    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        Ok(unsafe { *self.0.get_unchecked(idx) })
    }
}

impl<'a> ReadSlice<'a> for Bytes<'a> {
    type Elem = &'a BStr;

    fn try_read_slice(&self, range: Range<usize>) -> crate::Result<&'a [Self::Elem]> {
        let Some(slice) = self.0.get(range.clone()) else {
            cold_path();
            return Err(Error::RangeOutOfBounds(range, "String"));
        };
        Ok(slice)
    }
}

/// Reads ClickHouse `String` values as `&str`.
///
/// [`TryFrom`] validates the complete column once.
#[derive(Clone, Copy)]
pub struct Str<'a>(&'a [&'a BStr]);

impl<'a> TryFrom<&'a Mark<'a>> for Str<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        let reader = Bytes::try_from(value)?;
        for value in reader.0 {
            decode_utf8(value)?;
        }
        Ok(Self(reader.0))
    }
}

impl<'a> TryRead<'a> for Str<'a> {
    type Item = &'a str;

    const NAME: &'static str = "String";

    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        // SAFETY: construction validated every value in the column.
        Ok(unsafe { std::str::from_utf8_unchecked(self.0.get_unchecked(idx)) })
    }
}

/// Reads ClickHouse `String` values as `&str` without any UTF-8 validation.
/// You are responsible for your own UB.
#[derive(Clone, Copy)]
pub struct TrustedStr<'a>(&'a [&'a BStr]);

impl<'a> TryFrom<&'a Mark<'a>> for TrustedStr<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        let reader = Bytes::try_from(value)?;
        Ok(Self(reader.0))
    }
}

impl<'a> TryRead<'a> for TrustedStr<'a> {
    type Item = &'a str;

    const NAME: &'static str = "String";

    #[inline(always)]
    fn len(&self) -> usize {
        self.0.len()
    }

    #[inline(always)]
    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        Ok(Self::trusted(unsafe { self.0.get_unchecked(idx) }))
    }
}

impl<'a> TrustedStr<'a> {
    #[inline(always)]
    fn trusted(value: &'a BStr) -> &'a str {
        debug_assert!(
            std::str::from_utf8(value).is_ok(),
            "TrustedStr read invalid UTF-8"
        );
        // SAFETY: choosing this reader asserts the column is valid UTF-8.
        unsafe { std::str::from_utf8_unchecked(value) }
    }
}

#[derive(Clone, Copy)]
pub struct FixedBytes<'a>(pub &'a FixedStringMark<'a>);

impl<'a> TryFrom<&'a Mark<'a>> for FixedBytes<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            Mark::FixedString(fixed) => Ok(Self(fixed)),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a> TryRead<'a> for FixedBytes<'a> {
    type Item = &'a BStr;

    const NAME: &'static str = "FixedString";

    fn len(&self) -> usize {
        self.0.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        Ok(unsafe { self.0.get_bstr_unchecked(idx) })
    }
}

/// Reads ClickHouse `FixedString` values as `&str` with trailing zero padding
/// trimmed.
///
/// [`TryFrom`] validates the complete column once.
#[derive(Clone, Copy)]
pub struct FixedStr<'a>(pub(crate) &'a FixedStringMark<'a>);

impl<'a> TryFrom<&'a Mark<'a>> for FixedStr<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        let reader = FixedBytes::try_from(value)?;
        if reader.0.size != 0 {
            for bytes in reader.0.data.chunks_exact(reader.0.size) {
                decode_utf8(bytes.rtrim_zeros())?;
            }
        }
        Ok(Self(reader.0))
    }
}

impl<'a> TryRead<'a> for FixedStr<'a> {
    type Item = &'a str;

    const NAME: &'static str = "FixedString";

    fn len(&self) -> usize {
        self.0.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        let bytes = unsafe { self.0.get_bstr_unchecked(idx) }.rtrim_zeros();
        // SAFETY: construction validated every trimmed value in the column.
        Ok(unsafe { std::str::from_utf8_unchecked(bytes) })
    }
}

/// Reads ClickHouse `FixedString` values as `&str` with trailing zero padding
/// trimmed, without any UTF-8 validation.
#[derive(Clone, Copy)]
pub struct TrustedFixedStr<'a>(pub(crate) &'a FixedStringMark<'a>);

impl<'a> TryFrom<&'a Mark<'a>> for TrustedFixedStr<'a> {
    type Error = Error;

    fn try_from(value: &'a Mark<'a>) -> Result<Self, Self::Error> {
        FixedBytes::try_from(value).map(|reader| Self(reader.0))
    }
}

impl<'a> TryRead<'a> for TrustedFixedStr<'a> {
    type Item = &'a str;

    const NAME: &'static str = "FixedString";

    fn len(&self) -> usize {
        self.0.len()
    }

    unsafe fn try_read_unchecked(&self, idx: usize) -> crate::Result<Self::Item> {
        Ok(Self::trusted(
            unsafe { self.0.get_bstr_unchecked(idx) }.rtrim_zeros(),
        ))
    }
}

impl<'a> TrustedFixedStr<'a> {
    fn trusted(value: &'a [u8]) -> &'a str {
        debug_assert!(
            std::str::from_utf8(value).is_ok(),
            "TrustedFixedStr read invalid UTF-8"
        );
        // SAFETY: choosing this reader asserts the column is valid UTF-8.
        unsafe { std::str::from_utf8_unchecked(value) }
    }
}

impl<'a> Readable<'a> for &'a str {
    type Reader = Str<'a>;
}

impl<'a> Readable<'a> for &'a BStr {
    type Reader = Bytes<'a>;
}
