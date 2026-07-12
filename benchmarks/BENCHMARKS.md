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

Clean serial reference runs on source `12817bcd` and pinned DuckDB 1.5.4 produced:

| Rows | Idle RSS | RSS delta | First row | Total | In-flight batches |
|---:|---:|---:|---:|---:|---:|
| 1,000,000 | 84.47 MiB | 1.72 MiB | 1.74 ms | 150.28 ms | 1 |
| 10,000,000 | 84.97 MiB | 2.36 MiB | 1.28 ms | 1,494.12 ms | 1 |

Both exact count/sum oracles pass and remain below the +128 MiB reference budget
on the recorded Ryzen 7 7700X/64 GiB Bazzite host. This closes the cardinality
independence, first-row, and measured 1M/10M BIGINT-stream portions of M1.

### Wide variable-width results

The companion wide-result profile returns ordered BIGINT, nullable VARCHAR up to
the configured 256-byte payload, and nullable BLOB up to 128 bytes. It checks every
row, NULL disposition, text value, and binary byte pattern while recording native
Arrow batch count, one-batch high water, first-row latency, throughput, and RSS.

```sh
mise exec -- just duckdb-wide-result-profile \
  level=local rows=100000 \
  out=.tmp/duckdb-wide-result/local-r100k.json
```

The clean 1M-row reference run on source `b240507e` crossed 489 native batches
with one batch in flight, zero batch-limit rejections, exact values and NULLs,
19.17 MiB RSS delta, 19.25 ms to first row, and 799.43 ms total time. This closes
the M1 wide variable-width/native-batch result gate on the recorded reference
host.

## Cancellation profile

The cancellation profile opens a long generated query, observes its first row,
sends a PostgreSQL cancel request, drains to SQLSTATE `57014`, proves that same
session is explicitly quarantined, and verifies a fresh session remains usable.
Each sample uses a fresh session and records request-to-error latency. Reference
evidence requires exactly 100 samples and p95 at or below 500 ms.

```sh
mise exec -- just duckdb-cancellation-profile \
  level=local iterations=25 \
  out=.tmp/duckdb-cancellation/local-n25.json
```

The clean 100-sample serial reference run on source `8b0d1e46` passed with 1.18 ms
p50, 1.51 ms p95, 1.67 ms p99, 1.79 ms maximum, 100 completed native cancel calls,
zero failures, 100 explicit quarantines, and a usable fresh session. This closes
the M1 100-cancel/500 ms p95 gate for sequential long-query cancellation on the
recorded reference host.

## COPY ingest profile

The COPY profile generates the same BIGINT/VARCHAR/WKB rows through direct
streaming ADBC and bounded 60 KiB pgwire text chunks, then verifies exact counts,
ID sums, and WKB bytes in both published tables. It records pgwire RSS,
rows/bytes/batches, publication latency, both throughputs, and the pgwire/direct
throughput ratio.

```sh
mise exec -- just duckdb-copy-profile \
  level=local rows=1000000 \
  out=.tmp/duckdb-copy/local-r1m.json
```

The first dirty-tree 1M-row local run accepted 59.87 MiB of wire data with 64 MiB
RSS delta and a 0.272 pgwire/direct row-throughput ratio, passing the reduced
384 MiB and 0.25 budgets. The clean 10M-row reference gate remains open; it uses
the identical generator, publication path, and oracle with 256 MiB and 0.50
budgets. A first clean 10M attempt measured a 0.200 ratio and correctly failed the
required 0.50 reference budget, so COPY throughput remains an open M2 gate rather
than a performance claim.

## Next profiles

E0 first adds the common evidence envelope and gate-oriented scenario support.
E1 then adds transport overhead and mixed-class concurrency after the completed
profile implementations for result RSS, wide results, cancellation, and COPY.
Later profiles cover selective scans, grouped aggregates, bounded spatial joins, fragmented-file
compaction, plans, bytes scanned, spill, and configured-concurrency evidence. The
exact 10M profile must pass twice before introducing 100M.
