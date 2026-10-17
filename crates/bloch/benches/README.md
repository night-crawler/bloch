# Fixtures

`testdata/benchmark_sample.{native,rb}` hold the same 100k rows of a simplified profiling
`events` table (one stack sample per row). `testdata/benchmark_sample_lc.native` is the same
rows with `LowCardinality` on every repetitive column, including UUID, DateTime, Float64, UInt32,
IPv6 and the UInt128/UInt64 frame arrays (`allow_suspicious_low_cardinality_types`): per 65k-row
block, real data has 1–40 distinct values in those scalars and 0.1–2 % distinct frame elements.
The read benches use the plain files; the write bench reads and writes the LC one.

Regenerate all three with:

```bash
clickhouse-local --multiquery --queries-file crates/bloch/benches/testdata/benchmark_sample.sql
```
