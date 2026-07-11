# Compatibility and limitations

QuackGIS currently exposes an owned PostgreSQL wire edge over DuckDB Spatial and
official DuckLake. DataFusion and Sedona SQL execution are no longer part of the
runtime.

## Proven local contract

The required real-driver workflow proves:

- PostgreSQL simple and extended query framing through `pgwire`;
- SCRAM-SHA-256 startup and normalized read/write table allowlists;
- one structurally parsed `SELECT`, `CREATE TABLE`, `INSERT`, `UPDATE`, `DELETE`,
  `BEGIN`, `COMMIT`, or `ROLLBACK` statement;
- parameterized reads and mutations without SQL interpolation;
- PostgreSQL text `COPY FROM STDIN` for the maintained scalar and WKB types;
- portal paging, transaction isolation, disconnect rollback, restart, and reopen;
- Arrow result encoding for maintained scalar types; and
- 42 curated spatial cases sent with their original PostGIS spelling through the
  real server, using DuckDB Spatial plus bounded QuackGIS rewrites/macros.

Run the evidence:

```sh
mise run duckdb-bootstrap
mise exec -- just ci
```

## Current client status

| Client/surface | Current status |
|---|---|
| `psql` / PostgreSQL protocol clients | bounded simple/extended protocol supported |
| `tokio-postgres` | maintained real-driver integration client |
| PostgreSQL text COPY clients | bounded maintained type set supported |
| QGIS, GDAL/OGR, GeoServer, Martin | target; prior legacy traces must be rerun against DuckDB before being claimed |
| SQLAlchemy, GeoPandas, psycopg, pg_featureserv | target; named dependency workflows remain open |
| `pg_dump`, logical replication, PL/pgSQL, triggers, LISTEN/NOTIFY | unsupported/non-goals |

## Spatial contract

DuckDB Spatial owns geometry execution. Durable geometry transport is binary
WKB/EWKB through Arrow and pgwire. The server currently rewrites these maintained
function spellings without touching quoted SQL text or comments:

- `ST_MakePoint` → `ST_Point`;
- `ST_NumPoints` → `ST_NPoints`;
- `ST_GeomFromEWKT`, `ST_AsHEXEWKB`, `GeometryType`, `ST_GeometryType`,
  `ST_CurveToLine`, and `ST_HasArc` → bounded QuackGIS-owned DuckDB macros; and
- `postgis_lib_version()` / `postgis_version()` → compatibility markers.

The 57-case ledger currently classifies 31 native DuckDB cases, five rewrites,
six macros, 10 Rust/catalog-edge gaps, and five extension candidates. The first
42 execute through pgwire. SRID-preserving EWKB behavior, geography, dimensions,
general `ST_GeometryN`, extent/catalog helpers, MVT, and broad PostGIS catalog
surfaces remain open unless a focused test says otherwise.

## Deliberate runtime limits

- Local official DuckLake catalog and local data paths only.
- Exact pinned DuckDB library version/digest and preinstalled signed extensions.
- Results are materialized at the ADBC boundary.
- Native DuckDB statement cancellation is not wired through pgwire cancellation.
- COPY is bounded to 16 MiB and does not yet implement full PostgreSQL options,
  escaping, arrays, JSON, time zones, or every scalar type.
- `pg_catalog`, `information_schema`, geometry/geography OID discovery, and GIS
  client-specific metadata are incomplete.
- Shared PostgreSQL/object-storage DuckLake, multi-writer recovery, migration,
  production packaging, soak, and disaster-recovery evidence remain open.

See [DUCKDB_SPATIAL_GAP_LEDGER.md](./DUCKDB_SPATIAL_GAP_LEDGER.md) and
[ENGINE_CAPABILITY_LEDGER.md](./ENGINE_CAPABILITY_LEDGER.md) for detailed gaps.
