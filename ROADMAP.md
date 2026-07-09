# Roadmap

This is the consolidated forward roadmap. Implemented work is summarized once in
the evidence snapshot/status index; forward milestones should describe only the
next evidence or product capability that is still missing. See
[docs/ROADMAP_STATUS.md](./docs/ROADMAP_STATUS.md) for the concise split between
closed local contracts and execution-heavy work. The active execution loop remains
the maximum trusted local Kind+Linkerd envelope in
[docs/LOCAL_KIND_LINKERD_FOCUS.md](./docs/LOCAL_KIND_LINKERD_FOCUS.md), promoted
to real external services only when local evidence is boring.

## Full project goal

**QuackGIS is the PostGIS-compatible front door to a spatial lakehouse**: one
Rust pgwire service that lets platform teams keep very large spatial data in an
open DuckLake/Parquet lake while serving familiar PostgreSQL/PostGIS clients and
running high-throughput analytical SQL through DataFusion + SedonaDB.

The ambitious end state is not “a smaller PostgreSQL.” It is an operational
spatial data platform for city/regional/national datasets where object-store data
feels like PostGIS to QGIS, GeoServer, Martin, GDAL/OGR, psql, psycopg,
SQLAlchemy/GeoPandas, API servers, and BI tools, while analytical users get
DuckDB-style OLAP over geometry, geography, temporal/spatial layouts,
raster/asset footprints, point-cloud/CAD/reality-capture indexes, and provenance
sidecars — without PostgreSQL or DuckDB owning the query/data plane.

The 1.0+ bar is a credible operational spatial lakehouse: tens of millions to
billions of features in routine gates, multi-terabyte object-store prefixes in
manual/scheduled stress paths, many stateless readers, parallel ingest/edit
writers, recoverable operations, and enough PostGIS behavior that common GIS
clients do not need to know they are talking to a lakehouse. Post-1.0 ambition is
trillion-row-class spatial/asset indexes, releaseable dataset branches, protected
snapshots, maintained tile/coverage summaries, and multi-modal asset inventories
that make QuackGIS the SQL/control plane for digital-twin-scale spatial lakes.

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
13. **Do not let docs-only contracts masquerade as delivery.** A plan/runbook is a
     useful local contract; a roadmap item closes only when the intended probe,
     benchmark, release artifact, or external-service drill has run at the stated
     scale and source SHA.

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
  `geography_columns`, `spatial_ref_sys`, `postgis_version()` family,
  SRID/extent/geometry-metadata/point-coordinate/bbox-coordinate helpers, common
  privilege helpers, renderer/tile helpers, MVT helpers, bbox
  operators, and key catalog shims are covered by focused tests/probes. The
  curated PostGIS regress subset now covers the claimed starter surface, emits
  pass-rate evidence in CI and a scheduled/manual workflow, and has an explicit
  conformance/unsupported ledger in `docs/POSTGIS_CONFORMANCE.md`.
- **Client compatibility.** Maintained Kind probes cover QGIS read/render/filter/
  identify, QGIS edit/save plus compaction-after-edit rowid stability, GDAL/OGR
  load/read with keyless identity, GeoServer WFS/WMS/WFS-T with keyless identity,
  Martin tile serving, and OSM copied-layer MVT SQL bytes. A local API/client
  surface smoke now covers
  psycopg-style text/binary WKB params, SQLAlchemy-style reflection,
  GeoPandas-style WKB reads, pg_featureserv-style bbox filters, BI grouped
  aggregates, and non-empty MVT bytes before heavier named-client containers.
  Scheduled/manual compatibility jobs collect artifacts.
- **Spatial layout.** Hidden `_qg_*` bbox/bucket/sort columns, safe bbox pruning,
  simple temporal `BETWEEN` bucket prefilters, exact recheck, LayoutBench `sf0`
  oracle, local `sf1` evidence, whole-table compaction, and native bucket-local
  delete+append compaction are implemented. Local native mutation failpoint tests
  now prove abort-before-commit leaves catalog metadata unchanged and a retry
  publishes the intended delete/update/compaction. Local compaction tests report
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
  privilege helpers, read-only write shapes fail closed and recognized denials
  increment `quackgis_write_denied_total`, and local backup/restore plus DuckLake
  metadata-table-function oracles cover cheap operations checks. An opt-in
  Prometheus `/metrics` endpoint exposes safe process counters, including native
  mutation aborts before catalog commit, and is scraped by the external-profile
  Kind probe. Metrics artifacts can now be gated by
  `just metrics-budget-check` so dashboards with explicit budgets or missing
  artifact downloads fail closed.
- **Snapshot reads.** Safe DuckLake metadata UDTFs are exposed and simple
  snapshot-pinned reads work through pgwire with `snapshot` or `snapshot_id`
  named selectors for one-table `SELECT` statements; the local oracle now checks
  selector variants plus count and extent parity at the pinned snapshot.
  Parser-level SQL `AS OF`, protected retention, rollback integration, and CDC row
  UDTFs remain future work.
- **CI/packaging.** mise-backed CI runs the fast Rust/local gates, host-local
  preview smokes, static Kind probe validation, and static validation for the
  production-style Kubernetes example. Scheduled compatibility and storage
  reports upload run-stamped `metrics.json` artifacts and run budget/required-
  check assertions on them; the artifact workflow publishes Linux binaries, GHCR
  runtime images, and a release-evidence manifest on release/main refs.
  `docs/RELEASE_EVIDENCE.md` defines the release packet and dashboard attachment
  policy.
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
| G5 | UPDATE/DELETE on DuckLake tables | Autocommit `DELETE` and `UPDATE` use fork-backed atomic positional delete files; `UPDATE` stages replacement rows as pending data files and commits delete+append metadata under one snapshot. Bucket-scoped compaction also uses delete+pending-append metadata under one snapshot when row-lineage planning succeeds. SQLite/local and external PostgreSQL/S3 probes cover native delete/update/compaction metadata. Local before-commit failpoints prove native `DELETE`, `UPDATE`, and bucket compaction leave no visible catalog data-file/delete-file rows when aborting after prewrites but before `commit_table_mutation`, increment `quackgis_native_mutation_aborts_total` once, and retrying the one-shot fault publishes the intended mutation. Explicit transactions and fallback paths keep correct staged/full-table rewrites including `RETURNING`. Details in `docs/NATIVE_DML_FORK_PLAN.md` and `docs/MUTATION_FAILURE_DRILLS.md`. | Extend crash/retry probes to process-kill/retry, Kind/external storage, orphan cleanup, and transaction batching only from edit traces. |
| G6 | Spec-compliant PostgreSQL catalog + S3 profile | Kind Alpha lake profile proves catalog/object-store wiring, multi-pod readers, writers, QPS, OLAP, metrics scrape, and native mutation metadata. A production-style Kubernetes example documents external secrets/TLS/metrics/resources. | Move from in-cluster/local S3 stand-in to real external PostgreSQL/S3-compatible services and failure-mode docs. |
| G7 | File/partition pruning from spatial layout | Hidden layout columns, safe spatial rewrites, simple temporal `BETWEEN` bucket prefilters, exact recheck, LayoutBench, QPS/OLAP scan budgets, local compaction scan-byte/row-group evidence, and native bucket-scoped partial compaction are implemented. | Broaden temporal predicate shapes only from traces, add real-data scale/external bucket-compaction evidence, and cost/plan trend dashboards. |
| G8 | SQL time travel over DuckLake snapshots | A first pgwire-safe snapshot selector supports simple one-table reads with `public.table(snapshot => <snapshot_id>)` or `public.table(snapshot_id => <snapshot_id>)` and count/extent parity. Safe metadata UDTFs (`ducklake_snapshots()`, `ducklake_table_info()`, `ducklake_list_files()`) are exposed through pgwire for inspection. Parser-level SQL `AS OF`, positional table-function selectors, protected retention, rollback integration, and CDC row UDTFs are not implemented. | Promote to parser-level `AS OF`, protected retention, rollback/restore integration, and CDC only after pgwire projection is safe. |
| G9 | SedonaDB Rust dependency | Consumed through `adonm/sedona-db@quackgis/df54`; no native GEOS/PROJ/GDAL runtime required. | Rebase at milestone boundaries; keep pure-Rust runtime path. |
| G10 | Multi-statement transactions/rollback | Single-table staged DML transactions work and detect conflicts. | Multi-table atomicity needs stable DuckLake batch-commit API; read-your-writes needs session overlay. |
| G11 | DataFusion version alignment | Current fork stack aligned on DF54/Arrow58. | Rebaseline deliberately; never mix DataFusion majors in one milestone. |
| G12 | Runtime native geometry deps | Closed for QuackGIS binary. Native libs live only in external client/test images. | Keep runtime image native-dependency-free. |
| G13 | Martin/tile compatibility | Martin v1.11.0 real binary E2E and 18/18 upstream table fixtures pass. OSM copied layers now emit non-empty MVT SQL bytes, and the MVT encoder has unit coverage for feature key/value dictionaries. | Wire feature-attribute tags through SQL/client probes and real Martin OSM layer matrix. |
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
| Alpha evidence loop | PostgreSQL/S3 multi-process probe | Implemented as maintained Kind gates; now needs external-service promotion before production claims. |

## Forward roadmap

Implemented work above is the floor, not the plan. A forward item is closed only
when its target evidence has run, produced artifacts, and updated the owning docs.

### Promotion ladder

| Stage | Evidence bar | Exit condition |
|---|---|---|
| Local maximum Alpha | Full Kind+Linkerd ladder, real-data smoke, QPS/OLAP/compaction dashboards, mutation/auth failure probes where possible | local artifacts are boring, repeatable, and budgeted |
| External Alpha | Same claims on managed PostgreSQL + S3-compatible storage with credential rotation, catalog restart, object-store latency/throttling, backup/restore, cleanup | platform-managed service runbook passes with release evidence |
| Beta city-scale | copied OSM/Overture-style regional datasets, full client matrix, API clients, real edit traces, native maintenance, security probes | a platform team can operate a regional vector lake without reading source |
| 1.0 regional lakehouse | reproducible release packet, upgrade path, DR story, compatibility matrix, benchmark report, documented hard limits | stable public release for spatial lakehouse workloads |
| 2.x spatial asset lakehouse | multi-modal asset indexes, protected releases/branches, maintained summaries, large-object inventories | QuackGIS is the SQL/control plane for digital-twin-scale spatial lakes |

### Active Alpha — local maximum, then external promotion

- Make the full Kind+Linkerd ladder routine: compatibility, alpha lake, mTLS/QPS,
  deep QPS, lake LayoutBench, OSM parity, compatibility report, and dashboard.
  Raise scale only with recorded `metrics.json`, dashboard, hardware/profile, and
  source SHA.
- Promote the same claims to real external PostgreSQL/S3-compatible services:
  credential rotation, catalog restart, network hiccups, object-store throttling,
  backup/restore, failed-writer cleanup, prefix lifecycle, and catalog refresh
  visibility.
- Turn metrics dashboards from passive artifacts into release gates: define
  acceptable p95/p99, scan-byte, file-group, row-group, candidate-row, conflict,
  native-DML, compaction, and catalog-refresh budgets per profile.
- Keep every heavy gate paired with a cheap static/unit/local companion so scale
  runs catch regressions, not basic drift.

### M6 / Beta-0 — real-data client and API matrix

- Run one copied real-data matrix through QGIS, OGR/GDAL, GeoServer, Martin, and
  PostGIS side-by-side: OSM points/lines/multipolygons first, then
  Overture/GeoParquet-style layers with wide attributes and mixed geometries.
- Track workflow outputs, not just connection success: feature counts, filtered
  counts, rendered tile/WMS/WFS result shapes, WFS-T edit results, MVT attributes,
  keyless identity stability after DML/compaction, and representative timings.
- Add API/client probes for psycopg, SQLAlchemy/GeoPandas, pg_featureserv-style
  readers, MVT consumers through Martin, and BI/SQL tools that only need pgwire.
  The local pgwire/catalog surface probe exists; named client containers and
  scheduled report rows remain the support-claim gate.
- Reduce every new client gap to a PostgreSQL catalog/protocol surface test before
  adding compatibility code.

### M7 / Beta-1 — native maintenance, snapshots, and write performance

- Extend native DuckLake delete-file DML and bucket-local compaction into
  explicit-transaction batching while preserving `RETURNING`, conflict detection,
  and one visible snapshot boundary.
- Add mutation crash/retry/orphan probes around native DML and compaction commits:
  prewritten objects may become cleanup candidates, but partial catalog mutations
  must never become visible.
- Scale compaction from synthetic fragmentation to time/space-local real-data edit
  traces with measured file-group, row-group, bytes-scanned, latency, and exact
  result improvements.
- Promote snapshot reads into parser-level SQL `AS OF`, protected
  snapshot/retention semantics, rollback/restore integration, and safe CDC row
  exposure after pgwire projection is proven safe.
- Treat upstream DuckLake deletion-vector/Puffin, branch/merge, protected
  snapshots, materialized views, Bloom filters, and metadata-scan improvements as
  migration targets, not optional background reading.

### M8 / Beta-2 — production security, deployment, and operations

- Promote coarse roles into object/schema/table-level RBAC only from real
  admin/client traces; keep write authorization fail-closed at the DuckLake SQL
  boundary.
- Execute auth/TLS/secret-rotation, external backup/restore, orphan cleanup,
  snapshot pruning, catalog/object-prefix lifecycle, and reference-reader interop
  drills against managed services.
- Add object-store IO, catalog roundtrip, writer conflict/retry, compaction queue,
  snapshot-retention, and failed-cleanup metrics/logs with profile-specific
  budgets.
- Build production packaging from evidence: Helm/manifests, upgrade/migration
  notes, resource/capacity guidance, and native-dependency-free runtime images.

### M9 / Beta-3 — advanced spatial analytics and PostGIS conformance

- Grow the curated PostGIS regress suite into an upstream-derived matrix with
  pass/skip/fail reporting, explicit unsupported reasons, and pgwire/client
  promotion for functions real traces need.
- Benchmark spatial joins, window queries, grouped stats, coverage/asset inventory,
  candidate narrowing, and mixed spatial+attribute workloads at regional scale.
- Improve planning from evidence: layout selectivity stats, plan budget assertions,
  row-group sizing, DuckLake Bloom/metadata scans, and fewer catalog roundtrips.
- Keep exact SedonaDB recheck as the invariant for every pruning or cost-based
  improvement.

### 1.0 — credible regional spatial lakehouse release

- Publish a reproducible release packet: compatibility matrix, benchmark report,
  operations/security evidence, upgrade/migration notes, DuckLake alignment ledger,
  known limits, and selected raw artifacts for the release SHA.
- Require local, external PostgreSQL/S3, and copied real-data profiles to pass their
  budgeted evidence gates before public production claims.
- Prove release-to-release catalog/object-prefix migration on copied catalogs and
  object prefixes, including rollback guidance and reference-reader checks where
  possible.

### M10 / 2.x — multi-modal spatial asset lakehouse

- Validate raster mosaics, point-cloud tiles, 3D tiles, CAD/BIM, imagery/aerial
  frames, and reality-capture assets as queryable footprint/index tables plus
  high-fidelity object-store sidecars.
- Add benchmarked workloads for asset inventory, coverage/gap analysis, change
  detection candidate narrowing, transform/audit metadata, tile/catalog serving,
  and provenance queries over mixed geometry + asset tables.
- Prefer DuckLake-native VARIANT/UDT/fixed-size-array and materialized-view support
  for asset metadata, calibration vectors, typed handles, maintained summaries,
  and releaseable dataset views.
- Keep simple-feature PostGIS compatibility non-negotiable: multi-modal richness
  augments QGIS/GeoServer/GDAL workflows; it must not destabilize them.

## Next execution queue

1. **Make local maximum evidence boring.** Run the full Kind+Linkerd ladder often
   enough to establish budgets, dashboard trends, flakes, and capacity knobs.
2. **Widen the Kind real-data matrix.** Extend OSM/PostGIS parity to GeoServer
   and real Martin binary/attribute checks on copied layers, then add one
   Overture/GeoParquet-derived dataset with wide attributes and mixed geometries.
3. **Promote mutation crash evidence.** Local before-commit failpoints now cover
   native delete/update/bucket compaction plus retry after the one-shot fault;
   extend them to process-kill, stale-generation, orphan-cleanup, and Kind before
   external storage.
4. **Promote snapshot/time-travel.** The first simple snapshot selector exists;
   add parser-level SQL `AS OF`, protected snapshot retention, and CDC row
   exposure only after pgwire projection is safe.
5. **Promote API/client probes.** The local surface probe exists; promote psycopg,
   SQLAlchemy/GeoPandas, pg_featureserv-style, MVT attribute, and BI probes into
   named containers and scheduled compatibility evidence.
6. **Promote to external services.** Execute the managed PostgreSQL/S3 runbook and
   attach artifacts to the release-evidence packet for the exact source SHA.
7. **Start real asset inventories.** Run raster/point-cloud/3D/CAD/BIM footprint
   inventories through query, client, and benchmark gates without putting heavy
   decoders in the SQL hot path.
8. **Keep the boring base boring.** Preserve `just check-fast`,
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
| Roadmap churn turns into docs-only progress | Ambition looks complete without product evidence | Keep implemented plans in the status index, require artifacts before closing forward items, and attach release evidence by source SHA. |

## Retired v0.1 assets

| Asset | Fate |
|---|---|
| DuckDB extension code (`src/` / `sedonadb`) | Retired; SedonaDB used natively. PostGIS SQL rewriting ideas may be salvaged only as compatibility helpers. |
| `vendor/pg_ducklake`, `pg_geometry/` | Deleted from main; history retained. |
| PostgreSQL SQL stubs and bridge tables | Deleted; functions execute in-engine. |
| v0.1 Helm/BuildKit deploy tree | Replaced by current Kind smoke manifests under `deploy/kind/*`; production Helm is deferred until ops gates are stable. |
