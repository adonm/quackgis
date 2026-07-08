# Roadmap

## Goal

**QuackGIS is a PostGIS-compatible, Sedona-powered spatial lakehouse database**:
a single Rust pgwire server that lets platform/application developers answer
large, complex spatial questions over shared DuckLake/Parquet data with high
throughput, horizontal read scaling, and DuckDB-style columnar OLAP analysis.

Common GIS clients and tools — QGIS, GeoServer, Martin, GDAL/OGR/`ogr2ogr`,
psql, psycopg — should connect without significant changes. They are the
compatibility surface for the lakehouse, not evidence that QuackGIS is trying to
be PostgreSQL or an OLTP database.

## Success metrics

1. **Primary — scaled spatial questions.** Benchmarks/probes prove large
   analytical spatial SQL over DuckLake/Parquet: selective bbox/spatial
   predicates, joins/window queries where supported, high-QPS parallel readers,
   many parallel ingest jobs, OLAP fanout aggregates/calculations over columnar
   records, and shared SQL catalog + object storage.
2. **Secondary — client workflows.** Scripted end-to-end suites, run in CI:
   - *QGIS*: connect → browse schemas → add layer → render (feature paging) →
     identify → filter → edit (insert/update/delete) → save.
   - *GeoServer*: register PostGIS datastore → publish layer → WMS GetMap →
     WFS GetFeature (paged) → WFS-T insert/update.
   - *OGR/GDAL*: `ogr2ogr` load into QuackGIS and read back, both PostgreSQL
     wire drivers (`PG:` and PostgreSQL-compatible connection strings). Treat
     `ogr2ogr` as a first-class PostGIS-wire compatibility target alongside
     QGIS, GeoServer, and Martin.
3. **Tertiary — PostGIS function conformance.** Pass rate on a curated subset
   of upstream PostGIS regress tests (function semantics, not PG internals).

## Strategy

1. **Wire compatibility, not Postgres.** v0.1 stacked PostgreSQL + vendored
   pg_ducklake + a C geometry type + a DuckDB extension to get a PG-compatible
   front door. The redesign serves pgwire directly from Rust over a SedonaDB
   `SessionContext`. Every layer that existed to host or bridge PG/DuckDB is
   deleted.
2. **Fork/vendor-preferred.** The three pillars are active Apache-2.0
   projects: datafusion-postgres (v0.17-era fork, pg_catalog + auth + TLS), SedonaDB
   (0.4, vector/raster/geography), datafusion-ducklake (0.3, alpha) — but
   several capabilities the best design needs **do not exist upstream yet**
   (see the gap ledger below). We do not block milestones on upstream review:
   pin exact revisions, and the moment a needed capability is missing, fork
   the crate and build it there. Upstreaming is opportunistic, done from the
   fork when convenient — never on the critical path.
3. **Client-trace-driven testing.** Capture the exact SQL QGIS, GeoServer,
   Martin, and OGR/`ogr2ogr` send (query logs), replay as fixtures, fix in
   priority order. PostGIS regress covers function semantics second.
4. **Transparent spatial layout.** Geometry tables get bbox/quadkey/Hilbert
   layout columns at write time; scans prune with them at read time.
5. **SQL catalog + object storage is the product storage contract.** SQLite +
   local files and PostgreSQL + S3 are both first-class DuckLake profiles. The
   current preview proves SQLite/local; Alpha must prove PostgreSQL/S3 with
   multi-process readers/writers and operational failure-mode docs.
6. **DuckDB-style OLAP, without DuckDB.** Analytical users should be able to fan
   out over many geometries/assets, compute grouped stats and primitive columnar
   calculations, push filters/projections down to Parquet/DuckLake where possible,
   and use the calculated result to narrow exact SedonaDB spatial work.

## Upstream gap ledger

Capabilities the design needs that upstream may not provide. **Status** is
what we have verified from upstream docs/READMEs; unverified rows get a probe
spike at the start of the owning milestone. **Plan** default: fork the crate,
build the capability, ship from the fork.

| # | Capability | Needed by | Upstream status | Plan |
|---|---|---|---|---|
| G1 | `geometry`/`geography` type OID on the wire | M2 | **Equivalent OID implemented.** `::geometry`/`::geography` casts still preprocess to `::bytea` (commit 912823e), and binary WKB columns whose name matches the spatial convention are now advertised on the wire and in `pg_attribute.atttypid` with dedicated sentinel type OIDs (`GEOMETRY_OID=90001` / `GEOGRAPHY_OID=90002`) via datafusion-postgres fork `2c35282`, with client type-resolution fixes in `2c2e5d9`. Wire payload is bytea-identical (raw WKB / hex-EWKB), so Martin/psql are unaffected; QGIS/GeoServer now see a distinct spatial type. PostGIS' real OIDs are dynamic per-install, so a fixed sentinel + QuackGIS `pg_type` typname shims keep catalog introspection consistent. | Closed for the equivalent path. A real PostGIS-OID-compatible registration is deferred until a client strictly requires `format_type`/`pg_type` row parity. |
| G2 | pg_catalog depth for spatial-client introspection (pg_index, pg_am, regclass casts, `format_type`, array/oidvector columns) | M3 | **QGIS, OGR, and GeoServer WFS/WMS catalog traces are green.** The common pg_catalog tables work natively, `pg_roles` stack overflow is fixed in `adonm/datafusion-postgres@quackgis/fixes`, and QuackGIS's `CatalogCompatHook` now shims PostGIS-wire boundary gaps by catalog surface: custom geometry/geography `pg_type` rows, pgjdbc table/column/primary-key/type-info probes, layer/style existence checks, pg_class/pg_attribute/pg_index shape probes, description/inherits probes, schema-derived synthetic `id` unique-index metadata, and key-column lookup. | Closed for maintained QGIS, OGR, and GeoServer WFS/WMS smoke paths. General PostgreSQL index fidelity, WFS-T, and extra metadata funcs remain M4+ hardening. |
| G3 | SQL cursors: `DECLARE ... BINARY CURSOR` / `FETCH FORWARD n` (feature paging) | M3 | **Cursors work for the simple-query/libpq path** (psql, QGIS, GDAL). **BINARY cursor format FIXED in `adonm/datafusion-postgres@quackgis/fixes` (commit `98b3865`)** — `DECLARE x BINARY CURSOR` now returns raw binary-protocol bytes instead of hex-text bytea. Regression test `binary_cursor_returns_raw_bytes` verifies the wire bytes are i64 BE for `SELECT 42`. QuackGIS also carries a narrow PostgreSQL-driver cursor shim for OGR's `DECLARE OGRPGLayerReader...` / `FETCH ...` read path. **Remaining sub-gap**: general extended-protocol FETCH/portal suspension for pgjdbc/tokio-postgres still needs deeper pgwire work. | Fork active (BINARY patch upstreamable). General extended-protocol FETCH deferred until GeoServer/pgjdbc requires it. |
| G4 | Portal suspension honoring `Execute.max_rows` (JDBC `setFetchSize`, GeoServer) | M4 | **Extended-protocol PREPARE/EXECUTE verified through pgjdbc and official GeoServer WFS/WMS smoke.** General `setFetchSize` portal suspension is still deferred because the maintained GeoServer gate does not require it. | Keep probing with pgjdbc/WFS-T; fix in the datafusion-postgres fork only when a client requires suspension semantics. |
| G5 | UPDATE/DELETE on DuckLake tables | M4 | **Basic single-table DML implemented in QuackGIS** via full-table rewrite/replace semantics over DuckLake writer API. `INSERT`/`UPDATE`/`DELETE ... RETURNING` now returns edit-client refresh rows for simple and extended pgwire paths, including QGIS' parameterized WKB edit SQL. Correct but not optimal; no delete files yet. | Native delete-file UPDATE/DELETE remains future optimization in datafusion-ducklake fork if performance requires it. |
| G6 | Spec-compliant SQL catalog + object-storage profiles | Alpha | SQLite catalog + local Parquet is the current validated preview path. PostgreSQL catalog + S3 Parquet is the required scaled profile and must support multi-process readers/writers through DuckLake snapshot semantics. | Extend/fork datafusion-ducklake as needed for spec-compatible PostgreSQL catalog writes, S3/object-store configuration, conflict handling, and operational probes. Keep SQLite/local as a first-class correctness profile, not a throwaway dev mode. |
| G7 | File/partition pruning from DuckLake stats (bbox/quadkey layout) | M5/Alpha | **Preview path implemented.** datafusion-ducklake marks filters Inexact so Parquet can use stats and DataFusion reapplies filters. QuackGIS now materializes hidden WKB-derived `_qg_*` layout columns, rewrites recognized single-table spatial predicates to safe bbox filters, and rechecks exact SedonaDB predicates for correctness. LayoutBench `sf0` is the correctness oracle; local `sf1` runs document ingest/layout/compaction behavior. | Harden coverage from trace/benchmark evidence: add more predicate shapes only when safe, keep exact recheck mandatory, move from whole-table to bucket-local compaction when needed, and add OLAP fanout queries that prove projection/filter/aggregate pushdown evidence over large columnar spatial tables. |
| G8 | SQL time travel (`AS OF`) over DuckLake snapshots | M7 (nice-to-have) | Missing — programmatic snapshot selection only | Fork if/when prioritized |
| G9 | SedonaDB Rust crates consumable as a dependency | M0 | **Verified consumable**: not on crates.io but git dependency works. QuackGIS consumes `adonm/sedona-db@quackgis/df54` to align with the DuckLake 1.0+ / DF54 stack. | Track upstream head through fork branch; rebaseline at milestones. |
| G10 | Multi-statement transactions / rollback for edit sessions | M4 | **Single-table edit DML implemented.** Explicit transactions stage one DuckLake table per connection, publish the final table through one DuckLake writer snapshot at `COMMIT`, discard on `ROLLBACK`, and fail closed on concurrent replace conflicts. DDL, multi-table write transactions, and read-your-writes for arbitrary `SELECT` remain outside the current claim. | Extend only when client traces require it: multi-table atomic commit needs a stable DuckLake batch-commit API; ordinary in-transaction SELECT visibility needs a per-session catalog overlay. |
| G11 | DataFusion version alignment across SedonaDB ↔ datafusion-postgres ↔ datafusion-ducklake | M0/M1 | **Resolved for current stack** by fork-bumps: `adonm/sedona-db@quackgis/df54` and `adonm/datafusion-postgres@quackgis/fixes` align with `datafusion-ducklake` main (DF54 / Arrow58), the DuckLake 1.0+ target path. | Follow upstream heads through fork branches; rebaseline on each milestone. |
| G12 | Runtime native geometry deps (`libgeos`/`libproj`/`libgdal`) | M0/Alpha | **Closed for QuackGIS itself.** The active Sedona dependency disables native default features and uses pure-Rust/vector paths (`geo`, `tg`, `proj-rust`); `cargo tree -p quackgis-server -i geos-sys` has no match. | Keep the Rust binary/runtime image free of native GEOS/PROJ/GDAL. Native libraries may exist only in external client/test containers such as QGIS/GDAL. |
| G13 | Martin tile-server compatibility | M2 | **Done — real binary E2E green.** Martin v1.11.0 connects, discovers tables, and serves MVT tiles (`GET /points/0/0/0` → 200, 12-byte protobuf). All compatibility gaps closed: `PostGIS_Lib_Version()` ✅, `current_setting()` ✅, `geometry_columns` ✅ (dynamic catalog-scanning TableProvider), `spatial_ref_sys` ✅, `ST_AsMVT` ✅, `ST_AsMVTGeom` ✅, `ST_TileEnvelope` ✅ (3/4/5-arg overloads via Sedona WKB helpers), `ST_MakeEnvelope` ✅, `ST_Expand` ✅, `ST_CurveToLine` ✅, `&&` ✅, `::geometry` ✅, `ST_Transform` ✅ (pure-Rust). Fork carries: Martin table/function discovery shortcuts, JSONB `properties` encoding, named-`margin` → positional rewrite, PostGIS fixture DDL rewrites, and deterministic sanitizing for pathological PostgreSQL quoted identifiers. Opt-in upstream Martin table fixture coverage: **18/18** load unmodified. | Closed. Feature-attribute MVT tags remain future work. |

Fork mechanics: forks are consumed as git branch dependencies (not vendored);
in-tree `vendor/` subtree only if a fork needs deep, long-lived divergence
(precedent: v0.1 `vendor/pg_ducklake`). Every fork carries a `DIVERGENCE.md`
listing each patch and its upstream PR (if any); rebase onto upstream tags at
each milestone boundary.

## Milestones

### M0 — Skeleton server — gate: `psql` works

- New `quackgis-server` crate: SedonaDB `SessionContext` served by
  `datafusion_postgres::serve`, `setup_pg_catalog`, TLS + password auth wired.
- `SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'))` from psql.
- Retire the v0.1 stack: `vendor/pg_ducklake`, `pg_geometry/`,
  `container/init.d/` SQL stubs, DuckDB extension packaging. Keep the code in
  git history; delete from main.
- Fork infrastructure: org forks of datafusion-postgres / arrow-pg /
  datafusion-pg-catalog / datafusion-ducklake, consumed as tracked git fork
  branches where needed; `DIVERGENCE.md`
  convention in place.
- CI: cargo build/test + psql smoke test on the binary.

### M1 — DuckLake storage — gate: round-trip + restart persistence ✅ VALIDATED

Validated against **DuckLake 1.0+ target path**: `datafusion-ducklake` main HEAD
(DF 54 / Arrow 58), with the rest of the stack bumped to DF 54.

- `DuckLakeCatalog` registered as catalog `quackgis`; persisted tables live at
  `quackgis.main.<table>`. Default catalog remains `datafusion` so pg_catalog
  can attach there; DuckLake rejects schema registration.
- SQLite catalog + local Parquet storage wired through the default storage
  profile (`QUACKGIS_CATALOG_PATH`, `QUACKGIS_DATA_PATH`). The Alpha storage
  profile wires PostgreSQL catalog metadata plus S3/object storage and has a
  Kind smoke via `just kind-lake-smoke`; hardening scaled behaviour
  remains in scope.
- SQL routing validated: CTAS, bare `CREATE TABLE (...)`, `INSERT ... SELECT`,
  `INSERT ... VALUES` with column mapping, single-table UPDATE, and single-table
  DELETE route through the DuckLake writer API, refresh the snapshot-bound
  catalog, and are visible through pgwire.
- Writer API round-trip and restart persistence validated: write Parquet +
  metadata, query through pgwire, rebuild context, query again. 6 DuckLake
  tests green.
- Geometry WKB persistence validated: hard-coded WKB written via writer API,
  read back through `ST_AsText(ST_GeomFromWKB(geom))`.
- Remaining storage gaps: production PostgreSQL/S3 hardening (G6), advanced
  SQL (RETURNING, multi-table UPDATE/DELETE), and native delete-file updates
  as a performance optimization. Spatial-layout pruning remains M5.

### M2 — PostGIS SQL surface — gate: psycopg + OGR round-trip

- `geometry`/`geography` type OIDs in pg_type; hex-EWKB text + EWKB binary
  encoding for SedonaDB geometry arrays via the arrow-pg fork (G1); WKT/EWKB
  parameter decoding.
- `geometry_columns`, `spatial_ref_sys` (EPSG from PROJ data),
  `postgis_version()` family.
- SedonaDB function catalog registered; PostGIS-compat aliases where names or
  arities differ; `&&`, `<->` operators mapped to SedonaDB equivalents.
- Session shims: tolerate `SET client_min_messages/application_name/...`,
  `BEGIN/COMMIT` (single-statement semantics documented).
- Gate: psycopg binary/text geometry round-trips and PostGIS SQL surface; Martin
  discovery/tile path. OGR load+read was moved to the M3/M4 client-probe track
  because GDAL's PostgreSQL driver exposes additional catalog/write-path gaps.

### M3 — QGIS read path — gate: scripted QGIS browse/render/identify

**Status: read path closed for the current headless PyQGIS gate.** The Kind Job
using `docker.io/qgis/qgis:ltr-questing` creates a WKB-backed DuckLake table,
adds it through the QGIS postgres provider, validates the layer, sees attributes,
and reads both geometries through QGIS's binary cursor path (`features_read 2`).

- Fix the introspection surface QGIS's postgres provider uses: pg_class,
  pg_attribute, pg_type, pg_namespace, pg_index (key detection), regclass
  casts, `format_type`, `version()` — done for the QGIS trace via
  `CatalogCompatHook` plus the datafusion-postgres fork (G2).
- Cursors: `DECLARE ... BINARY CURSOR` / `FETCH FORWARD n` (QGIS feature
  paging) — done for the simple-query/libpq QGIS path (G3).
- `ST_AsBinary(geom, 'NDR')` over WKB-backed Binary columns — done as a
  byte-preserving QuackGIS UDF overload for the QGIS fetch query.
- Extent/SRID metadata: exact `ST_Extent` now returns PostGIS-style BOX bounds,
  `ST_EstimatedExtent` returns NULL when no statistics are available so clients
  can fall back to exact bounds, and `Find_SRID` mirrors the current
  `geometry_columns.srid = 0` metadata. Per-row EWKB SRID tags now round-trip
  through `ST_SetSRID`, `ST_GeomFromEWKT`, `ST_MakeEnvelope`, and
  `ST_Transform`. DuckLake file-stat estimates and declared-SRID column
  metadata remain future render/UX optimizations.
- Unique-key strategy: real conventional `id` columns are exposed as
   schema-derived synthetic unique indexes, and keyless spatial tables now get
   `_quackgis_rowid` metadata plus a read projection (persisted for SQL-created
   keyless tables, virtual for writer-backed tables). There is still no ctid;
   QGIS edit/save smoke now uses `_quackgis_rowid` successfully.
- Gate: headless PyQGIS suite in CI — add connection, list layers, render,
  identify, attribute filter.

Remaining M3 hardening before calling the milestone fully CI-ready:

1. Promote the Kind PyQGIS Job into CI and extend it from add/read to render,
   identify, and attribute-filter assertions.
2. Promote the QGIS edit/save Kind probe into CI alongside the read probe.
3. Add DuckLake file-stat-backed `ST_EstimatedExtent` and declared-SRID
   metadata so QGIS layer canvas UX does not rely on full feature scans.
4. Promote the in-cluster GDAL/OGR PostgreSQL-driver load/read probe into CI;
   the maintained gate now covers `ALTER TABLE ... ADD COLUMN` and
   `PG_USE_COPY=NO` INSERT-mode append.

### M4 — Editing + GeoServer — gate: QGIS edit session, GeoServer WMS/WFS-T

Start M4 with client traces, not abstractions. The QGIS edit/save smoke gate is
green in Kind: a PyQGIS layer commits insert/update/delete against a keyless
spatial table via `_quackgis_rowid`. The official GeoServer 3.0.0 smoke gate is
also green for PostGIS datastore publish, WFS GeoJSON GetFeature, and WMS PNG
GetMap. WFS-T remains future hardening.

- UPDATE/DELETE on DuckLake tables — delete-file support built in our
  datafusion-ducklake fork (G5; the highest-risk item, start earliest).
- `INSERT ... RETURNING` / `UPDATE ... RETURNING` / `DELETE ... RETURNING` for
   QGIS/GeoServer feature creation and post-save refresh — basic single-table
   simple/extended pgwire shapes now route through the DuckLake full-table
   rewrite path; QGIS' parameterized WKB insert/update/delete smoke path passes.
- Edit-session transaction semantics: single-table DML now buffers in explicit
  transactions, commits as one DuckLake snapshot, rolls back cleanly, and fails
  closed on stale-base conflicts (G10). Multi-table atomicity remains future
  hardening.
- JDBC path: extended-protocol prepared statements and Describe now work for the
  maintained pgjdbc/GeoServer smoke path; general fetch-size portal suspension
  remains deferred (G4, datafusion-postgres fork) until a trace requires it.
- GeoServer catalog/query gaps closed for datastore publish, WFS GetFeature, and
  WMS GetMap. Remaining hardening: WFS-T insert/update, geometry/geography
  parameter binding beyond current WKB paths, role/privilege metadata, and
  general pgjdbc `setFetchSize` portal suspension.
- Gate: QGIS edit-and-save suite; GeoServer PostGIS datastore publish, WMS
  GetMap PNG, WFS GetFeature count. WFS-T insert/update remains the next
  GeoServer trace.

### M5 — Spatial layout + performance — gate: pruning benchmark ✅ PREVIEW

Implemented preview direction: [DuckLake spatial-temporal layout](docs/DUCKLAKE_SPATIAL_LAYOUT.md).

- Auto-materialize hidden bbox, coarse spatial bucket, and spatial sort key on
  WKB-backed geometry tables at write time. Temporal bounds/buckets remain a
  future extension.
- Keep M5 WKB-first: compute layout columns from WKB in one write-batch pass;
  tag fields as `geoarrow.wkb` only for interoperability, not as a dependency for
  pruning.
- Support type tiers: first-class OGC simple-feature `geometry`/`geography` for
  SQL, high-fidelity CAD/BIM/reality-capture sidecars for curves/meshes/source
  objects, and asset-index rows for point clouds, COG rasters, and 3D tiles.
- Preserve coordinate fidelity metadata: full CRS definitions, vertical datums,
  coordinate/acquisition epoch, transform pipeline, accuracy, and tessellation
  tolerance so aerial/CAD data can be reprocessed as datums drift over time.
- Default to automatic spatial ordering: coarse geographic/projected buckets and
  table-local sort keys. Time-aware layout remains future hardening.
- Keep partition fanout bounded for trillion-row / 10 TB+ ingest: target large
  Parquet files, avoid per-feature partitions, and rely on sorted row groups plus
  file statistics for fine pruning.
- Support many writers without mutable global spatial indexes; writers produce
  independent files/snapshots, and explicit `CALL quackgis_compact_table(...)`
  is the current maintenance path. Bucket-local compaction remains future
  optimization.
- Scan-time pruning uses QuackGIS spatial predicate → bbox rewrite above
  DuckLake/Parquet statistics. Exact SedonaDB predicate recheck remains the
  correctness layer.
- LayoutBench `sf0` exact-vs-pruned oracle is in the local gate, and `sf1` local
  runs document COPY vs INSERT, ingest order, row-group, and compaction behavior
  in `benchmarks/`.

### Alpha — Scaled lakehouse storage + ops — gate: PostgreSQL/S3 multi-process probe

This is the next named milestone after the developer preview. It turns the
strategic storage contract into an externally credible platform path.

- PostgreSQL catalog + S3/object-store data profile configured from CLI/env and
  covered by integration tests. SQLite catalog + local files remains a first-class
  local/correctness profile.
- Multi-process deployment: many stateless QuackGIS readers against the same
  DuckLake catalog/data prefix, plus parallel ingest writers with documented
  DuckLake snapshot conflict/retry behavior.
- High-QPS read probe: parallel pgwire clients issuing selective spatial queries
  over layout-pruned DuckLake/Parquet data with stable latency/throughput metrics.
- OLAP fanout probe: scan many geometries/assets, compute grouped
  spatial/attribute statistics with primitive aggregates/calculations, verify
  projection/filter/aggregate pushdown evidence, and use those calculations to
  filter records for exact SedonaDB spatial recheck.
- Object-store operations: credentials/secrets, catalog backup/restore, Parquet
  prefix lifecycle, compaction scheduling, and failed-writer cleanup documented.
- SCRAM/auth, TLS-by-default option, and RBAC roles (readonly/readwrite) from
  datafusion-postgres hardened for non-local deployments.
- Slim runtime image: single binary + required runtime data; target < 100 MB
  (v0.1 was ~500 MB with PG + DuckDB + GDAL). Current Kind image path is the
  development base for this.
- Helm/Kubernetes production packaging remains deferred until the multi-process
  PostgreSQL/S3 probe is stable; current K8s smoke path is `deploy/kind/*`.

### M7 — Compatibility sprint (ongoing)

- Grow the client-trace fixture corpus (QGIS versions, GeoServer versions,
  pg_featureserv/martin as stretch clients).
- Curated PostGIS regress subset tracked in CI; fix by failure count.
- Delete remaining bespoke tests as trace/regress coverage supersedes them.

## Risk register

| Risk | Impact | Mitigation |
|---|---|---|
| datafusion-ducklake is alpha; PostgreSQL/S3 scaled path needs hardening (G6) | Blocks Alpha platform use | Keep SQLite/local correctness gates green, then build PostgreSQL catalog + S3 multi-process probes; fork datafusion-ducklake where needed and document conflict/retry semantics |
| Binary cursors / portal suspension gaps (G3/G4) | Blocks QGIS paging, GeoServer fetch size | Probe spikes at milestone start; implement in the datafusion-postgres fork — never blocked on upstream review |
| pg_catalog fidelity (G2 — QGIS queries are gnarly) | Layer discovery breaks | Replay captured traces against the datafusion-pg-catalog fork; fix in-fork same day |
| SedonaDB geometry Arrow encoding vs arrow-pg (G1) | Wire encoding bugs | Single encoding module in the arrow-pg fork, round-trip property tests |
| PG-catalog DuckLake writes are non-spec upstream (G6) | Multi-writer scaled profile deferred | Treat PostgreSQL catalog + S3 as Alpha, not an optional production afterthought; implement/fork to spec-compatible behavior and keep SQLite/local parity |
| Fork drift vs upstream velocity | Painful rebases, missed fixes | Pinned revs + `DIVERGENCE.md` per fork; rebase at milestone boundaries; upstream patches opportunistically to shrink the diff |

## Retired v0.1 assets

| Asset | Fate |
|---|---|
| `src/` DuckDB extension (`sedonadb`) | Retired — SedonaDB used natively; PostGIS rewriter (`sedonadb-migrate`) may be salvaged as a compat-layer module |
| `vendor/pg_ducklake`, `pg_geometry/` | Deleted (git history retains) |
| `container/init.d/` stubs, bridge table, DOMAIN geometry | Deleted — obsolete by design |
| PostGIS test harness (`tests/postgis_port/`) | Kept — re-pointed at the new server for the secondary metric |
| DuckLake layout SQL helpers | Re-implemented in Rust at M5 |
| KinD/BuildKit dev loop, Helm chart | Replaced — stale v0.1 deploy tree removed; current Kind smoke manifests live in `deploy/kind/*` |

## Current state

- M0 proper landed: real `quackgis-server` workspace crate; v0.1 stack retired
  from main; CI uses pure-Rust wire tests.
- Stack now targets DuckLake 1.0+ via `datafusion-ducklake` main HEAD (DF 54).
  Forks: `adonm/sedona-db@quackgis/df54`,
  `adonm/datafusion-postgres@quackgis/fixes`, `adonm/datafusion-ducklake@main`.
- Workspace toolchain now targets Rust 1.95 / edition 2024 to match the active
  fork stack and avoid downlevel edition constraints.
- M1 storage gate validated: CTAS, bare CREATE TABLE, INSERT SELECT/VALUES,
  UPDATE, DELETE, writer API roundtrip through pgwire, restart persistence,
  filter predicates, and WKB geometry persistence all green.
- M2/Martin compatibility gate is green, including real Martin binary E2E and
  **18/18** unmodified upstream Martin table fixtures.
- M3 QGIS read-path Kind probe is green with `qgis/qgis:ltr-questing` (QGIS
  3.44.11): `valid True`, `feature_count 2`, `fields ['id', 'name']`, and
  `features_read 2`. The provider connects, `public.points` resolves to
  DuckLake `quackgis.main.points`, `geometry_columns` exposes `public`, custom
  geometry OID / `pg_type` lookups resolve sentinels 90001/90002, synthetic
  `id` key metadata satisfies QGIS primary-key discovery, and
  `ST_AsBinary(geom, 'NDR')` feeds the binary cursor fetch path. Fork head
  `2c2e5d9` is pushed and consumed by `Cargo.lock`.
- Kind image refresh now builds the Rust binary on the host first and copies it
  into a runtime-only image, keeping normal Cargo `target/` caching in the dev
  loop. Use `just kind-build-image-container` for a clean container-native
  rebuild.
- M3/M4 OGR load/read Kind probe is green with GDAL's PostgreSQL driver:
  `ogrinfo` sees the WKB-backed layer, `ogr2ogr -f GeoJSON` reads two Point
  features back through pgwire, and `ogr2ogr -append -addfields` loads GeoJSON
  geometries/attributes through `PG_USE_COPY=NO` INSERT mode after
  `ALTER TABLE ... ADD COLUMN`.
- M4 GeoServer smoke is green with official `docker.osgeo.org/geoserver:3.0.0`:
  the Kind probe registers a PostGIS datastore, publishes a WKB-backed layer,
  verifies WFS GeoJSON (`wfs_point_count 2`), and verifies WMS returns PNG bytes
  (`wms_png_header 89504e470d0a1a0a`).
- The coherent developer preview is documented in
  `docs/DEVELOPER_PREVIEW.md`. `just preview-smoke` starts a temporary server and
  verifies CREATE TABLE, PostgreSQL text COPY FROM STDIN, WKB spatial query,
  explicit compaction, and stable results after compaction.
- The focused project direction is documented in `docs/PROJECT_DIRECTION.md`:
  platform/app developers, high-throughput spatial lakehouse workloads,
  PostGIS-compatible tools as the ecosystem interface, DuckLake SQL catalog +
  object/file storage as the durable contract, DuckDB-style columnar OLAP over
  spatial datasets, and Alpha as the PostgreSQL/S3 scaled-storage milestone.
- M5 spatial layout preview is implemented: hidden `_qg_*` WKB-derived layout
  columns, safe single-table spatial-predicate bbox rewrites with exact SedonaDB
  recheck, LayoutBench `sf0` oracle, COPY/INSERT ingest variants, and whole-table
  compaction via `CALL quackgis_compact_table(...)`.
- QGIS synthetic key metadata is schema-derived for conventional `id` fields:
  `pg_index.indexrelid`, `indkey`, key-column lookup, and `pg_get_indexdef()` now
  track the target table/column instead of hard-coding `points(id)`.
- CI artifact workflow is mise-backed: pushes to `main`/`v*` validate the pinned
  dev toolchain, publish GHCR runtime images on non-PR refs, upload Linux x86_64
  binaries, and attach binaries to `v*` GitHub Releases.

## Next logical steps

1. **Start Alpha scaled-storage work.** Add PostgreSQL catalog + S3/object-store
   configuration, integration tests, and a multi-process probe with parallel
   readers and writers against one DuckLake catalog/data prefix. Keep SQLite +
   local files as a first-class parity profile.
2. **Add the high-QPS spatial read probe.** Use LayoutBench tables and selective
   bbox/spatial predicates to measure parallel pgwire reader throughput, latency,
   pruning, bytes scanned, and conflict-free read scaling.
3. **Add the OLAP fanout probe.** Add LayoutBench queries that compute grouped
   spatial/attribute stats over many geometries/assets, prove projection/filter/
   aggregate pushdown evidence where available, and use calculated values to
   filter records before exact SedonaDB predicates.
4. **Keep compatibility assertions evidence-rich.** The nightly/manual
   `Compatibility probes` workflow now runs Kind QGIS read/render/identify/filter,
   QGIS edit, OGR, GeoServer WFS/WMS/WFS-T, and scheduled real OSM parity
   reports. Keep the uploaded probe logs as the compatibility record, and add new
   client trace gaps as small shared probe scripts under `deploy/kind/probes/`.
5. **Extend real OSM client coverage.** OGR multi-layer OSM copy/read parity now
   covers points, lines, and multipolygons. Next open those copied layers through
   QGIS, GeoServer, and Martin in the side-by-side matrix.
6. **Harden keyless-layer fallback across clients.** Schema-derived `id` key
   metadata and `_quackgis_rowid` fallback are in place; keep adding client
   traces that prove keyless read/edit identity through QGIS, OGR, and GeoServer.
7. **Extend M4 from traces.** Keep the green QGIS edit/save and GeoServer
   WFS/WMS/WFS-T probes as gates. Continue implementing only blocking
   SQL/protocol gaps discovered by real clients: pgjdbc fetch-size portals if
   required, geometry/geography write parameters, and remaining
   catalog/privilege metadata.
8. **Harden catalog-surface shims.** The simple-query router is now organized by
   PostgreSQL catalog surface. Keep migrating trace fixtures into surface-focused
   tests (`pg_class`, `pg_attribute`, `pg_index`, `pg_type`) before adding new
   client-specific branches.
9. **Harden the M5 layout preview.** Keep `sf0` as the exact-vs-pruned oracle,
   grow only safe predicate rewrites, add time layout when traces require it, and
   evolve whole-table compaction toward bucket-local maintenance.
10. **Keep deployment boring.** Keep the runtime image single-binary and native-
   dependency-free; reintroduce Helm only after the Kind smoke path covers QGIS,
   OGR, and GeoServer.
