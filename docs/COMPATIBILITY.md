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
| GDAL/OGR `ogr2ogr` (PostgreSQL driver) | ✅ read | M3 | Kind OGR probe green: WKB-backed table seeded over pgwire, `ogrinfo`, and GeoJSON read-back through PostgreSQL-driver cursors |
| GDAL/OGR write/load | target | M4 | close `ALTER TABLE ... ADD COLUMN`, COPY/insert mode, and append metadata before promoting to load+read gate |
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
| COPY subprotocol | not planned initially; `ogr2ogr`/INSERT for bulk load |
| LISTEN/NOTIFY, PL/pgSQL, triggers | ❌ non-goals |

## Spatial engine (SedonaDB)

- ST_* vector functions, geography, CRS propagation, spatial joins — per
  [SedonaDB docs](https://sedona.apache.org). Raster in progress upstream.
- PostGIS-compat aliases and `&&` / `<->` operator mapping are quackgis-owned
  (M2).
- Function conformance tracked against a curated PostGIS regress subset
  (secondary metric).

## Storage (DuckLake 1.0+ via datafusion-ducklake)

| Capability | Status upstream |
|---|---|
| Dev path: SQLite catalog + local Parquet files | ✅ validated in M1 tests |
| Production target: PostgreSQL catalog + AWS S3 Parquet | 🎯 priority target |
| datafusion-ducklake main HEAD (DF54) | ✅ current integration target |
| SQL writes into DuckLake from pgwire | ✅ CTAS, bare CREATE TABLE, INSERT SELECT, INSERT VALUES with column mapping, UPDATE, DELETE (single-table/full-table rewrite) |
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
- No ctid: the catalog compatibility layer exposes a conventional `id` column as
  a synthetic unique index for current read probes; general row-id synthesis for
  editing remains M4.
- Typmod enforcement (`geometry(Point, 4326)`) is metadata + EWKB-level, not
  PG typmod machinery.
