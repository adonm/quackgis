# Roadmap

## Goal

**QGIS and GeoServer connect to QuackGIS as if it were PostGIS — without
significant changes** — from a single Rust binary: datafusion-postgres (wire) +
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
| G1 | `geometry`/`geography` type OID + hex-EWKB/EWKB wire encoding for SedonaDB's WKB Arrow arrays | M2 | Partial — `postgis` feature exists but is wired to geodatafusion, not SedonaDB | **Fork `arrow-pg` + `datafusion-postgres`**; generalize the type-extension hook, register SedonaDB encodings |
| G2 | pg_catalog depth for QGIS introspection (pg_index, pg_am, regclass casts, `format_type`, array/oidvector columns) | M3 | **Probe (commit 99e3a7d): the common 5 tables + pg_index/pg_proc/pg_namespace/pg_database work natively on master** (counts: 69 pg_class, 617 pg_type, 684 pg_attribute, 164 pg_index). information_schema fully populated. **pg_roles stack overflow FIXED in adonm/datafusion-postgres@quackgis/fixes (commit 2c43dc6)** — blanket `PgCatalogContextProvider for Arc<T>` was self-recursing. Regression test `pg_roles_does_not_crash` added. `pg_postmaster_start_time()` and other metadata funcs still unregistered (small UDF additions later). | Fork active. Upstream PR candidate. Remaining work tracked for M3. |
| G3 | SQL cursors: `DECLARE ... BINARY CURSOR` / `FETCH FORWARD n` (QGIS feature paging) | M3 | **Cursors work on datafusion-postgres master for the simple-query / libpq path** (psql, QGIS, GDAL). Two sub-gaps remain (M0 wire test verified): (a) BINARY keyword is accepted but encoding stays hex-text bytea — `010100...` is valid WKB, QGIS reads it; ~2× bandwidth loss vs raw binary protocol. (b) FETCH via extended protocol (pgjdbc, tokio-postgres, psycopg3 with prepared FETCH) fails: `CursorStatementHook` stores a portal with no schema, FETCH emits DataRows without a matching RowDescription — `"DataRow field count does not match"`. | Fork `CursorStatementHook` (small): honor BINARY keyword for (a); emit/describe proper RowDescription for FETCH for (b). |
| G4 | Portal suspension honoring `Execute.max_rows` (JDBC `setFetchSize`, GeoServer) | M4 | **Extended-protocol PREPARE/EXECUTE verified working through SedonaDB** (M0 spike); `setFetchSize` portal suspension still needs pgjdbc probe | Probe with pgjdbc; fix in the datafusion-postgres fork |
| G5 | UPDATE/DELETE on DuckLake tables (delete files per spec) | M4 | **Missing** (upstream README: writes = create/append only) | **Fork `datafusion-ducklake`**, implement delete files + DataFusion DML plumbing |
| G6 | Spec-compliant single-catalog PostgreSQL writes | M4+ | Missing — PG writes only via experimental non-spec multi-catalog layout | Extend the ducklake fork; SQLite catalog unblocks single-node until then |
| G7 | File/partition pruning from DuckLake stats (bbox/quadkey layout) | M5 | **Missing** (no pruning upstream) | Implement in the ducklake fork: stats → `PruningPredicate`, spatial predicate → bbox rewrite in our layer |
| G8 | SQL time travel (`AS OF`) over DuckLake snapshots | M7 (nice-to-have) | Missing — programmatic snapshot selection only | Fork if/when prioritized |
| G9 | SedonaDB Rust crates consumable as a dependency | M0 | **Verified consumable** (M0 spike): not on crates.io but `sedona = { git = "...apache/sedona-db.git", package = "sedona" }` builds clean; `SedonaContext::new_local_interactive` registers the full catalog | Git-dependency pinned to a rev; bump on each SedonaDB release |
| G10 | Multi-statement transactions / rollback for edit sessions | M4 | Missing everywhere (DuckLake commits are per-snapshot). **BEGIN/COMMIT/ROLLBACK accepted as no-ops via v0.15 `TransactionStatementHook`** (M0 spike) | Own it in quackgis: buffer edit-session DML, commit as one DuckLake snapshot; document semantics |
| G11 | DataFusion version alignment SedonaDB ↔ datafusion-postgres | M0 | **Resolved** by fork-bump: `adonm/sedona-db@quackgis/df53` (commit `f274c942`, 8 mechanical files) aligns SedonaDB to DF 53 / Arrow 58 / object_store 0.13 to ride datafusion-postgres master. Upstream PR candidate. | Follow both upstream heads; rebaseline on each SedonaDB release. |
| G12 | Runtime libgeos (sedona-geos default feature) | M0 | **Verified (M0 spike)**: binary needs `libgeos_c.so.1` on `LD_LIBRARY_PATH` | Deploy requirement: install libgeos in container image; document for dev |

Fork mechanics: org forks consumed via `[patch.crates-io]` / pinned git revs;
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
  datafusion-pg-catalog / datafusion-ducklake, consumed via
  `[patch.crates-io]` pins; SedonaDB as pinned git dep (G9); `DIVERGENCE.md`
  convention in place.
- CI: cargo build/test + psql smoke test on the binary.

### M1 — DuckLake storage — gate: round-trip + restart persistence

- Register `DuckLakeCatalog` (datafusion-ducklake) as the default catalog.
  SQLite catalog for single-node; PostgreSQL catalog mode documented as
  experimental (upstream multi-catalog layout is non-spec, preview).
- Object stores: local FS + S3/MinIO via `object_store`.
- `CREATE TABLE` / CTAS / `INSERT INTO` mapped to the DuckLake writer
  (upstream supports SQLite CTAS+INSERT today; PG write path experimental).
- Geometry columns persisted as WKB + CRS in DuckLake column metadata;
  readable by DuckDB's `ducklake` extension (interop test in CI).
- Confirms/updates gap ledger rows G5–G8 against the pinned
  datafusion-ducklake revision; fork branches opened for what M4/M5 need.

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
  round-trips of all geometry types + SRID.

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

- Redesign adopted (this document). v0.1 facade (PG + pg_ducklake + DuckDB
  extension) validated through Phase B but retired — see git history and
  `CHANGELOG.md`.
- **M0 spike PASSED round 2** (`spike/m00-wire-spike/FINDINGS.md`): riding
  upstream datafusion-postgres master + `adonm/sedona-db@quackgis/df53` fork.
  Cursors work natively (G3 mostly closed); DF version alignment resolved by
  fork-bump (G11). Gates green: ST_AsText, ST_Area, ST_Intersects, extended
  protocol, DECLARE/FETCH/CLOSE, BEGIN/COMMIT.
- Forks stood up: `adonm/sedona-db` (active — DF 53 bump branch),
  `adonm/datafusion-postgres` (placeholder for G1/G3-BINARY fork work).
- Next action: M0 — retire v0.1, fold the spike into a real `quackgis-server`
  crate with `[patch.crates-io]` consumption of the forks.
