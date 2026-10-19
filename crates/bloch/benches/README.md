# Fixtures

`testdata/benchmark_sample.{native,rb}` hold the same 100k rows of a simplified profiling
`events` table (one stack sample per row). `testdata/benchmark_sample_lc.native` is the same
rows with `LowCardinality` on the repetitive columns, including UUID, DateTime, Float64,
UInt32, IPv6 and the UInt128/UInt64 frame arrays (`allow_suspicious_low_cardinality_types`).
The read benches use the plain files. The write bench uses only the RowBinary file
to prepare ordinary input values before timing; it does not copy Native dictionaries.

Regenerate all three with:

```bash
clickhouse-local --multiquery --queries-file crates/bloch/benches/testdata/benchmark_sample.sql
```

## Write benchmark

```bash
cargo bench --bench write --features mimalloc
```

