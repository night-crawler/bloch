use std::{fs, hint::black_box, time::Duration};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common;
#[path = "common/sample.rs"]
mod sample;

use bloch::{BlockRow, BlocksIterator, FromBlock as _, parse::block::parse_many};
use chrono::Utc;
use clickhouse::rowbinary::de::deserialize_row_without_validation;
use criterion::{Criterion, criterion_group, criterion_main};
use testresult::TestResult;

use crate::common::BenchmarkCols;
use crate::sample::BenchmarkSample;

#[inline(always)]
fn trusted_str(bytes: &bloch::BStr) -> &str {
    // SAFETY: the benchmark fixture contains UTF-8 string columns.
    unsafe { std::str::from_utf8_unchecked(bytes) }
}

impl<'data, 'iter> TryFrom<BlockRow<'data, 'iter>> for BenchmarkSample<'data> {
    type Error = bloch::error::Error;

    fn try_from(row: BlockRow<'data, 'iter>) -> Result<Self, Self::Error> {
        let i = row.row_index();

        let [
            project,
            kind,
            timestamp,
            count,
            rate_hz,
            host,
            thread,
            pod,
            container,
            namespace,
            version,
            kernel,
            host_ip,
            service,
            tags,
            exe,
            frame_kinds,
            frame_symbolized,
            frame_file_ids,
            frame_addrs,
            ..,
        ] = row.cols()
        else {
            unreachable!()
        };

        let tags = tags
            .get_array_lc_strs(i)?
            .unwrap()
            .map(|value| value.map(trusted_str))
            .collect::<Result<Vec<_>, _>>()?;
        let frame_kinds = frame_kinds
            .get_array_lc_strs(i)?
            .unwrap()
            .map(|value| value.map(trusted_str))
            .collect::<Result<Vec<_>, _>>()?;

        let mut file_ids = Vec::with_capacity(frame_kinds.len());
        let slice: &[zerocopy::little_endian::U128] =
            frame_file_ids.get_arr_uint128_slice(i)?.unwrap();
        file_ids.extend(slice.iter().map(|v| v.get()));

        let mut addrs = Vec::with_capacity(frame_kinds.len());
        let slice: &[zerocopy::little_endian::U64] = frame_addrs.get_arr_uint64_slice(i)?.unwrap();
        addrs.extend(slice.iter().map(|v| v.get()));

        let row = Self {
            project: project.get_uuid(i)?.unwrap(),
            kind: trusted_str(kind.get_str(i)?.unwrap()),
            timestamp: timestamp.get_datetime(i, Utc)?.unwrap(),
            count: count.get_f64(i)?.unwrap(),
            rate_hz: rate_hz.get_u32(i)?.unwrap(),
            host: host.get_opt_str(i)?.unwrap().map(trusted_str),
            thread: thread.get_opt_str(i)?.unwrap().map(trusted_str),
            pod: pod.get_opt_str(i)?.unwrap().map(trusted_str),
            container: container.get_opt_str(i)?.unwrap().map(trusted_str),
            namespace: namespace.get_opt_str(i)?.unwrap().map(trusted_str),
            version: version.get_opt_str(i)?.unwrap().map(trusted_str),
            kernel: kernel.get_opt_str(i)?.unwrap().map(trusted_str),
            host_ip: host_ip.get_opt_ipv6(i)?.unwrap(),
            service: service.get_opt_str(i)?.unwrap().map(trusted_str),
            tags,
            exe: exe.get_opt_str(i)?.unwrap().map(trusted_str),
            frame_kinds,
            frame_symbolized: frame_symbolized.get_arr_bool_iter(i)?.unwrap().collect(),
            frame_file_ids: file_ids,
            frame_addrs: addrs,
        };

        Ok(row)
    }
}

fn ch_rs_read(mut input: &[u8]) -> TestResult {
    while !input.is_empty() {
        let value: BenchmarkSample = deserialize_row_without_validation(&mut input)?;
        black_box(value);
    }
    Ok(())
}

fn native_read(input: &[u8]) -> TestResult<()> {
    let mut blocks = parse_many(input)?;
    let it = BlocksIterator::new_ordered(
        &mut blocks,
        &[
            "project",
            "kind",
            "timestamp",
            "count",
            "rate_hz",
            "host",
            "thread",
            "pod",
            "container",
            "namespace",
            "version",
            "kernel",
            "host_ip",
            "service",
            "tags",
            "exe",
            "frames.kind",
            "frames.symbolized",
            "frames.file_id",
            "frames.addr",
        ],
    )?;

    for row in it {
        let row: BenchmarkSample = row.try_into()?;
        black_box(row);
    }

    Ok(())
}

// Both derived benchmarks parse the same Native input and construct the same
// `BenchmarkSample`. `BenchmarkCols::iter_blocks` calls the generated
// `BenchmarkCols::try_read`, which returns a `BenchmarkColsItem` containing
// every field before the five owned `Vec` conversions allocate.
fn native_derive_read(input: &[u8]) -> TestResult<()> {
    let blocks = parse_many(input)?;

    for row in BenchmarkCols::iter_blocks(&blocks) {
        let row = row?;
        let sample = BenchmarkSample {
            project: row.project,
            kind: row.kind,
            timestamp: row.timestamp.with_timezone(&Utc),
            count: row.count,
            rate_hz: row.rate_hz,
            host: row.host,
            thread: row.thread,
            pod: row.pod,
            container: row.container,
            namespace: row.namespace,
            version: row.version,
            kernel: row.kernel,
            host_ip: row.host_ip,
            service: row.service,
            tags: row.tags.try_collect_vec()?,
            exe: row.exe,
            frame_kinds: row.frame_kinds.try_collect_vec()?,
            frame_symbolized: row
                .frame_symbolized
                .try_as_slice()?
                .iter()
                .map(|&v| v == 1)
                .collect(),
            frame_file_ids: row
                .frame_file_ids
                .try_as_slice()?
                .iter()
                .map(|v| v.get())
                .collect(),
            frame_addrs: row
                .frame_addrs
                .try_as_slice()?
                .iter()
                .map(|v| v.get())
                .collect(),
        };
        black_box(sample);
    }

    Ok(())
}

// This variant bypasses `BlocksRows` and the generated `BenchmarkCols::try_read`,
// but still uses each field reader's `TryRead::try_read`. Rust evaluates the
// `BenchmarkSample` field initializers in order, so array collection and its
// allocations occur between reads of later `BenchmarkCols` fields. It measures
// this interleaved read-and-convert ordering rather than an allocation-free decoder.
fn native_derive_direct_read(input: &[u8]) -> TestResult<()> {
    use bloch::reader::TryRead as _;

    let blocks = parse_many(input)?;

    for block in &blocks {
        let cols = BenchmarkCols::from_block(block)?;
        for i in 0..block.num_rows {
            let sample = BenchmarkSample {
                project: cols.project.try_read(i)?,
                kind: cols.kind.try_read(i)?,
                timestamp: cols.timestamp.try_read(i)?.with_timezone(&Utc),
                count: cols.count.try_read(i)?,
                rate_hz: cols.rate_hz.try_read(i)?,
                host: cols.host.try_read(i)?,
                thread: cols.thread.try_read(i)?,
                pod: cols.pod.try_read(i)?,
                container: cols.container.try_read(i)?,
                namespace: cols.namespace.try_read(i)?,
                version: cols.version.try_read(i)?,
                kernel: cols.kernel.try_read(i)?,
                host_ip: cols.host_ip.try_read(i)?,
                service: cols.service.try_read(i)?,
                tags: cols.tags.try_read(i)?.try_collect_vec()?,
                exe: cols.exe.try_read(i)?,
                frame_kinds: cols.frame_kinds.try_read(i)?.try_collect_vec()?,
                frame_symbolized: cols
                    .frame_symbolized
                    .try_read(i)?
                    .try_as_slice()?
                    .iter()
                    .map(|&v| v == 1)
                    .collect(),
                frame_file_ids: cols
                    .frame_file_ids
                    .try_read(i)?
                    .try_as_slice()?
                    .iter()
                    .map(|v| v.get())
                    .collect(),
                frame_addrs: cols
                    .frame_addrs
                    .try_read(i)?
                    .try_as_slice()?
                    .iter()
                    .map(|v| v.get())
                    .collect(),
            };
            black_box(sample);
        }
    }

    Ok(())
}

fn bench_readers(c: &mut Criterion) {
    let rb_data = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/benches/testdata/benchmark_sample.rb"
    ))
    .expect("missing benches/testdata/benchmark_sample.rb");
    let native_data = fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/benches/testdata/benchmark_sample.native"
    ))
    .expect("missing benches/testdata/benchmark_sample.native");

    c.bench_function("serde", |b| {
        b.iter(|| ch_rs_read(black_box(&rb_data)).unwrap())
    });

    c.bench_function("bloch", |b| {
        b.iter(|| native_read(black_box(&native_data)).unwrap())
    });

    c.bench_function("bloch_derive", |b| {
        b.iter(|| native_derive_read(black_box(&native_data)).unwrap())
    });

    c.bench_function("bloch_derive_direct", |b| {
        b.iter(|| native_derive_direct_read(black_box(&native_data)).unwrap())
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(10))
        .measurement_time(Duration::from_secs(20))
        .sample_size(200);
    targets = bench_readers
}
criterion_main!(benches);
