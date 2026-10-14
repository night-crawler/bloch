use std::hint::black_box;

use bloch::FromBlock as _;
use testresult::TestResult;

use crate::common::BenchmarkCols;

// Kept out of line so the row loop is one inspectable symbol (`scripts/asm_summary.py`,
// `scripts/perf_profile.py --annotate`); criterion already called it out of line.
#[inline(never)]
pub fn consume_blocks(blocks: &[bloch::ParsedBlock<'_>]) -> TestResult<()> {
    for row in BenchmarkCols::iter_blocks(blocks) {
        let row = row?;

        black_box(row.project);
        black_box(row.kind);
        black_box(row.timestamp);
        black_box(row.count);
        black_box(row.rate_hz);

        black_box(row.host);
        black_box(row.thread);
        black_box(row.pod);
        black_box(row.container);
        black_box(row.namespace);
        black_box(row.version);
        black_box(row.kernel);
        black_box(row.service);
        black_box(row.exe);

        black_box(row.host_ip);

        for s in row.tags {
            black_box(s?);
        }
        for s in row.frame_kinds {
            black_box(s?);
        }
        for &flag in row.frame_symbolized.try_as_slice()? {
            black_box(flag);
        }
        for v in row.frame_file_ids.try_as_slice()? {
            black_box(v.get());
        }
        for v in row.frame_addrs.try_as_slice()? {
            black_box(v.get());
        }
    }

    Ok(())
}
