//! `bloch` is renamed for testing purposes
use renamed_bloch::reader::{Array, ArrayIter, I64, LcStr, Str, TryRead as _, U8, VariantNullable};
use renamed_bloch::{BlocksIterator, FromBlock as _, FromVariant, ParsedBlock};

#[derive(renamed_bloch::FromVariant, renamed_bloch::IntoVariant)]
enum Payload<'a> {
    #[col(reader = Array<'a, I64<'a>>)]
    Array(ArrayIter<'a, I64<'a>>),
    Integer(i64),
    Text(&'a str),
}

#[derive(renamed_bloch::FromBlock)]
struct GenericRow<'a, T: FromVariant<'a>> {
    id: I64<'a>,
    var: VariantNullable<'a, T>,
}

// T is a produced value, not a reader: neither Copy nor Clone is required.
impl<'a, T: FromVariant<'a>> Copy for GenericRow<'a, T> {}
impl<'a, T: FromVariant<'a>> Clone for GenericRow<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}

#[test]
fn renamed_derives_read_noncopy_payloads_with_manual_copy() -> renamed_bloch::Result<()> {
    let data = include_bytes!("../../bloch/testdata/variant.native");
    let (_, block) = renamed_bloch::parse::block::parse_single(data)?;
    let rows = GenericRow::<Payload<'_>>::rows(&block)?
        .map(|row| {
            let row = row?;
            let value = match row.var {
                Some(Payload::Array(values)) => format!("{:?}", values.try_collect_vec()?),
                Some(Payload::Integer(value)) => value.to_string(),
                Some(Payload::Text(value)) => value.to_owned(),
                None => "null".to_owned(),
            };
            Ok((row.id, value))
        })
        .collect::<renamed_bloch::Result<Vec<_>>>()?;
    assert_eq!(
        rows,
        [
            (0, "1"),
            (1, "a"),
            (2, "[1, 2, 3]"),
            (3, "2"),
            (4, "b"),
            (5, "[4, 5, 6]"),
            (6, "3")
        ]
        .map(|(id, value)| (id, value.to_owned()))
    );
    Ok(())
}

#[derive(renamed_bloch::FromBlock, renamed_bloch::WriteBlock, Copy, Clone)]
struct VariantWriteRow<'a> {
    payload: VariantNullable<'a, Payload<'a>>,
}

#[test]
fn variant_writer_outlives_payloads_and_source_reader() -> renamed_bloch::Result<()> {
    let mut writer = renamed_bloch::BlockWriter::<VariantWriteRowWriter>::new();
    for expected in [
        [Some("first"), None, Some("")],
        [Some("next"), Some("owned"), None],
    ] {
        for text in expected {
            let text = text.map(str::to_owned);
            writer.write(VariantWriteRowItem {
                payload: text.as_deref().map(Payload::Text),
            })?;
            drop(text);
        }
        let mut data = Vec::new();
        writer.encode(&mut data)?;
        let (_, block) = renamed_bloch::parse::block::parse_single(&data)?;
        let values = VariantWriteRow::rows(&block)?
            .map(|row| {
                Ok(match row?.payload {
                    Some(Payload::Text(text)) => Some(text),
                    None => None,
                    _ => panic!("expected only text or NULL"),
                })
            })
            .collect::<renamed_bloch::Result<Vec<_>>>()?;
        assert_eq!(values, expected);

        // The next iteration uses reader-derived configuration after its source block
        // and bytes have gone out of scope, and still borrows only each fresh payload.
        writer = renamed_bloch::BlockWriter::from_reader(&VariantWriteRow::from_block(&block)?);
    }
    Ok(())
}

#[derive(renamed_bloch::FromBlock, Copy, Clone)]
struct RepeatedName<'a> {
    #[col(name = "x")]
    first: U8<'a>,
    #[col(name = "x")]
    again: U8<'a>,
}

#[test]
fn name_lookup_repeats_first_match_but_ordering_consumes_occurrences() -> renamed_bloch::Result<()>
{
    // Exercise both the small in-place and large allocated reorder paths.
    for width in [4, 64] {
        let values = (10..10 + width).collect::<Vec<u8>>();
        let mut names = vec!["tail"; usize::from(width)];
        names[..3].copy_from_slice(&["x", "y", "x"]);
        let mut blocks = [ParsedBlock {
            markers: values
                .chunks_exact(1)
                .map(|bytes| renamed_bloch::mark::Mark::UInt8(bytes.try_into().unwrap()))
                .collect(),
            col_names: names.into(),
            num_rows: 1,
        }];
        let row = RepeatedName::from_block(&blocks[0])?.try_read(0)?;
        assert_eq!((row.first, row.again), (10, 10));

        BlocksIterator::new_ordered(&mut blocks, &["x", "x"])?;
        let actual = blocks[0]
            .markers
            .iter()
            .map(|mark| U8::try_from(mark)?.try_read(0))
            .collect::<renamed_bloch::Result<Vec<_>>>()?;
        let mut expected = values.clone();
        expected.swap(1, 2);
        assert_eq!(actual, expected);
        let row = RepeatedName::from_block(&blocks[0])?.try_read(0)?;
        assert_eq!((row.first, row.again), (10, 10));
    }
    Ok(())
}

const TEXT_COLUMN: &str = "renamed_text";

#[derive(renamed_bloch::FromBlock, renamed_bloch::WriteBlock, Copy, Clone)]
struct GenericWriteRow<'a, R: renamed_bloch::reader::TryRead<'a>>
where
    R::Item: AsRef<str>,
{
    prefix: Str<'a>,
    #[col(name = TEXT_COLUMN)]
    value: R,
}

#[test]
fn renamed_writer_preserves_generics_and_column_name_expressions()
-> Result<(), Box<dyn std::error::Error>> {
    let writer = std::sync::Arc::new(std::sync::Mutex::new(renamed_bloch::BlockWriter::new()));
    for expected in [
        [("first", "shared"), ("second", ""), ("third", "shared")],
        [("next", "fresh"), ("again", "fresh"), ("empty", "")],
    ] {
        for (prefix, value) in expected {
            let prefix = prefix.to_owned();
            let value = value.to_owned();
            writer
                .lock()
                .map_err(|error| error.to_string())?
                .write(GenericWriteRowItem::<LcStr<'_>> {
                    prefix: &prefix,
                    value: &value,
                })?;
            drop(prefix);
            drop(value);
        }
        let mut data = Vec::new();
        writer
            .lock()
            .map_err(|error| error.to_string())?
            .encode(&mut data)?;
        let (rest, block) = renamed_bloch::parse::block::parse_single(&data)?;
        assert!(rest.is_empty());
        assert_eq!(&*block.col_names, &["prefix", TEXT_COLUMN]);
        let rows = GenericWriteRow::<LcStr<'_>>::rows(&block)?
            .map(|row| {
                let row = row?;
                Ok((row.prefix, row.value))
            })
            .collect::<renamed_bloch::Result<Vec<_>>>()?;
        assert_eq!(rows, expected);
    }
    Ok(())
}

#[derive(renamed_bloch::FromBlock, renamed_bloch::WriteBlock, Copy, Clone)]
struct FixedWriteRow<'a> {
    code: renamed_bloch::reader::FixedStr<'a>,
}

#[test]
fn runtime_typed_columns_come_from_explicit_writers_or_the_reader() -> renamed_bloch::Result<()> {
    // `FixedWriteRowWriter` is not `Default`: the length is a runtime parameter only a
    // reader or the caller knows.
    let mut writer =
        renamed_bloch::BlockWriter::<FixedWriteRowWriter>::with_writer(FixedWriteRowWriter {
            code: renamed_bloch::writer::FixedStr { size: 3 }.into(),
        });
    let code = "ab".to_owned();
    writer.write(FixedWriteRowItem { code: &code })?;
    drop(code);
    let mut data = Vec::new();
    writer.encode(&mut data)?;
    let (_, block) = renamed_bloch::parse::block::parse_single(&data)?;
    let reader = FixedWriteRow::from_block(&block)?;
    assert_eq!(reader.try_read(0)?.code, "ab");

    let mut writer = renamed_bloch::BlockWriter::from_reader(&reader);
    drop(block);
    drop(data);
    let code = "xyz".to_owned();
    writer.write(FixedWriteRowItem { code: &code })?;
    drop(code);
    assert!(matches!(
        writer.write(FixedWriteRowItem { code: "long" }),
        Err(renamed_bloch::Error::FixedStringOverflow(4, 3))
    ));
    assert!(matches!(
        writer.write(FixedWriteRowItem { code: "" }),
        Err(renamed_bloch::Error::WriteFailed)
    ));
    assert!(matches!(
        writer.encode(&mut Vec::new()),
        Err(renamed_bloch::Error::WriteFailed)
    ));
    Ok(())
}

#[derive(renamed_bloch::FromBlock, renamed_bloch::WriteBlock, Copy, Clone)]
struct HasherRow<'a> {
    #[col(writer = renamed_bloch::writer::Lc<renamed_bloch::writer::Str, std::hash::RandomState>)]
    tag: LcStr<'a>,
}

#[test]
fn column_writer_can_be_chosen_per_field() -> Result<(), Box<dyn std::error::Error>> {
    // The default writer parameters are lifetime-free, even with a custom hasher.
    let writer: std::sync::Arc<std::sync::Mutex<renamed_bloch::BlockWriter<HasherRowWriter>>> =
        std::sync::Arc::new(std::sync::Mutex::new(
            renamed_bloch::BlockWriter::with_capacity(3),
        ));
    for expected in [["a", "b", "a"], ["", "new", "new"]] {
        let producer = std::sync::Arc::clone(&writer);
        std::thread::spawn(move || -> Result<(), String> {
            for tag in expected {
                let tag = tag.to_owned();
                producer
                    .lock()
                    .map_err(|error| error.to_string())?
                    .write(HasherRowItem { tag: &tag })
                    .map_err(|error| error.to_string())?;
                drop(tag);
            }
            Ok(())
        })
        .join()
        .map_err(|_| "producer thread panicked")??;
        let mut data = Vec::new();
        writer
            .lock()
            .map_err(|error| error.to_string())?
            .encode(&mut data)?;
        let (_, block) = renamed_bloch::parse::block::parse_single(&data)?;
        let tags = HasherRow::rows(&block)?
            .map(|row| row.map(|row| row.tag))
            .collect::<renamed_bloch::Result<Vec<_>>>()?;
        assert_eq!(tags, expected);
    }
    Ok(())
}

#[derive(renamed_bloch::FromBlock, renamed_bloch::WriteBlock, Copy, Clone)]
struct Sample<'a> {
    id: renamed_bloch::reader::Lc<'a, I64<'a>>,
    name: renamed_bloch::reader::LcNullableStr<'a>,
    at: renamed_bloch::reader::DateTime<'a>,
    tags: Array<'a, LcStr<'a>>,
    nested: Array<'a, Array<'a, U8<'a>>>,
    payload: VariantNullable<'a, Payload<'a>>,
}

#[derive(renamed_bloch::FromBlock, renamed_bloch::WriteBlock, Copy, Clone)]
struct Level<'a> {
    level: U8<'a>,
}

#[test]
fn row_values_and_slices_round_trip() -> renamed_bloch::Result<()> {
    let at = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let mut writer = renamed_bloch::BlockWriter::<(LevelWriter, SampleWriter)>::new();
    let name = "owned".to_owned();
    writer.write((
        LevelRow { level: 1 },
        SampleRow {
            id: 7,
            name: Some(&name),
            at,
            tags: &["x", "", "x"],
            nested: &[&[1, 2][..], &[]],
            payload: Some(Payload::Integer(3)),
        },
    ))?;
    writer.write((
        LevelRow { level: 2 },
        SampleRow {
            id: 7,
            name: None,
            at,
            tags: &[],
            nested: &[],
            payload: None,
        },
    ))?;
    drop(name);

    let mut data = Vec::new();
    writer.encode(&mut data)?;
    let (_, block) = renamed_bloch::parse::block::parse_single(&data)?;
    assert_eq!(
        &*block.col_names,
        &["level", "id", "name", "at", "tags", "nested", "payload"]
    );
    let levels = Level::rows(&block)?
        .map(|row| row.map(|row| row.level))
        .collect::<renamed_bloch::Result<Vec<_>>>()?;
    assert_eq!(levels, [1, 2]);
    let mut rows = Sample::rows(&block)?;
    let first = rows.next().unwrap()?;
    assert_eq!(first.id, 7);
    assert_eq!(first.name, Some("owned"));
    assert_eq!(first.at.timestamp(), 1_700_000_000);
    assert_eq!(first.tags.try_collect_vec()?, ["x", "", "x"]);
    let nested = first
        .nested
        .map(|inner| inner?.try_collect_vec())
        .collect::<renamed_bloch::Result<Vec<_>>>()?;
    assert_eq!(nested, [vec![1, 2], vec![]]);
    assert!(matches!(first.payload, Some(Payload::Integer(3))));
    let second = rows.next().unwrap()?;
    assert_eq!(second.name, None);
    assert_eq!(second.tags.count(), 0);
    assert!(second.payload.is_none());
    assert!(rows.next().is_none());
    Ok(())
}
