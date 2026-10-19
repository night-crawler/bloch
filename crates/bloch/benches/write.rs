use std::{fs, hint::black_box, time::Duration};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[path = "common/sample.rs"]
mod sample;

use bloch::reader::{
    Array, Bool, DateTime, F64, Ipv6, Lc, LcNullable, LcNullableTrustedStr, LcTrustedStr, Nullable,
    U32, U64, U128, Uuid,
};
use bloch::writer::Row;
use bloch::{BlockWriter, FromBlock, WriteBlock, parse::block::parse_many};
use clickhouse::rowbinary::{de::deserialize_row_without_validation, ser::serialize_row_binary};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use testresult::TestResult;

use crate::sample::BenchmarkSample;

const ROWS: usize = 1_000_000;
/// Rows per batch: each batch gets a fresh unsized writer and output buffer.
const BATCH_ROWS: usize = 300_000;

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

// Both layouts accept the same ordinary values. Arrays borrow materialized Vecs;
// there are no Native readers, dictionary lookups, or array allocations on input.
macro_rules! value_rows {
    ($($row:ident),+ $(,)?) => {$(
        impl<'a> From<&'a BenchmarkSample<'_>> for $row<'a> {
            fn from(value: &'a BenchmarkSample<'_>) -> Self {
                Self {
                    project: value.project,
                    kind: value.kind,
                    timestamp: value.timestamp,
                    count: value.count,
                    rate_hz: value.rate_hz,
                    host: value.host,
                    thread: value.thread,
                    pod: value.pod,
                    container: value.container,
                    namespace: value.namespace,
                    version: value.version,
                    kernel: value.kernel,
                    host_ip: value.host_ip,
                    service: value.service,
                    tags: &value.tags,
                    exe: value.exe,
                    frame_kinds: &value.frame_kinds,
                    frame_symbolized: &value.frame_symbolized,
                    frame_file_ids: &value.frame_file_ids,
                    frame_addrs: &value.frame_addrs,
                }
            }
        }
    )+};
}
value_rows!(LcColsRow, PlainColsRow);

fn write_rowbinary(rows: &[BenchmarkSample<'_>]) -> TestResult<usize> {
    let mut input = rows.iter().cycle().take(ROWS);
    let mut bytes = 0;
    for _ in 0..ROWS.div_ceil(BATCH_ROWS) {
        let mut out = Vec::new();
        for row in input.by_ref().take(BATCH_ROWS) {
            serialize_row_binary(&mut out, row)?;
        }
        bytes += black_box(&out).len();
    }
    Ok(bytes)
}

/// Fresh unsized writer and output allocation per batch, as in the ingest cache's
/// mem::take/encode/drop lifecycle. No source-derived capacity hints or buffer reuse.
fn write_native<R>(rows: &[R], mut observe: impl FnMut(&[u8])) -> TestResult<usize>
where
    R: Row + Copy,
    R::Writer: Default,
{
    let mut input = rows.iter().copied().cycle().take(ROWS);
    let mut bytes = 0;
    for _ in 0..ROWS.div_ceil(BATCH_ROWS) {
        let mut writer = BlockWriter::<R::Writer>::new();
        for row in input.by_ref().take(BATCH_ROWS) {
            writer.write(row)?;
        }
        let mut out = Vec::new();
        writer.encode(&mut out)?;
        bytes += out.len();
        observe(&out);
    }
    Ok(bytes)
}

fn bench_writers(c: &mut Criterion) {
    let testdata = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/testdata/");
    let rb_data = fs::read(format!("{testdata}benchmark_sample.rb")).expect("RowBinary fixture");
    let rows = rowbinary_rows(&rb_data).expect("RowBinary fixture decodes");
    assert!(!rows.is_empty(), "fixture must contain rows");
    let lc_rows: Vec<_> = rows.iter().map(LcColsRow::from).collect();
    let plain_rows: Vec<_> = rows.iter().map(PlainColsRow::from).collect();

    // Untimed preflight: check every emitted value, including the final partial
    // batch, against the ordinary input rows. Only output is decoded here.
    macro_rules! scalar {
        ($actual:ident, $expected:ident, $($field:ident),+ $(,)?) => {$(
            assert_eq!($actual.$field, $expected.$field, stringify!($field));
        )+};
    }
    macro_rules! array {
        ($actual:ident, $expected:ident, $($field:ident),+ $(,)?) => {$(
            assert_eq!(
                $actual.$field.try_collect_vec().unwrap(),
                $expected.$field,
                stringify!($field),
            );
        )+};
    }
    macro_rules! validate_native {
        ($reader:ident, $values:ident) => {{
            let mut expected = rows.iter().cycle();
            let mut checked = 0;
            let bytes = write_native(&$values, |out| {
                for block in parse_many(out).expect("encoded block parses") {
                    for actual in $reader::rows(&block).expect("encoded columns decode") {
                        let actual = actual.expect("encoded row decodes");
                        let expected = expected.next().unwrap();
                        scalar!(
                            actual, expected, project, kind, rate_hz, host, thread, pod, container,
                            namespace, version, kernel, host_ip, service, exe,
                        );
                        assert_eq!(actual.timestamp.to_utc(), expected.timestamp);
                        assert_eq!(actual.count.to_bits(), expected.count.to_bits());
                        array!(
                            actual,
                            expected,
                            tags,
                            frame_kinds,
                            frame_symbolized,
                            frame_file_ids,
                            frame_addrs,
                        );
                        checked += 1;
                    }
                }
            })
            .unwrap();
            assert_eq!(checked, ROWS);
            bytes
        }};
    }
    let rb_bytes = write_rowbinary(&rows).unwrap();
    let lc_bytes = validate_native!(LcCols, lc_rows);
    let plain_bytes = validate_native!(PlainCols, plain_rows);
    eprintln!(
        "{ROWS} slice-backed rows in {} fresh batches (max {BATCH_ROWS} rows): RowBinary {rb_bytes} bytes, Native LC {lc_bytes} bytes, Native plain {plain_bytes} bytes",
        ROWS.div_ceil(BATCH_ROWS),
    );

    let mut group = c.benchmark_group("write");
    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("rowbinary", |b| {
        b.iter(|| write_rowbinary(black_box(&rows)).unwrap())
    });
    group.bench_function("native_lc", |b| {
        b.iter(|| {
            write_native(black_box(&lc_rows), |out| {
                black_box(out);
            })
            .unwrap()
        })
    });
    group.bench_function("native_plain", |b| {
        b.iter(|| {
            write_native(black_box(&plain_rows), |out| {
                black_box(out);
            })
            .unwrap()
        })
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
