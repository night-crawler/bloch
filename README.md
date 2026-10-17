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

## Known limitations

1. Because header parsing is zero-copy, it doesn't support escaping, so escaped enum values will break the parser.

## Perf

Keep in mind that it still needs the whole block in memory, and, for example, if you use the official `clickhouse-rs`,
you will need to allocate memory for all blocks and only then parse/process it. Since the original `clickhouse-rs`
does not expose the RowBinary reader, I had to hack it to have some sort of apples-to-apples comparison.

- `serde` means `clickhouse-rs` deserializer from a pinned version I need to update some day
- `bloch` means manual read with column sorting
- `bloch_derive` - derive a struct and use the generated `*Item` column to access
- `bloch_derive_direct` read with try_read.

```bash
# LTO, mimalloc, 100k profiling events: LC metadata + ~16-frame stack arrays, see benches/README.md
# AMD Ryzen 9 7940HS
cargo bench --bench refs --features mimalloc
serde                   time:   [37.673 ms 37.774 ms 37.876 ms]
bloch                   time:   [13.020 ms 13.125 ms 13.253 ms]
bloch_derive            time:   [10.185 ms 10.260 ms 10.349 ms]
bloch_derive_direct     time:   [9.9254 ms 10.010 ms 10.112 ms]
```

Writing 1M rows of the same table (`cargo bench --bench write --features mimalloc`), 20 blocks
of ≤65k rows, one output buffer. `native_lc` uses the `LowCardinality` layout a writer of this
data would pick (`benchmark_sample_lc.native`: LC on the UUID, DateTime, Float64, UInt32, IPv6
scalars and the UInt128/UInt64 frame arrays); `native_plain` keeps LC on strings only
(`benchmark_sample.native`). RowBinary cannot express either.

```
1000000 rows: RowBinary 726662250 bytes, Native LC 165456800 bytes, Native plain 531113960 bytes
write/rowbinary         time:   [196.94 ms 198.80 ms 201.04 ms]
write/native_lc         time:   [630.05 ms 642.64 ms 656.89 ms]
write/native_plain      time:   [296.76 ms 303.75 ms 315.12 ms]
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

## Writing blocks

Add `WriteBlock` to a `FromBlock` struct and write the generated `RowItem` values back out as an
uncompressed `Native` block.

```rust
use bloch::parse::block::parse_single;
use bloch::reader::{LcStr, Map};
use bloch::{BlockWriter, FromBlock, WriteBlock};

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct Row<'a> {
    name: LcStr<'a>,
    attrs: Map<'a, LcStr<'a>, LcStr<'a>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read("testdata/activity_hw.native")?;
    let (_, block) = parse_single(&data)?;

    // `BlockWriter::for_reader(&Row::from_block(&block)?)` when a column type carries runtime
    // parameters, e.g. a FixedString length.
    let mut writer = BlockWriter::<RowWriter>::with_capacity(block.num_rows);
    for row in Row::rows(&block)? {
        let mut row = row?;
        row.name = "rewritten";
        writer.write(row)?;
    }
    // `encode` writes into any `io::Write`: a `Vec<u8>`, a `BufWriter<File>`, or a
    // compressing encoder such as `flate2::write::GzEncoder` / `zstd::Encoder`.
    let mut out = std::io::BufWriter::new(std::fs::File::create("rewritten.native")?);
    writer.encode(&mut out)?;
    std::io::Write::flush(&mut out)?;
    Ok(())
}
```

`WriteBlock` generates `RowWriter`, one column writer per field; a field can pick another
writer with `#[col(writer = ...)]`, e.g. `writer::Lc<writer::Str, GxBuildHasher>` to hash
`LowCardinality` dictionaries differently.

It also generates `RowRow`, the same columns as plain values, for callers that have no parsed
block: scalars by value, strings as `&str`, `Nullable` as `Option`, `Array`/`Map` as slices,
`Variant` as `Option<Enum>`. Rows are `Copy` unless a `Variant` payload owns data. Two schemas
written side by side form one block through a tuple: `BlockWriter<(LevelWriter, RowWriter)>`
takes `(LevelRow { .. }, RowRow { .. })`.

```rust
writer.write(RowRow {
    name: "name",
    attrs: &[("k", "v")],
})?;
```

Generic reader, iterator, and writer structs declare their required trait bounds on the type.
Additional capabilities such as default column construction remain conditional on the relevant impl.

`BlockWriter` is parameterized by writer storage, not the borrowed row:
`BlockWriter<RowWriter>` accepts a fresh `RowItem<'_>` on each `write`. Built-in column writers
retain no input borrows, so request-owned strings and source Native blocks can be dropped
after their rows have been written. A writer can live in `Arc<Mutex<BlockWriter<RowWriter>>>`
across requests and encoded blocks. Generic reader schemas use inferred column-writer types.

`LowCardinality` writers encode each value immediately. Their dictionaries own unique
string/byte keys; repeated values use borrowed lookups without allocating another key.
No staging buffer retains input references. Conversion errors are returned by `write`,
which invalidates the block; `encode` discards a failed block and resets the writer.
Successful `encode` also resets dictionaries and retains their table and output-buffer capacities.

Every reader type except `Value` (JSON/Dynamic) has a writer.

More examples:

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