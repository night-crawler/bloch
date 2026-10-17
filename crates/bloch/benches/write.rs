use std::{fs, hint::black_box, time::Duration};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[path = "common/sample.rs"]
mod sample;

use bloch::reader::{
    Array, Bool, DateTime, F64, Ipv6, Lc, LcNullable, LcNullableTrustedStr, LcTrustedStr, Nullable,
    TryRead, U32, U64, U128, Uuid,
};
use bloch::writer::Row;
use bloch::{BlockWriter, FromBlock, WriteBlock, parse::block::parse_many};
use clickhouse::rowbinary::{de::deserialize_row_without_validation, ser::serialize_row_binary};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use testresult::TestResult;

use crate::sample::BenchmarkSample;

const ROWS: usize = 1_000_000;

/// `benchmark_sample_lc`: every repetitive column is `LowCardinality`, including the UUID,
/// DateTime, Float64, UInt32, IPv6 scalars and the UInt128/UInt64 frame arrays.
#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct LcCols<'a> {
    project: Lc<'a, Uuid<'a>>,
    kind: LcTrustedStr<'a>,
    timestamp: Lc<'a, DateTime<'a>>,
    count: Lc<'a, F64<'a>>,
    rate_hz: Lc<'a, U32<'a>>,

    host: LcNullableTrustedStr<'a>,
    thread: LcNullableTrustedStr<'a>,
    pod: LcNullableTrustedStr<'a>,
    container: LcNullableTrustedStr<'a>,
    namespace: LcNullableTrustedStr<'a>,
    version: LcNullableTrustedStr<'a>,
    kernel: LcNullableTrustedStr<'a>,

    host_ip: LcNullable<'a, Ipv6<'a>>,

    service: LcNullableTrustedStr<'a>,
    tags: Array<'a, LcTrustedStr<'a>>,
    exe: LcNullableTrustedStr<'a>,

    #[col(name = "frames.kind")]
    frame_kinds: Array<'a, LcTrustedStr<'a>>,

    #[col(name = "frames.symbolized")]
    frame_symbolized: Array<'a, Bool<'a>>,

    #[col(name = "frames.file_id")]
    frame_file_ids: Array<'a, Lc<'a, U128<'a>>>,

    #[col(name = "frames.addr")]
    frame_addrs: Array<'a, Lc<'a, U64<'a>>>,
}

/// `benchmark_sample`: `LowCardinality` on strings only; the UUID/DateTime/Float64/UInt32/IPv6
/// scalars and the frame arrays are plain.
#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct PlainCols<'a> {
    project: Uuid<'a>,
    kind: LcTrustedStr<'a>,
    timestamp: DateTime<'a>,
    count: F64<'a>,
    rate_hz: U32<'a>,

    host: LcNullableTrustedStr<'a>,
    thread: LcNullableTrustedStr<'a>,
    pod: LcNullableTrustedStr<'a>,
    container: LcNullableTrustedStr<'a>,
    namespace: LcNullableTrustedStr<'a>,
    version: LcNullableTrustedStr<'a>,
    kernel: LcNullableTrustedStr<'a>,

    host_ip: Nullable<'a, Ipv6<'a>>,

    service: LcNullableTrustedStr<'a>,
    tags: Array<'a, LcTrustedStr<'a>>,
    exe: LcNullableTrustedStr<'a>,

    #[col(name = "frames.kind")]
    frame_kinds: Array<'a, LcTrustedStr<'a>>,

    #[col(name = "frames.symbolized")]
    frame_symbolized: Array<'a, Bool<'a>>,

    #[col(name = "frames.file_id")]
    frame_file_ids: Array<'a, U128<'a>>,

    #[col(name = "frames.addr")]
    frame_addrs: Array<'a, U64<'a>>,
}

fn rowbinary_rows(mut input: &[u8]) -> TestResult<Vec<BenchmarkSample<'_>>> {
    let mut rows = Vec::new();
    while !input.is_empty() {
        rows.push(deserialize_row_without_validation(&mut input)?);
    }
    Ok(rows)
}

fn native_blocks<'a, R: FromBlock<'a>>(blocks: &'a [bloch::ParsedBlock<'a>]) -> TestResult<Vec<R>> {
    Ok(blocks.iter().map(R::from_block).collect::<Result<_, _>>()?)
}

fn write_rowbinary(rows: &[BenchmarkSample<'_>], out: &mut Vec<u8>) -> TestResult<usize> {
    out.clear();
    for row in rows.iter().cycle().take(ROWS) {
        serialize_row_binary(&mut *out, row)?;
    }
    Ok(out.len())
}

/// One output block per source block; one writer (sized from the first block) and one output
/// buffer for the whole stream. Items hold array iterators, so every row is `try_read` from
/// the parsed block again (dictionary lookups, no allocation).
fn write_native<'a, R>(blocks: &[R], out: &mut Vec<u8>) -> TestResult<usize>
where
    R: TryRead<'a>,
    R::Item: Row<Reader = R>,
{
    let mut writer = BlockWriter::from_reader(&blocks[0]);
    let mut bytes = 0;
    let mut rows = 0;
    for reader in blocks.iter().cycle() {
        if rows == ROWS {
            break;
        }
        for i in 0..reader.len() {
            writer.write(reader.try_read(i)?)?;
        }
        rows += reader.len();
        out.clear();
        writer.encode(out)?;
        bytes += black_box(&out).len();
    }
    Ok(bytes)
}

fn bench_writers(c: &mut Criterion) {
    let testdata = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/testdata/");
    let rb_data = fs::read(format!("{testdata}benchmark_sample.rb")).expect("RowBinary fixture");
    let lc_data = fs::read(format!("{testdata}benchmark_sample_lc.native")).expect("LC fixture");
    let plain_data = fs::read(format!("{testdata}benchmark_sample.native")).expect("plain fixture");

    let rows = rowbinary_rows(&rb_data).expect("RowBinary fixture decodes");
    let lc_parsed = parse_many(&lc_data).expect("LC fixture parses");
    let lc_blocks: Vec<LcCols<'_>> = native_blocks(&lc_parsed).expect("LC fixture decodes");
    let plain_parsed = parse_many(&plain_data).expect("plain fixture parses");
    let plain_blocks: Vec<PlainCols<'_>> =
        native_blocks(&plain_parsed).expect("plain fixture decodes");
    let source_rows: usize = lc_blocks.iter().map(LcCols::len).sum();
    assert_eq!(rows.len(), source_rows, "fixtures hold the same rows");
    assert_eq!(
        plain_blocks.iter().map(PlainCols::len).sum::<usize>(),
        source_rows
    );
    assert_eq!(ROWS % source_rows, 0, "cycle whole fixtures");

    let mut rb_out = Vec::new();
    let mut native_out = Vec::new();
    let rb_bytes = write_rowbinary(&rows, &mut rb_out).unwrap();
    let lc_bytes = write_native(&lc_blocks, &mut native_out).unwrap();
    let plain_bytes = write_native(&plain_blocks, &mut native_out).unwrap();
    eprintln!(
        "{ROWS} rows in {} blocks of {:?} rows: RowBinary {rb_bytes} bytes, Native LC {lc_bytes} bytes, Native plain {plain_bytes} bytes",
        lc_blocks.len() * (ROWS / source_rows),
        lc_blocks.iter().map(LcCols::len).collect::<Vec<_>>(),
    );

    let mut group = c.benchmark_group("write");
    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("rowbinary", |b| {
        b.iter(|| write_rowbinary(black_box(&rows), &mut rb_out).unwrap())
    });
    group.bench_function("native_lc", |b| {
        b.iter(|| write_native(black_box(&lc_blocks), &mut native_out).unwrap())
    });
    group.bench_function("native_plain", |b| {
        b.iter(|| write_native(black_box(&plain_blocks), &mut native_out).unwrap())
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(5))
        .measurement_time(Duration::from_secs(20))
        .sample_size(20);
    targets = bench_writers
}
criterion_main!(benches);
