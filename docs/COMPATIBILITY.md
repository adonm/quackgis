# Compatibility and limitations

Current compatibility claims for the wire-adaptor architecture. See
[DEVELOPER_PREVIEW.md](./DEVELOPER_PREVIEW.md) for the runnable preview contract
and [PROJECT_DIRECTION.md](./PROJECT_DIRECTION.md) for the focused product
direction: platform/application developers, high-throughput spatial lakehouse
queries, DuckDB-style columnar OLAP over spatial datasets, and PostGIS-compatible
tools as the ecosystem interface. See [COMPATIBILITY_MATRIX.md](./COMPATIBILITY_MATRIX.md)
for probed client versions. See [ROADMAP.md](../ROADMAP.md) for forward outcomes,
[ROADMAP_STATUS.md](./ROADMAP_STATUS.md) for active frontiers, and
[../DIVERGENCE.md](../DIVERGENCE.md) for forked upstream capabilities.

## Client compatibility targets

| Client | Status | Notes |
|---|---|---|
| `psql` | ✅ | simple + extended protocol via datafusion-postgres |
| psycopg (v3) | ✅ profile surface | text + binary geometry round-trip through pgwire/tokio-postgres and Kind Python profile probe; named psycopg dependency probe pending |
| QGIS (postgres provider) | ✅ read/render/identify/filter + edit smoke | Kind PyQGIS probe validates layer open, feature read, attribute filter, feature-id identify, and headless render; edit probe opens a keyless spatial layer and commits insert/update/delete/save through `_quackgis_rowid` |
| GDAL/OGR `ogr2ogr` (PostgreSQL driver) | ✅ load/read | Kind OGR probe seeds a WKB-backed layer, reads it with `ogrinfo`/GeoJSON export, then appends GeoJSON with `PG_USE_COPY=NO` + `-addfields` and verifies SQL + GeoJSON read-back including appended fields |
| GDAL/OGR bulk load path | ✅ COPY path | PostgreSQL text `COPY FROM STDIN` is implemented for simple and extended pgwire, including chunked `CopyData`, GDAL-style bytea/WKB hex and octal escapes, autocommit writes, and explicit single-table transactions. Maintained Kind OGR gate still covers INSERT append mode; COPY has focused Rust and preview-smoke coverage. |
| QGIS (editing) | ✅ smoke | `INSERT`/`UPDATE`/`DELETE ... RETURNING`, parameterized WKB edits, synthetic rowid metadata, and single-table explicit transaction rollback/commit pass wire regressions; Kind PyQGIS edit/save gate passes |
| GeoServer (PostGIS datastore) | ✅ WFS/WMS/WFS-T smoke | Official GeoServer 3.0.0 Kind probe registers a PostGIS datastore, publishes a WKB-backed layer, verifies WFS GeoJSON feature count, receives a WMS PNG, and performs real WFS-T insert/update/delete transactions; Rust wire tests keep trace-shaped DML coverage |
| Martin (MapLibre tile server) | ✅ | auto-discover geometry tables; MVT tile serving via ST_AsMVT; QuackGIS SQL probes cover layer/key/value dictionary tags while real OSM Martin-attribute matrices remain opt-in future evidence |
| SQLAlchemy/GeoPandas/API readers | ✅ maintained profile surface | `just api-client-local-smoke` and `just kind-api-client-probe` cover information_schema reflection, WKB row reads, bbox filters, grouped aggregates, and MVT bytes with representative attribute tags before named client containers |
| pg_featureserv | local bbox surface | real server remains trace-driven |
| `pg_dump` / logical replication | ❌ | back up the DuckLake catalog + Parquet instead |

## Wire protocol surface (datafusion-postgres)

| Feature | Status |
|---|---|
| Simple + extended query protocol | ✅ upstream |
| TLS, password auth, RBAC roles | TLS and SCRAM-SHA-256 password startup are wired; coarse read/write vs read-only DuckLake write authorization, matching `pg_roles`, and explicit-user privilege helper metadata are implemented. Full object-level RBAC remains production hardening; see `docs/SECURITY_RBAC.md`. |
| pg_catalog + information_schema emulation | ✅ upstream (datafusion-pg-catalog) |
| Portals / fetch-size suspension | general `setFetchSize` suspension is not implemented; maintained GeoServer WFS/WMS smoke does not require it; city-scale pgjdbc evidence is the decision gate |
| `DECLARE BINARY CURSOR` / `FETCH` | ✅ simple-query/libpq path; narrow PostgreSQL-driver extended cursor shim for OGR read; general extended-protocol FETCH remains unsupported |
| COPY subprotocol | ✅ PostgreSQL text `COPY FROM STDIN` for simple + extended pgwire; focused coverage for GDAL-style WKB/bytea escapes and chunked `CopyData` |
| LISTEN/NOTIFY, PL/pgSQL, triggers | ❌ non-goals |

## Spatial engine (SedonaDB)

- ST_* vector functions, geography, CRS propagation, spatial joins — per
  [SedonaDB docs](https://sedona.apache.org). Raster in progress upstream.
- PostGIS-compat aliases and `&&` / `<->` operator mapping are QuackGIS-owned.
- `ST_Extent(geom)` returns PostGIS-style `BOX(minx miny,maxx maxy)` text for
  WKB-backed geometry columns. `ST_EstimatedExtent(...)` is present and returns
  `NULL` until DuckLake/statistics-backed estimates are implemented, matching
  PostGIS' no-statistics fallback shape.
- `Find_SRID(schema, table, column)` is exposed for client metadata probes and
  currently mirrors `geometry_columns.srid = 0` for WKB-backed columns.
- Per-row EWKB SRID tags are preserved by `ST_SetSRID`, `ST_GeomFromEWKT`,
  `ST_MakeEnvelope(..., srid)`, and `ST_Transform(..., srid)`; `ST_SRID`
  reads those tags and returns `0` for untagged WKB.
- Function conformance is tracked in
  [POSTGIS_CONFORMANCE.md](./POSTGIS_CONFORMANCE.md). The release pgwire claim is
  `just postgis-regress`, a starter curated PostGIS regress subset that prints
  `postgis_regress_subset passed=<n> total=<n> pass_rate=<x>` for scheduled trend
  artifacts. Broader SQL portability fixtures remain function-kernel evidence
  until promoted by pgwire tests or client traces.

## Storage (DuckLake 1.0+ via datafusion-ducklake)

| Capability | Status |
|---|---|
| Dev path: SQLite catalog + local Parquet files | ✅ validated by persistence/restart tests |
| Scaled profile: PostgreSQL catalog + S3/object-store Parquet | ✅ Alpha Kind gate via `kind-lake-smoke`, `kind-lake-multipod-smoke`, `kind-write-smoke`, `kind-qps-smoke`, and `kind-olap-smoke`; not a production durability claim |
| Vendored datafusion-ducklake (DF54) | ✅ based on `adonm/datafusion-ducklake@117c0c5` plus QuackGIS atomic-mutation patches; see `DIVERGENCE.md` |
| SQL writes into DuckLake from pgwire | ✅ CTAS, bare CREATE TABLE, INSERT SELECT, INSERT VALUES with column mapping, autocommit UPDATE/DELETE, PostgreSQL text `COPY FROM STDIN`, plus simple/extended `INSERT`/`UPDATE`/`DELETE ... RETURNING` for edit-client refresh |
| PostgreSQL catalog writes | ⚠️ exercised by Alpha Kind gates; current multicatalog schema is library-specific/non-spec and needs managed-service plus reference-reader/export evidence |
| UPDATE / DELETE | ✅ Autocommit `DELETE` uses fork-backed atomic positional delete files; autocommit `UPDATE` stages replacement rows and commits delete+append metadata under one snapshot; `... RETURNING` preserves current result semantics. Explicit transactions still use staged rewrites. |
| Spatial layout | ✅ WKB-first hidden `_qg_*` bbox/bucket/sort columns are automatically materialized on spatial writes and hidden from client metadata |
| Spatial predicate pruning | ✅ Safe bbox rewrite for recognized single-table spatial predicates, plus simple temporal `BETWEEN` bucket prefilters when a recognized time column exists; exact SedonaDB recheck remains authoritative and unsupported predicates remain correct but may scan more |
| Columnar OLAP fanout | ✅ Alpha smoke for grouped spatial/attribute stats, primitive calculations, pruning/aggregate evidence, and candidate filtering before exact SedonaDB recheck; larger benchmark variants remain future hardening |
| Compaction | ✅ `CALL quackgis_compact_table('schema.table')` and alias `CALL quackgis_compact(...)` rewrite a table into layout order; optional `(time_bucket, space_bucket)` uses native bucket-scoped delete+pending-append metadata when row-lineage planning succeeds, falling back to an atomic full-table replacement for unsupported shapes. Local tests report visible-file, scan-byte, and row-group deltas for fragmented appends |
| DuckLake metadata table functions | ✅ `ducklake_snapshots()`, `ducklake_table_info()`, and `ducklake_list_files()` are exposed through pgwire for catalog inspection; CDC row table functions stay disabled until pgwire projection is safe |
| Snapshot time travel | ✅ simple `AS OF SNAPSHOT <id>`, `public.table(snapshot => <id>)`, or `public.table(snapshot_id => <id>)` selector for one-table reads; positional/timestamp selectors, protected retention, rollback integration, and CDC remain future; see `docs/SNAPSHOT_OPERATIONS.md` |
| Generic filter pushdown/pruning path | ✅ datafusion-ducklake declares inexact filter pushdown; QuackGIS adds spatial-layout rewrites above it |
| DuckDB-inlined data reads | ❌ — avoid inlining when writing from DuckDB |
| Object stores | ✅ local FS validated; S3-compatible storage exercised in Kind with `s3s-fs`; production object-store soak remains future hardening |
| Multi-modal asset footprint tables | ✅ starter raster/point-cloud/3D/CAD/BIM/aerial/reality-capture footprint/sidecar schemas documented; `footprint` columns are discoverable as geometry metadata |

Interop target: QuackGIS storage changes should remain forward-compatible with
official DuckLake 1.0+ and readable by reference DuckLake readers where
practical. SQLite/local remains the deterministic, spec-oriented single-catalog
path but is not yet a drop-in DuckDB-writable catalog. PostgreSQL/S3 is
a maintained QuackGIS Alpha profile, but its current multicatalog metadata is not
standard DuckLake. Kind gates exercise it for multi-pod readers, parallel writers,
snapshot conflict/retry, high-QPS selective reads, and grouped OLAP fanout;
managed-service and export/reference-reader proof remain release work.

## Known limitations (architecture)

- Explicit `BEGIN`/`COMMIT`/`ROLLBACK` now stage single-table DuckLake DML and the
  maintained `ALTER TABLE ... ADD COLUMN` edit workflow for
  edit sessions: `ROLLBACK` discards staged changes, and `COMMIT` publishes the
  final table through one DuckLake writer snapshot with optimistic conflict
  detection. This is not full PostgreSQL transaction emulation yet: other DDL and
  multi-table write transactions fail closed, and ordinary `SELECT` statements
  inside the transaction read the committed catalog rather than private staged
  rows.
- No PostgreSQL ctid. The catalog compatibility layer exposes a real
  conventional `id` column as a schema-derived synthetic unique index when
  present; keyless spatial layers get `_quackgis_rowid` metadata and a stable
  read projection for QGIS/GDAL feature identity. QGIS edit/save smoke tests now
  pass on `_quackgis_rowid`; multi-table transaction semantics remain future
  hardening.
- `CALL quackgis_compact_table(...)` whole-table mode commits through one
  replacement snapshot. Optional bucket arguments use native partial-file
  delete+append compaction when row-lineage planning succeeds, and fall back to
  the safe full-table replacement path for unsupported shapes.
- QuackGIS is not a document database or OLTP application database. It emulates
  enough transactional/catalog behavior for common GIS clients when possible, but
  the core workload is large analytical spatial and columnar OLAP queries over
  DuckLake/Parquet.
- Typmod enforcement (`geometry(Point, 4326)`) is metadata + EWKB-level, not
  PG typmod machinery.
