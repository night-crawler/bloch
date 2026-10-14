extern crate self as bloch;

use std::hint::cold_path;

use chrono::NaiveDate;
use chrono_tz::Tz;
use log::debug;
use std::collections::HashMap;
use std::{
    iter::Peekable,
    net::{Ipv4Addr, Ipv6Addr},
    ops::Range,
};
use uuid::Uuid;

pub(crate) mod conv;
pub mod error;
pub mod interval;
mod macros;
pub mod mark;
pub mod parse;
pub mod reader;
pub mod slice;
pub(crate) mod types;
pub mod value;
pub mod zc;

pub use bloch_derive::{FromBlock, FromVariant};
pub use bstr::BStr;
pub use error::Error;
pub use interval::Interval;
// Same name as the derive macro on purpose (macro vs type namespace):
// `use crate::FromBlock;` imports both, serde-style.
pub use reader::{FromBlock, FromVariant};

pub type Result<T> = std::result::Result<T, Error>;

fn mark_by_name<'a, T, N: AsRef<str>>(
    col_names: &[N],
    columns: &'a [T],
    name: &str,
) -> Result<&'a T> {
    let column = col_names
        .iter()
        .zip(columns)
        .find_map(|(column_name, column)| (column_name.as_ref() == name).then_some(column));
    match column {
        Some(column) => Ok(column),
        None => {
            cold_path();
            Err(Error::ColumnNotFound(name.to_owned()))
        }
    }
}

pub(crate) trait ByteExt {
    fn rtrim_zeros(&self) -> &[u8];
}

impl ByteExt for [u8] {
    fn rtrim_zeros(&self) -> &[u8] {
        let mut end = self.len();
        while end > 0 && self[end - 1] == 0 {
            end -= 1;
        }
        &self[..end]
    }
}

/// This range represents a starting offset and a length, as opposed to the
/// Rust's range, which stores start and end positions.
/// In particular, this range encodes row numbers/offsets within a ClickHouse block,
/// so it should not be wildly huge. Nevertheless, if the end position exceeds [`u32::MAX`],
/// we still have a good chance of not failing to convert the [`Range<usize>`] to [`TinyRange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TinyRange {
    pub start: u32,
    pub length: u32,
}

impl From<TinyRange> for Range<usize> {
    fn from(value: TinyRange) -> Self {
        let start = value.start as usize;
        Range {
            start,
            end: start + value.length as usize,
        }
    }
}

impl TryFrom<Range<usize>> for TinyRange {
    type Error = Error;

    fn try_from(value: Range<usize>) -> std::result::Result<Self, Self::Error> {
        let Ok(start) = u32::try_from(value.start) else {
            cold_path();
            return Err(Error::ValueOutOfRange(
                "usize",
                "u32",
                value.start.to_string(),
            ));
        };

        let Some(raw_length) = value.end.checked_sub(value.start) else {
            cold_path();
            return Err(Error::ValueOutOfRange(
                "Range<usize>",
                "TinyRange",
                format!("{}..{}", value.start, value.end),
            ));
        };
        let Ok(length) = u32::try_from(raw_length) else {
            cold_path();
            return Err(Error::ValueOutOfRange(
                "usize",
                "u32",
                raw_length.to_string(),
            ));
        };

        Ok(TinyRange { start, length })
    }
}

macro_rules! transparent_newtype {
    ( $( $vis:vis $name:ident ( $inner:ty ) ; )+ ) => {
        $(
            #[repr(transparent)]
            #[derive(
                Clone,
                Copy,
                Eq,
                Hash,
                Ord,
                PartialEq,
                PartialOrd,
                Debug,
                Default,
                zerocopy::FromBytes,
                zerocopy::Unaligned,
            )]
            $vis struct $name(pub(crate) $inner);
        )+
    };
}

macro_rules! impl_from {
    ( $src:ty => $dst:ty , |$v:ident| $body:expr ) => {
        impl From<$src> for $dst {
            #[inline]
            fn from($v: $src) -> Self {
                $body
            }
        }
    };
}

transparent_newtype! {
    pub I256 ([u8; 32]);
    pub U256 ([u8; 32]);
    pub UuidData([zc::U64; 2]);
    pub Ipv4Data (zc::U32);
    pub Ipv6Data ([u8; 16]);
    pub Date16Data (zc::U16);
    pub Date32Data (zc::I32);
    pub DateTime32Data (zc::U32);
    pub DateTime64Data (zc::I64);
    pub Decimal32Data (zc::I32);
    pub Decimal64Data (zc::I64);
    pub Decimal128Data (zc::I128);
    pub Decimal256Data (I256);
    pub Bf16Data ([u8; 2]);
}

impl I256 {
    #[inline]
    pub const fn as_le_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl U256 {
    #[inline]
    pub const fn as_le_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Decimal256Data {
    #[inline]
    pub const fn as_le_bytes(&self) -> &[u8; 32] {
        self.0.as_le_bytes()
    }
}

impl_from!(Bf16Data => half::bf16, |value| half::bf16::from_le_bytes(value.0));
impl_from!(Ipv6Data => Ipv6Addr, |d| Ipv6Addr::from(d.0));
impl_from!(Ipv4Data => Ipv4Addr, |d| Ipv4Addr::from(d.0.get()));
impl_from!(UuidData => Uuid, |d| {
    let [hi, lo] = d.0;
    Uuid::from_u64_pair(hi.get(), lo.get())
});
impl_from!(Date16Data => NaiveDate, |d| conv::date16(d.0.get()));
impl_from!(DateTime32Data => chrono::DateTime<chrono::Utc>, |d| conv::datetime32(d.0.get()));

impl TryFrom<Date32Data> for NaiveDate {
    type Error = Error;

    #[inline]
    fn try_from(value: Date32Data) -> Result<Self> {
        conv::date32(value.0.get())
    }
}

impl DateTime64Data {
    pub(crate) fn with_tz_and_precision(
        &self,
        tz: Tz,
        precision: u8,
    ) -> Result<chrono::DateTime<Tz>> {
        conv::datetime64_tz(self.0.get(), precision, tz)
    }
}

impl DateTime32Data {
    #[inline(always)]
    pub(crate) fn with_tz(&self, tz: Tz) -> chrono::DateTime<Tz> {
        conv::datetime32_tz(self.0.get(), tz)
    }
}

impl Decimal32Data {
    pub(crate) fn with_scale(&self, scale: u8) -> rust_decimal::Decimal {
        let value = self.0.get();
        rust_decimal::Decimal::new(i64::from(value), u32::from(scale))
    }
}

impl Decimal64Data {
    pub(crate) fn with_scale(&self, scale: u8) -> rust_decimal::Decimal {
        let value = self.0.get();
        rust_decimal::Decimal::new(value, u32::from(scale))
    }
}

impl Decimal128Data {
    pub(crate) fn with_scale(&self, scale: u8) -> Result<rust_decimal::Decimal> {
        if u32::from(scale) > rust_decimal::Decimal::MAX_SCALE {
            cold_path();
            return Err(Error::NotImplemented(format!(
                "Decimal128 with scale {scale} (rust_decimal supports at most {})",
                rust_decimal::Decimal::MAX_SCALE
            )));
        }
        let value = self.0.get();
        match rust_decimal::Decimal::try_from_i128_with_scale(value, u32::from(scale)) {
            Ok(value) => Ok(value),
            Err(_) => {
                cold_path();
                Err(Error::Overflow(value.to_string()))
            }
        }
    }
}

pub struct ParsedBlock<'a> {
    pub markers: Box<[mark::Mark<'a>]>,
    pub col_names: Box<[&'a str]>,
    pub num_rows: usize,
}

impl<'a> ParsedBlock<'a> {
    pub fn mark(&self, name: &str) -> Result<&mark::Mark<'a>> {
        mark_by_name(&self.col_names, &self.markers, name)
    }

    fn reorder_no_alloc(&mut self, order: &[&str]) {
        // It's O(1) space but ~O(nk) ~ O(n^2) and 0 allocations.
        // cols: [x, b, a1, y, a2, a3, z]
        // order: [a, b, a]
        //         0  1  2
        // First `a` is found at index 2 in cols
        // Everything in range [0..=2] is rotated right, so an item at index 2 (a) goes to index 0
        // cols become [a1, x, b, y, a2, a3, z]
        // On the next iteration we skip all previously handled elements
        for (left, name) in order.iter().copied().enumerate() {
            let pos = self.col_names[left..]
                .iter()
                .copied()
                .position(|col_name| col_name == name)
                .expect("bug: we validated columns exists but apparently not good enough");
            if pos == 0 {
                continue;
            }
            let right = left + pos + 1;
            self.col_names[left..right].rotate_right(1);
            self.markers[left..right].rotate_right(1);
        }
    }
}

#[derive(Clone)]
pub struct BlocksIterator<'data: 'iter, 'iter> {
    blocks: Peekable<std::slice::Iter<'iter, ParsedBlock<'data>>>,
    block_row: usize,
}

impl<'data, 'iter> BlocksIterator<'data, 'iter> {
    pub fn new(blocks: &'iter [ParsedBlock<'data>]) -> Self {
        Self {
            blocks: blocks.iter().peekable(),
            block_row: 0,
        }
    }

    /// Reorders each block's columns in place, then iterates over its rows.
    ///
    /// The requested columns form a prefix in `order`. Duplicate names are matched
    /// by occurrence: the kth request for a name selects the kth occurrence of
    /// that name in the original column layout. All unselected columns, including
    /// surplus occurrences of requested names, follow in their original relative
    /// order. Column names and their markers move together; row order is unchanged.
    /// The reordered layout remains in `blocks` after the iterator is dropped.
    ///
    /// For example, source columns `[x, a₁, b, a₂, y, a₃]` and an order of
    /// `["a", "b", "a"]` produce `[a₁, b, a₂, x, y, a₃]`, where subscripts
    /// distinguish source columns all named `"a"`.
    ///
    /// If either `blocks` or `order` is empty, no reordering or validation occurs.
    /// Otherwise, before mutating any block, checks that the first block contains
    /// every requested occurrence and that all blocks have identical column-name
    /// sequences (including duplicates and their positions). These checks do not
    /// compare column types. A failed check returns [`Error::InvalidColumnOrder`]
    /// without changing any block.
    ///
    /// For name-based reader construction, see [`macro@FromBlock`].
    pub fn new_ordered(blocks: &'iter mut [ParsedBlock<'data>], order: &[&str]) -> Result<Self> {
        reorder_block_cols(blocks, order)?;
        Ok(Self {
            blocks: blocks.iter().peekable(),
            block_row: 0,
        })
    }
}

pub fn reorder_block_cols(blocks: &mut [ParsedBlock<'_>], order: &[&str]) -> Result<()> {
    if blocks.is_empty() || order.is_empty() {
        return Ok(());
    }

    // Opinionated validation that allows reorders be infallible
    validate_blocks(blocks, order)?;

    // I read numbers from my ceiling, sorry
    if order.len() * blocks[0].col_names.len() < 128 {
        for block in blocks.iter_mut() {
            block.reorder_no_alloc(order);
        }
    } else {
        reorder_alloc(blocks, order);
    }

    if let Some(first) = blocks.first() {
        debug!("reordered: {:?}", first.col_names);
    }

    Ok(())
}

fn reorder_alloc(blocks: &mut [ParsedBlock<'_>], order: &[&str]) {
    let Some(first) = blocks.first() else {
        return;
    };

    let mut positions = HashMap::<&str, (Vec<usize>, usize)>::with_capacity(order.len());

    for (index, name) in order.iter().copied().enumerate() {
        let (indices, _used) = positions.entry(name).or_default();
        indices.push(index);
    }

    let mut destinations = Vec::with_capacity(first.col_names.len());
    let mut tail = order.len();

    for &name in &first.col_names {
        let target = match positions.get_mut(name) {
            Some((indices, used)) if *used < indices.len() => {
                let target = indices[*used];
                *used += 1;
                target
            }
            _ => {
                let target = tail;
                tail += 1;
                target
            }
        };

        destinations.push(target);
    }

    // At this moment we assume that we are working with validated data and the column layout this
    // the same everywhere, otherwise we'd need to build dest arr for each col.

    // Not an n^2
    for i in 0..destinations.len() {
        while destinations[i] != i {
            let target = destinations[i];
            for block in blocks.iter_mut() {
                block.col_names.swap(i, target);
                block.markers.swap(i, target);
            }
            destinations.swap(i, target);
        }
    }
}

fn validate_blocks(blocks: &[ParsedBlock<'_>], cols: &[&str]) -> Result<()> {
    // It's more likely that user code messed up columns rather than CH returned some broken blocks
    // with unmatched columns (unless user hasn't created the vec of blocks manually).
    let mut want_counts = HashMap::with_capacity(cols.len());
    for &col in cols {
        *want_counts.entry(col).or_insert(0usize) += 1;
    }

    let mut n = cols.len();
    for &col in &blocks[0].col_names {
        let Some(count) = want_counts.get_mut(col) else {
            continue;
        };
        if *count == 0 {
            continue;
        }
        *count -= 1;
        n -= 1;
        if n == 0 {
            break;
        }
    }

    want_counts.retain(|_, count| *count != 0);

    if !want_counts.is_empty() {
        return Err(Error::InvalidColumnOrder(format!(
            "Missing requested column occurrences: {want_counts:?}"
        )));
    }

    // It is questionable if all blocks should share the same layout, because there can exist
    // such a set of blocks that has a sufficient but different set of columns that can still
    // satisfy the order / columns can be shuffled for some reason / someone manually created
    // a bunch of blocks and wants to iterate over them. Anyway, this assumption lets speculate more
    // and check less in other annoying code here.
    validate_block_layouts(blocks)?;

    Ok(())
}

fn validate_block_layouts(blocks: &[ParsedBlock<'_>]) -> Result<()> {
    let Some((first, rest)) = blocks.split_first() else {
        return Ok(());
    };

    for (index, block) in rest.iter().enumerate() {
        if block.col_names != first.col_names {
            return Err(Error::InvalidColumnOrder(format!(
                "Block {} has different column names: {:?}",
                index + 1,
                block.col_names,
            )));
        }
    }

    Ok(())
}

pub struct BlockRow<'data: 'iter, 'iter> {
    col_names: &'iter [&'data str],
    cols: &'iter [mark::Mark<'data>],
    row_index: usize,
}

impl<'data, 'iter> BlockRow<'data, 'iter> {
    pub const fn cols(&self) -> &'iter [mark::Mark<'data>] {
        self.cols
    }

    pub const fn col_names(&self) -> &'iter [&'data str] {
        self.col_names
    }

    pub const fn row_index(&self) -> usize {
        self.row_index
    }
}

impl<'data, 'iter> Iterator for BlocksIterator<'data, 'iter> {
    type Item = BlockRow<'data, 'iter>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let block = *self.blocks.peek()?;
            if self.block_row >= block.num_rows {
                self.blocks.next();
                self.block_row = 0;
                continue;
            }

            let block_row = BlockRow {
                col_names: &block.col_names,
                cols: &block.markers,
                row_index: self.block_row,
            };
            self.block_row += 1;

            break Some(block_row);
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let mut blocks = self.blocks.clone();
        let mut remaining = match blocks.next() {
            Some(block) => block.num_rows.saturating_sub(self.block_row),
            None => 0,
        };
        for block in blocks {
            remaining += block.num_rows;
        }
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for BlocksIterator<'_, '_> {}

pub fn iter_blocks<'data, 'iter>(
    blocks: &'iter [ParsedBlock<'data>],
) -> BlocksIterator<'data, 'iter> {
    BlocksIterator::new(blocks)
}

/// Equivalent to [`BlocksIterator::new_ordered`]; see its ordering and validation contract.
pub fn iter_blocks_ordered<'data, 'iter>(
    blocks: &'iter mut [ParsedBlock<'data>],
    order: &[&str],
) -> Result<BlocksIterator<'data, 'iter>> {
    BlocksIterator::new_ordered(blocks, order)
}

#[cfg(test)]
pub(crate) mod common {
    use std::{io::Read as _, path::Path, sync::Once};

    use log::LevelFilter;

    static INIT: Once = Once::new();

    pub fn init_logger() {
        INIT.call_once(|| {
            use std::io::Write as _;
            env_logger::builder()
                .format(|buf, record| {
                    writeln!(
                        buf,
                        "{} [{:<5}] {}:{} {}",
                        buf.timestamp_millis(),
                        record.level(),
                        record.file().unwrap_or("<unknown>"),
                        record.line().unwrap_or(0),
                        record.args()
                    )
                })
                .filter_level(LevelFilter::Debug)
                .is_test(true)
                .init();
        });
    }

    pub fn load<P: AsRef<Path>>(path: P) -> std::io::Result<Vec<u8>> {
        init_logger();
        let mut file = std::fs::File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slice::ByteView;

    fn block<'a>(names: &[&'a str], cells: &'a [u8]) -> ParsedBlock<'a> {
        let markers = cells
            .iter()
            .map(|cell| mark::Mark::UInt8(ByteView::try_from(std::slice::from_ref(cell)).unwrap()))
            .collect();
        ParsedBlock {
            markers,
            col_names: names.into(),
            num_rows: 1,
        }
    }

    fn cells(block: &ParsedBlock<'_>) -> Vec<u8> {
        block
            .markers
            .iter()
            .map(|mark| mark.get_u8(0).unwrap().unwrap())
            .collect()
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn tiny_range_round_trips_when_end_exceeds_u32_max() -> Result<()> {
        let range = (u32::MAX as usize - 1)..(u32::MAX as usize + 10);
        let tiny = TinyRange::try_from(range.clone())?;
        assert_eq!(
            tiny,
            TinyRange {
                start: u32::MAX - 1,
                length: 11
            }
        );
        assert_eq!(Range::<usize>::from(tiny), range);
        Ok(())
    }

    #[test]
    fn reorder_moves_markers_with_names_and_keeps_unrequested_tail() -> Result<()> {
        let mut blocks = [block(&["a", "b", "c", "d", "e"], &[0, 1, 2, 3, 4])];
        reorder_block_cols(&mut blocks, &["e", "c", "a"])?;
        assert_eq!(*blocks[0].col_names, ["e", "c", "a", "b", "d"]);
        assert_eq!(cells(&blocks[0]), [4, 2, 0, 1, 3]);
        Ok(())
    }

    #[test]
    fn reorder_reports_missing_requested_columns() {
        let mut blocks = [block(&["a", "b"], &[0, 1])];
        let err = reorder_block_cols(&mut blocks, &["b", "zzz"]).unwrap_err();
        assert!(
            matches!(&err, Error::InvalidColumnOrder(msg) if msg.contains("zzz")),
            "{err}"
        );
    }

    #[test]
    fn reorder_rejects_blocks_with_different_layouts() {
        // Block 0 alone satisfies the order; block 1 must still be checked.
        let mut blocks = [block(&["a", "b"], &[0, 1]), block(&["a"], &[0])];
        let err = reorder_block_cols(&mut blocks, &["b", "a"]).unwrap_err();
        assert!(matches!(err, Error::InvalidColumnOrder(_)), "{err}");

        // Same width, different names: must not be silently permuted by block 0's layout.
        let mut blocks = [block(&["a", "b"], &[0, 1]), block(&["b", "a"], &[1, 0])];
        let err = reorder_block_cols(&mut blocks, &["b", "a"]).unwrap_err();
        assert!(matches!(err, Error::InvalidColumnOrder(_)), "{err}");
    }

    #[test]
    fn decimal128_unsupported_scale_is_not_implemented() {
        let data = Decimal128Data(zc::I128::new(1));
        for scale in [29u8, 38] {
            let err = data.with_scale(scale).unwrap_err();
            assert!(
                matches!(&err, Error::NotImplemented(msg) if msg.contains(&format!("scale {scale}"))),
                "{err}"
            );
        }
        assert_eq!(
            data.with_scale(28).unwrap(),
            rust_decimal::Decimal::try_from_i128_with_scale(1, 28).unwrap()
        );
    }

    #[test]
    fn decimal128_value_overflow_stays_overflow() {
        let data = Decimal128Data(zc::I128::new(i128::MAX));
        let err = data.with_scale(0).unwrap_err();
        assert!(matches!(err, Error::Overflow(_)), "{err}");
    }
}
