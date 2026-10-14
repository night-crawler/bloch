use std::collections::HashMap;

use testresult::TestResult;

use super::*;
use crate::common::load;
use crate::error::Error;
use crate::mark;
use crate::parse::block::parse_single;
use crate::{FromBlock, FromVariant};

#[test]
fn array_map_sample_typed() -> TestResult {
    type ArrMap<'a> = Array<'a, Map<'a, Str<'a>, Str<'a>>>;

    let buf = load("./testdata/array_map_sample.native")?;
    let (_, block) = parse_single(&buf)?;

    let reader: ArrMap = Array::try_from(&block.markers[1])?;

    let expected: [Vec<HashMap<&str, &str>>; 6] = [
        vec![
            HashMap::from([("a", "apple"), ("b", "banana")]),
            HashMap::from([("c", "cherry")]),
        ],
        vec![
            HashMap::from([("d", "date")]),
            HashMap::from([("e", "elderberry"), ("f", "fig")]),
        ],
        vec![HashMap::from([("g", "grape"), ("h", "honeydew")])],
        vec![HashMap::from([("i", "kiwi")])],
        vec![],
        vec![HashMap::from([("j", "lemon"), ("k", "mango")])],
    ];

    assert_eq!(block.num_rows, expected.len());
    for (row, expected_row) in expected.iter().enumerate() {
        let outer = reader.try_read(row)?;
        let mut actual_row = Vec::new();

        for mp in outer {
            let mut h = HashMap::new();
            for kv in mp? {
                let (k, v) = kv?;
                h.insert(k, v);
            }
            actual_row.push(h);
        }

        assert_eq!(actual_row, *expected_row, "mismatch at top-level row {row}");
    }

    Ok(())
}

#[test]
fn datetime_timezone_forms() -> TestResult {
    const _SQL: &str = r#"
drop table if exists datetime_tz;

create table datetime_tz
(
    a DateTime,
    b DateTime('Europe/Berlin'),
    c DateTime64(3),
    d DateTime64(6, 'Asia/Tokyo'),
    e DateTime64,
    f Nullable(DateTime64(9)),
    g Array(DateTime('UTC'))
) engine = Memory;

insert into datetime_tz values
    (1700000000, 1700000000, 1700000000.123, 1700000000.123456, 1700000000.5, NULL, [1700000000]);

select a, b, c, d, e, f, g, cast(b, 'Nullable(DateTime(''Europe/Berlin''))') as h
from datetime_tz format Native;
"#;

    let buf = load("./testdata/datetime_tz.native")?;
    let (_, block) = parse_single(&buf)?;

    let a = DateTime::try_from(&block.markers[0])?.try_read(0)?;
    let b = DateTime::try_from(&block.markers[1])?.try_read(0)?;
    let c = DateTime64::try_from(&block.markers[2])?.try_read(0)?;
    let d = DateTime64::try_from(&block.markers[3])?.try_read(0)?;
    let e = DateTime64::try_from(&block.markers[4])?.try_read(0)?;
    let f = Nullable::<DateTime64>::try_from(&block.markers[5])?.try_read(0)?;
    let g: Vec<_> = Array::<DateTime>::try_from(&block.markers[6])?
        .try_read(0)?
        .collect::<Result<_, _>>()?;
    let h = Nullable::<DateTime>::try_from(&block.markers[7])?.try_read(0)?;

    assert_eq!(a.timezone(), chrono_tz::UTC);
    assert_eq!(a.timestamp(), 1_700_000_000);
    assert_eq!(b.timezone(), chrono_tz::UTC);
    assert_eq!(b.timestamp(), 1_700_000_000);
    assert_eq!(c.timezone(), chrono_tz::UTC);
    assert_eq!(c.timestamp_millis(), 1_700_000_000_123);
    assert_eq!(d.timezone(), chrono_tz::Asia::Tokyo);
    assert_eq!(d.timestamp_micros(), 1_700_000_000_123_456);
    assert_eq!(d.to_rfc3339(), "2023-11-15T07:13:20.123456+09:00");
    assert_eq!(e.timestamp_millis(), 1_700_000_000_500);
    assert_eq!(f, None);
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].timezone(), chrono_tz::UTC);
    assert_eq!(g[0].timestamp(), 1_700_000_000);
    let h = h.expect("h is non-null");
    assert_eq!(h.timezone(), chrono_tz::Europe::Berlin);
    assert_eq!(h.to_rfc3339(), "2023-11-14T23:13:20+01:00");

    Ok(())
}

fn date32_block(days: i32) -> Vec<u8> {
    let mut bytes = b"\x01\x01\x01d\x06Date32".to_vec();
    bytes.extend_from_slice(&days.to_le_bytes());
    bytes
}

#[test]
fn date32_out_of_range_returns_error() -> TestResult {
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    let min_days = i32::try_from((chrono::NaiveDate::MIN - epoch).num_days())?;
    let max_days = i32::try_from((chrono::NaiveDate::MAX - epoch).num_days())?;

    for days in [i32::MIN, min_days - 1, max_days + 1, i32::MAX] {
        let bytes = date32_block(days);
        let (_, block) = parse_single(&bytes)?;
        let mark = block.mark("d")?;
        assert!(matches!(mark.get(0), Err(Error::ValueOutOfRange(..))));
        assert!(matches!(
            Date32::try_from(mark)?.try_read(0),
            Err(Error::ValueOutOfRange(..))
        ));
        assert!(mark.get(1)?.is_none());
    }
    Ok(())
}

#[test]
fn date32_preserves_representable_dates() -> TestResult {
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    let min_days = i32::try_from((chrono::NaiveDate::MIN - epoch).num_days())?;
    let max_days = i32::try_from((chrono::NaiveDate::MAX - epoch).num_days())?;

    for (days, expected) in [
        (min_days, chrono::NaiveDate::MIN),
        (-1, chrono::NaiveDate::from_ymd_opt(1969, 12, 31).unwrap()),
        (0, epoch),
        // ClickHouse DATE_LUT_MAX_EXTEND_DAY_NUM.
        (
            2_932_896,
            chrono::NaiveDate::from_ymd_opt(9999, 12, 31).unwrap(),
        ),
        (max_days, chrono::NaiveDate::MAX),
    ] {
        let bytes = date32_block(days);
        let (_, block) = parse_single(&bytes)?;
        let mark = block.mark("d")?;
        assert!(matches!(
            mark.get(0)?,
            Some(crate::value::Value::Date32(date)) if date == expected
        ));
        assert_eq!(Date32::try_from(mark)?.try_read(0)?, expected);
    }
    Ok(())
}

#[cfg(feature = "serde1")]
#[test]
fn date32_json_conversion_checks_range() -> TestResult {
    for days in [i32::MIN, 2_932_896, i32::MAX] {
        let bytes = date32_block(days);
        let (_, block) = parse_single(&bytes)?;
        let mark = mark::Mark::Json(mark::Json::new(
            vec!["d".into()],
            block.markers.into_vec(),
            1,
            1,
        )?);
        let result = Json::try_from(&mark)?
            .try_read(0)?
            .deserialize::<serde_json::Value>();
        if days == 2_932_896 {
            assert_eq!(result?, serde_json::json!({"d": "9999-12-31"}));
        } else {
            assert!(matches!(
                result,
                Err(JsonDeserializeError::Source(error))
                    if matches!(*error, Error::ValueOutOfRange(..))
            ));
        }
    }
    Ok(())
}

#[test]
fn array_all_empty_rows_from_fixture() -> TestResult {
    let buf = load("./testdata/array_lc_string_empty.native")?;
    let (_, block) = parse_single(&buf)?;

    let reader: Array<LcStr> = Array::try_from(&block.markers[1])?;

    for row in 0..block.num_rows {
        let count = reader.try_read(row)?.count();
        assert_eq!(count, 0, "expected empty array at row {row}");
    }

    Ok(())
}

#[test]
fn derive_from_block_with_names() -> TestResult {
    const ID_COL: &str = "id";

    #[derive(FromBlock, Copy, Clone)]
    struct ArrMapRow<'a> {
        #[col(name = ID_COL)]
        id: I64<'a>,

        #[col(name = "arr_map")]
        maps: Array<'a, Map<'a, Str<'a>, Str<'a>>>,
    }

    let buf = load("./testdata/array_map_sample.native")?;
    let (_, block) = parse_single(&buf)?;

    let expected: [Vec<HashMap<&str, &str>>; 6] = [
        vec![
            HashMap::from([("a", "apple"), ("b", "banana")]),
            HashMap::from([("c", "cherry")]),
        ],
        vec![
            HashMap::from([("d", "date")]),
            HashMap::from([("e", "elderberry"), ("f", "fig")]),
        ],
        vec![HashMap::from([("g", "grape"), ("h", "honeydew")])],
        vec![HashMap::from([("i", "kiwi")])],
        vec![],
        vec![HashMap::from([("j", "lemon"), ("k", "mango")])],
    ];

    let mut num_rows = 0;
    for (row_idx, row) in ArrMapRow::rows(&block)?.enumerate() {
        let row = row?;
        assert_eq!(row.id, i64::try_from(row_idx)?);

        let mut actual = Vec::new();
        for map in row.maps {
            actual.push(map?.collect::<crate::Result<HashMap<_, _>>>()?);
        }
        assert_eq!(actual, expected[row_idx], "mismatch at row {row_idx}");
        num_rows += 1;
    }
    assert_eq!(num_rows, 6);

    Ok(())
}

#[test]
fn derive_nested_struct_in_array_of_tuples() -> TestResult {
    #[derive(FromBlock, Copy, Clone)]
    struct Fruit<'a> {
        name: LcStr<'a>,
        rank: I64<'a>,
    }

    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        id: I64<'a>,
        arr: Array<'a, Fruit<'a>>,
    }

    let buf = load("./testdata/array_of_tuples.native")?;
    let (_, block) = parse_single(&buf)?;

    let expected: [Vec<(&str, i64)>; 6] = [
        vec![("apple", 1), ("banana", 2), ("cherry", 3)],
        vec![("date", 4), ("elderberry", 5)],
        vec![("fig", 6), ("grape", 7), ("honeydew", 8)],
        vec![("kiwi", 9)],
        vec![],
        vec![("lemon", 10), ("mango", 11)],
    ];

    for (row_idx, row) in Row::rows(&block)?.enumerate() {
        let row = row?;
        assert_eq!(row.id, i64::try_from(row_idx)?);
        let actual = row
            .arr
            .map(|f| f.map(|f| (f.name, f.rank)))
            .collect::<crate::Result<Vec<_>>>()?;
        assert_eq!(actual, expected[row_idx], "mismatch at row {row_idx}");
    }

    Ok(())
}

#[test]
fn derive_nested_column_positional() -> TestResult {
    #[derive(FromBlock, Copy, Clone)]
    struct Child<'a> {
        child_id: U64<'a>,
        child_name: Str<'a>,
    }

    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        id: I64<'a>,
        nes: Array<'a, Child<'a>>,
    }

    let buf = load("./testdata/simple_nested.native")?;
    let (_, block) = parse_single(&buf)?;

    let expected: [Vec<(u64, &str)>; 6] = [
        vec![(1, "Alice"), (2, "Bob")],
        vec![(3, "Charlie"), (4, "Diana")],
        vec![(5, "Eve")],
        vec![],
        vec![(6, "Frank"), (7, "Grace")],
        vec![(8, "Heidi")],
    ];

    for (row_idx, row) in Row::rows(&block)?.enumerate() {
        let row = row?;
        assert_eq!(row.id, i64::try_from(row_idx)?);
        let actual = row
            .nes
            .map(|c| c.map(|c| (c.child_id, c.child_name)))
            .collect::<crate::Result<Vec<_>>>()?;
        assert_eq!(actual, expected[row_idx], "mismatch at row {row_idx}");
    }

    Ok(())
}

#[test]
fn derive_col_tuple() -> TestResult {
    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        id: I64<'a>,
        tup: Tuple<(I64<'a>, Str<'a>)>,
    }

    let buf = load("./testdata/tuple.native")?;
    let (_, block) = parse_single(&buf)?;

    let expected: [(i64, &str); 7] = [
        (1, "a"),
        (3, "ab"),
        (7, "ac"),
        (9, "ad"),
        (11, "ae"),
        (2, "af"),
        (3, "ag"),
    ];

    for (row_idx, row) in Row::rows(&block)?.enumerate() {
        let row = row?;
        assert_eq!(row.id, i64::try_from(row_idx)?);
        assert_eq!(row.tup, expected[row_idx], "mismatch at row {row_idx}");
    }

    Ok(())
}

#[test]
fn derive_nullable() -> TestResult {
    #[derive(FromBlock, Copy, Clone)]
    struct NullableRow<'a> {
        id: I64<'a>,
        nstr: Nullable<'a, Str<'a>>,
    }

    let buf = load("./testdata/nullable_string.native")?;
    let (_, block) = parse_single(&buf)?;

    let expected = [
        Some("hello"),
        None,
        Some("world"),
        Some("clickhouse"),
        None,
        Some("test"),
    ];

    for (row_idx, row) in NullableRow::rows(&block)?.enumerate() {
        let row = row?;
        assert_eq!(row.id, i64::try_from(row_idx)?);
        assert_eq!(row.nstr, expected[row_idx], "mismatch at row {row_idx}");
    }
    Ok(())
}

#[test]
fn derive_lc_nullable() -> TestResult {
    #[derive(FromBlock, Copy, Clone)]
    struct LcRow<'a> {
        id: I64<'a>,
        nlc_str: LcNullableStr<'a>,
    }

    let buf = load("./testdata/nullable_lc_str.native")?;
    let (_, block) = parse_single(&buf)?;

    let expected = [
        Some("apple"),
        None,
        Some("banana"),
        Some("cherry"),
        None,
        Some("date"),
    ];

    for (row_idx, row) in LcRow::rows(&block)?.enumerate() {
        let row = row?;
        assert_eq!(row.id, i64::try_from(row_idx)?);
        assert_eq!(row.nlc_str, expected[row_idx], "mismatch at row {row_idx}");
    }

    Ok(())
}

#[test]
fn derive_missing_column() -> TestResult {
    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        #[col(name = "no_such_column")]
        id: I64<'a>,
    }

    let buf = load("./testdata/nullable_string.native")?;
    let (_, block) = parse_single(&buf)?;

    let Err(err) = Row::from_block(&block) else {
        panic!("expected missing column error");
    };
    assert!(
        matches!(&err, Error::ColumnNotFound(name) if name == "no_such_column"),
        "unexpected error: {err:?}"
    );

    Ok(())
}

#[test]
fn array_try_as_slice() -> TestResult {
    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        id: I64<'a>,
        u128_array: Array<'a, U128<'a>>,
    }

    let buf = load("./testdata/sample_128.native")?;
    let (_, block) = parse_single(&buf)?;

    let row = Row::rows(&block)?.next().expect("expected a row")?;
    assert_eq!(row.id, 0);

    let slice = row.u128_array.try_as_slice()?;
    let values = slice.iter().map(|v| v.get()).collect::<Vec<_>>();
    assert_eq!(
        values,
        [
            12_345_678_901_234_567_890_123_456_789_012u128,
            98_765_432_109_876_543_210_987_654_321_098u128,
        ]
    );

    // the slice matches element-wise iteration
    let iterated = row
        .u128_array
        .map(|v| v.map_err(Into::into))
        .collect::<TestResult<Vec<_>>>()?;
    assert_eq!(values, iterated);

    Ok(())
}

#[test]
fn derive_col_value_escape_hatch() -> TestResult {
    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        id: I64<'a>,
        #[col(name = "dyn")]
        value: Value<'a>,
    }

    let buf = load("./testdata/dynamic.native")?;
    let (_, block) = parse_single(&buf)?;

    let first = Row::rows(&block)?.next().expect("expected a row")?;
    assert_eq!(first.id, 0);

    let s: &str = first.value.try_into()?;
    assert_eq!(s, "string value");

    Ok(())
}

#[test]
fn derive_iter_blocks_flat() -> TestResult {
    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        id: Uuid<'a>,
    }

    // Native is a bare sequence of self-describing blocks, so this fixture needs no query:
    //   cat testdata/uuid_and_dates.native testdata/uuid_and_dates.native > testdata/multi_block.native
    let buf = load("./testdata/multi_block.native")?;
    let blocks = crate::parse::block::parse_many(&buf)?;
    assert!(blocks.len() > 1, "expected a multi-block file");

    let expected: usize = blocks.iter().map(|b| b.num_rows).sum();
    let mut count = 0usize;
    for row in Row::iter_blocks(&blocks) {
        let row = row?;
        let _: uuid::Uuid = row.id;
        count += 1;
    }
    assert_eq!(count, expected);

    Ok(())
}

#[test]
fn iter_blocks_size_hint_bounds_error_items() {
    // The `id` column is missing, so each block yields exactly one `Err`
    // regardless of `num_rows`.
    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        id: I64<'a>,
    }

    fn bounds_hold(blocks: &[crate::ParsedBlock<'_>]) {
        let iterator = Row::iter_blocks(blocks);
        let (lower, upper) = iterator.size_hint();
        let count = iterator.count();
        assert!(lower <= count, "lower {lower} > count {count}");
        assert!(count <= upper.unwrap(), "count {count} > upper {upper:?}");
    }

    let empty_block = |num_rows| crate::ParsedBlock {
        markers: Box::new([]),
        col_names: Box::new([]),
        num_rows,
    };
    bounds_hold(&[empty_block(0)]);
    bounds_hold(&[empty_block(5)]);
}

#[test]
fn try_read_out_of_bounds() -> TestResult {
    let buf = load("./testdata/nullable_string.native")?;
    let (_, block) = parse_single(&buf)?;

    let reader = Nullable::<Str>::try_from(&block.markers[1])?;
    let Err(err) = reader.try_read(block.num_rows) else {
        panic!("expected out of bounds error");
    };
    assert!(
        matches!(err, Error::IndexOutOfBounds(idx, "Nullable") if idx == block.num_rows),
        "unexpected error: {err:?}"
    );

    Ok(())
}

#[test]
fn named_tuple_by_name() -> TestResult {
    // Field order deliberately doesn't match the def
    #[derive(FromBlock, Copy, Clone)]
    struct Fruit<'a> {
        rank: I64<'a>,
        #[col(name = "name")]
        title: Str<'a>,
    }

    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        id: I64<'a>,
        tup: Fruit<'a>,
    }

    let buf = load("./testdata/named_tuple.native")?;
    let (_, block) = parse_single(&buf)?;

    let expected = ["apple", "banana", "cherry", "date", "elderberry", "fig"];

    let mut num_rows = 0;
    for (row_idx, row) in Row::rows(&block)?.enumerate() {
        let row = row?;
        assert_eq!(row.id, i64::try_from(row_idx)?);
        assert_eq!(
            row.tup.title, expected[row_idx],
            "mismatch at row {row_idx}"
        );
        assert_eq!(row.tup.rank, row.id * 10);
        num_rows += 1;
    }
    assert_eq!(num_rows, expected.len());

    Ok(())
}

#[test]
fn named_tuple_missing_field() -> TestResult {
    #[derive(FromBlock, Copy, Clone)]
    struct Fruit<'a> {
        #[col(name = "no_such_field")]
        rank: I64<'a>,
    }

    let buf = load("./testdata/named_tuple.native")?;
    let (_, block) = parse_single(&buf)?;

    let Err(err) = Fruit::try_from(&block.markers[1]) else {
        panic!("expected missing tuple field error");
    };
    assert!(
        matches!(&err, Error::ColumnNotFound(name) if name == "no_such_field"),
        "unexpected error: {err:?}"
    );

    Ok(())
}

#[test]
fn col_tuple_reads_named_tuple_positionally() -> TestResult {
    let buf = load("./testdata/named_tuple.native")?;
    let (_, block) = parse_single(&buf)?;

    let reader = Tuple::<(Str, I64)>::try_from(&block.markers[1])?;
    assert_eq!(reader.try_read(0)?, ("apple", 0));
    assert_eq!(reader.try_read(5)?, ("fig", 50));

    Ok(())
}

#[test]
fn empty_tuple_keeps_row_count() -> TestResult {
    // Needs a ClickHouse 26.8 server and client: a 25.3 client desynchronises on the
    // Array(Tuple()) column, so the stream cannot be captured there.
    const _SQL: &str = r#"
select
    tuple()            as t,
    [tuple(), tuple()] as at,
    number
from numbers(2) format Native;
"#;

    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        t: Value<'a>,
        at: Array<'a, Value<'a>>,
        number: U64<'a>,
    }

    let buf = load("./testdata/empty_tuple.native")?;
    let (_, block) = parse_single(&buf)?;
    assert_eq!(block.num_rows, 2);
    assert_eq!(block.mark("t")?.len(), 2);

    let mut rows = 0;
    for (row_idx, row) in Row::rows(&block)?.enumerate() {
        let row = row?;
        assert!(
            matches!(row.t, crate::value::Value::Tuple { mark, index } if mark.values.is_empty() && index == row_idx),
            "row {row_idx}: {:?}",
            row.t
        );
        let at: Vec<_> = row.at.collect::<Result<_, _>>()?;
        assert_eq!(at.len(), 2, "row {row_idx}: [tuple(), tuple()]");
        assert!(at.iter().all(
            |v| matches!(v, crate::value::Value::Tuple { mark, .. } if mark.values.is_empty())
        ));
        assert_eq!(row.number, u64::try_from(row_idx)?);
        rows += 1;
    }
    assert_eq!(rows, 2);

    Ok(())
}

#[test]
fn derive_variant_enum() -> TestResult {
    // Variants in the server-canonicalized order of Variant(Array(Int64), Int64, String).
    #[derive(FromVariant)]
    enum Var<'a> {
        Arr(ArrayIter<'a, I64<'a>>),
        Num(i64),
        Str(&'a str),
    }

    let buf = load("./testdata/variant.native")?;
    let (_, block) = parse_single(&buf)?;

    let reader: Variant<Var> = Variant::try_from(&block.markers[1])?;

    let mut repr = Vec::with_capacity(block.num_rows);
    for i in 0..block.num_rows {
        repr.push(match reader.try_read(i)? {
            Var::Arr(it) => format!("{:?}", it.try_collect_vec()?),
            Var::Num(n) => n.to_string(),
            Var::Str(s) => s.to_owned(),
        });
    }
    assert_eq!(repr, ["1", "a", "[1, 2, 3]", "2", "b", "[4, 5, 6]", "3"]);

    Ok(())
}

#[test]
fn derive_variant_reader_override() -> TestResult {
    #[derive(FromVariant)]
    enum Var<'a> {
        #[col(reader = Array<'a, I64<'a>>)]
        Arr(ArrayIter<'a, I64<'a>>),
        #[col(reader = I64<'a>)]
        Num(i64),
        Str(&'a str),
    }

    let buf = load("./testdata/variant.native")?;
    let (_, block) = parse_single(&buf)?;

    let reader: Variant<Var> = Variant::try_from(&block.markers[1])?;
    let mut repr = Vec::with_capacity(block.num_rows);
    for i in 0..block.num_rows {
        repr.push(match reader.try_read(i)? {
            Var::Arr(it) => format!("{:?}", it.try_collect_vec()?),
            Var::Num(n) => n.to_string(),
            Var::Str(s) => s.to_owned(),
        });
    }
    assert_eq!(repr, ["1", "a", "[1, 2, 3]", "2", "b", "[4, 5, 6]", "3"]);

    Ok(())
}

#[test]
fn variant_null_rows_and_arity() -> TestResult {
    use crate::mark::{Mark, Variant as VariantMark};
    use crate::slice::ByteView;

    // No lifetime on purpose: exercises the synthesized-lifetime derive path.
    #[derive(FromVariant)]
    enum JustNum {
        Num(i64),
    }

    // Enum arity must match the number of inner types.
    #[derive(FromVariant)]
    #[expect(dead_code, reason = "construction is expected to fail")]
    enum TooWide<'a> {
        Num(i64),
        Str(&'a str),
    }

    let data = [1i64, 2]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<u8>>();
    // Rows: 1, NULL, 2.
    let mark = Mark::Variant(VariantMark {
        offsets: vec![0, 0, 1].into(),
        discriminators: &[0, VariantMark::NULL_DISCRIMINATOR, 0],
        types: vec![Mark::Int64(ByteView::try_from(data.as_slice())?)].into(),
    });

    let strict: Variant<JustNum> = Variant::try_from(&mark)?;
    let JustNum::Num(n) = strict.try_read(0)?;
    assert_eq!(n, 1);
    assert!(matches!(
        strict.try_read(1),
        Err(Error::MismatchedType("Null", _))
    ));

    let nullable: VariantNullable<JustNum> = VariantNullable::try_from(&mark)?;
    assert!(matches!(nullable.try_read(0)?, Some(JustNum::Num(1))));
    assert!(nullable.try_read(1)?.is_none());
    assert!(matches!(nullable.try_read(2)?, Some(JustNum::Num(2))));
    assert!(matches!(
        nullable.try_read(3),
        Err(Error::IndexOutOfBounds(3, _))
    ));

    assert!(matches!(
        Variant::<TooWide>::try_from(&mark),
        Err(Error::MismatchedType("Variant", _))
    ));

    Ok(())
}

#[test]
fn derive_variant_in_from_block() -> TestResult {
    #[derive(FromVariant)]
    enum Var<'a> {
        Arr(ArrayIter<'a, I64<'a>>),
        Num(i64),
        Str(&'a str),
    }

    #[derive(FromBlock, Copy, Clone)]
    struct Row<'a> {
        id: I64<'a>,
        var: VariantNullable<'a, Var<'a>>,
    }

    let buf = load("./testdata/variant.native")?;
    let (_, block) = parse_single(&buf)?;

    let mut repr = Vec::with_capacity(block.num_rows);
    for row in Row::rows(&block)? {
        let row = row?;
        repr.push(match row.var {
            Some(Var::Arr(it)) => format!("{}: {:?}", row.id, it.try_collect_vec()?),
            Some(Var::Num(n)) => format!("{}: {n}", row.id),
            Some(Var::Str(s)) => format!("{}: {s}", row.id),
            None => format!("{}: null", row.id),
        });
    }
    assert_eq!(
        repr,
        [
            "0: 1",
            "1: a",
            "2: [1, 2, 3]",
            "3: 2",
            "4: b",
            "5: [4, 5, 6]",
            "6: 3"
        ]
    );

    Ok(())
}

#[test]
fn low_cardinality_reader_rejects_readable_indices_without_dictionary() {
    let indices = [1_u8];
    let mark = mark::Mark::LowCardinality(mark::lc::LowCardinality {
        is_nullable: false,
        indices: mark::lc::Indices::U8(&indices),
        global_dictionary: None,
        additional_keys: None,
    });

    assert!(matches!(
        LcStr::try_from(&mark),
        Err(Error::CorruptedData(_))
    ));
}

#[test]
fn nullable_low_cardinality_reader_rejects_readable_indices_without_dictionary() {
    let indices = [0_u8];
    let mark = mark::Mark::LowCardinality(mark::lc::LowCardinality {
        is_nullable: true,
        indices: mark::lc::Indices::U8(&indices),
        global_dictionary: None,
        additional_keys: None,
    });

    assert!(matches!(
        LcNullableStr::try_from(&mark),
        Err(Error::CorruptedData(_))
    ));
}

#[test]
fn string_readers_separate_bytes_checked_and_trusted_utf8() -> TestResult {
    let valid = bstr::BStr::new(b"valid");
    let invalid = bstr::BStr::new(b"\xff");
    let mark = mark::Mark::String(mark::StringView {
        data: vec![valid, invalid].into(),
    });

    assert_eq!(mark.get_str(1)?.expect("valid row"), invalid);

    let nullable_mask = [0_u8];
    let nullable = mark::Mark::Nullable(mark::Nullable {
        mask: &nullable_mask,
        data: Box::new(mark::Mark::String(mark::StringView {
            data: vec![invalid].into(),
        })),
    });
    assert_eq!(nullable.get_opt_str(0)?.expect("valid row"), Some(invalid));

    let lc_indices = [0_u8];
    let low_cardinality = mark::Mark::LowCardinality(mark::lc::LowCardinality {
        is_nullable: false,
        indices: mark::lc::Indices::U8(&lc_indices),
        global_dictionary: None,
        additional_keys: Some(Box::new(mark::Mark::String(mark::StringView {
            data: vec![invalid].into(),
        }))),
    });
    assert_eq!(low_cardinality.get_str(0)?.expect("valid row"), invalid);

    let offsets = 1_u64.to_le_bytes();
    let array = mark::Mark::Array(mark::Array {
        offsets: crate::slice::ByteView::try_from(offsets.as_slice())?,
        values: Box::new(low_cardinality),
    });
    let raw_array = array
        .get_array_lc_strs(0)?
        .expect("valid row")
        .collect::<crate::Result<Vec<_>>>()?;
    assert_eq!(raw_array, [invalid]);

    let bytes = Bytes::try_from(&mark)?;
    assert_eq!(bytes.try_read(0)?, b"valid");
    assert_eq!(bytes.try_read(1)?, b"\xff");

    assert!(matches!(Str::try_from(&mark), Err(Error::Utf8Decode(_, _))));

    let valid_mark = mark::Mark::String(mark::StringView {
        data: vec![valid].into(),
    });
    let checked = Str::try_from(&valid_mark)?;
    assert_eq!(checked.try_read(0)?, "valid");

    // Trusted readers never validate: construction succeeds despite row 1,
    // and reading the valid row is fine.
    let trusted = TrustedStr::try_from(&mark)?;
    assert_eq!(trusted.try_read(0)?, "valid");

    let value = mark.get(1)?.expect("valid row");
    let raw: &bstr::BStr = value.clone().try_into()?;
    assert_eq!(raw, b"\xff");
    let checked_value: crate::Result<&str> = value.try_into();
    assert!(matches!(checked_value, Err(Error::Utf8Decode(_, _))));

    Ok(())
}

#[test]
fn fixed_string_readers_separate_bytes_checked_and_trusted_utf8() -> TestResult {
    let data = [b'o', b'k', 0, 0xff, 0, 0];
    let mark = mark::Mark::FixedString(mark::FixedString {
        size: 3,
        data: &data,
    });

    // Raw readers keep the full record; only `&str` readers trim padding.
    let bytes = FixedBytes::try_from(&mark)?;
    assert_eq!(bytes.try_read(0)?, b"ok\0");
    assert_eq!(bytes.try_read(1)?, b"\xff\0\0");

    assert!(matches!(
        FixedStr::try_from(&mark),
        Err(Error::Utf8Decode(_, _))
    ));

    let valid_data = [b'o', b'k', 0];
    let valid_mark = mark::Mark::FixedString(mark::FixedString {
        size: 3,
        data: &valid_data,
    });
    let checked = FixedStr::try_from(&valid_mark)?;
    assert_eq!(checked.try_read(0)?, "ok");

    // Trusted readers never validate: construction succeeds despite row 1,
    // and reading the valid row is fine.
    let trusted = TrustedFixedStr::try_from(&mark)?;
    assert_eq!(trusted.try_read(0)?, "ok");

    Ok(())
}

#[test]
fn nullable_mask_treats_any_nonzero_byte_as_null() -> TestResult {
    // ClickHouse's `ColumnNullable::isNullAt` is `null_map[n] != 0`; only `0` means present.
    let mask = [0_u8, 1, 2, 0xff];
    let mark = mark::Mark::Nullable(mark::Nullable {
        mask: &mask,
        data: Box::new(mark::Mark::UInt8(crate::slice::ByteView::try_from(
            [10_u8, 11, 12, 13].as_slice(),
        )?)),
    });

    let reader: Nullable<U8> = Nullable::try_from(&mark)?;
    let expected = [Some(10), None, None, None];
    for (index, expected) in expected.into_iter().enumerate() {
        assert_eq!(
            reader.try_read(index)?,
            expected,
            "reader mismatch at index {index}"
        );
        assert_eq!(
            mark.get_opt_u8(index)?.expect("in range"),
            expected,
            "getter mismatch at index {index}"
        );
    }

    Ok(())
}
