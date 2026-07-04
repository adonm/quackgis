# Compatibility and limitations

Targets for the wire-adaptor architecture (see [ROADMAP.md](../ROADMAP.md) for
milestone gates). Until a milestone lands, rows are **targets**, not claims.
`G#` references point to the upstream gap ledger in ROADMAP.md — capabilities
missing upstream that we build in pinned forks.

## Client compatibility targets

| Client | Target | Milestone | Notes |
|---|---|---|---|
| `psql` | ✅ | M0 | simple + extended protocol via datafusion-postgres |
| psycopg (v3) | ✅ | M2 | text + binary geometry round-trip |
| GDAL/OGR (`ogr2ogr`) | ✅ | M2 | PG driver load + read-back is the M2 gate |
| QGIS (postgres provider) | ✅ read | M3 | introspection, binary cursors, extents |
| QGIS (editing) | ✅ | M4 | needs DuckLake UPDATE/DELETE (upstream) |
| GeoServer (PostGIS datastore) | ✅ | M4 | JDBC extended protocol, WMS/WFS-T |
| pg_featureserv / martin | stretch | M7 | trace-driven |
| `pg_dump` / logical replication | ❌ | — | back up the DuckLake catalog + Parquet instead |

## Wire protocol surface (datafusion-postgres)

| Feature | Status |
|---|---|
| Simple + extended query protocol | ✅ upstream |
| TLS, password auth, RBAC roles | ✅ upstream |
| pg_catalog + information_schema emulation | ✅ upstream (datafusion-pg-catalog) |
| Portals / fetch-size suspension | probe; built in our fork if missing (G4, M4) |
| `DECLARE BINARY CURSOR` / `FETCH` | probe; built in our fork if missing (G3, M3) |
| COPY subprotocol | not planned initially; `ogr2ogr`/INSERT for bulk load |
| LISTEN/NOTIFY, PL/pgSQL, triggers | ❌ non-goals |

## Spatial engine (SedonaDB)

- ST_* vector functions, geography, CRS propagation, spatial joins — per
  [SedonaDB docs](https://sedona.apache.org). Raster in progress upstream.
- PostGIS-compat aliases and `&&` / `<->` operator mapping are quackgis-owned
  (M2).
- Function conformance tracked against a curated PostGIS regress subset
  (secondary metric).

## Storage (datafusion-ducklake)

| Capability | Status upstream |
|---|---|
| Read catalogs: DuckDB, SQLite, PostgreSQL, MySQL | ✅ |
| Write: SQLite catalog (spec-compliant), CTAS + INSERT | ✅ |
| Write: PostgreSQL catalog | ⚠️ experimental, non-spec multi-catalog layout (spec layout in our fork when prioritized, G6) |
| UPDATE / DELETE | ❌ upstream — built in our fork for M4 (G5) |
| Snapshot time travel (SQL `AS OF`) | ❌ programmatic only (G8) |
| Partition/file pruning | ❌ upstream — built in our fork for M5 (G7) |
| DuckDB-inlined data reads | ❌ — avoid inlining when writing from DuckDB |
| Object stores | local FS, S3/MinIO |

Interop: tables written by QuackGIS must stay readable by DuckDB's `ducklake`
extension (CI check from M1). Use the SQLite catalog for spec-compliant
single-node deployments; treat the PostgreSQL catalog as experimental until
upstream adopts a spec layout.

## Known limitations (architecture)

- Transactions are accepted (`BEGIN`/`COMMIT`) but DuckLake commits are
  per-statement snapshots; no multi-statement rollback initially.
- No ctid: tables without primary keys get a synthesized row-id for QGIS
  editing (M3).
- Typmod enforcement (`geometry(Point, 4326)`) is metadata + EWKB-level, not
  PG typmod machinery.
