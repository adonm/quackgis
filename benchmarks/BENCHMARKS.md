# Benchmarks

## Maintained transport smoke

`duckdb-current-smoke-r100k-v1` compares direct DuckDB CLI, in-process ADBC, and
pgwire over the same deterministic official-DuckLake table:

```sh
mise run duckdb-bootstrap
mise exec -- just duckdb-current-benchmark
```

The profile creates 100,000 deterministic WKB points and runs one scalar full-scan
query three times per path. It asserts row count, id sum, group count, bbox count,
exact `ST_Intersects` count, text bytes, WKB bytes, and bbox/exact equality. The
manifest is written to `.tmp/duckdb-current-benchmark/manifest.json` and uploaded
by required CI.

Budgets are deliberately broad liveness ceilings: load under 30 seconds,
handshake under 5 seconds, each direct sample under 15 seconds, and each ADBC/
pgwire sample under 10 seconds. Ratios are evidence only because CLI process wall
time and in-process client wall time have different scopes.

This proves deterministic current-path comparison, not streaming, concurrency,
memory, spill, selective pruning, COPY throughput, or scale.

## Next profiles

M1–M4 must add time-to-first-row, bounded RSS/batches, cancellation, COPY,
selective scans, grouped aggregates, bounded spatial joins, fragmented-file
compaction, plans, bytes scanned, spill, and configured-concurrency evidence. The
exact 10M profile must pass twice before introducing 100M.
