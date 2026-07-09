# Roadmap

See [docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md) for the concise split
between locally closed contracts and execution-heavy remaining work.

## Full project goal

**QuackGIS is the PostGIS-compatible front door to a spatial lakehouse**: one
Rust pgwire service that lets platform teams keep very large spatial data in an
open DuckLake/Parquet lake while serving familiar PostgreSQL/PostGIS clients and
running high-throughput analytical SQL through DataFusion + SedonaDB.

The ambitious end state is not “a smaller PostgreSQL.” It is an operational data
platform where object-store data feels like PostGIS to QGIS, GeoServer, Martin,
GDAL/OGR, psql, psycopg, SQLAlchemy/GeoPandas, and BI tools, while analytical
users get DuckDB-style OLAP over geometry, geography, temporal/spatial layouts,
raster/asset footprints, point-cloud/CAD/reality-capture indexes, and provenance
sidecars — without PostgreSQL or DuckDB owning the query/data plane.

The 1.0+ bar is a credible city/regional spatial lakehouse: tens of millions to
billions of features in routine gates, multi-terabyte object-store prefixes in
manual/scheduled stress paths, many stateless readers, parallel writers,
recoverable operations, and enough PostGIS behavior that common GIS clients do
not need to know they are talking to a lakehouse.

## North-star success metrics

1. **Scaled open-lakehouse execution.** Evidence gates prove selective spatial
   predicates, spatial/temporal pruning, joins/window queries where supported,
   high-QPS parallel readers, parallel ingest writers, compaction, and DuckLake
   snapshot conflict/retry over shared PostgreSQL catalog + S3-compatible storage.
   Routine scheduled gates should reach millions to billions of features; manual
   stress paths should exercise 10 TB+ and eventually trillion-row-class tables.
2. **Spatial OLAP and candidate narrowing.** Benchmarks/probes prove projection,
   filter, aggregate, grouping, primitive calculation, and join pushdown where
   available, with exact SedonaDB spatial recheck after pruning. QPS, p95/p99,
   bytes scanned, file groups, row groups, candidate rows, and mutation counters
   are machine-readable and trendable.
3. **Client workflows, not toy SQL.** Scripted end-to-end suites run in CI or
   scheduled jobs:
   - *QGIS*: connect → browse schemas → add layer → render with paging → identify
     → filter → edit insert/update/delete → save, including keyless layers.
   - *GeoServer*: register PostGIS datastore → publish layer → WMS GetMap → WFS
     GetFeature paged → WFS-T insert/update/delete.
   - *OGR/GDAL*: `ogr2ogr` load into QuackGIS and read back through PostgreSQL
     wire drivers, including multi-layer real OSM/Overture-style imports.
   - *Martin / tile clients*: table discovery, MVT tile generation, and feature
     attributes over copied real-world layers.
   - *Python/SQL ecosystem*: psycopg, SQLAlchemy/GeoPandas, pg_featureserv-style
     API servers, and BI tools that only require pgwire SQL.
4. **Broad but honest PostGIS conformance.** A curated upstream-derived regress
   subset grows beyond the starter surface, reports pass/skip/fail rates, and
   documents unsupported functions by reason (out of scope, missing Sedona kernel,
   unsafe semantics, or future work).
5. **Production operability.** Production profiles have SCRAM/RBAC/TLS, secret
   rotation guidance, backup/restore, failed-writer cleanup, snapshot retention,
   compaction scheduling, catalog/object-store health metrics, reproducible
   images, upgrade/migration notes, a DuckLake alignment ledger, and documented
   limits.

## Design posture and lessons learned

1. **PostGIS is the interface, not the architecture.** QuackGIS speaks pgwire and
   enough PostgreSQL catalogs for spatial clients, but the query/data plane is
   DataFusion + SedonaDB + DuckLake/Parquet.
2. **DuckLake is the durable contract.** SQLite/local and PostgreSQL/S3 are both
   first-class profiles. The scaled product path is many stateless QuackGIS pods
   sharing one SQL catalog and object-store prefix.
3. **Metadata is part of the product.** `pg_catalog`, `geometry_columns`,
   DuckLake metadata UDTFs, metrics artifacts, and compatibility reports are not
   secondary surfaces; they are how users and operators trust a lakehouse that is
   pretending to be PostGIS.
4. **Exact correctness beats clever pruning.** Spatial layout rewrites are
   additive and deny-by-default: inject hidden bbox filters only for recognized
   safe single-table predicate shapes, ignore quoted/commented SQL, skip unsafe
   top-level `OR`, and always keep exact SedonaDB recheck.
5. **WKB-first is the boring winning path.** Durable/client boundaries use WKB or
   EWKB. Hidden bbox/layout columns are computed once per write batch. GeoArrow
   metadata remains useful for interoperability, not a prerequisite for pruning.
6. **Bulk writes and grouping matter more than micro-optimizing row writes.** COPY
   and transaction-grouped writes produce better layouts and much faster ingest
   than autocommit `INSERT`. Compaction is a core maintenance primitive.
7. **Native mutation paths need one visible snapshot boundary.** Positional
   deletes, replacement rows, and compaction metadata must publish atomically.
   Prewritten objects may become cleanup work, but catalog visibility must never
   expose half a DML statement.
8. **Client traces outrank abstraction.** QGIS, GeoServer, Martin, and OGR traces
   define the compatibility work. Catalog hooks are organized by PostgreSQL
   surface (`pg_class`, `pg_attribute`, `pg_index`, `pg_type`, pgjdbc, OGR), not by
   one-off client branches.
9. **Fork fast; upstream later.** datafusion-postgres, SedonaDB, and
   datafusion-ducklake are tracked through pinned fork branches. Missing
   capabilities are fixed in-fork first; upstreaming is opportunistic and never on
   the release critical path.
10. **Evidence must be cheap before it is large.** Every heavy Kind/manual probe
   should have a cheap static/unit gate and should emit trendable metrics so
   larger scheduled runs are useful rather than anecdotal.
11. **External-service claims require external-service evidence.** Kind/local
    profiles are repeatable correctness and smoke evidence. Production durability,
    backup/restore, credential rotation, and object-store behavior are not claimed
    until they run against real platform services.
12. **Stay aligned with DuckLake upstream.** QuackGIS may fork to ship missing
    storage semantics, but the preferred long-term shape is official DuckLake
    behavior: deletion-vector/Puffin evolution, VARIANT/UDT/type support,
    protected snapshots, branch/merge, materialized views, Bloom/metadata scan
    performance, and PostgreSQL catalog roundtrip reductions should replace
    QuackGIS-only workarounds when they become stable.

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
  shims are covered by focused tests/probes. The curated PostGIS regress subset
  now covers the claimed starter surface, emits pass-rate evidence in CI and a
  scheduled/manual workflow, and has an explicit conformance/unsupported ledger in
  `docs/POSTGIS_CONFORMANCE.md`.
- **Client compatibility.** Maintained Kind probes cover QGIS read/render/filter/
  identify, QGIS edit/save plus compaction-after-edit rowid stability, GDAL/OGR
  load/read with keyless identity, GeoServer WFS/WMS/WFS-T with keyless identity,
  and Martin tile serving. Scheduled/manual compatibility jobs collect artifacts.
- **Spatial layout.** Hidden `_qg_*` bbox/bucket/sort columns, safe bbox pruning,
  simple temporal `BETWEEN` bucket prefilters, exact recheck, LayoutBench `sf0`
  oracle, local `sf1` evidence, whole-table compaction, and native bucket-local
  delete+append compaction are implemented. Local compaction tests now report
  visible-file, scan-byte, and row-group deltas for fragmented append workloads.
  Pruning ignores quoted/commented envelope matches and skips unsafe top-level
  `OR` rewrites.
- **Multi-modal asset path.** `docs/MULTIMODAL_ASSETS.md` defines the current
  footprint/sidecar table pattern for raster, point-cloud, 3D tile, CAD/BIM,
  aerial, and reality-capture assets. The cheap LayoutBench `sf0` oracle already
  validates representative aerial/CAD/asset/control-point schemas against hidden
  bbox sidecars and exact SedonaDB recheck.
- **Alpha lake evidence.** `just kind-alpha-smoke` bundles PostgreSQL/S3 storage,
  multi-pod access, writer conflict/retry, native DML/compaction metadata, high-
  QPS selective readers, and OLAP fanout gates. QPS/OLAP probes enforce bytes-
  scanned and file-group budgets, report artifacts include `metrics.json`, and
  `just metrics-dashboard` renders release-ready Markdown trend summaries.
- **Security/ops preview.** Password mode negotiates SCRAM-SHA-256, configured
  read/write and read-only users are reflected in `pg_roles` plus explicit-user
  privilege helpers, read-only write shapes fail closed, and local backup/restore
  plus DuckLake metadata-table-function oracles cover cheap operations checks.
  An opt-in Prometheus `/metrics` endpoint exposes safe process counters and is
  scraped by the external-profile Kind probe.
- **CI/packaging.** mise-backed CI runs the fast Rust/local gates, host-local
  preview smokes, static Kind probe validation, and static validation for the
  production-style Kubernetes example. Scheduled compatibility and storage
  reports upload run-stamped `metrics.json` artifacts, and artifact workflow
  publishes Linux binaries, GHCR runtime images, and a release-evidence manifest
  on release/main refs. `docs/RELEASE_EVIDENCE.md` defines the release packet and
  dashboard attachment policy.
- **DuckLake alignment.** `docs/DUCKLAKE_ALIGNMENT.md` maps fork-backed and
  upstream-sensitive storage behavior to upstream DuckLake direction, interop
  gates, and migration triggers.

## Upstream gap ledger

Capabilities QuackGIS needs beyond upstream defaults. **Plan** remains: fork and
ship immediately, upstream later when useful.

| # | Capability | Status | Next decision |
|---|---|---|---|
| G1 | `geometry`/`geography` OID on the wire | Equivalent sentinel OIDs 90001/90002 work for maintained clients; WKB/EWKB payloads remain bytea-identical. | Keep sentinel path until a client requires real PostGIS dynamic-OID parity. |
| G2 | pg_catalog depth for spatial clients | QGIS/OGR/GeoServer/Martin maintained traces are green through native catalog tables plus surface-specific hooks. | Keep moving trace fixtures into surface-focused tests before adding new branches. |
| G3 | SQL cursors and binary cursor payloads | Simple-query/libpq cursors and BINARY cursor bytes work for QGIS/GDAL; OGR cursor shim exists. | Implement general extended-protocol portal suspension only when a pgjdbc trace requires it. |
| G4 | Portal suspension honoring `Execute.max_rows` | GeoServer WFS/WMS/WFS-T smoke passes without general fetch-size portals. | Probe pgjdbc fetch-size on larger WFS pages; fork datafusion-postgres if required. |
| G5 | UPDATE/DELETE on DuckLake tables | Autocommit `DELETE` and `UPDATE` use fork-backed atomic positional delete files; `UPDATE` stages replacement rows as pending data files and commits delete+append metadata under one snapshot. Bucket-scoped compaction also uses delete+pending-append metadata under one snapshot when row-lineage planning succeeds. SQLite/local and external PostgreSQL/S3 probes cover native delete/update/compaction metadata. Explicit transactions and fallback paths keep correct staged/full-table rewrites including `RETURNING`. Details in `docs/NATIVE_DML_FORK_PLAN.md`. | Add crash/retry probes around mutation commit boundaries and extend transaction batching only from edit traces. |
| G6 | Spec-compliant PostgreSQL catalog + S3 profile | Kind Alpha lake profile proves catalog/object-store wiring, multi-pod readers, writers, QPS, OLAP, metrics scrape, and native mutation metadata. A production-style Kubernetes example documents external secrets/TLS/metrics/resources. | Move from in-cluster/local S3 stand-in to real external PostgreSQL/S3-compatible services and failure-mode docs. |
| G7 | File/partition pruning from spatial layout | Hidden layout columns, safe spatial rewrites, simple temporal `BETWEEN` bucket prefilters, exact recheck, LayoutBench, QPS/OLAP scan budgets, local compaction scan-byte/row-group evidence, and native bucket-scoped partial compaction are implemented. | Broaden temporal predicate shapes only from traces, add real-data scale/external bucket-compaction evidence, and cost/plan trend dashboards. |
| G8 | SQL time travel over DuckLake snapshots | SQL `AS OF` is not implemented; programmatic snapshot selection remains the only read-time-travel path. Safe metadata UDTFs (`ducklake_snapshots()`, `ducklake_table_info()`, `ducklake_list_files()`) are exposed through pgwire for inspection. | Prioritize `AS OF` for M7 once snapshot retention/ops docs exist; keep CDC row UDTFs disabled until pgwire projection is safe. |
| G9 | SedonaDB Rust dependency | Consumed through `adonm/sedona-db@quackgis/df54`; no native GEOS/PROJ/GDAL runtime required. | Rebase at milestone boundaries; keep pure-Rust runtime path. |
| G10 | Multi-statement transactions/rollback | Single-table staged DML transactions work and detect conflicts. | Multi-table atomicity needs stable DuckLake batch-commit API; read-your-writes needs session overlay. |
| G11 | DataFusion version alignment | Current fork stack aligned on DF54/Arrow58. | Rebaseline deliberately; never mix DataFusion majors in one milestone. |
| G12 | Runtime native geometry deps | Closed for QuackGIS binary. Native libs live only in external client/test images. | Keep runtime image native-dependency-free. |
| G13 | Martin/tile compatibility | Martin v1.11.0 real binary E2E and 18/18 upstream table fixtures pass. | Add feature-attribute MVT tags and real OSM layer matrix. |
| G14 | Upstream DuckLake deletion-vector format | QuackGIS native DML/compaction currently publishes positional deletes and replacement data under one snapshot. The DuckLake roadmap calls out multi-deletion-vector Puffin files. | Keep the QuackGIS commit boundary spec-compatible, add reference-reader interop gates, and migrate fork-backed DML to upstream deletion-vector primitives when they land. |
| G15 | DuckLake type-system roadmap | QuackGIS currently stores geometry as spec `GEOMETRY` WKB plus conventional SQL columns. Footprint/sidecar schemas document asset metadata/provenance as ordinary columns while DuckLake VARIANT/UDT/fixed-size-array support remains future. | Use stable upstream features for richer asset metadata/provenance, geometry/geography tagging, calibration/embedding arrays, and client identity metadata instead of inventing permanent custom type islands. |
| G16 | DuckLake snapshot, branch, and materialized-view roadmap | QuackGIS needs SQL time travel, restore/rollback, edit/import staging, and precomputed spatial summaries. Upstream future work includes protected snapshots, branching/merge, and materialized views with incremental maintenance. | Align APIs where possible: protected snapshots for backups/releases, branch/merge for staged imports or edit review, and materialized views for tile/coverage/asset summaries. |
| G17 | DuckLake metadata/read-performance roadmap | QuackGIS already enforces bytes-scanned/file-group budgets and targets PostgreSQL/S3 catalogs. Upstream future work includes Parquet Bloom filters, metadata scan improvements, and fewer PostgreSQL catalog roundtrips/stored-procedure-style optimizations. | Add catalog-roundtrip and metadata-scan latency budgets, then prefer upstream performance primitives over QuackGIS-specific caching. |

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
| M4 — editing + GeoServer | QGIS edit/save, GeoServer WFS/WMS/WFS-T | Implemented for maintained traces; autocommit native delete/update DML now optimizes the common edit path. |
| M5 — spatial layout preview | LayoutBench pruning oracle | Implemented with safe bbox rewrites, exact recheck, compaction, and scan budgets. |
| Alpha evidence loop | PostgreSQL/S3 multi-process probe | Implemented as maintained Kind gates; now needs external-scale hardening before production claims. |

## Forward milestones

### Alpha hardening — scaled lakehouse credibility

Alpha is no longer about proving the shape once; it is about making the evidence
credible for external platform developers.

- Publish trend dashboards from run-stamped `metrics.json` artifacts: QPS,
  p95/p99, bytes scanned, file groups, row groups, native DML/compaction counters,
  writer conflicts/retries, and OLAP candidate rows. **Implemented locally via
  `just metrics-dashboard`; scheduled compatibility, storage, and PostGIS regress
  workflows now upload `metrics-dashboard.md` and append it to job summaries.
  Release attachment/promotion policy is documented in
  `docs/RELEASE_EVIDENCE.md`.**
- Add scheduled and manual scale ladders: LayoutBench `sf10+`, copied OSM extracts,
  and at least one Overture/GeoParquet-derived workload with documented hardware,
  object-store profile, row counts, file counts, and budgets. **Benchmark ladder
  and budget policy are documented in `docs/ANALYTICS_BENCHMARKS.md`; a manual
  `Benchmark ladder` workflow now uploads benchmark reports/metrics for maintained
  LayoutBench/QPS/OLAP recipes. Larger executions remain.**
- Exercise real external PostgreSQL and S3-compatible services, not only in-cluster
  stand-ins: credential rotation, catalog restart, network hiccups, object-store
  throttling/latency, backup/restore, failed-writer cleanup, and prefix lifecycle.
  **Runbook exists in `docs/ALPHA_EXTERNAL_SERVICES.md`; real-service executions
  still gate production claims.**
- Promote native DML/compaction evidence from metadata counts into latency,
  scan-byte, and retry/conflict probes around the exact snapshot commit boundary.
  **Local scan-byte/row-group compaction evidence exists; external latency and
  commit-boundary crash-drill execution remains. The drill contract is documented
  in `docs/MUTATION_FAILURE_DRILLS.md`.**
- Publish an Alpha operations guide: storage profiles, auth/TLS profile, compaction
  playbook, metrics/logs, capacity knobs, safe retry behavior, backup/restore, and
  known limits. **Operations docs now cover these Alpha basics plus the
  production-style Kubernetes example; real-service drills still gate production
  claims.**
- Keep SQLite/local parity gates green for deterministic developer correctness;
  every external gate needs a cheap local/static companion.

### M6 — real-data client matrix

- Run one copied real-data matrix through QGIS, OGR/GDAL, GeoServer, and Martin
  side-by-side against QuackGIS and PostGIS: OSM points/lines/multipolygons first,
  then Overture/GeoParquet-style layers with wide attributes and mixed geometry
  columns. **The evidence contract is documented in
  `docs/REAL_DATA_CLIENT_MATRIX.md`; wider executions remain.**
- Track workflow-level outputs, not just connection success: feature counts,
  filtered counts, rendered tile/WMS/WFS result shapes, WFS-T edit results, MVT
  attributes, and representative query timings.
- Prove keyless identity across QGIS, OGR, GeoServer, and WFS-T edits, including
  `_quackgis_rowid` stability after native DML and compaction.
- Add Python/API/client probes: psycopg, SQLAlchemy/GeoPandas, pg_featureserv-like
  readers, tippecanoe/MVT consumers through Martin, and common BI/SQL tools where
  they only require pgwire SQL. **Probe contract is documented in
  `docs/API_CLIENT_PROBES.md`; implementations remain future.**
- Keep every new client gap reduced to a PostgreSQL catalog/protocol surface test
  before adding another compatibility branch.

### M7 — native lakehouse maintenance and write performance

- Extend native DuckLake delete-file DML beyond autocommit `DELETE`/`UPDATE` and
  bucket-local partial compaction into explicit-transaction batching while
  preserving current correctness and `RETURNING` semantics.
- Evolve bucket-local compaction from catalog metadata deltas into time-local and
  real-data compaction with unchanged exact results and measurable file-group,
  row-group, and bytes-scanned improvements.
- Add mutation crash/retry probes around the native DML and compaction commit
  boundary: prewritten objects may become cleanup candidates, but no partial
  catalog mutation may become visible.
- Track upstream multi-deletion-vector Puffin work and test reference-reader
  interoperability before freezing QuackGIS fork-backed DML formats.
- Broaden safe temporal+spatial pruning beyond simple `BETWEEN` only from traces
  or benchmarks, and keep deny-by-default scanners plus exact recheck oracles.
- Add protected snapshot/retention semantics, SQL `AS OF`/time-travel reads,
  rollback/restore docs, safe CDC row table-function exposure, and catalog
  backup/restore integration. **Snapshot/rollback/time-travel/CDC target policy
  is documented in `docs/SNAPSHOT_OPERATIONS.md`; SQL support remains future.**
- Treat DuckLake branch/merge support, when stable, as the preferred foundation
  for staged imports, reviewable edits, and release-style dataset publication.
- Add `CALL quackgis_analyze_table(...)` / stats refresh if DataFusion/DuckLake
  planning needs explicit table statistics beyond Parquet metadata.

### M8 — production security, deployment, and operations

- SCRAM/password auth, readonly/readwrite roles, `pg_roles`, and explicit-user
  privilege helper metadata are implemented and documented; TLS is easy to turn
  on and can be defaulted in production examples. **Security/RBAC hardening
  contract is documented in `docs/SECURITY_RBAC.md`.**
- Grow coarse roles into object/schema/table-level RBAC only where it is needed by
  real client/admin workflows; keep write authorization fail-closed at the
  DuckLake SQL boundary. **Target order and failure probes are documented in
  `docs/SECURITY_RBAC.md`; implementation remains trace-driven.**
- Observability: process-local query ids and write-denial log counters exist;
  an opt-in metrics export endpoint exposes safe process counters, catalog
  refresh counters, and native DML/compaction mutation counters. Next add
  object-store IO counters and writer conflict counters.
- Packaging: slim runtime image stays single-binary and native-dependency-free;
  a production-style Kubernetes example documents TLS/secrets/probes/resource
  limits. Helm and production support still wait for Alpha/M6 real-service gates.
- Disaster recovery: external-service backup/restore, catalog/object prefix
  lifecycle, snapshot pruning, failed compaction cleanup, orphan detection, and
  compatibility with DuckLake reference readers where possible.

### M9 — advanced spatial analytics and conformance

- Curated PostGIS regress subset runs in CI/scheduled mode with pass-rate
  tracking; grow it into an upstream-derived matrix with explicit unsupported
  function docs and skip reasons. **Conformance ledger and static fixture summary
  tooling now exist; broader pgwire/client promotion remains trace-driven.**
- Spatial joins, window queries, grouped stats, coverage/asset inventory queries,
  and candidate narrowing become benchmarked workloads, not one-off probes.
  **Benchmark query-family contract is documented in
  `docs/ANALYTICS_BENCHMARKS.md`; broader executions remain.**
- Raster, point-cloud, 3D tiles, CAD/BIM, and reality-capture data are represented
  as queryable footprint/index rows plus high-fidelity sidecars and provenance
  metadata; exact heavy-format readers remain out of the hot path until needed.
- Use DuckLake materialized views/incremental maintenance, when stable, for tile
  metadata, coverage summaries, and asset inventories before building bespoke
  refresh machinery.
- Cost-based improvements: better selectivity estimates from layout stats, plan
  budget assertions, DuckLake Bloom/metadata scan improvements, and benchmark-
  driven partition/row-group sizing. **Budget policy documented in
  `docs/ANALYTICS_BENCHMARKS.md`.**

### M10 — multi-modal spatial asset lakehouse

- Promote sidecar/index patterns into documented schemas for raster mosaics,
  point-cloud tiles, 3D tiles, CAD/BIM objects, imagery/aerial capture frames, and
  reality-capture assets. **Starter schemas are documented in
  `docs/MULTIMODAL_ASSETS.md`; benchmarked real-data validation remains future.**
- Query footprints, CRS/epoch metadata, quality/resolution fields, lineage, and
  storage URIs in QuackGIS while preserving high-fidelity source artifacts outside
  the hot SQL path.
- Prefer DuckLake-native VARIANT/UDT/fixed-size-array support, when stable, for
  semi-structured asset metadata, provenance, calibration vectors, and typed asset
  handles instead of hard-coding QuackGIS-only encodings.
- Add benchmarked workloads for asset inventory, coverage/gap analysis, change
  detection candidate narrowing, transform/audit metadata, and tile/catalog
  serving over mixed geometry + asset tables.
- Keep the PostGIS simple-feature contract intact: asset richness must augment,
  not destabilize, QGIS/GeoServer/GDAL compatibility.

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
- DuckLake alignment ledger exists for every fork-backed storage behavior: what
  upstream feature it maps to, what interop gate protects it, and what migration
  trigger replaces it. **Implemented in `docs/DUCKLAKE_ALIGNMENT.md`; keep it
  updated with every storage compatibility change.**

## Next execution queue

1. **Run the external-service Alpha ladder.** The runbook exists; execute the
   env-driven PostgreSQL/S3 profile against real services and collect evidence for
   credential rotation, catalog restart, object-store latency/throttling,
   backup/restore, failed-writer cleanup, and catalog refresh visibility.
2. **Promote real-data client matrices.** The matrix contract exists; extend
   OSM/PostGIS parity from QGIS/OGR to GeoServer and Martin over the same copied
   layers, then add Overture/GeoParquet-derived layers with wide attributes and
   mixed geometries.
3. **Scale compaction read-improvement evidence.** The benchmark contract exists;
   next run local fragmented append scan-byte/row-group evidence on larger/copied
   real-data edit traces and external object storage.
4. **Harden native mutation failure modes.** The crash/retry/orphan runbook
   exists; next automate or execute the drills around native DML and
   bucket-compaction commit boundaries, then compare fork formats against
   DuckLake's upstream deletion-vector/Puffin direction.
5. **Move toward snapshot operations.** The design/runbook exists; next prototype
   SQL `AS OF`, protected snapshot/retention behavior, and safe CDC row
   table-function exposure on top of DuckLake metadata.
6. **Grow client and conformance breadth.** The API/client probe contract exists;
   next implement psycopg/SQLAlchemy/GeoPandas, pg_featureserv-style readers, MVT
   attribute clients, and an upstream-derived PostGIS regress matrix with explicit
   unsupported skips.
7. **Deepen security/ops.** The security/RBAC contract exists; next execute
   external auth/TLS/secret-rotation failure-mode probes and implement
   object/schema/table RBAC only where traces require it.
8. **Validate the multi-modal asset path on real data.** Starter sidecar schemas
   and cheap LayoutBench coverage exist; next run raster/point-cloud/3D/CAD/BIM
   footprint inventories through client and benchmark gates.
9. **Keep the boring base boring.** Preserve `just check-fast`,
    `just probe-static-check`, and `just runtime-static-check` as required cheap
    gates: one small Rust binary, no native GIS runtime libraries, no unreviewed
    compatibility branches.

## Risk register

| Risk | Impact | Mitigation |
|---|---|---|
| datafusion-ducklake remains young; PostgreSQL/S3 behavior may change | Production storage claims slip | Keep correctness gates on SQLite/local, run Alpha gates on PostgreSQL/S3, fork for missing semantics, and document every storage assumption. |
| Upstream DuckLake semantics diverge from QuackGIS fork assumptions | Reference-reader interoperability and future upgrades get harder | Gate fork behavior against reference readers, avoid exposing private storage contracts, and migrate to stable upstream features as they land. |
| Object-store IO can dominate selective scans | QPS/latency regressions | Enforce bytes-scanned/file-group budgets, use per-query target partitions for selective scans, trend metrics, and improve compaction/layout before claiming scale. |
| Full-table DML rewrites are correct but expensive | Large transactional edits and broad maintenance slow | Keep autocommit native delete/update and bucket compaction on the single-snapshot mutation path; move transaction batching next and gate by real edit traces. |
| pgwire/catalog compatibility can sprawl | Fragile client-specific branches | Keep trace fixtures, classify by catalog surface, add unit/static gates, and only implement gaps seen in maintained clients. |
| Fork drift vs upstream velocity | Painful rebases, missed fixes | Pin revisions, maintain `DIVERGENCE.md`, rebase at milestones, upstream small patches opportunistically. |
| Spatial pruning bugs could return wrong answers | Data correctness failure | Deny-by-default rewrites, exact SedonaDB recheck, sf0 oracle, quoted/comment/OR guards, and integration tests for every new predicate shape. |
| Production auth/ops lag feature work | External users cannot safely deploy | Make M8 security/ops a milestone, not an afterthought; fail closed on secrets/auth config errors and require external-service restore drills before production claims. |
| Multi-modal asset ambition overwhelms the simple-feature path | Core PostGIS workflows regress while chasing raster/CAD/point-cloud breadth | Keep heavy formats as sidecar/index schemas first; require QGIS/GeoServer/GDAL simple-feature gates to stay green for every asset milestone. |
| Metrics without budgets become dashboards only | Regressions look visible but do not block releases | Keep cheap gates with explicit budgets; trend dashboards augment but do not replace pass/fail thresholds. |

## Retired v0.1 assets

| Asset | Fate |
|---|---|
| DuckDB extension code (`src/` / `sedonadb`) | Retired; SedonaDB used natively. PostGIS SQL rewriting ideas may be salvaged only as compatibility helpers. |
| `vendor/pg_ducklake`, `pg_geometry/` | Deleted from main; history retained. |
| PostgreSQL SQL stubs and bridge tables | Deleted; functions execute in-engine. |
| v0.1 Helm/BuildKit deploy tree | Replaced by current Kind smoke manifests under `deploy/kind/*`; production Helm is deferred until ops gates are stable. |
