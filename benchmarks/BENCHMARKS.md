# Benchmarks

## Maintained transport smoke

`duckdb-current-smoke-r100k-v2` compares direct DuckDB CLI, in-process ADBC, and
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
pgwire sample under 10 seconds. The profile warms ADBC and pgwire, interleaves five
samples from each path, and records their p50 ratio. Smoke/local ratios are
diagnostic; reference evidence additionally requires a direct-ADBC p50 of at least
one second and pgwire p50 no more than 15% above direct ADBC. CLI ratios remain
evidence only because process wall time and in-process client wall time differ.

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
description, and enforce the M1 overhead budget only when the direct ADBC scan is
at least one second. They remain scalar transport evidence rather than M1
streaming-result or M4 selective-scan evidence:

```sh
QUACKGIS_PROFILE_STORAGE='local NVMe model/filesystem' \
mise exec -- just duckdb-transport-profile \
  level=reference rows=50000000 \
  out=.tmp/duckdb-transport-profile/reference-r50m.json
```

The clean 50M-row run on source `fc0b6069` used pinned DuckDB 1.5.4 on the
recorded Ryzen 7 7700X/64 GiB/HP FX700 NVMe host. Its exact seven-value oracle
passed on every path; direct ADBC p50 was 1200.05 ms, pgwire p50 was 1198.48 ms,
and the 0.999 ratio passed the 1.15 ceiling. This closes the eligible M1 transport
overhead gate for the reference host; it is not an M4 selective-scan claim.

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
RSS delta and a 0.272 pgwire/direct row-throughput ratio. After replacing
per-field/per-row allocations with contiguous compact batch storage and direct
bounded parsing, the clean 10M reference on source `9e4611ed` accepted
647,777,780 wire bytes in 2,269.23 ms, versus 1,198.89 ms for direct ADBC. Its
0.528 ratio passes the 0.50 floor; 126 MiB RSS delta passes the 256 MiB ceiling,
and the exact count/sum/WKB oracle passes for both tables. Commit publication took
152.83 ms. This closes the M2 10M/RSS/throughput reference gate on the recorded
Ryzen 7 7700X/64 GiB/HP FX700 NVMe host.

## Mixed-class concurrency profile

The mixed-class profile fills a three-operation global limit with two suspended
reader portals and one open COPY. It then observes one reader, writer, and
authorized maintenance call waiting at the same time, releases the holders, and
requires all queued work to complete with class high-water values at their
configured limits and no admission rejection or timeout.

```sh
mise exec -- just duckdb-mixed-concurrency-profile \
  level=local out=.tmp/duckdb-mixed-concurrency/local.json
```

The smoke run is a functional concurrency oracle rather than a throughput claim.
Together with the maintained 32-client/eight-reader native workflow, it closes the
open M1 mixed-class admission evidence slice; write/commit interruption and the
Local 1.0 mixed-workload soak remain separate gates.

## Spatial scan profile

The M4 scan profile creates the same ordered point fixture in 25 official-DuckLake
Parquet files for two layouts: maintained WKB plus four bbox columns, and DuckDB
1.5.4 native `GEOMETRY`. It runs selective and deliberately unpruned exact
oracles, requires every count to agree through pgwire, inspects JSON plans for a
candidate plus the original `ST_Intersects`, and enforces DuckDB's
`OPERATOR_ROW_GROUPS_SCANNED` metrics against both a 5% ceiling and 20x reduction.

```sh
mise exec -- just duckdb-spatial-scan-profile \
  level=local rows=1000000 \
  out=.tmp/duckdb-spatial-scan/local-r1m.json
```

The clean 100k smoke on source `26156bd` uses 4,000 ordered points per file. All
four queries return the same 100 rows. Maintained bbox pruning dispatches and
scans one of 25 row groups; native geometry dispatches all 25 row groups to the
Parquet reader, whose geometry statistics scan one. Both therefore pass at 4%
and 25x. This establishes the plan/row-group oracle but does not claim compressed
bytes or representative scale. Reference mode requires exactly 10M rows and a
clean source with storage metadata; two passing 10M runs are required before a
100M profile is introduced.

## Termination and restart profile

The process-level termination profile starts the actual server binary, commits a
baseline row, leaves a second row uncommitted in an explicit transaction, sends
SIGTERM, and lets the configured 100 ms drain deadline force connection cleanup.
It restarts against the same local DuckLake paths, checks the exact committed
count/sum and zero visibility for the uncommitted row, then commits a fresh write.

```sh
mise exec -- just duckdb-termination-profile \
  level=local out=.tmp/duckdb-termination/local.json
```

The clean smoke run on source `59c1a381` terminated in 122 ms and became queryable
after restart in 135 ms, below the 60-second M5 budget. The reduced fixture does
not establish release-catalog recovery timing, relocated recovery, or general write/commit
interruption behavior.

## Next profiles

E1's result, cancellation, transport, mixed-class, and COPY reference budgets now
pass. The first selective scan/plan/row-group smoke now passes. Later profiles add
compressed bytes, grouped aggregates, bounded spatial joins, fragmented-file
compaction, spill, and configured-concurrency evidence. The exact M4 10M profile
must pass twice before introducing 100M.
