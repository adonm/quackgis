# Compatibility and limitations

Current compatibility claims for the wire-adaptor architecture. See
[DEVELOPER_PREVIEW.md](./DEVELOPER_PREVIEW.md) for the runnable preview contract
and [PROJECT_DIRECTION.md](./PROJECT_DIRECTION.md) for the focused product
direction: platform/application developers, high-throughput spatial lakehouse
queries, DuckDB-style columnar OLAP over spatial datasets, and PostGIS-compatible
tools as the ecosystem interface. See
[ROADMAP.md](../ROADMAP.md) for milestone history and open hardening.
`G#` references point to the upstream gap ledger in ROADMAP.md — capabilities
missing upstream that we build in tracked fork branches.

## Client compatibility targets

| Client | Target | Milestone | Notes |
|---|---|---|---|
| `psql` | ✅ | M0 | simple + extended protocol via datafusion-postgres |
| psycopg (v3) | ✅ | M2 | text + binary geometry round-trip |
| QGIS (postgres provider) | ✅ read/render/identify/filter + edit smoke | M3/M4 | Kind PyQGIS probe validates layer open, feature read, attribute filter, feature-id identify, and headless render; edit probe opens a keyless spatial layer and commits insert/update/delete/save through `_quackgis_rowid` |
| GDAL/OGR `ogr2ogr` (PostgreSQL driver) | ✅ load/read | M3/M4 | Kind OGR probe seeds a WKB-backed layer, reads it with `ogrinfo`/GeoJSON export, then appends GeoJSON with `PG_USE_COPY=NO` + `-addfields` and verifies SQL + GeoJSON read-back including appended fields |
| GDAL/OGR bulk load path | ✅ COPY path | M5 | PostgreSQL text `COPY FROM STDIN` is implemented for simple and extended pgwire, including chunked `CopyData`, GDAL-style bytea/WKB hex and octal escapes, autocommit writes, and explicit single-table transactions. Maintained Kind OGR gate still covers INSERT append mode; COPY has focused Rust and preview-smoke coverage. |
| QGIS (editing) | ✅ smoke | M4 | `INSERT`/`UPDATE`/`DELETE ... RETURNING`, parameterized WKB edits, synthetic rowid metadata, and single-table explicit transaction rollback/commit pass wire regressions; Kind PyQGIS edit/save gate passes |
| GeoServer (PostGIS datastore) | ✅ WFS/WMS/WFS-T smoke | M4 | Official GeoServer 3.0.0 Kind probe registers a PostGIS datastore, publishes a WKB-backed layer, verifies WFS GeoJSON feature count, receives a WMS PNG, and performs real WFS-T insert/update/delete transactions; Rust wire tests keep trace-shaped DML coverage |
| Martin (MapLibre tile server) | ✅ | M2+ | auto-discover geometry tables; MVT tile serving via ST_AsMVT |
| pg_featureserv | stretch | M7 | trace-driven |
| `pg_dump` / logical replication | ❌ | — | back up the DuckLake catalog + Parquet instead |

## Wire protocol surface (datafusion-postgres)

| Feature | Status |
|---|---|
| Simple + extended query protocol | ✅ upstream |
| TLS, password auth, RBAC roles | ✅ upstream |
| pg_catalog + information_schema emulation | ✅ upstream (datafusion-pg-catalog) |
| Portals / fetch-size suspension | general `setFetchSize` suspension still deferred (G4); maintained GeoServer WFS/WMS smoke does not require it |
| `DECLARE BINARY CURSOR` / `FETCH` | ✅ simple-query/libpq path; narrow PostgreSQL-driver extended cursor shim for OGR read; general extended-protocol FETCH still deferred (G3/G4) |
| COPY subprotocol | ✅ PostgreSQL text `COPY FROM STDIN` for simple + extended pgwire; focused coverage for GDAL-style WKB/bytea escapes and chunked `CopyData` |
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

| Capability | Status |
|---|---|
| Dev path: SQLite catalog + local Parquet files | ✅ validated in M1 tests |
| Scaled profile: PostgreSQL catalog + S3/object-store Parquet | 🎯 Alpha requirement; not claimed by current local preview |
| datafusion-ducklake main HEAD (DF54) | ✅ current integration target |
| SQL writes into DuckLake from pgwire | ✅ CTAS, bare CREATE TABLE, INSERT SELECT, INSERT VALUES with column mapping, UPDATE, DELETE (single-table/full-table rewrite), PostgreSQL text `COPY FROM STDIN`, plus simple/extended `INSERT`/`UPDATE`/`DELETE ... RETURNING` for edit-client refresh |
| PostgreSQL catalog writes | ⚠️ upstream path is experimental/non-spec; QuackGIS will extend/fork toward spec-compatible behavior (G6) |
| UPDATE / DELETE | ✅ QuackGIS full-table rewrite semantics for single-table statements; native delete files still future optimization |
| Spatial layout | ✅ WKB-first hidden `_qg_*` bbox/bucket/sort columns are automatically materialized on spatial writes and hidden from client metadata |
| Spatial predicate pruning | ✅ Safe bbox rewrite for recognized single-table spatial predicates with exact SedonaDB recheck; unsupported predicates remain correct but may scan more |
| Columnar OLAP fanout | 🎯 Alpha benchmark target: grouped spatial/attribute stats, primitive calculations, projection/filter/aggregate pushdown evidence, and candidate filtering before exact SedonaDB recheck |
| Compaction | ✅ `CALL quackgis_compact_table('schema.table')` and alias `CALL quackgis_compact(...)` rewrite a table into layout order; currently whole-table, not bucket-local |
| Snapshot time travel (SQL `AS OF`) | ❌ programmatic only (G8) |
| Generic filter pushdown/pruning path | ✅ datafusion-ducklake declares inexact filter pushdown; QuackGIS adds spatial-layout rewrites above it |
| DuckDB-inlined data reads | ❌ — avoid inlining when writing from DuckDB |
| Object stores | local FS validated; S3/object-store support is an Alpha scaled-storage requirement |

Interop target: QuackGIS storage changes should remain forward-compatible with
official DuckLake 1.0+ and readable by reference DuckLake readers where
practical. SQLite/local is the validated preview path. SQLite/local and
PostgreSQL/S3 are both first-class storage profiles; Alpha makes the
PostgreSQL/S3 profile real for multi-process readers/writers. Extending
datafusion-ducklake for that target is explicitly in scope.

## Known limitations (architecture)

- Explicit `BEGIN`/`COMMIT`/`ROLLBACK` now stage single-table DuckLake DML for
  edit sessions: `ROLLBACK` discards staged changes, and `COMMIT` publishes the
  final table through one DuckLake writer snapshot with optimistic conflict
  detection. This is not full PostgreSQL transaction emulation yet: DDL and
  multi-table write transactions fail closed, and ordinary `SELECT` statements
  inside the transaction read the committed catalog rather than private staged
  rows.
- No PostgreSQL ctid. The catalog compatibility layer exposes a real
  conventional `id` column as a schema-derived synthetic unique index when
  present; keyless spatial layers get `_quackgis_rowid` metadata and a stable
  read projection for QGIS/GDAL feature identity. QGIS edit/save smoke tests now
  pass on `_quackgis_rowid`; multi-table transaction semantics remain future
  hardening.
- `CALL quackgis_compact_table(...)` currently rewrites the whole table. It is
  the preview maintenance command for fragmented autocommit/INSERT layouts;
  bucket-local/incremental compaction is future optimization.
- QuackGIS is not a document database or OLTP application database. It emulates
  enough transactional/catalog behavior for common GIS clients when possible, but
  the core workload is large analytical spatial and columnar OLAP queries over
  DuckLake/Parquet.
- Typmod enforcement (`geometry(Point, 4326)`) is metadata + EWKB-level, not
  PG typmod machinery.
