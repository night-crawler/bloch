use std::hint::cold_path;

use crate::{error::Error, slice::ByteView, types::Offsets, zc};

pub mod block;
pub(crate) mod column;
pub mod consts;
pub(crate) mod header;
mod lexer;
pub(crate) mod typ;

pub(crate) type IResult<I, O, E = Error> = Result<(I, O), E>;

const DATA: u8 = 0x7F;
const CONT: u8 = 0x80;

fn parse_varuint<T>(input: &[u8]) -> IResult<&[u8], T>
where
    T: TryFrom<u64>,
{
    let (value, rest) = get_unsigned_leb128(input)?;

    let Ok(value) = T::try_from(value) else {
        cold_path();
        return Err(Error::Overflow(value.to_string()));
    };

    Ok((rest, value))
}

fn get_unsigned_leb128(input: &[u8]) -> Result<(u64, &[u8]), Error> {
    // We parse strings, realistically it will always be way less on average, so there's no point
    // unfolding loops and messing with unneeded complexity.
    let Some(head) = input.first_chunk::<3>() else {
        return get_unsigned_leb128_slow(input);
    };

    let b0 = head[0];
    if b0 & CONT == 0 {
        return Ok((u64::from(b0), &input[1..]));
    }
    let value = u64::from(b0 & DATA);

    let b1 = head[1];
    if b1 & CONT == 0 {
        return Ok((value | u64::from(b1) << 7, &input[2..]));
    }
    let value = value | u64::from(b1 & DATA) << 7;

    let b2 = head[2];
    if b2 & CONT == 0 {
        return Ok((value | u64::from(b2) << 14, &input[3..]));
    }

    get_unsigned_leb128_slow(input)
}

#[cold]
#[inline(never)]
fn get_unsigned_leb128_slow(input: &[u8]) -> Result<(u64, &[u8]), Error> {
    let mut acc: u64 = 0;

    for (idx, &byte) in input.iter().take(10).enumerate() {
        if byte & CONT == 0 {
            if idx == 9 && byte > 1 {
                return Err(Error::Overflow("varuint too large for u64".into()));
            }
            return Ok((acc | (u64::from(byte) << (idx * 7)), &input[idx + 1..]));
        }
        if idx == 9 {
            return Err(Error::Overflow("varuint too large for u64".into()));
        }
        acc |= u64::from(byte & DATA) << (idx * 7);
    }

    Err(Error::Length(input.len() + 1))
}

fn parse_u64<T>(input: &[u8]) -> IResult<&[u8], T>
where
    T: TryFrom<u64>,
{
    if input.len() < 8 {
        cold_path();
        return Err(Error::Length(8));
    }
    let (bytes, rest) = input.split_at(8);
    let value = u64::from_le_bytes(bytes.try_into().expect("we checked"));

    let Ok(value) = T::try_from(value) else {
        cold_path();
        return Err(Error::Overflow(value.to_string()));
    };

    Ok((rest, value))
}

fn parse_var_str_bytes(input: &[u8]) -> IResult<&[u8], &[u8]> {
    let (input, len) = parse_varuint(input)?;
    if input.len() < len {
        cold_path();
        return Err(Error::Length(len));
    }

    let (str_bytes, remainder) = input.split_at(len);
    Ok((remainder, str_bytes))
}

pub(crate) fn parse_var_str(input: &[u8]) -> IResult<&[u8], &str> {
    let (input, len) = parse_varuint(input)?;
    if input.len() < len {
        cold_path();
        return Err(Error::UnexpectedEndOfInput);
    }

    let (str_bytes, remainder) = input.split_at(len);

    let str_value = match std::str::from_utf8(str_bytes) {
        Ok(str_value) => str_value,
        Err(e) => {
            cold_path();
            return Err(Error::Utf8Decode(e, str_bytes.to_vec()));
        }
    };
    Ok((remainder, str_value))
}

fn take_elements<'a>(
    input: &'a [u8],
    left: usize,
    right: usize,
    description: &str,
) -> IResult<&'a [u8], &'a [u8]> {
    let Some(byte_len) = left.checked_mul(right) else {
        cold_path();
        return Err(Error::Overflow(format!("{description}: {left} * {right}")));
    };
    let Some((data, input)) = input.split_at_checked(byte_len) else {
        cold_path();
        return Err(Error::Length(byte_len));
    };
    Ok((input, data))
}

fn parse_offsets(input: &[u8], num_rows: usize) -> IResult<&[u8], Offsets<'_>> {
    let (input, offsets) = take_elements(input, num_rows, size_of::<u64>(), "offset byte length")?;
    let offsets = ByteView::<zc::U64>::try_from(offsets)?;

    if cfg!(debug_assertions) {
        check_monotonic(offsets.as_slice())?;
    }

    Ok((input, offsets))
}

fn check_monotonic(offsets: &[zc::U64]) -> Result<(), Error> {
    let mut prev = 0u64;
    for (i, offset) in offsets.iter().enumerate() {
        let cur = offset.get();
        if cur < prev {
            cold_path();
            return Err(Error::CorruptedData(format!(
                "offsets not monotonic: offset[{i}] = {cur} < offset[{}] = {prev}",
                i - 1
            )));
        }
        prev = cur;
    }
    Ok(())
}
