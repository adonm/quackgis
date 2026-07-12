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

## Evidence levels and execution environments

Profiles use the roadmap's `smoke`, `local`, `reference`, and `external` levels.
Reduced local profiles must execute the same scenario and exact-result oracle as
their reference form. Host processes or one constrained container own performance
budgets; Kind runs are topology/packaging companion evidence and cannot satisfy
RSS, latency, throughput, spill, or scan-byte budgets.

All new profiles use one evidence envelope with source/dirty state, profile ID and
level, native versions/digests, host and cgroup capacity, rows/bytes/files/row
groups, correctness results, measurements, budgets, and status.

Validate any emitted envelope independently:

```sh
mise exec -- just evidence-manifest-check \
  manifest=.tmp/duckdb-current-benchmark/manifest.json
```

Run the same deterministic transport scenario and correctness oracle at reduced
local scale:

```sh
mise exec -- just duckdb-transport-profile \
  level=local rows=1000000 \
  out=.tmp/duckdb-transport-profile/local-r1m.json
```

Reference runs use the identical scenario, require a clean tree and storage
description, and still remain scalar transport evidence rather than M1
streaming-result or M4 selective-scan evidence:

```sh
QUACKGIS_PROFILE_STORAGE='local NVMe model/filesystem' \
mise exec -- just duckdb-transport-profile \
  level=reference rows=10000000 \
  out=.tmp/duckdb-transport-profile/reference-r10m.json
```

## Streaming-result profile

The first E1 profile streams generated BIGINT rows through pgwire, samples process
RSS every two milliseconds, and records idle/peak/delta RSS, time to first row,
completion time, throughput, exact row/sum results, and Arrow batch in-flight high
water. The server and test client share one process, which is stated in the
evidence scope.

```sh
mise exec -- just duckdb-result-stream-profile \
  level=local rows=1000000 \
  out=.tmp/duckdb-result-stream/local-r1m.json
```

The current dirty-tree 1M local run observed one in-flight batch, first row before
completion, and approximately 1.7 MiB RSS growth. This is functional local
evidence; the clean 1M and 10M reference runs under the 128 MiB budget remain the
M1 exit evidence.

## Next profiles

E0 first adds the common evidence envelope and gate-oriented scenario support.
E1 then adds time-to-first-row, bounded RSS/batches, cancellation, COPY,
selective scans, grouped aggregates, bounded spatial joins, fragmented-file
compaction, plans, bytes scanned, spill, and configured-concurrency evidence. The
exact 10M profile must pass twice before introducing 100M.
