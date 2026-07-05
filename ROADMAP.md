# Roadmap

## Goal

**QGIS, GeoServer, and Martin connect to QuackGIS as if it were PostGIS —
without significant changes** — from a single Rust binary: datafusion-postgres (wire) +
SedonaDB (spatial) + DuckLake (storage). No PostgreSQL, no DuckDB.

## Success metrics

1. **Primary — client workflows.** Scripted end-to-end suites, run in CI:
   - *QGIS*: connect → browse schemas → add layer → render (feature paging) →
     identify → filter → edit (insert/update/delete) → save.
   - *GeoServer*: register PostGIS datastore → publish layer → WMS GetMap →
     WFS GetFeature (paged) → WFS-T insert/update.
   - *OGR/GDAL*: `ogr2ogr` load into QuackGIS and read back, both PG drivers.
2. **Secondary — PostGIS function conformance.** Pass rate on a curated subset
   of upstream PostGIS regress tests (function semantics, not PG internals).

## Strategy

1. **Wire compatibility, not Postgres.** v0.1 stacked PostgreSQL + vendored
   pg_ducklake + a C geometry type + a DuckDB extension to get a PG-compatible
   front door. The redesign serves pgwire directly from Rust over a SedonaDB
   `SessionContext`. Every layer that existed to host or bridge PG/DuckDB is
   deleted.
2. **Fork/vendor-preferred.** The three pillars are active Apache-2.0
   projects: datafusion-postgres (v0.16, pg_catalog + auth + TLS), SedonaDB
   (0.4, vector/raster/geography), datafusion-ducklake (0.3, alpha) — but
   several capabilities the best design needs **do not exist upstream yet**
   (see the gap ledger below). We do not block milestones on upstream review:
   pin exact revisions, and the moment a needed capability is missing, fork
   the crate and build it there. Upstreaming is opportunistic, done from the
   fork when convenient — never on the critical path.
3. **Client-trace-driven testing.** Capture the exact SQL QGIS, GeoServer, and
   OGR send (query logs), replay as fixtures, fix in priority order. PostGIS
   regress covers function semantics second.
4. **Transparent spatial layout.** Geometry tables get bbox/quadkey/Hilbert
   layout columns at write time; scans prune with them at read time.

## Upstream gap ledger

Capabilities the design needs that upstream may not provide. **Status** is
what we have verified from upstream docs/READMEs; unverified rows get a probe
spike at the start of the owning milestone. **Plan** default: fork the crate,
build the capability, ship from the fork.

| # | Capability | Needed by | Upstream status | Plan |
|---|---|---|---|---|
| G1 | `geometry`/`geography` type OID on the wire | M2 | **Deferred (lowest-maintenance decision).** `::geometry` casts preprocessed to `::bytea` in datafusion-postgres fork (commit 912823e). WKB IS bytea — the type OID only matters for pg_type introspection, not data transfer. Martin/QGIS read geometry bytes fine as bytea. A real OID can be added later if a client strictly requires type-based discovery. | Deferred — no functional gap today. |
| G2 | pg_catalog depth for QGIS introspection (pg_index, pg_am, regclass casts, `format_type`, array/oidvector columns) | M3 | **Probe (commit 99e3a7d): the common 5 tables + pg_index/pg_proc/pg_namespace/pg_database work natively on master** (counts: 69 pg_class, 617 pg_type, 684 pg_attribute, 164 pg_index). information_schema fully populated. **pg_roles stack overflow FIXED in adonm/datafusion-postgres@quackgis/fixes (commit 2c43dc6)** — blanket `PgCatalogContextProvider for Arc<T>` was self-recursing. Regression test `pg_roles_does_not_crash` added. `pg_postmaster_start_time()` and other metadata funcs still unregistered (small UDF additions later). | Fork active. Upstream PR candidate. Remaining work tracked for M3. |
| G3 | SQL cursors: `DECLARE ... BINARY CURSOR` / `FETCH FORWARD n` (QGIS feature paging) | M3 | **Cursors work for the simple-query/libpq path** (psql, QGIS, GDAL). **BINARY cursor format FIXED in `adonm/datafusion-postgres@quackgis/fixes` (commit `98b3865`)** — `DECLARE x BINARY CURSOR` now returns raw binary-protocol bytes instead of hex-text bytea. Regression test `binary_cursor_returns_raw_bytes` verifies the wire bytes are i64 BE for `SELECT 42`. **Remaining sub-gap**: FETCH via extended protocol (pgjdbc, tokio-postgres) still fails with `"DataRow field count does not match"` — deeper pgwire investigation; QGIS path unaffected. | Fork active (BINARY patch upstreamable). Extended-protocol FETCH deferred until a real client needs it. |
| G4 | Portal suspension honoring `Execute.max_rows` (JDBC `setFetchSize`, GeoServer) | M4 | **Extended-protocol PREPARE/EXECUTE verified working through SedonaDB** (M0 spike); `setFetchSize` portal suspension still needs pgjdbc probe | Probe with pgjdbc; fix in the datafusion-postgres fork |
| G5 | UPDATE/DELETE on DuckLake tables | M4 | **Basic single-table UPDATE/DELETE implemented in QuackGIS** via full-table rewrite/replace semantics over DuckLake writer API. Correct but not optimal; no delete files yet. | Native delete-file UPDATE/DELETE remains future optimization in datafusion-ducklake fork if performance requires it. |
| G6 | Spec-compliant single-catalog PostgreSQL writes | M4+ | Missing — PG writes only via experimental non-spec multi-catalog layout | Extend the ducklake fork; SQLite catalog unblocks single-node until then |
| G7 | File/partition pruning from DuckLake stats (bbox/quadkey layout) | M5 | **Generic predicate pushdown path validated**: datafusion-ducklake marks filters Inexact so Parquet can use stats and DataFusion reapplies filters. Spatial-layout pruning (bbox/quadkey/Hilbert) still missing. | Implement spatial layout columns + predicate rewrite at M5; fork datafusion-ducklake only if generic pruning hooks are insufficient. |
| G8 | SQL time travel (`AS OF`) over DuckLake snapshots | M7 (nice-to-have) | Missing — programmatic snapshot selection only | Fork if/when prioritized |
| G9 | SedonaDB Rust crates consumable as a dependency | M0 | **Verified consumable**: not on crates.io but git dependency works. QuackGIS consumes `adonm/sedona-db@quackgis/df54` to align with the DuckLake 1.0+ / DF54 stack. | Track upstream head through fork branch; rebaseline at milestones. |
| G10 | Multi-statement transactions / rollback for edit sessions | M4 | Missing everywhere (DuckLake commits are per-snapshot). **BEGIN/COMMIT/ROLLBACK accepted as no-ops via v0.15 `TransactionStatementHook`** (M0 spike) | Own it in quackgis: buffer edit-session DML, commit as one DuckLake snapshot; document semantics |
| G11 | DataFusion version alignment across SedonaDB ↔ datafusion-postgres ↔ datafusion-ducklake | M0/M1 | **Resolved for current stack** by fork-bumps: `adonm/sedona-db@quackgis/df54` and `adonm/datafusion-postgres@quackgis/fixes` align with `datafusion-ducklake` main (DF54 / Arrow58), the DuckLake 1.0+ target path. | Follow upstream heads through fork branches; rebaseline on each milestone. |
| G12 | Runtime libgeos (sedona-geos default feature) | M0 | **Verified (M0 spike)**: binary needs `libgeos_c.so.1` on `LD_LIBRARY_PATH` | Deploy requirement: install libgeos in container image; document for dev |
| G13 | Martin tile-server compatibility | M2 | **Done — real binary E2E green.** Martin v1.11.0 connects, discovers tables, and serves MVT tiles (`GET /points/0/0/0` → 200, 12-byte protobuf). All compatibility gaps closed: `PostGIS_Lib_Version()` ✅, `current_setting()` ✅, `geometry_columns` ✅ (dynamic catalog-scanning TableProvider), `spatial_ref_sys` ✅, `ST_AsMVT` ✅, `ST_AsMVTGeom` ✅, `ST_TileEnvelope` ✅ (3/4/5-arg overloads via Sedona WKB helpers), `ST_MakeEnvelope` ✅, `ST_Expand` ✅, `ST_CurveToLine` ✅, `&&` ✅, `::geometry` ✅, `ST_Transform` ✅ (pure-Rust). Fork carries: Martin table-discovery shortcut, function-discovery shortcut, JSONB `properties` encoding, named-`margin` → positional rewrite. | Closed. Feature-attribute MVT tags remain future work. |

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
- SQLite catalog + local Parquet storage wired through `StoragePaths`
  (`QUACKGIS_CATALOG_PATH`, `QUACKGIS_DATA_PATH`). Production target is PostgreSQL
  catalog + AWS S3; extending datafusion-ducklake for spec-compatible production
  behaviour is in scope.
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
- Gate: `ogr2ogr -f PostgreSQL` load + read-back; psycopg binary and text
  round-trips of all geometry types + SRID. Martin connects and discovers
  tables (no tile serve yet — tile serving is M3 once ST_AsMVT path is live).

### M3 — QGIS read path — gate: scripted QGIS browse/render/identify

- Fix the introspection surface QGIS's postgres provider uses: pg_class,
  pg_attribute, pg_type, pg_namespace, pg_index (key detection), regclass
  casts, `format_type`, `version()` — in the datafusion-pg-catalog fork (G2).
- Cursors: `DECLARE ... BINARY CURSOR` / `FETCH FORWARD n` (QGIS feature
  paging) — probe upstream, implement cursor→portal mapping in the
  datafusion-postgres fork (G3).
- Extent queries: `ST_Extent`, `ST_EstimatedExtent` (from DuckLake file
  stats — cheap and accurate).
- Unique-key strategy for tables without declared PKs (row-id synthesis),
  since there is no ctid.
- Gate: headless PyQGIS suite in CI — add connection, list layers, render,
  identify, attribute filter.

### M4 — Editing + GeoServer — gate: QGIS edit session, GeoServer WMS/WFS-T

- UPDATE/DELETE on DuckLake tables — delete-file support built in our
  datafusion-ducklake fork (G5; the highest-risk item, start earliest).
- `INSERT ... RETURNING` for QGIS/GeoServer feature creation.
- Edit-session transaction semantics: buffer DML, commit as one DuckLake
  snapshot (G10).
- JDBC path: extended-protocol prepared statements, Describe, fetch-size
  portal suspension (G4, datafusion-postgres fork), `bytea` + geometry
  binary params.
- Gate: QGIS edit-and-save suite; GeoServer PostGIS datastore publish, WMS
  GetMap image diff, WFS GetFeature paging, WFS-T insert/update.

### M5 — Spatial layout + performance — gate: pruning benchmark

- Auto-materialize `minx/miny/maxx/maxy`, `spatial_cell` (quadkey),
  `spatial_sort` (Hilbert) on geometry tables at write time.
- Scan-time pruning against DuckLake file statistics, built in the ducklake
  fork (G7); spatial predicate → bbox prune rewrite in the quackgis layer.
- SpatialBench + tile-rendering benchmark vs PostGIS baseline; publish
  numbers in `benchmarks/`.

### M6 — Ops + slim image — gate: released container

- SCRAM auth, TLS by default, RBAC roles (readonly/readwrite) from
  datafusion-postgres.
- Distroless/static image: single binary + PROJ data; target < 100 MB
  (v0.1 was ~500 MB with PG + DuckDB + GDAL).
- Backup/restore = catalog DB file + Parquet prefix; document snapshot
  workflow. Refresh `docs/OPERATIONS.md` (currently describes the v0.1
  stack).
- Helm chart updated for single-container deployment.

### M7 — Compatibility sprint (ongoing)

- Grow the client-trace fixture corpus (QGIS versions, GeoServer versions,
  pg_featureserv/martin as stretch clients).
- Curated PostGIS regress subset tracked in CI; fix by failure count.
- Delete remaining bespoke tests as trace/regress coverage supersedes them.

## Risk register

| Risk | Impact | Mitigation |
|---|---|---|
| datafusion-ducklake is alpha; no UPDATE/DELETE (G5) | Blocks M4 editing | Read-only QGIS/GeoServer (M3) ships first; delete files built in our fork starting M1; SQLite-catalog scope first |
| Binary cursors / portal suspension gaps (G3/G4) | Blocks QGIS paging, GeoServer fetch size | Probe spikes at milestone start; implement in the datafusion-postgres fork — never blocked on upstream review |
| pg_catalog fidelity (G2 — QGIS queries are gnarly) | Layer discovery breaks | Replay captured traces against the datafusion-pg-catalog fork; fix in-fork same day |
| SedonaDB geometry Arrow encoding vs arrow-pg (G1) | Wire encoding bugs | Single encoding module in the arrow-pg fork, round-trip property tests |
| PG-catalog DuckLake writes are non-spec upstream (G6) | Multi-writer prod deferred | SQLite catalog is spec-compliant; spec PG layout built in the ducklake fork when prioritized |
| Fork drift vs upstream velocity | Painful rebases, missed fixes | Pinned revs + `DIVERGENCE.md` per fork; rebase at milestone boundaries; upstream patches opportunistically to shrink the diff |

## Retired v0.1 assets

| Asset | Fate |
|---|---|
| `src/` DuckDB extension (`sedonadb`) | Retired — SedonaDB used natively; PostGIS rewriter (`sedonadb-migrate`) may be salvaged as a compat-layer module |
| `vendor/pg_ducklake`, `pg_geometry/` | Deleted (git history retains) |
| `container/init.d/` stubs, bridge table, DOMAIN geometry | Deleted — obsolete by design |
| PostGIS test harness (`tests/postgis_port/`) | Kept — re-pointed at the new server for the secondary metric |
| DuckLake layout SQL helpers | Re-implemented in Rust at M5 |
| KinD/BuildKit dev loop, Helm chart | Kept — simplified to single container at M6 |

## Current state

- M0 proper landed: real `quackgis-server` workspace crate; v0.1 stack retired
  from main; CI uses pure-Rust wire tests.
- Stack now targets DuckLake 1.0+ via `datafusion-ducklake` main HEAD (DF 54).
  Forks: `adonm/sedona-db@quackgis/df54`,
  `adonm/datafusion-postgres@quackgis/fixes`, `adonm/datafusion-ducklake@main`.
- M1 storage gate validated: CTAS, bare CREATE TABLE, INSERT SELECT/VALUES,
  UPDATE, DELETE, writer API roundtrip through pgwire, restart persistence,
  filter predicates, and WKB geometry persistence all green.
- Next action: M2 PostGIS surface (`geometry_columns`, `spatial_ref_sys`,
  `postgis_version()`), plus production PostgreSQL/S3 hardening when ready.
