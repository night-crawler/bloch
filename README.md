# bloch (ClickHouse block reader)

A zero-copy-ish parser for the ClickHouse `Native` format. It does not talk to ClickHouse itself, so you need to get the
bytes using some existing client, i.e., `clickhouse-client` a dump, whatever.

## Why this exists

Quite often, perf problems come from strings/interning and all the related topics. When you request a ton of data from
CH and need to handle it, you'll likely be allocating for strings at the very least. But CH already returns
LowCardinality strings, and there's no point in allocating them. Non-owning deserializers can help with that, but they
can't help when you want to iterate over the same data multiple times, because, for example, you can't handle everything
in a single pass.

This crate is an attempt to implement a random access iterator over CH blocks.

## Types

It supposedly supports all CH types, supposedly correctly, including stuff like `Dynamic`,
`JSON`, `LowCardinality`, etc.

Escaped type-header field names, enum labels, and JSON paths are supported. Unescaped
names borrow the input; escaped names are decoded into storage retained by the parsed
block, so readers can borrow either form for the lifetime of the block.

## Known limitations

1. Type-header names must be UTF-8. Invalid UTF-8 in the header, or an escape sequence that
   decodes to invalid UTF-8, is an error rather than a lossy name.
2. Valid `AggregateFunction(...)` state columns and `QBit(...)` declarations are
   recognized at any nesting depth but return `Error::NotImplemented`; these columns
   cannot currently be decoded or skipped.

## Perf

Keep in mind that it still needs the whole block in memory, and, for example, if you use the official `clickhouse-rs`,
you will need to allocate memory for all blocks and only then parse/process it. Since the original `clickhouse-rs`
does not expose the RowBinary reader, I had to hack it to have some sort of apples-to-apples comparison.

- `serde` means `clickhouse-rs` deserializer from a pinned version I need to update some day
- `bloch` means manual read with column sorting
- `bloch_derive` - derive a struct and use the generated `*Item` column to access
- `bloch_derive_direct` read with try_read.

```bash
# LTO, mimalloc, a mixture of LC strings, arrays, and dates, 100k rows
# AMD Ryzen 9 7940HS
cargo bench --bench refs --features mimalloc
serde                   time:   [18.406 ms 18.484 ms 18.562 ms]
bloch                    time:   [7.6797 ms 7.7583 ms 7.8431 ms]
bloch_derive             time:   [8.1552 ms 8.2321 ms 8.3130 ms]
bloch_derive_direct      time:   [7.1309 ms 7.1904 ms 7.2531 ms]
```

## Quick start

Create a table and populate:

```sh
clickhouse-client --host 127.0.0.1 --port 9000 \
    --database qweqwe --user lol --password wut \
    --multiquery "
CREATE TABLE bloch_example
(
    id      UInt32,
    tags    Array(String),
    attrs   Map(String, String),
    payload Variant(Array(Int64), Int64, String)
)
ENGINE = MergeTree
ORDER BY id;

INSERT INTO bloch_example VALUES
    (1, ['fast', 'cpu'], {'region': 'eu', 'host': 'a1'}, 'hello'),
    (2, [], {'region': 'us'}, 42),
    (3, ['gpu'], {}, [1, 2, 3]),
    (4, ['idle'], {'region': 'ap'}, NULL);
"
```

Dump the table in `Native` format:

```sh
clickhouse-client --host 127.0.0.1 --port 9000 \
    --database qweqwe --user lol --password wut \
    --query "SELECT * FROM bloch_example ORDER BY id FORMAT Native" \
    > testdata/example.native
```

Parse it:

```rust
use bloch::{BStr, BlocksIterator, parse::block::parse_many, value::Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read("testdata/example.native")?;

    // Now we know the schema in blocks.
    let mut blocks = parse_many(&data)?;

    // Reorder columns so we know what to expect during parsing.
    let it = BlocksIterator::new_ordered(&mut blocks, &["id", "tags", "attrs", "payload"])?;

    for row in it {
        // Or use proc macro
        let [id, tags, attrs, payload] = row.cols() else {
            return Err("unexpected column count".into());
        };
        let i = row.row_index();

        // Parse u32 using accessor method avoiding fat Value creation.
        // Outer Result: type mismatch / malformed data; Option: index out of range.
        let id = id.get_u32(i)?.expect("valid row index");

        // Extract a str arr
        let tags: &[&BStr] = tags.get(i)?.expect("valid row index").try_into()?;

        // Convenience method for maps
        let mut attrs_vec = vec![];
        if let Some(map) = attrs.get_map::<&str, &str>(i)? {
            for kv in map {
                let (key, value) = kv?;
                attrs_vec.push((key, value));
            }
        }

        // Fat Value TryFrom path for Variant(Array(Int64), Int64, String).
        // Variant is implicitly nullable; a NULL row is Value::Empty.
        let payload = match payload.get(i)?.expect("valid row index") {
            Value::String(s) => format!("string: {s}"),
            Value::Int64(n) => format!("int: {n}"),
            Value::Int64Slice(xs) => {
                let xs = xs.iter().map(|x| x.get()).collect::<Vec<i64>>();
                format!("array: {xs:?}")
            }
            Value::Empty => "null".to_owned(),
            other => format!("unexpected: {other:?}"),
        };

        println!("id={id} tags={tags:?} attrs={attrs_vec:?} payload={payload}");
    }

    Ok(())
}
```

Or, instead of matching on `Value` and destructuring `row.cols()` by hand, derive a reader.

```rust
use bloch::parse::block::parse_many;
use bloch::reader::{Array, ArrayIter, I64, Map, Str, U32, VariantNullable};
use bloch::{FromBlock, FromVariant};

// Same order as in Variant(Array(Int64), Int64, String)
#[derive(FromVariant)]
enum Payload<'a> {
    Array(ArrayIter<'a, I64<'a>>),
    Int(i64),
    Str(&'a str),
}

#[derive(FromBlock, Copy, Clone)]
struct Row<'a> {
    id: U32<'a>,
    tags: Array<'a, Str<'a>>,
    #[col(name = "attrs")]
    attributes: Map<'a, Str<'a>, Str<'a>>,
    // Variant is implicitly nullable; `Variant<'a, Payload<'a>>` errors on NULL rows instead.
    payload: VariantNullable<'a, Payload<'a>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read("testdata/example.native")?;
    let blocks = parse_many(&data)?;

    // Column lookup by name once per block
    // Row::rows(&block) does the same for a single block
    // This example uses generated RowItem (for the `struct Row<'a>` above)
    for row in Row::iter_blocks(&blocks) {
        let row = row?;

        // Arrays, maps, nested / !scalar cols are lazy
        let tags: Vec<&str> = row.tags.try_collect_vec()?;
        let attrs: Vec<(&str, &str)> = row.attributes.collect::<bloch::Result<_>>()?;

        let payload = match row.payload {
            Some(Payload::Str(s)) => format!("string: {s}"),
            Some(Payload::Int(n)) => format!("int: {n}"),
            Some(Payload::Array(xs)) => format!("array: {:?}", xs.try_collect_vec()?),
            None => "null".to_owned(),
        };

        println!("id={} tags={tags:?} attrs={attrs:?} payload={payload}", row.id);
    }

    Ok(())
}
```

Read data somewhat more manually:

```rust
use bloch::parse::block::parse_many;
use bloch::reader::TryRead as _;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read("testdata/example.native")?;
    let blocks = parse_many(&data)?;

    for block in &blocks {
        let cols = Row::from_block(block)?;

        // Backwards, because we can
        for i in (0..block.num_rows).rev() {
            let id = cols.id.try_read(i)?;

            // Not interested in the payload for this one, so it's never touched
            if id == 2 {
                continue;
            }

            let payload = match cols.payload.try_read(i)? {
                Some(Payload::Str(s)) => format!("string: {s}"),
                Some(Payload::Int(n)) => format!("int: {n}"),
                Some(Payload::Array(xs)) => format!("array: {:?}", xs.try_collect_vec()?),
                None => "null".to_owned(),
            };

            // Same row again because why not
            let tag_count = cols.tags.try_read(i)?.count();
            println!("id={id} tag_count={tag_count} payload={payload}");
        }
    }

    Ok(())
}
```

The standalone examples crate keeps both access styles as executable tests:

- [`crates/bloch-examples/tests/procedural`](crates/bloch-examples/tests/procedural) manual parsing using generic `Value` and TryFrom
- [`crates/bloch-examples/tests/derive`](crates/bloch-examples/tests/derive) derive macro

```sh
cargo test -p bloch-examples
```

## ~Slop~ LLM policy

- Every commit I didn't do myself I explicitly prefix with `slop:`
- Around 100% of new added tests are LLM-generated
- Derive examples are 100% LLM generated

Newer types I don't use in production were slopped (JSON/Dynamic). Everything else has been initially written in the
pre-slop era.