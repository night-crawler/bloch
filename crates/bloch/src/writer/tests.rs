use chrono::TimeDelta;
use chrono_tz::Tz;
use rust_decimal::Decimal;

use super::*;
use crate::mark::Mark;
use crate::mark::lc::Indices;
use crate::reader::{self, LcStr};
use crate::{
    BlockWriter, Error, FromBlock, ParsedBlock, WriteBlock, interval, parse::block::parse_single,
};

fn parse(bytes: &[u8]) -> ParsedBlock<'_> {
    let (rest, block) = parse_single(bytes).unwrap();
    assert!(rest.is_empty());
    block
}

fn encode<W: RowWriter>(mut writer: BlockWriter<W>) -> crate::Result<Vec<u8>> {
    let mut out = Vec::new();
    writer.encode(&mut out)?;
    Ok(out)
}

fn assert_same_values(expected: &ParsedBlock<'_>, actual: &ParsedBlock<'_>) {
    assert_eq!(expected.num_rows, actual.num_rows);
    for (name, actual_mark) in actual.col_names.iter().zip(&actual.markers) {
        let expected_mark = expected.mark(name).unwrap();
        assert_eq!(
            expected_mark.as_str(),
            actual_mark.as_str(),
            "column {name}"
        );
        for row in 0..expected.num_rows {
            assert_eq!(
                format!("{:?}", expected_mark.get(row).unwrap()),
                format!("{:?}", actual_mark.get(row).unwrap()),
                "column {name}, row {row}"
            );
        }
    }
}

/// Reads `block` through `R` and writes it back with a writer built from the reader.
fn round_trip<'a, R>(block: &'a ParsedBlock<'a>) -> Vec<u8>
where
    R: FromBlock<'a>,
    R::Item: Row<Reader = R>,
{
    let mut writer =
        BlockWriter::<<R::Item as Row>::Writer>::from_reader(&R::from_block(block).unwrap());
    writer.reserve(block.num_rows);
    for row in R::rows(block).unwrap() {
        writer.write(row.unwrap()).unwrap();
    }
    assert_eq!(writer.rows(), block.num_rows);
    encode(writer).unwrap()
}

fn single_column_block(name: &str, writer: &mut dyn ColumnWriter, rows: usize) -> Vec<u8> {
    let mut out = vec![1];
    write_varuint(rows as u64, &mut out);
    writer.flush().unwrap();
    encode_column(name, writer, rows, &mut out).unwrap();
    out
}

fn type_name(writer: &dyn ColumnWriter) -> String {
    let mut name = String::new();
    writer.type_name(&mut name);
    name
}

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct Sample<'a> {
    id: reader::U8<'a>,
    i: reader::I64<'a>,
    f: reader::F64<'a>,
    b: reader::Bool<'a>,
    s: reader::Str<'a>,
    fs: reader::FixedStr<'a>,
    lc: LcStr<'a>,
    ns: reader::Nullable<'a, reader::Str<'a>>,
    ni: reader::Nullable<'a, reader::I64<'a>>,
    alc: reader::Array<'a, LcStr<'a>>,
    an: reader::Array<'a, reader::Nullable<'a, reader::I32<'a>>>,
    m: reader::Map<'a, LcStr<'a>, LcStr<'a>>,
    ma: reader::Map<'a, reader::Str<'a>, reader::Array<'a, reader::U64<'a>>>,
    t: reader::Tuple<(reader::Str<'a>, reader::U8<'a>)>,
    at: reader::Array<'a, reader::Tuple<(LcStr<'a>, reader::I64<'a>)>>,
    aa: reader::Array<'a, reader::Array<'a, reader::U64<'a>>>,
}

#[test]
fn every_column_type_round_trips() {
    let source = parse(include_bytes!("../../testdata/writer_sample.native"));
    let encoded = round_trip::<Sample<'_>>(&source);
    assert_same_values(&source, &parse(&encoded));
}

#[test]
fn array_iter_values_preserve_empty_cells_and_reset_offsets() {
    let rows = [
        vec![String::from("a"), String::from("b"), String::from("a")],
        vec![],
        vec![String::from("b")],
    ];
    let mut writer = Array::new(Lc::<Str>::default());
    for row in &rows {
        writer.write_iter(row.iter().map(String::as_str)).unwrap();
    }
    let bytes = single_column_block("values", &mut writer, rows.len());
    let block = parse(&bytes);
    let reader = reader::Array::<LcStr<'_>>::try_from(&block.markers[0]).unwrap();
    for (i, expected) in rows.iter().enumerate() {
        assert_eq!(
            reader.try_read(i).unwrap().try_collect_vec().unwrap(),
            *expected
        );
    }

    writer.clear();
    writer.write_iter(["", "new"]).unwrap();
    let bytes = single_column_block("values", &mut writer, 1);
    let block = parse(&bytes);
    let reader = reader::Array::<LcStr<'_>>::try_from(&block.markers[0]).unwrap();
    assert_eq!(
        reader.try_read(0).unwrap().try_collect_vec().unwrap(),
        ["", "new"]
    );
}

/// `encode` takes any `io::Write`; a compressing encoder must see the same bytes a `Vec` gets.
#[test]
fn encode_streams_into_a_compressing_writer() {
    use std::io::Read as _;

    let source = parse(include_bytes!("../../testdata/writer_sample.native"));
    let plain = round_trip::<Sample<'_>>(&source);

    let reader = Sample::from_block(&source).unwrap();
    let mut writer = BlockWriter::<<SampleItem<'_> as Row>::Writer>::from_reader(&reader);
    for row in Sample::rows(&source).unwrap() {
        writer.write(row.unwrap()).unwrap();
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    writer.encode(&mut gz).unwrap();
    let compressed = gz.finish().unwrap();

    let mut inflated = Vec::new();
    flate2::read::GzDecoder::new(&compressed[..])
        .read_to_end(&mut inflated)
        .unwrap();
    assert_eq!(inflated, plain);
}

#[derive(crate::FromVariant, crate::IntoVariant)]
enum Payload<'a> {
    Array(reader::ArrayIter<'a, reader::I64<'a>>),
    Int(i64),
    Str(&'a str),
}

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct Sample2<'a> {
    i8: reader::I8<'a>,
    i16: reader::I16<'a>,
    i32: reader::I32<'a>,
    i128: reader::I128<'a>,
    i256: reader::I256<'a>,
    u16: reader::U16<'a>,
    u32: reader::U32<'a>,
    u128: reader::U128<'a>,
    u256: reader::U256<'a>,
    f32: reader::F32<'a>,
    bf16: reader::Bf16<'a>,
    uuid: reader::Uuid<'a>,
    ip4: reader::Ipv4<'a>,
    ip6: reader::Ipv6<'a>,
    d: reader::Date<'a>,
    d32: reader::Date32<'a>,
    dt_server: reader::DateTime<'a>,
    adt_tokyo: reader::Array<'a, reader::DateTime<'a>>,
    dt64: reader::DateTime64<'a>,
    dt64_berlin: reader::DateTime64<'a>,
    dt64_neg: reader::DateTime64<'a>,
    t: reader::Time<'a>,
    t64: reader::Time64<'a>,
    iday: reader::Interval<'a>,
    ims: reader::Interval<'a>,
    dec32: reader::Decimal32<'a>,
    dec64: reader::Decimal64<'a>,
    dec128: reader::Decimal128<'a>,
    e8: reader::Enum8<'a>,
    e16: reader::Enum16<'a>,
    lcn: reader::LcNullableStr<'a>,
    alcn: reader::Array<'a, reader::LcNullableStr<'a>>,
    v: reader::VariantNullable<'a, Payload<'a>>,
    nn: reader::Nullable<'a, reader::Nothing>,
    an: reader::Array<'a, reader::Nothing>,
    pt: reader::Point<'a>,
    ring: reader::Ring<'a>,
    poly: reader::Polygon<'a>,
    mpoly: reader::MultiPolygon<'a>,
    ls: reader::LineString<'a>,
    mls: reader::MultiLineString<'a>,
    usz: reader::Usize<'a>,
}

#[test]
fn parameterized_and_structural_types_round_trip() {
    let source = parse(include_bytes!("../../testdata/writer_sample2.native"));
    let encoded = round_trip::<Sample2<'_>>(&source);
    let output = parse(&encoded);
    assert_same_values(&source, &output);
    assert_eq!(round_trip::<Sample2<'_>>(&output), encoded);
}

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct EnumRow<'a> {
    e8: reader::Enum8<'a>,
    e16: reader::Enum16<'a>,
}

#[test]
fn enum_writer_owns_names_after_reader_and_rows_drop() {
    let (mut writer, expected) = {
        let data = include_bytes!("../../testdata/writer_sample2.native").to_vec();
        let source = parse(&data);
        let reader = EnumRow::from_block(&source).unwrap();
        let expected = EnumRow::rows(&source)
            .unwrap()
            .map(|row| {
                let row = row.unwrap();
                (row.e8.to_owned(), row.e16.to_owned())
            })
            .collect::<Vec<_>>();
        (BlockWriter::from_reader(&reader), expected)
    };
    for (e8, e16) in expected.clone() {
        writer.write(EnumRowItem { e8: &e8, e16: &e16 }).unwrap();
    }
    let data = encode(writer).unwrap();
    let block = parse(&data);
    let actual = EnumRow::rows(&block)
        .unwrap()
        .map(|row| {
            let row = row.unwrap();
            (row.e8.to_owned(), row.e16.to_owned())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "enum labels must survive their source buffers"
    );
}

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct GeometryRow<'a> {
    geo: reader::VariantNullable<'a, reader::Geometry<'a>>,
    arr: reader::Array<'a, reader::VariantNullable<'a, reader::Geometry<'a>>>,
}

#[test]
fn geometry_is_spelled_by_its_alias_and_keeps_fixed_discriminators() {
    let source = parse(include_bytes!("../../testdata/geometry_sample.native"));
    let encoded = round_trip::<GeometryRow<'_>>(&source);
    let output = parse(&encoded);
    for expected in [&b"\x03geo\x08Geometry"[..], b"\x03arr\x0fArray(Geometry)"] {
        assert!(encoded.windows(expected.len()).any(|w| w == expected));
    }
    assert_same_values(&source, &output);
    assert_eq!(round_trip::<GeometryRow<'_>>(&output), encoded);
}

#[derive(crate::FromVariant, crate::IntoVariant)]
enum Unsorted<'a> {
    Str(&'a str),
    Int(i64),
}

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct UnsortedRow<'a> {
    v: reader::Variant<'a, Unsorted<'a>>,
}

/// ClickHouse would reorder `Variant(String, Int64)` to `Variant(Int64, String)` and read the
/// sub-streams in that order, so the writer refuses to spell it.
#[test]
#[should_panic(expected = "sorted by ClickHouse type name")]
fn variant_elements_must_be_declared_in_clickhouse_order() {
    encode(BlockWriter::<<UnsortedRowItem<'_> as Row>::Writer>::new()).unwrap();
}

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct LcRow<'a> {
    value: LcStr<'a>,
}

#[test]
fn low_cardinality_index_width_follows_dictionary_size() {
    let mut writer = BlockWriter::<<LcRowItem<'_> as Row>::Writer>::with_capacity(65_538);
    for (dictionary_size, width) in [(256, 1), (257, 2), (65_536, 2), (65_537, 4)] {
        writer.write(LcRowItem { value: "" }).unwrap();
        for index in 1..dictionary_size {
            let value = index.to_string();
            writer.write(LcRowItem { value: &value }).unwrap();
        }
        // Every owner has dropped; a repeated value grows neither dictionary nor index width.
        writer
            .write(LcRowItem {
                value: &String::from("1"),
            })
            .unwrap();
        let mut bytes = Vec::new();
        writer.encode(&mut bytes).unwrap();
        let block = parse(&bytes);
        let Mark::LowCardinality(lc) = &block.markers[0] else {
            panic!("not LowCardinality");
        };
        let actual_width = match lc.indices {
            Indices::U8(_) => 1,
            Indices::U16(_) => 2,
            Indices::U32(_) => 4,
            Indices::U64(_) => 8,
        };
        assert_eq!(actual_width, width, "dictionary of {dictionary_size}");
        let values: Vec<_> = LcRow::rows(&block)
            .unwrap()
            .map(|row| row.unwrap().value)
            .collect();
        assert_eq!(values.len(), dictionary_size + 1);
        assert_eq!(values[0], "");
        assert_eq!(
            values[dictionary_size - 1],
            (dictionary_size - 1).to_string()
        );
        assert_eq!(values[dictionary_size], "1");
    }
}

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct HashedLcRow<'a> {
    #[col(writer = Lc<Str, std::hash::RandomState>)]
    value: LcStr<'a>,
}

/// The dictionary hasher is a type parameter; the bytes do not depend on it.
#[test]
fn low_cardinality_hasher_is_selectable() {
    let rows = ["b", "a", "b", ""];
    let mut fx = BlockWriter::<<LcRowItem<'_> as Row>::Writer>::new();
    let mut random = BlockWriter::<<HashedLcRowItem<'_> as Row>::Writer>::new();
    for value in rows {
        fx.write(LcRowItem { value }).unwrap();
        random.write(HashedLcRowItem { value }).unwrap();
    }
    assert_eq!(encode(fx).unwrap(), encode(random).unwrap());
}

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct MapRow<'a> {
    #[col(name = "m")]
    map: reader::Map<'a, LcStr<'a>, LcStr<'a>>,
}

#[test]
fn empty_block_has_types_only_and_empty_maps_keep_prefixes() {
    let empty = encode(BlockWriter::<<MapRowItem<'_> as Row>::Writer>::new()).unwrap();
    assert_eq!(parse(&empty).num_rows, 0);
    assert_eq!(
        empty,
        single_column_block("m", &mut <MapRowItem<'_> as Row>::Writer::default().map, 0)
    );

    let mut columns = <MapRowItem<'_> as Row>::Writer::default();
    columns.map.push_default();
    columns.map.push_default();
    let bytes = single_column_block("m", &mut columns.map, 2);
    let block = parse(&bytes);
    let mut writer = BlockWriter::<<MapRowItem<'_> as Row>::Writer>::new();
    for row in MapRow::rows(&block).unwrap() {
        let row = row.unwrap();
        assert_eq!(row.map.len(), 0);
        writer.write(row).unwrap();
    }
    let encoded = encode(writer).unwrap();
    assert_eq!(encoded, bytes);
    // Two offsets plus the two LowCardinality prefixes, nothing else.
    assert_eq!(encoded.len() - empty.len(), 2 * 8 + 2 * 8);
}

#[test]
fn row_error_invalidates_the_block() {
    let source = parse(include_bytes!("../../testdata/writer_sample.native"));
    let good = round_trip::<MapRow<'_>>(&source);
    let mut bad = good.clone();
    // The block ends with the value indices; an index past the dictionary is found only when
    // the last row's map is iterated, after its key was already written.
    *bad.last_mut().unwrap() = 200;
    let good = parse(&good);
    let bad = parse(&bad);
    let last = source.num_rows - 1;
    let bad_row = MapRow::from_block(&bad).unwrap().try_read(last).unwrap();
    assert_eq!(bad_row.map.len(), 1);

    let mut writer = BlockWriter::<<MapRowItem<'_> as Row>::Writer>::new();
    for row in MapRow::rows(&good).unwrap().take(last) {
        writer.write(row.unwrap()).unwrap();
    }
    assert!(matches!(
        writer.write(bad_row),
        Err(Error::IndexOutOfBounds(..))
    ));
    let good_row = MapRow::from_block(&good).unwrap().try_read(last).unwrap();
    assert!(matches!(writer.write(good_row), Err(Error::WriteFailed)));
    assert!(matches!(
        writer.encode(&mut Vec::new()),
        Err(Error::WriteFailed)
    ));

    // The failed block is discarded; the same writer then encodes a complete block.
    assert_eq!(writer.rows(), 0);
    for row in MapRow::rows(&good).unwrap() {
        writer.write(row.unwrap()).unwrap();
    }
    let mut out = Vec::new();
    writer.encode(&mut out).unwrap();
    assert_same_values(&good, &parse(&out));
}

/// `encode` empties the writer: the next block gets fresh dictionaries (slot zero reseeded,
/// no stale entries) and byte-identical output for identical rows.
#[test]
fn writer_is_reusable_after_encode() {
    let source = parse(include_bytes!("../../testdata/writer_sample.native"));
    let reader = Sample::from_block(&source).unwrap();
    let mut writer = BlockWriter::<<SampleItem<'_> as Row>::Writer>::from_reader(&reader);
    let mut blocks = Vec::new();
    for _ in 0..2 {
        for row in Sample::rows(&source).unwrap() {
            writer.write(row.unwrap()).unwrap();
        }
        let mut out = Vec::new();
        writer.encode(&mut out).unwrap();
        blocks.push(out);
    }
    assert_eq!(blocks[0], blocks[1]);
    assert_same_values(&source, &parse(&blocks[1]));
}

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct OwnedLcRow<'a> {
    s: LcStr<'a>,
    n: reader::LcNullable<'a, reader::Str<'a>>,
    d: reader::Lc<'a, reader::Date<'a>>,
}

/// Each input owner is dropped before encoding; NULL, empty, and repeated values survive
/// dictionary reuse and the UInt8/UInt16 index-width boundary.
#[test]
fn low_cardinality_owns_short_lived_values_and_reuses_dictionaries() {
    let rows = 907;
    let date = |i: usize| {
        chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap() + chrono::Days::new((i % 50) as u64)
    };
    let mut writer = BlockWriter::<<OwnedLcRowItem<'_> as Row>::Writer>::new();
    for pass in 0..2 {
        for i in 0..rows {
            let value = format!("v{pass}-{}", i % 300);
            let nullable = format!("n{pass}-{}", i % 300);
            writer
                .write(OwnedLcRowItem {
                    s: if i % 97 == 0 { "" } else { &value },
                    n: match i % 13 {
                        0 => None,
                        1 => Some(""),
                        _ => Some(&nullable),
                    },
                    d: date(i),
                })
                .unwrap();
        }
        let mut encoded = Vec::new();
        writer.encode(&mut encoded).unwrap();
        let block = parse(&encoded);
        assert_eq!(block.num_rows, rows);
        for (i, row) in OwnedLcRow::rows(&block).unwrap().enumerate() {
            let row = row.unwrap();
            let value = format!("v{pass}-{}", i % 300);
            let nullable = format!("n{pass}-{}", i % 300);
            assert_eq!(row.s, if i % 97 == 0 { "" } else { &value }, "row {i}");
            assert_eq!(
                row.n,
                match i % 13 {
                    0 => None,
                    1 => Some(""),
                    _ => Some(nullable.as_str()),
                },
                "row {i}"
            );
            assert_eq!(row.d, date(i), "row {i}");
        }
        for (name, keys) in [("s", 301), ("n", 302)] {
            let Mark::LowCardinality(lc) = block.mark(name).unwrap() else {
                panic!("not LowCardinality");
            };
            assert_eq!(lc.keys().unwrap().len(), keys);
            assert!(matches!(lc.indices, Indices::U16(_)));
        }
    }
}

/// Dictionary conversion happens within `write`, and a failed value invalidates the block.
#[test]
fn low_cardinality_value_error_fails_the_block_at_write() {
    let mut writer = BlockWriter::<<OwnedLcRowItem<'_> as Row>::Writer>::new();
    let row = |d| OwnedLcRowItem { s: "a", n: None, d };
    let ok = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
    writer.write(row(ok)).unwrap();
    assert!(matches!(
        writer.write(row(chrono::NaiveDate::from_ymd_opt(1900, 1, 1).unwrap())),
        Err(Error::ValueOutOfRange("NaiveDate", "Date", _))
    ));
    assert!(matches!(writer.write(row(ok)), Err(Error::WriteFailed)));
    assert!(matches!(
        writer.encode(&mut Vec::new()),
        Err(Error::WriteFailed)
    ));

    assert_eq!(writer.rows(), 0);
    writer.write(row(ok)).unwrap();
    let encoded = encode(writer).unwrap();
    let block = parse(&encoded);
    assert_eq!(block.num_rows, 1);
    assert_eq!(
        OwnedLcRow::rows(&block).unwrap().next().unwrap().unwrap().d,
        ok
    );
}

#[test]
fn low_cardinality_owns_non_utf8_bytes_and_clears() {
    type Bytes<'a> = reader::Lc<'a, reader::Bytes<'a>>;
    type NullableBytes<'a> = reader::LcNullable<'a, reader::Bytes<'a>>;
    let mut writer = Lc::<super::Bytes>::default();
    let mut nullable = LcNullable::<super::Bytes>::default();
    for pass in 0..2 {
        for i in 0..907 {
            let [lo, hi] = u16::try_from(i % 300).unwrap().to_le_bytes();
            let value = vec![0xff, pass, lo, hi];
            let value = bstr::BStr::new(&value);
            FromReader::<Bytes<'_>>::write_item(&mut writer, value).unwrap();
            FromReader::<NullableBytes<'_>>::write_item(
                &mut nullable,
                match i % 13 {
                    0 => None,
                    1 => Some(bstr::BStr::new(b"")),
                    _ => Some(value),
                },
            )
            .unwrap();
        }
        let bytes = single_column_block("bytes", &mut writer, 907);
        let nullable_bytes = single_column_block("nullable", &mut nullable, 907);
        let block = parse(&bytes);
        let nullable_block = parse(&nullable_bytes);
        let reader = Bytes::try_from(&block.markers[0]).unwrap();
        let nullable_reader = NullableBytes::try_from(&nullable_block.markers[0]).unwrap();
        for i in 0..907 {
            let [lo, hi] = u16::try_from(i % 300).unwrap().to_le_bytes();
            let expected = [0xff, pass, lo, hi];
            let expected = bstr::BStr::new(&expected);
            assert_eq!(reader.try_read(i).unwrap(), expected);
            assert_eq!(
                nullable_reader.try_read(i).unwrap(),
                match i % 13 {
                    0 => None,
                    1 => Some(bstr::BStr::new(b"")),
                    _ => Some(expected),
                }
            );
        }
        for (block, keys) in [(&block, 301), (&nullable_block, 302)] {
            let Mark::LowCardinality(lc) = &block.markers[0] else {
                panic!("not LowCardinality");
            };
            assert_eq!(lc.keys().unwrap().len(), keys);
            assert!(matches!(lc.indices, Indices::U16(_)));
        }
        writer.clear();
        nullable.clear();
    }
}

#[test]
fn low_cardinality_preserves_float_bits() {
    type Floats<'a> = reader::Lc<'a, reader::F64<'a>>;
    let mut writer = Lc::<FixedWidthColumn<F64>>::default();
    let bits = [
        0,
        (-0.0_f64).to_bits(),
        0x7ff8_0000_0000_0001,
        0x7ff8_0000_0000_0002,
        0x7ff8_0000_0000_0001,
    ];
    for bits in bits {
        FromReader::<Floats<'_>>::write_item(&mut writer, f64::from_bits(bits)).unwrap();
    }
    let bytes = single_column_block("floats", &mut writer, bits.len());
    let block = parse(&bytes);
    let reader = Floats::try_from(&block.markers[0]).unwrap();
    for (i, bits) in bits.into_iter().enumerate() {
        assert_eq!(reader.try_read(i).unwrap().to_bits(), bits);
    }
    let Mark::LowCardinality(lc) = &block.markers[0] else {
        panic!("not LowCardinality");
    };
    assert_eq!(lc.keys().unwrap().len(), 4);
}

#[test]
fn fixed_string_rejects_long_values() {
    let mut writer = FixedWidthColumn::with_capacity(FixedStr { size: 2 }, 1);
    assert!(matches!(
        FromReader::<reader::FixedStr<'_>>::write_item(&mut writer, "abc"),
        Err(Error::FixedStringOverflow(3, 2))
    ));
    FromReader::<reader::FixedStr<'_>>::write_item(&mut writer, "a").unwrap();
    let mut data = Vec::new();
    writer.write_data(&mut data).unwrap();
    assert_eq!(data, b"a\0");
}

#[test]
fn parameterized_type_names_match_clickhouse_spelling() {
    assert_eq!(
        type_name(&FixedWidthColumn::new(Enum8::new(&[
            ("it's", 1),
            ("b\\c", -2)
        ]))),
        r"Enum8('it\'s' = 1, 'b\\c' = -2)"
    );
    assert_eq!(
        type_name(&FixedWidthColumn::new(DateTime64 {
            precision: 6,
            tz: Some(Tz::Europe__Berlin),
        })),
        "DateTime64(6, 'Europe/Berlin')"
    );
    assert_eq!(
        type_name(&FixedWidthColumn::<DateTime>::default()),
        "DateTime"
    );
    assert_eq!(
        type_name(&FixedWidthColumn::new(Decimal64 { scale: 4 })),
        "Decimal(18, 4)"
    );
    assert_eq!(
        type_name(&FixedWidthColumn::new(Interval {
            kind: interval::Kind::Week,
        })),
        "IntervalWeek"
    );
}

#[test]
fn values_that_do_not_fit_the_column_are_rejected() {
    let mut writer = FixedWidthColumn::new(Enum8::new(&[("a", 1)]));
    assert!(FromReader::<reader::Enum8<'_>>::write_item(&mut writer, "b").is_err());

    let mut writer = FixedWidthColumn::new(Decimal32 { scale: 2 });
    FromReader::<reader::Decimal32<'_>>::write_item(&mut writer, Decimal::new(12_345, 3)).unwrap();
    assert!(
        FromReader::<reader::Decimal32<'_>>::write_item(
            &mut writer,
            Decimal::new(i64::from(i32::MAX), 0)
        )
        .is_err()
    );
    let mut data = Vec::new();
    writer.write_data(&mut data).unwrap();
    // 12.345 rounds to 12.35 at scale 2.
    assert_eq!(data, 1235i32.to_le_bytes());

    let mut writer = FixedWidthColumn::new(Interval {
        kind: interval::Kind::Day,
    });
    assert!(
        FromReader::<reader::Interval<'_>>::write_item(&mut writer, TimeDelta::hours(36)).is_err()
    );
    FromReader::<reader::Interval<'_>>::write_item(&mut writer, TimeDelta::days(-2)).unwrap();
    let mut data = Vec::new();
    writer.write_data(&mut data).unwrap();
    assert_eq!(data, (-2i64).to_le_bytes());

    let before_epoch = chrono::DateTime::from_timestamp(-1, 0)
        .unwrap()
        .with_timezone(&Tz::UTC);
    let mut writer = FixedWidthColumn::<DateTime>::default();
    assert!(FromReader::<reader::DateTime<'_>>::write_item(&mut writer, before_epoch).is_err());

    // Precision beyond nanoseconds is an error, not an overflow.
    let mut writer = FixedWidthColumn::new(DateTime64 {
        precision: 10,
        tz: None,
    });
    assert!(matches!(
        FromReader::<reader::DateTime64<'_>>::write_item(&mut writer, before_epoch),
        Err(Error::ValueOutOfRange(_, "precision (0..=9)", _))
    ));
}

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct LcFixedRow<'a> {
    id: reader::Lc<'a, reader::Uuid<'a>>,
    t: reader::Lc<'a, reader::DateTime<'a>>,
    x: reader::Lc<'a, reader::F64<'a>>,
    n: reader::Lc<'a, reader::U32<'a>>,
    ip: reader::LcNullable<'a, reader::Ipv6<'a>>,
    ids: reader::Array<'a, reader::Lc<'a, reader::U128<'a>>>,
    addrs: reader::Array<'a, reader::Lc<'a, reader::U64<'a>>>,
}

/// `LowCardinality` of fixed-size types (`allow_suspicious_low_cardinality_types`): `f64` and
/// `DateTime` have no `Hash`/`Default`, so the dictionary keys them through `LcKey`.
#[test]
fn low_cardinality_of_fixed_size_types_round_trips() {
    let source = parse(include_bytes!("../../testdata/lc_fixed_types.native"));
    let encoded = round_trip::<LcFixedRow<'_>>(&source);
    let output = parse(&encoded);
    assert_same_values(&source, &output);
    assert_eq!(round_trip::<LcFixedRow<'_>>(&output), encoded);
}
