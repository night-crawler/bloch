CREATE TABLE benchmark_sample
(
    project           UUID,
    kind              LowCardinality(String),
    timestamp         DateTime,
    count             Float64,
    rate_hz           UInt32,
    host              LowCardinality(Nullable(String)),
    thread            LowCardinality(Nullable(String)),
    pod               LowCardinality(Nullable(String)),
    container         LowCardinality(Nullable(String)),
    namespace         LowCardinality(Nullable(String)),
    version           LowCardinality(Nullable(String)),
    kernel            LowCardinality(Nullable(String)),
    host_ip           Nullable(IPv6),
    service           LowCardinality(Nullable(String)),
    tags              Array(LowCardinality(String)),
    exe               LowCardinality(Nullable(String)),
    `frames.kind`     Array(LowCardinality(String)),
    `frames.symbolized` Array(Bool),
    `frames.file_id`  Array(UInt128),
    `frames.addr`     Array(UInt64)
)
ENGINE = MergeTree
ORDER BY (project, kind, toStartOfHour(timestamp), arrayZip(`frames.file_id`, `frames.addr`));

INSERT INTO benchmark_sample
WITH
    -- per-row pseudo-random draws
    cityHash64(number, 1) AS r1, cityHash64(number, 2) AS r2, cityHash64(number, 3) AS r3,
    cityHash64(number, 4) AS r4, cityHash64(number, 5) AS r5, cityHash64(number, 6) AS r6,
    cityHash64(number, 7) AS r7, cityHash64(number, 8) AS r8, cityHash64(number, 9) AS r9,
    r1 % 5 AS host_no,
    r2 % 10 = 0 AS no_k8s,
    -- stack depth: 2 + Exp(mean 15) frames; p50 ~12, p90 ~36, max ~170
    toUInt32(2 + 15 * -log(1 - (r3 % 1000000) / 1e6)) AS depth
SELECT
    if(number % 2 = 0, toUUID('5b1d3a2e-0f4c-4d7e-9a6b-1c2d3e4f5a60'), toUUID('9e8d7c6b-5a4f-4e3d-8c2b-1a0f9e8d7c6b')),
    if(r4 % 15 = 0, 'off_cpu', 'on_cpu'),
    toDateTime('2026-10-14 08:00:00', 'UTC') + intDiv(number, 10),
    toFloat64(if(r5 % 25 = 0, 6 + r6 % 300, 1 + r5 % 5)),
    if(r6 % 20 = 0, 19, 20),
    concat('gpu-node-', toString(host_no), '.cluster.internal'),
    concat('worker-', toString(r7 % 400)),
    if(no_k8s, NULL, concat('inference-', hex(cityHash64(r8 % 40)), '-', substring(hex(r8 % 40 + 7), 1, 5))),
    if(no_k8s, NULL, concat('container-', toString(r8 % 15))),
    if(no_k8s, NULL, concat('namespace-', toString(r8 % 12))),
    NULL,
    if(host_no = 0, '6.8.0-45-generic', '6.5.0-1025-aws'),
    toIPv6(concat('fd00::', toString(host_no + 1))),
    NULL,
    if(r9 % 5 = 0, ['env:prod', 'team:infra', 'tier:gpu', 'region:eu'], []),
    if(r9 % 12 = 0, NULL, concat('/usr/local/bin/service-', toString(r9 % 60))),
    arrayMap(i -> ['native', 'native', 'native', 'native', 'native', 'native', 'native', 'native', 'native', 'native', 'native',
                   'kernel', 'kernel', 'kernel', 'kernel', 'python', 'python', 'jvm', 'go', 'idle'][1 + cityHash64(number, i, 1) % 20],
             range(depth)),
    arrayMap(i -> cityHash64(number, i, 2) % 2 = 0, range(depth)),
    -- 30k distinct frames over 5k files: the same (file, addr) pairs recur across stacks
    arrayMap(i -> bitShiftLeft(toUInt128(cityHash64(cityHash64(number, i, 3) % 30000 % 5000)), 64)
                  + toUInt128(cityHash64(cityHash64(number, i, 3) % 30000 % 5000, 1)),
             range(depth)),
    arrayMap(i -> cityHash64(number, i, 3) % 30000 * 16 + 4096, range(depth))
FROM numbers(100000);

-- Without the setting the in-order read emits one block per 8192-row granule; a server
-- response streams max_block_size blocks (65409 + 34591 rows).
SELECT * FROM benchmark_sample ORDER BY project, kind, toStartOfHour(timestamp), arrayZip(`frames.file_id`, `frames.addr`)
INTO OUTFILE 'crates/bloch/benches/testdata/benchmark_sample.native' TRUNCATE FORMAT Native
SETTINGS optimize_read_in_order = 0;

SELECT * FROM benchmark_sample ORDER BY project, kind, toStartOfHour(timestamp), arrayZip(`frames.file_id`, `frames.addr`)
INTO OUTFILE 'crates/bloch/benches/testdata/benchmark_sample.rb' TRUNCATE FORMAT RowBinary
SETTINGS optimize_read_in_order = 0;

-- The same rows with LowCardinality on every repetitive column (real blocks hold 1-40 distinct
-- values in these scalars and 0.1-2% distinct frame elements); the write bench reads this one.
SET allow_suspicious_low_cardinality_types = 1;
SELECT
    project::LowCardinality(UUID) AS project,
    kind,
    timestamp::LowCardinality(DateTime) AS timestamp,
    count::LowCardinality(Float64) AS count,
    rate_hz::LowCardinality(UInt32) AS rate_hz,
    host, thread, pod, container, namespace, version, kernel,
    host_ip::LowCardinality(Nullable(IPv6)) AS host_ip,
    service, tags, exe,
    `frames.kind`,
    `frames.symbolized`,
    `frames.file_id`::Array(LowCardinality(UInt128)) AS `frames.file_id`,
    `frames.addr`::Array(LowCardinality(UInt64)) AS `frames.addr`
FROM benchmark_sample ORDER BY project, kind, toStartOfHour(timestamp), arrayZip(`frames.file_id`, `frames.addr`)
INTO OUTFILE 'crates/bloch/benches/testdata/benchmark_sample_lc.native' TRUNCATE FORMAT Native
SETTINGS optimize_read_in_order = 0;
