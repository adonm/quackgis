# Compatibility and limitations

Targets for the wire-adaptor architecture (see [ROADMAP.md](../ROADMAP.md) for
milestone gates). Until a milestone lands, rows are **targets**, not claims.
`G#` references point to the upstream gap ledger in ROADMAP.md — capabilities
missing upstream that we build in tracked fork branches.

## Client compatibility targets

| Client | Target | Milestone | Notes |
|---|---|---|---|
| `psql` | ✅ | M0 | simple + extended protocol via datafusion-postgres |
| psycopg (v3) | ✅ | M2 | text + binary geometry round-trip |
| QGIS (postgres provider) | ✅ read | M3 | Kind PyQGIS add-layer probe green: valid layer, attributes, and two WKB point features fetched through binary cursor |
| GDAL/OGR `ogr2ogr` (PostgreSQL driver) | ✅ load/read | M3/M4 | Kind OGR probe seeds a WKB-backed layer, reads it with `ogrinfo`/GeoJSON export, then appends GeoJSON with `PG_USE_COPY=NO` + `-addfields` and verifies SQL + GeoJSON read-back |
| GDAL/OGR write/load hardening | target | M4 | COPY subprotocol and schema-derived append metadata remain future hardening; current maintained gate uses OGR INSERT mode |
| QGIS (editing) | target | M4 | basic UPDATE/DELETE storage rewrite works; client workflow + RETURNING/keys still pending |
| GeoServer (PostGIS datastore) | target | M4 | JDBC extended protocol, WMS/WFS-T; trace/probe next after QGIS+OGR read gates |
| Martin (MapLibre tile server) | ✅ | M2+ | auto-discover geometry tables; MVT tile serving via ST_AsMVT |
| pg_featureserv | stretch | M7 | trace-driven |
| `pg_dump` / logical replication | ❌ | — | back up the DuckLake catalog + Parquet instead |

## Wire protocol surface (datafusion-postgres)

| Feature | Status |
|---|---|
| Simple + extended query protocol | ✅ upstream |
| TLS, password auth, RBAC roles | ✅ upstream |
| pg_catalog + information_schema emulation | ✅ upstream (datafusion-pg-catalog) |
| Portals / fetch-size suspension | target for pgjdbc/GeoServer (G4, M4) |
| `DECLARE BINARY CURSOR` / `FETCH` | ✅ simple-query/libpq path; narrow PostgreSQL-driver extended cursor shim for OGR read; general extended-protocol FETCH still deferred (G3/G4) |
| COPY subprotocol | deferred; maintained `ogr2ogr` gate uses `PG_USE_COPY=NO` INSERT mode |
| LISTEN/NOTIFY, PL/pgSQL, triggers | ❌ non-goals |

## Spatial engine (SedonaDB)

- ST_* vector functions, geography, CRS propagation, spatial joins — per
  [SedonaDB docs](https://sedona.apache.org). Raster in progress upstream.
- PostGIS-compat aliases and `&&` / `<->` operator mapping are quackgis-owned
  (M2).
- `ST_Extent(geom)` returns PostGIS-style `BOX(minx miny,maxx maxy)` text for
  WKB-backed geometry columns. `ST_EstimatedExtent(...)` is present and returns
  `NULL` until DuckLake/statistics-backed estimates are implemented, matching
  PostGIS' no-statistics fallback shape.
- `Find_SRID(schema, table, column)` is exposed for client metadata probes and
  currently mirrors `geometry_columns.srid = 0` for WKB-backed columns.
- Per-row EWKB SRID tags are preserved by `ST_SetSRID`, `ST_GeomFromEWKT`,
  `ST_MakeEnvelope(..., srid)`, and `ST_Transform(..., srid)`; `ST_SRID`
  reads those tags and returns `0` for untagged WKB.
- Function conformance tracked against a curated PostGIS regress subset
  (secondary metric).

## Storage (DuckLake 1.0+ via datafusion-ducklake)

| Capability | Status upstream |
|---|---|
| Dev path: SQLite catalog + local Parquet files | ✅ validated in M1 tests |
| Production target: PostgreSQL catalog + AWS S3 Parquet | 🎯 priority target |
| datafusion-ducklake main HEAD (DF54) | ✅ current integration target |
| SQL writes into DuckLake from pgwire | ✅ CTAS, bare CREATE TABLE, INSERT SELECT, INSERT VALUES with column mapping, UPDATE, DELETE (single-table/full-table rewrite), plus simple/extended `INSERT`/`UPDATE`/`DELETE ... RETURNING` for edit-client refresh |
| PostgreSQL catalog writes | ⚠️ upstream path is experimental/non-spec; QuackGIS will extend/fork toward spec-compatible behavior (G6) |
| UPDATE / DELETE | ✅ QuackGIS full-table rewrite semantics for single-table statements; native delete files still future optimization |
| Snapshot time travel (SQL `AS OF`) | ❌ programmatic only (G8) |
| Generic filter pushdown/pruning path | ✅ datafusion-ducklake declares inexact filter pushdown; predicate path covered by tests. Spatial layout pruning remains M5 |
| DuckDB-inlined data reads | ❌ — avoid inlining when writing from DuckDB |
| Object stores | local FS validated; S3/AWS production target |

Interop target: QuackGIS storage changes should remain forward-compatible with official DuckLake 1.0+ and readable by reference DuckLake readers where practical. SQLite/local is the validated dev path. PostgreSQL/S3 is the production target; extending datafusion-ducklake for that target is explicitly in scope.

## Known limitations (architecture)

- Transactions are accepted (`BEGIN`/`COMMIT`) but DuckLake commits are
  per-statement snapshots; no multi-statement rollback initially.
- No PostgreSQL ctid. The catalog compatibility layer exposes a real
  conventional `id` column as a schema-derived synthetic unique index when
  present; keyless spatial layers get `_quackgis_rowid` metadata and a stable
  read projection for QGIS/GDAL feature identity. Edit-session row-id semantics
  still need M4 transaction/RETURNING hardening.
- Typmod enforcement (`geometry(Point, 4326)`) is metadata + EWKB-level, not
  PG typmod machinery.
