# Analytics and benchmark evidence contract

QuackGIS' product claim is not just that GIS clients connect: it must answer
large spatial/lakehouse questions with measurable pruning, columnar OLAP, and
stable client results. This document defines the benchmark evidence contract for
Alpha hardening, M9 analytics, and 1.0 release reports.

## Current benchmark base

| Workload | Status | Evidence |
|---|---|---|
| LayoutBench `sf0` oracle | ✅ cheap correctness gate | `just layoutbench-sf0` |
| Local LayoutBench smoke | ✅ temp-server smoke | `just layoutbench-local-smoke` |
| Kind QPS selective readers | ✅ Alpha evidence | `just kind-qps-smoke` |
| Kind grouped OLAP fanout | ✅ Alpha evidence | `just kind-olap-smoke` |
| Local fragmented compaction read improvement | ✅ local evidence | `ducklake_full_compaction_reports_scan_metric_improvement` |
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
- failure or skip reasons for unsupported join/window/client shapes.

Metrics should land in `metrics.json` when they are useful across runs. Large
query-specific detail can stay in the rendered compatibility/storage report.

## Scale ladder

| Phase | Dataset/profile | Purpose | Promotion condition |
|---|---|---|---|
| A0 | LayoutBench `sf0`, local SQLite/files | correctness and static pruning oracle | required in fast/local checks |
| A1 | LayoutBench local smoke, compacted/uncompacted variants | catch runner and layout regressions | temp-server smoke passes |
| A2 | Kind PostgreSQL/s3s-fs `kind-qps-smoke` + `kind-olap-smoke` | Alpha multi-pod/storage evidence | scheduled artifacts with budgets |
| A3 | LayoutBench `sf10+` manual | larger generated rows and row-group/file budgets | dashboard published with hardware profile |
| A4 | Monaco/Andorra OSM-derived layers | real geometry/attribute distribution | real-data matrix report across clients |
| A5 | Overture/GeoParquet-style wide layers | wide attributes, mixed geometry columns, OLAP fanout | copied-layer schema parity + OLAP dashboard |
| A6 | External PostgreSQL/S3 regional/manual stress | object-store/catalog behavior under real services | external-service evidence packet |

Do not move a phase from manual to scheduled until it has a deterministic size
budget and a cheap companion gate.

The manual `Benchmark ladder` GitHub workflow runs the maintained benchmark
recipes (`layoutbench-local-smoke`, `kind-qps-smoke`, `kind-olap-smoke`,
`kind-qps-deep-smoke`, and `kind-lake-layoutbench-smoke`) and uploads
`benchmark-report-*` plus `benchmark-metrics-*` artifacts. Treat those artifacts
as manual evidence unless/until a recipe is promoted to a scheduled gate.

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

## Report template

Each manual or scheduled benchmark report should include:

```text
Dataset/profile:
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
