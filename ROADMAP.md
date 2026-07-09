# Roadmap

## Goal

**QuackGIS is a PostGIS-compatible, Sedona-powered spatial lakehouse database**:
a single Rust pgwire server that lets platform/application developers ask very
large spatial and columnar questions over shared DuckLake/Parquet data with high
throughput, horizontal read scaling, and a familiar PostGIS client surface.

The ambitious end state is not “PostgreSQL with fewer features.” It is a spatial
data platform where object-store data feels like PostGIS to QGIS, GeoServer,
Martin, GDAL/OGR, psql, and psycopg, while analytical users get DuckDB-style
columnar OLAP over geometry, geography, raster/asset indexes, and high-fidelity
CAD/reality-capture sidecars — without embedding PostgreSQL or DuckDB in the data
plane.

## Success metrics

1. **Primary — scaled spatial lakehouse questions.** Evidence gates prove
   selective spatial predicates, joins/window queries where supported, high-QPS
   parallel readers, parallel ingest writers, object-store scans, compaction, and
   DuckLake snapshot conflict/retry over shared PostgreSQL catalog + S3-compatible
   storage. The roadmap target is city/regional/planetary datasets: millions to
   billions of features in routine gates, and manual stress paths for 10 TB+ /
   trillion-row class tables.
2. **Secondary — spatial OLAP.** Benchmarks/probes prove projection/filter/
   aggregate pushdown, grouped spatial/attribute stats, primitive calculations,
   candidate narrowing, and exact SedonaDB spatial recheck on the narrowed result.
   QPS/latency/bytes-scanned/file-group metrics must be machine-readable and
   trendable across scheduled runs.
3. **Client workflows.** Scripted end-to-end suites, run in CI/scheduled jobs:
   - *QGIS*: connect → browse schemas → add layer → render with paging → identify
     → filter → edit insert/update/delete → save, including keyless layers.
   - *GeoServer*: register PostGIS datastore → publish layer → WMS GetMap → WFS
     GetFeature paged → WFS-T insert/update/delete.
   - *OGR/GDAL*: `ogr2ogr` load into QuackGIS and read back through PostgreSQL
     wire drivers (`PG:` and PostgreSQL-compatible connection strings), including
     multi-layer real OSM/Overture-style imports.
   - *Martin / tile clients*: table discovery and MVT tile generation over spatial
     lakehouse tables, then real copied OSM layers.
4. **PostGIS function conformance.** A curated upstream PostGIS regress subset
   runs regularly; pass-rate and intentionally skipped functions are documented.
5. **Operability.** Production profile has SCRAM/RBAC/TLS, secrets handling,
   backup/restore, failed-writer cleanup, compaction scheduling, observability,
   reproducible images, and documented limits.

## Design posture and lessons learned

1. **PostGIS is the interface, not the architecture.** QuackGIS speaks pgwire and
   enough PostgreSQL catalogs for spatial clients, but the query/data plane is
   DataFusion + SedonaDB + DuckLake/Parquet.
2. **DuckLake is the durable contract.** SQLite/local and PostgreSQL/S3 are both
   first-class profiles. The scaled product path is many stateless QuackGIS pods
   sharing one SQL catalog and object-store prefix.
3. **Exact correctness beats clever pruning.** Spatial layout rewrites are
   additive and deny-by-default: inject hidden bbox filters only for recognized
   safe single-table predicate shapes, ignore quoted/commented SQL, skip unsafe
   top-level `OR`, and always keep exact SedonaDB recheck.
4. **WKB-first is the boring winning path.** Durable/client boundaries use WKB or
   EWKB. Hidden bbox/layout columns are computed once per write batch. GeoArrow
   metadata remains useful for interoperability, not a prerequisite for pruning.
5. **Bulk writes and grouping matter more than micro-optimizing row writes.** COPY
   and transaction-grouped writes produce better layouts and much faster ingest
   than autocommit `INSERT`. Compaction is a core maintenance primitive.
6. **Client traces outrank abstraction.** QGIS, GeoServer, Martin, and OGR traces
   define the compatibility work. Catalog hooks are organized by PostgreSQL
   surface (`pg_class`, `pg_attribute`, `pg_index`, `pg_type`, pgjdbc, OGR), not by
   one-off client branches.
7. **Fork fast; upstream later.** datafusion-postgres, SedonaDB, and
   datafusion-ducklake are tracked through pinned fork branches. Missing
   capabilities are fixed in-fork first; upstreaming is opportunistic and never on
   the release critical path.
8. **Evidence must be cheap before it is large.** Every heavy Kind/manual probe
   should have a cheap static/unit gate and should emit trendable metrics so
   larger scheduled runs are useful rather than anecdotal.

## Current evidence snapshot

The old M0–M5 developer-preview path is implemented and should no longer be
treated as future roadmap work:

- **Runtime and storage.** `quackgis-server` is the Rust pgwire binary; the v0.1
  PostgreSQL/DuckDB/C-extension stack is retired. DuckLake 1.0+ target path is
  wired through `datafusion-ducklake` on the DF54/Arrow58 stack. SQLite/local is
  the default preview profile; PostgreSQL catalog + S3-compatible storage is the
  maintained Alpha lake profile.
- **Writes and transactions.** CTAS, `CREATE TABLE`, `INSERT`, `COPY FROM STDIN`,
  single-table `UPDATE`/`DELETE`, and DML `RETURNING` route through DuckLake.
  Explicit transactions stage one table per connection, publish one snapshot at
  `COMMIT`, discard on `ROLLBACK`, and fail closed on stale replace conflicts.
- **PostGIS surface.** Sentinel geometry/geography OIDs, `geometry_columns`,
  `geography_columns`, `spatial_ref_sys`, `postgis_version()` family, SRID/extent
  helpers, common privilege helpers, MVT helpers, bbox operators, and key catalog
  shims are covered by focused tests/probes. A starter curated PostGIS regress
  subset now emits pass-rate evidence for the claimed function surface in CI and
  a scheduled/manual workflow.
- **Client compatibility.** Maintained Kind probes cover QGIS read/render/filter/
  identify, QGIS edit/save plus compaction-after-edit rowid stability, GDAL/OGR
  load/read with keyless identity, GeoServer WFS/WMS/WFS-T with keyless identity,
  and Martin tile serving. Scheduled/manual compatibility jobs collect artifacts.
- **Spatial layout.** Hidden `_qg_*` bbox/bucket/sort columns, safe bbox pruning,
  exact recheck, LayoutBench `sf0` oracle, local `sf1` evidence, and whole-table
  `CALL quackgis_compact_table(...)` are implemented. Recent hardening ignores
  quoted/commented envelope matches and skips unsafe top-level `OR` rewrites.
- **Alpha lake evidence.** `just kind-alpha-smoke` bundles PostgreSQL/S3 storage,
  multi-pod access, writer conflict/retry, high-QPS selective readers, and OLAP
  fanout gates. QPS/OLAP probes enforce bytes-scanned and file-group budgets, and
  compatibility report artifacts now include `metrics.json` for trends.
- **CI/packaging.** mise-backed CI runs the fast Rust/local gates, host-local
  preview smokes, and static Kind probe validation. Scheduled compatibility and
  storage reports upload run-stamped `metrics.json` artifacts, and artifact
  workflow publishes Linux binaries, GHCR runtime images, and a release-evidence
  manifest on release/main refs.

## Upstream gap ledger

Capabilities QuackGIS needs beyond upstream defaults. **Plan** remains: fork and
ship immediately, upstream later when useful.

| # | Capability | Status | Next decision |
|---|---|---|---|
| G1 | `geometry`/`geography` OID on the wire | Equivalent sentinel OIDs 90001/90002 work for maintained clients; WKB/EWKB payloads remain bytea-identical. | Keep sentinel path until a client requires real PostGIS dynamic-OID parity. |
| G2 | pg_catalog depth for spatial clients | QGIS/OGR/GeoServer/Martin maintained traces are green through native catalog tables plus surface-specific hooks. | Keep moving trace fixtures into surface-focused tests before adding new branches. |
| G3 | SQL cursors and binary cursor payloads | Simple-query/libpq cursors and BINARY cursor bytes work for QGIS/GDAL; OGR cursor shim exists. | Implement general extended-protocol portal suspension only when a pgjdbc trace requires it. |
| G4 | Portal suspension honoring `Execute.max_rows` | GeoServer WFS/WMS/WFS-T smoke passes without general fetch-size portals. | Probe pgjdbc fetch-size on larger WFS pages; fork datafusion-postgres if required. |
| G5 | UPDATE/DELETE on DuckLake tables | Correct single-table full-table rewrite/replace exists, including `RETURNING`. | Replace with native delete-file/partial rewrite once performance gates demand it. |
| G6 | Spec-compliant PostgreSQL catalog + S3 profile | Kind Alpha lake profile proves catalog/object-store wiring, multi-pod readers, writers, QPS, and OLAP. | Move from in-cluster/local S3 stand-in to real external PostgreSQL/S3-compatible services and failure-mode docs. |
| G7 | File/partition pruning from spatial layout | Hidden layout columns, safe rewrites, exact recheck, LayoutBench, QPS/OLAP scan budgets implemented. | Add temporal layout, bucket-local compaction, real-data scale, and cost/plan trend dashboards. |
| G8 | SQL time travel over DuckLake snapshots | Not implemented; programmatic snapshot selection only. | Prioritize for M7 once snapshot retention/ops docs exist. |
| G9 | SedonaDB Rust dependency | Consumed through `adonm/sedona-db@quackgis/df54`; no native GEOS/PROJ/GDAL runtime required. | Rebase at milestone boundaries; keep pure-Rust runtime path. |
| G10 | Multi-statement transactions/rollback | Single-table staged DML transactions work and detect conflicts. | Multi-table atomicity needs stable DuckLake batch-commit API; read-your-writes needs session overlay. |
| G11 | DataFusion version alignment | Current fork stack aligned on DF54/Arrow58. | Rebaseline deliberately; never mix DataFusion majors in one milestone. |
| G12 | Runtime native geometry deps | Closed for QuackGIS binary. Native libs live only in external client/test images. | Keep runtime image native-dependency-free. |
| G13 | Martin/tile compatibility | Martin v1.11.0 real binary E2E and 18/18 upstream table fixtures pass. | Add feature-attribute MVT tags and real OSM layer matrix. |

Fork mechanics: forks are consumed as git branch dependencies. In-tree `vendor/`
is reserved for deep/long-lived divergence. Every fork carries a `DIVERGENCE.md`
entry for each patch and rebases at milestone boundaries.

## Milestone history (baseline now implemented)

| Milestone | Gate | Current state |
|---|---|---|
| M0 — skeleton server | psql spatial query works | Implemented; v0.1 stack retired. |
| M1 — DuckLake storage | round-trip + restart persistence | Implemented for SQLite/local; PostgreSQL/S3 profile exists for Alpha gates. |
| M2 — PostGIS SQL surface | psycopg/Martin/PostGIS metadata | Implemented for maintained compatibility surface. |
| M3 — QGIS read path | browse/render/identify/filter | Implemented in Kind/CI compatibility workflow. |
| M4 — editing + GeoServer | QGIS edit/save, GeoServer WFS/WMS/WFS-T | Implemented for maintained traces; native delete-file DML still future optimization. |
| M5 — spatial layout preview | LayoutBench pruning oracle | Implemented with safe bbox rewrites, exact recheck, compaction, and scan budgets. |
| Alpha evidence loop | PostgreSQL/S3 multi-process probe | Implemented as maintained Kind gates; now needs external-scale hardening before production claims. |

## Forward milestones

### Alpha hardening — scaled lakehouse credibility

Alpha is no longer about proving the shape once; it is about making the evidence
credible for external platform developers.

- Run `kind-alpha-smoke` and compatibility probes on scheduled capacity with
  uploaded logs plus `metrics.json`; trend QPS, p95/p99 latency, bytes scanned,
  file groups, writer conflict/retry, and OLAP candidate counts.
- Add larger/manual QPS and OLAP runs over LayoutBench `sf10+` and real OSM or
  Overture-derived layers; document hardware, object-store profile, row counts,
  and budgets.
- Exercise real external PostgreSQL and S3-compatible services, not only in-cluster
  stand-ins: credential rotation, network hiccups, catalog restart, object-store
  prefix cleanup, failed writer cleanup, and backup/restore.
- Publish an Alpha operations guide: storage profiles, compaction playbook,
  metrics/logs, capacity knobs, safe retry behavior, and known limits.
- Keep SQLite/local parity gates green for deterministic developer correctness.

### M6 — real-data client matrix

- Extend OSM/PostGIS parity from QGIS/OGR into GeoServer and Martin over the same
  copied points, lines, and multipolygons.
- Add Overture/GeoParquet-style datasets with mixed geometry columns, large
  attributes, and realistic invalid/complex geometry distributions.
- Prove keyless identity across QGIS, OGR, GeoServer, and WFS-T edits, including
  `_quackgis_rowid` stability after compaction.
- Add stretch client probes: pg_featureserv, tippecanoe/MVT consumers through
  Martin, GeoPandas/SQLAlchemy/psycopg, and common BI/SQL tools where they only
  require pgwire SQL.

### M7 — native lakehouse maintenance and write performance

- Replace full-table DML rewrites with native DuckLake delete files or partial-file
  rewrites where supported, while preserving current correctness semantics.
- Evolve whole-table compaction into bucket-local/time-local compaction with
  unchanged exact results and measurable file-group/bytes-scanned improvements.
- Add temporal layout columns and safe temporal+spatial pruning for traces that
  include time windows.
- Add snapshot retention, `AS OF`/time-travel SQL, rollback docs, and catalog
  backup/restore integration.
- Add `CALL quackgis_analyze_table(...)` / stats refresh if DataFusion/DuckLake
  planning needs explicit table statistics beyond Parquet metadata.

### M8 — production security, deployment, and operations

- SCRAM/password auth and readonly/readwrite roles are first-class documented
  profiles; TLS is easy to turn on and can be defaulted in production examples.
- Observability: process-local query ids and write-denial log counters exist;
  plan/scan metrics are in probes. Object-store IO, catalog refresh counters,
  writer conflict counters, and metrics export remain hardening.
- Packaging: slim runtime image stays single-binary and native-dependency-free;
  Helm/Kubernetes production chart returns only after Alpha/M6 gates are stable.
- Disaster recovery: catalog backup/restore, object prefix lifecycle, snapshot
  pruning, failed compaction cleanup, and compatibility with DuckLake reference
  readers where possible.

### M9 — advanced spatial analytics and conformance

- Curated PostGIS regress subset runs in CI/scheduled mode with pass-rate
  tracking; explicit unsupported function-list docs still need to grow with the
  upstream-derived subset.
- Spatial joins, window queries, grouped stats, coverage/asset inventory queries,
  and candidate narrowing become benchmarked workloads, not one-off probes.
- Raster, point-cloud, 3D tiles, CAD/BIM, and reality-capture data are represented
  as queryable footprint/index rows plus high-fidelity sidecars and provenance
  metadata; exact heavy-format readers remain out of the hot path until needed.
- Cost-based improvements: better selectivity estimates from layout stats, plan
  budget assertions, and benchmark-driven partition/row-group sizing.

### 1.0 — credible external release

- Public compatibility matrix with supported/probed versions of QGIS, GeoServer,
  GDAL/OGR, Martin, psycopg-style clients, and psql exists; it must keep tracking
  probe version changes.
- Reproducible benchmark report for local, PostgreSQL/S3, and object-store profiles.
- Security/ops docs complete enough for a platform team to run QuackGIS without
  reading source code.
- Upgrade/rebase policy for forked dependencies and data/catalog compatibility
  notes are documented; release-to-release migration guidance must be validated
  against copied catalogs/prefixes as real releases cut.

## Immediate execution queue

1. Use the run-stamped Alpha/compatibility `metrics.json` artifacts plus
   `just metrics-trend` to publish trend dashboards, and attach matching metrics
   artifacts to release evidence records.
2. Run larger QPS/OLAP gates over real object storage and tune budgets by evidence,
   not by one laptop run.
3. Add GeoServer and Martin to the real OSM side-by-side matrix.
4. Extend keyless identity and compaction-after-edit coverage to copied real-data
   layers after the OSM side-by-side matrix grows.
5. Implement bucket-local compaction and compare file groups/bytes scanned before
   and after on fragmented append workloads.
6. Prototype native delete-file/partial rewrite DML in the datafusion-ducklake fork.
7. Add temporal layout only when a trace or benchmark proves the time-window need.
8. Upgrade the implemented password + read/write/read-only profile to SCRAM and
   richer PostgreSQL privilege metadata with broader denied-connection tests.
9. Grow the curated PostGIS regress subset from the claimed starter surface to a
   scheduled upstream-derived pass-rate report with explicit unsupported skips.
10. Convert remaining catalog compatibility branches into surface-focused tests.
11. Turn the Alpha backup/restore, failed-writer cleanup, and catalog-refresh
    runbooks into external-service failure-mode probes.
12. Keep runtime packaging boring with `just runtime-static-check`: one small Rust
    binary, no native GIS libraries.

## Risk register

| Risk | Impact | Mitigation |
|---|---|---|
| datafusion-ducklake remains young; PostgreSQL/S3 behavior may change | Production storage claims slip | Keep correctness gates on SQLite/local, run Alpha gates on PostgreSQL/S3, fork for missing semantics, and document every storage assumption. |
| Object-store IO can dominate selective scans | QPS/latency regressions | Enforce bytes-scanned/file-group budgets, use per-query target partitions for selective scans, trend metrics, and improve compaction/layout before claiming scale. |
| Full-table DML rewrites are correct but expensive | Large edit/update workloads slow | Treat native delete files/partial rewrites as M7, preserve current semantics, and gate by real edit traces. |
| pgwire/catalog compatibility can sprawl | Fragile client-specific branches | Keep trace fixtures, classify by catalog surface, add unit/static gates, and only implement gaps seen in maintained clients. |
| Fork drift vs upstream velocity | Painful rebases, missed fixes | Pin revisions, maintain `DIVERGENCE.md`, rebase at milestones, upstream small patches opportunistically. |
| Spatial pruning bugs could return wrong answers | Data correctness failure | Deny-by-default rewrites, exact SedonaDB recheck, sf0 oracle, quoted/comment/OR guards, and integration tests for every new predicate shape. |
| Production auth/ops lag feature work | External users cannot safely deploy | Make M8 security/ops a milestone, not an afterthought; fail closed on secrets/auth config errors. |

## Retired v0.1 assets

| Asset | Fate |
|---|---|
| DuckDB extension code (`src/` / `sedonadb`) | Retired; SedonaDB used natively. PostGIS SQL rewriting ideas may be salvaged only as compatibility helpers. |
| `vendor/pg_ducklake`, `pg_geometry/` | Deleted from main; history retained. |
| PostgreSQL SQL stubs and bridge tables | Deleted; functions execute in-engine. |
| v0.1 Helm/BuildKit deploy tree | Replaced by current Kind smoke manifests under `deploy/kind/*`; production Helm is deferred until ops gates are stable. |
