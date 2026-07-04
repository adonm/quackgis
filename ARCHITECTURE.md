# Architecture

QuackGIS is a PostGIS-compatible spatial database facade. PostgreSQL provides
the client boundary. DuckDB + sedonadb execute spatial queries. DuckLake
stores data in Parquet.

## Layer model

```text
┌──────────────────────────────────────────────────────────────┐
│ PostgreSQL clients                                           │
│ psql · JDBC · psycopg · BI tools                              │
├──────────────────────────────────────────────────────────────┤
│ PostgreSQL + quackgis C extension                             │
│ pgwire · auth · geometry type · typmods · casts · catalog     │
├──────────────────────────────────────────────────────────────┤
│ pg_ducklake (vendored, modified)                              │
│ DuckLake table AM · DuckDB lifecycle · query routing           │
├──────────────────────────────────────────────────────────────┤
│ DuckDB + spatial extension + sedonadb                         │
│ GEOMETRY type · ST_* functions · GEOS · PROJ · GDAL · SedonaDB│
├──────────────────────────────────────────────────────────────┤
│ DuckLake storage                                              │
│ catalog metadata · Parquet files · snapshots · layout columns │
├──────────────────────────────────────────────────────────────┤
│ Infrastructure                                                │
│ PostgreSQL data volume · DuckLake data volume / S3            │
└──────────────────────────────────────────────────────────────┘
```

## Design principles

1. **Layer on DuckDB spatial, not replace it.** DuckDB's `spatial` extension
   provides the GEOMETRY type and ~100 common functions. Sedonadb adds what
   spatial lacks. No duplicate implementations.

2. **Real PG geometry type.** A thin C extension registers `geometry` as a
   PostgreSQL type with typmods, casts, and I/O. All computation in DuckDB.

3. **DuckLake is the only storage.** User data lives in Parquet via DuckLake
   table AM. PostgreSQL stores only catalog metadata.

4. **Transparent spatial partitioning.** CREATE TABLE with a geometry column
   auto-materializes layout columns (bbox, quadkey, Hilbert sort) for pruning.

5. **Rust-first owned code.** C only at the PG type boundary. Everything else
   is Rust (sedonadb) or reused upstream (pg_ducklake, DuckDB spatial).

## How spatial queries route

1. Client sends SQL to PostgreSQL.
2. If the query touches a DuckLake table, pg_ducklake routes it to DuckDB.
3. DuckDB has `spatial` extension (GEOMETRY type + common functions) and
   `sedonadb` (additional functions, GEOS topology, CRS, raster).
4. DuckDB executes the query, returns results.
5. pg_ducklake converts DuckDB types back to PostgreSQL types.

## DuckLake spatial layout

Spatial tables materialize deterministic layout columns:

| Column | Purpose |
|---|---|
| `minx/miny/maxx/maxy` | File-level zone-map pruning |
| `spatial_cell` (quadkey) | Partition pruning |
| `spatial_sort` (Hilbert) | Spatial clustering within files |

Query: cell prune → bbox prune → exact predicate. Stages 1–2 are performance;
stage 3 is correctness.

## Trust boundaries

1. **Client SQL**: PostgreSQL parses. Only high-confidence rewrites are automatic.
2. **Geometry**: WKB/EWKB validated at the DuckDB/sedonadb boundary.
3. **Storage**: DuckLake owns all metadata, files, snapshots.
4. **Credentials**: Object-store credentials are deployment secrets.

## Non-goals

- Custom pgwire server.
- PostgreSQL heap as primary storage.
- GiST planner hooks (use layout columns instead).
- Topology schema, Tiger geocoder, SFCGAL, PL/pgSQL rewriting.
