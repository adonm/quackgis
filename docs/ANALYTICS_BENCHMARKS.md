# Analytics and benchmark evidence contract

QuackGIS' product claim is not just that GIS clients connect: it must answer
large spatial/lakehouse questions with measurable pruning, columnar OLAP, and
stable client results. This document defines the benchmark evidence contract for
local Alpha, managed-service promotion, M10 regional analytics, and 1.0 release reports.

## Current benchmark base

| Workload | Status | Evidence |
|---|---|---|
| LayoutBench `sf0` oracle | ✅ cheap correctness gate | `just layoutbench-sf0` |
| Local LayoutBench smoke | ✅ temp-server smoke | `just layoutbench-local-smoke` |
| Kind QPS selective readers | ✅ Alpha evidence | `just kind-qps-smoke` |
| Kind grouped OLAP fanout | ✅ Alpha evidence | `just kind-olap-smoke` |
| Local fragmented compaction read improvement | ✅ local evidence | `ducklake_full_compaction_reports_scan_metric_improvement` |
| Metrics budget gate | ✅ cheap artifact gate | `just metrics-budget-check path=.tmp/compatibility` |
| Regional 100M profile/catalog accounting contract | ✅ static contract, provider-call counter, bounded Kind measurement runner, and report parser; execution pending | `just benchmark-profile-check`; `just kind-layoutbench-catalog-measure`; `benchmarks/profiles/layoutbench-regional-r100m-v1.json`; `scripts/layoutbench_catalog_report.py` |
| Tiny real raster/point-cloud inventory | ✅ deterministic local companion; not COG/COPC promotion | `just multimodal-inventory-local`; `tests/data/multimodal/inventory-v1.json` |
| Real-data OSM copy/read | ⚠️ opt-in | `just kind-osm-postgis-parity` |
| External PostgreSQL/S3 scale ladder | ⏳ runbook, execution required | `docs/ALPHA_EXTERNAL_SERVICES.md` |

## Required metrics

Every benchmark artifact should include, where relevant:

- dataset name, row count, geometry type distribution, bbox, and storage profile;
- hardware/service profile, QuackGIS image digest, source SHA, pod/worker count;
- QPS and p50/p95/p99 latency;
- bytes scanned, file groups, row groups, and scan budget;
- candidate rows/groups before exact SedonaDB recheck;
- exact result counts and representative sample rows;
- native DML/compaction counters, file-count deltas, and object-prefix size;
- benchmark profile id, measured catalog provider calls, separately instrumented
  wire/read/write roundtrips if available, catalog refreshes, and matching budgets;
- failure or skip reasons for unsupported join/window/client shapes.

Metrics should land in `metrics.json` when they are useful across runs. Large
query-specific detail can stay in the rendered compatibility/storage report.

## Scale ladder

| Phase | Dataset/profile | Purpose | Promotion condition |
|---|---|---|---|
| A0 | LayoutBench `sf0` oracle, local SQLite/files | correctness and static pruning oracle | required in fast/local checks |
| A1 | `layoutbench-local-r22k8-v1`, compacted/uncompacted variants | catch runner and layout regressions on exactly 22,800 rows | temp-server smoke passes |
| A2 | Kind PostgreSQL/s3s-fs `kind-qps-smoke` + `kind-olap-smoke` | Alpha multi-pod/storage evidence | scheduled artifacts with budgets |
| A3 | explicit `layoutbench-city-r10m-*` then `layoutbench-regional-r100m-v1` | city/regional generated rows and row-group/file/catalog budgets | dashboard published with hardware profile |
| A4 | Monaco/Andorra OSM-derived layers | real geometry/attribute distribution | real-data matrix report across clients |
| A5 | Overture/GeoParquet-style wide layers | wide attributes, mixed geometry columns, OLAP fanout | copied-layer schema parity + OLAP dashboard |
| A6 | External PostgreSQL/S3 exact 100M/1B-row profiles and manual 10TB inventory profile | object-store/catalog behavior under real services | external-service evidence packet |

Do not move a phase from manual to scheduled until it has a deterministic size
budget and a cheap companion gate.

### Regional 100M v1 contract

`benchmarks/profiles/layoutbench-regional-r100m-v1.json` is the machine-readable
definition for the first regional run. It fixes:

- exactly 100,000,000 rows: 47,368,476 aerial frames, 42,105,220 CAD objects,
  and 10,526,304 asset footprints;
- a bounded 400 × 250 km deterministic grid and five-year UTC acquisition range;
- 500,000-row COPY batches (202 batches total), 524,288-row row groups, and no
  compaction during baseline load;
- 240 warm selective public-schema queries with exact-result/count/bounds/file/
  row-group oracles; and
- catalog-provider-call budgets of at most 7 per warm query (1,680 total), zero
  measured-phase refreshes, 13 calls for an isolated cold public query, and 4 for
  a direct internal diagnostic query.

`just benchmark-profile-check` validates all arithmetic, exact-scale naming,
bounded generator coordinates/time, one-row-group batch policy, required oracles,
budget completeness, report-parser failures, and release metadata. PostgreSQL
catalog provider calls are now instrumented and the warm execution path has a
deterministic 7-call accounting oracle, but this remains a **definition-only
contract**: the 100M claim stays open until a run publishes source-SHA/hardware/
storage/object-byte artifacts and exact Kind measurements. Bare `sf1` is rejected
by the QuackGIS pgwire runner; `sf*` terminology remains only where it refers to
the external Apache SpatialBench suite.

### Catalog provider-call accounting

One catalog read provider call is one call to a PostgreSQL/shared
`MetadataProvider` method. `MeteredMetadataProvider` increments
`quackgis_catalog_read_provider_calls_total` once immediately before delegation,
including failed calls. The counter excludes SQLite, metadata writes, pool and
catalog-id creation, pgwire requests, and object-store IO. It is process-local,
cumulative, and reset by process restart. It intentionally does not claim
physical PostgreSQL network roundtrips: SQLx connection acquisition, prepared
statement setup/cache state, and protocol exchanges are below this boundary.
Per-query evidence must scrape the exact server process or pod before and after
the query; do not subtract values from different pods or an aggregate Service
scrape. `quackgis_catalog_refresh_total` remains a separate logical refresh counter.

Warm selective extended execution has the following local call oracle: schema
preflight performs `schema + table + structure` (3), then the rewritten query
builds a snapshot-fresh plan with `schema + table + structure + files` (4), for
7 provider calls. The parse-time plan is not executed because it can retain an
older file set across prepared-statement executions; unrelated bind parameters
are rebound onto the fresh plan, and preflight provider errors fail instead of
falling back to the old plan. A direct internal plan is 4.
The cold-public ceiling of 13 is conservative rather than a locally measured
oracle; measured phases require a zero logical-refresh delta. The fake-provider
unit tests prove per-call/error accounting and schema-only lookup arithmetic;
exact `7/4/<=13` process deltas still must be measured in Kind.

`scripts/layoutbench_catalog_report.py` consumes exactly one
`layoutbench_catalog` line for each `cold_public`, `direct_internal`, and
`warm_public` phase. Each line must provide the profile id, 100M target, 240 warm
queries, all profile-required run metadata, a 40-character source SHA,
`correctness=pass`, numeric run id/attempt, RFC3339 start time, phase query count,
server process/pod identity, raw provider-counter start/end, derived total/max,
and refreshes. The parser requires a non-resetting same-process counter delta,
rejects impossible zero-call phases, and runs the complete benchmark-profile
validator before emitting evidence. Missing, duplicate, malformed, inconsistent,
or over-budget records fail and remove stale output. The output uses only
provider-call metric names and does not alias them to generic or network
roundtrips.

`just kind-layoutbench-catalog-measure` is the bounded Kind integration. It
publishes benchmark profiles to the cluster, scales the PostgreSQL/S3 `lake`
profile to one QuackGIS pod, enables `/metrics`, restarts the pod so counters are
process-local, runs the three measurement phases with
`deploy/kind/probes/layoutbench_catalog_probe.py`, saves
`.tmp/layoutbench-regional/catalog.log`, and writes the validated benchmark
artifact to `.tmp/compatibility/metrics.json`. The paired seed recipe refuses to
load the exact 100M rows unless `LAYOUTBENCH_ALLOW_EXACT_R100M=true` and
`LAYOUTBENCH_MAX_ROWS>=100000000` are set. This completes the runner/scrape
plumbing; the 100M evidence claim stays open until a run publishes source-SHA,
hardware, row/object-byte, metrics, and dashboard artifacts.

The manual `Benchmark ladder` GitHub workflow runs the maintained benchmark
recipes (`layoutbench-local-smoke`, `kind-qps-smoke`, `kind-olap-smoke`,
`kind-qps-deep-smoke`, and `kind-lake-layoutbench-smoke`) and uploads
`benchmark-report-*` plus `benchmark-metrics-*` artifacts. Treat those artifacts
as manual evidence unless/until a recipe is promoted to a scheduled gate. The
preferred near-term scale target is the local Kind+Linkerd envelope documented in
[LOCAL_KIND_LINKERD_FOCUS.md](./LOCAL_KIND_LINKERD_FOCUS.md).

## Query families

| Family | Minimum benchmark shape |
|---|---|
| Selective spatial reads | bbox/operator/predicate windows with exact recheck and bytes/file-group budgets |
| Grouped spatial stats | counts, area/length aggregates, grouped attribute summaries, candidate rows before exact recheck |
| Primitive OLAP | projection, filters, expressions, `count/sum/avg`, NULL-heavy columns, and wide attributes |
| Spatial joins | start with bounded/small generated joins; record unsupported planner shapes explicitly |
| Window queries | add only where DataFusion/Sedona support is stable; record sort/partition budgets |
| Compaction | visible-file, row-group, bytes-scanned, latency, and exact-result deltas before/after compaction |
| Asset inventory | footprint coverage/gap queries, resolution/quality filters, lineage/provenance filters |

## Budget policy

- Budgets are pass/fail gates, not dashboard suggestions.
- A budget can move only with a run-stamped artifact and a doc update explaining
  data size, layout, or accounting changes.
- Regressions must distinguish correctness failure, planner regression, object-
  store latency, and intentional scale increase.
- Trend dashboards augment the gate; they do not replace it.
- `just metrics-budget-check path=<metrics.json-or-dir>` is the cheap release gate
  for existing artifacts: it fails on failed checks and on any emitted
  `*_budget` value exceeded by the matching metric. Use `require_budgeted=true`
  for release candidates so dashboards cannot pass without at least one explicit
  budget assertion.
- Catalog provider-call budgets cover suite total, warm total and per-query
  maximum, cold, direct, and refresh values. A budget without its metric fails
  closed. Do not emit wire/read/write roundtrip values until that lower boundary
  is separately instrumented.

## Report template

Each manual or scheduled benchmark report should include:

```text
Dataset/profile:
Exact profile id / target rows:
Rows / files / object bytes:
Geometry columns and bbox:
Storage profile:
Hardware/service profile:
QuackGIS source SHA and image digest:
Commands run:
Budgets:
Metrics artifact / dashboard:
Result deltas or failures:
Unsupported/skipped query shapes:
Next action:
```

## Completion criteria

A benchmark item is complete only when it has a reproducible command, a metrics
artifact, explicit pass/fail budgets, and docs naming the dataset/profile. A
one-off performance anecdote does not change the roadmap claim.
