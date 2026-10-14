use std::ops::Range;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Overflow: {0}")]
    Overflow(String),

    #[error("Unexpected end of input")]
    UnexpectedEndOfInput,

    #[error("Length error: {0}")]
    Length(usize),

    #[error("Mismatched type: Internal type is {0}, but asked to get {1}")]
    MismatchedType(&'static str, &'static str),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Utf8 decode error: {0}; bytes: {1:0x?}")]
    Utf8Decode(std::str::Utf8Error, Vec<u8>),

    #[error("Conversion out of range: {0} for {1}, got {2}")]
    ValueOutOfRange(&'static str, &'static str, String),

    #[error("Index {0} out of bounds for {1}")]
    IndexOutOfBounds(usize, &'static str),

    #[error("Range {0:?} out of bounds for {1}")]
    RangeOutOfBounds(Range<usize>, &'static str),

    #[error("Corrupted data: {0}")]
    CorruptedData(String),

    #[error("TryFromIntError: {0}")]
    TryFromIntError(#[from] std::num::TryFromIntError),

    #[error("Invalid column order: {0}")]
    InvalidColumnOrder(String),

    #[error("Column not found: {0}")]
    ColumnNotFound(String),
}

pub(crate) fn decode_utf8(bytes: &[u8]) -> crate::Result<&str> {
    match std::str::from_utf8(bytes) {
        Ok(s) => Ok(s),
        Err(error) => Err(Error::Utf8Decode(error, bytes.to_vec())),
    }
}
