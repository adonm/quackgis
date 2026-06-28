# Migrating from PostGIS to DuckDB `sedonadb`

This cookbook shows the mechanical rewrites needed to port PostGIS SQL to the
`sedonadb` DuckDB extension. Each pattern is tested in
`tests/postgis_port/cases/`.

## Quick reference

| PostGIS | DuckDB `sedonadb` | Notes |
|---|---|---|
| `ST_GeomFromText(wkt)` | `ST_GeomFromText(wkt)` | Identical. |
| `ST_Point(x, y)` | `ST_Point(x, y)` | Identical. |
| `geom && other` | bbox column predicate | See [bbox overlap](#1-bbox-overlap---). |
| `geom <-> other` (KNN) | `ORDER BY ST_Distance(…) LIMIT k` | See [KNN](#2-knn-nearest-neighbor---). |
| `'wkt'::geometry` | `ST_GeomFromText('wkt')` | See [casts](#3-casts-and-typmods). |
| `geometry(Point, 4326)` typmod | WKB BLOB + explicit `ST_SetSRID` | See [SRID](#3-casts-and-typmods). |
| `CREATE INDEX … USING gist` | bbox + partition-key columns | See [indexing](#4-indexing-and-spatial-joins). |
| `ST_Collect(g1, g2)` scalar | Aggregate only; use `ST_Multi` for promotion | See [aggregates](#5-aggregates). |

## 1. Bbox overlap (`&&`)

PostGIS uses the `&&` operator for bounding-box overlap, backed by GiST:

```sql
-- PostGIS
SELECT a.* FROM a JOIN b ON a.geom && b.geom;
```

DuckDB extensions cannot register binary operators, so the rewrite is explicit
bbox columns:

```sql
-- DuckDB: materialize bbox columns once
ALTER TABLE a ADD COLUMN xmin DOUBLE;
ALTER TABLE a ADD COLUMN ymin DOUBLE;
ALTER TABLE a ADD COLUMN xmax DOUBLE;
ALTER TABLE a ADD COLUMN ymax DOUBLE;
UPDATE a SET xmin = st_xmin(geom), ymin = st_ymin(geom),
             xmax = st_xmax(geom), ymax = st_ymax(geom);
-- same for b

-- Then join on bbox overlap (indexable by DuckDB's optimizer)
SELECT a.*
FROM a JOIN b
  ON a.xmax >= b.xmin AND a.xmin <= b.xmax
 AND a.ymax >= b.ymin AND a.ymin <= b.ymax
WHERE st_intersects(a.geom, b.geom);   -- exact predicate last
```

The bbox predicate is a **performance filter**; the `st_intersects` call is
**correctness**. Never drop the exact predicate.

## 2. KNN nearest neighbor (`<->`)

PostGIS uses `<->` for KNN search backed by GiST:

```sql
-- PostGIS
SELECT * FROM pts
ORDER BY geom <-> ST_Point(5, 5)
LIMIT 3;
```

DuckDB rewrite:

```sql
-- DuckDB
SELECT * FROM pts
ORDER BY st_distance(geom, st_point(5.0, 5.0))
LIMIT 3;
```

For large datasets, materialize bbox columns and add a coarse prefilter
(e.g., `WHERE st_x(geom) BETWEEN 4 AND 6 AND st_y(geom) BETWEEN 4 AND 6`)
before the exact `ORDER BY st_distance`.

## 3. Casts and typmods

PostGIS uses PostgreSQL casting and typmods:

```sql
-- PostGIS
SELECT 'POINT(1 2)'::geometry;
SELECT 'POINT(1 2)'::geometry(Point, 4326);
```

DuckDB has no `geometry` type or typmods. Rewrite to explicit constructors:

```sql
-- DuckDB
SELECT st_geomfromtext('POINT(1 2)');            -- WKB BLOB
SELECT st_geomfromtext('POINT(1 2)', 4326);      -- with SRID (PostGIS 2-arg form)
SELECT st_setsrid(st_geomfromtext('POINT(1 2)'), 4326);  -- or tag afterwards
```

SRID follows PostGIS semantics via an EWKB SRID tag on the blob: `ST_SRID`
reads it back (0 when untagged), geometry-producing functions propagate it,
`ST_AsEWKT(geom)` prints `SRID=n;…`, and `ST_Transform(geom, to_srid)` uses it
as the source CRS (NULL for untagged input — tag first with `ST_SetSRID`).

## 4. Indexing and spatial joins

PostGIS uses GiST indexes:

```sql
-- PostGIS
CREATE INDEX idx_geom ON my_table USING gist(geom);
```

DuckDB has no GiST. Instead, materialize layout columns for pruning:

```sql
-- DuckDB: materialize bbox + spatial cell + sort key
CREATE TABLE my_table_layouted AS
SELECT *,
       st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
       st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
       st_quadkey(geom, 8) AS spatial_cell    -- Milestone 9+
FROM my_table;

-- Partition in DuckLake
-- ALTER TABLE my_table_layouted SET PARTITIONED BY (spatial_cell);
```

See [ARCHITECTURE.md](../ARCHITECTURE.md) for the full DuckLake layout design
and the canonical three-stage query pattern.

## 5. Aggregates

| PostGIS aggregate | DuckDB | Notes |
|---|---|---|
| `ST_Collect(geom)` | `ST_Collect(geom)` | Aggregate only — no scalar overload. |
| `ST_Union(geom)` | `ST_Union_Agg(geom)` | Cascaded polygonal union. |
| `ST_MemUnion(geom)` | `ST_Union_Agg(geom)` | Same engine. |
| `ST_Envelope(geom)` agg | `ST_Envelope_Agg(geom)` | Bbox union. |
| `ST_MakeLine(geom)` agg | `ST_MakeLine_Agg(geom)` | Points → LineString. |
| `ST_Intersection(geom)` agg | `ST_Intersection_Agg(geom)` | Cascaded. |

`ST_Collect(g1, g2)` scalar is unavailable because DuckDB cannot overload
scalar and aggregate on the same name. Use `ST_Multi(g)` for promotion or
collect via subquery.

## 6. WKT/WKB and I/O

All PostGIS I/O functions port directly:

| PostGIS | DuckDB |
|---|---|
| `ST_AsText(geom)` | `ST_AsText(geom)` |
| `ST_AsBinary(geom)` | `ST_AsBinary(geom)` |
| `ST_AsEWKB(geom)` | `ST_AsEWKB(geom)` |
| `ST_AsEWKT(geom)` | `ST_AsEWKT(geom)` |
| `ST_AsGeoJSON(geom)` | `ST_AsGeoJSON(geom)` |
| `ST_AsHEXEWKB(geom)` | `ST_AsHEXEWKB(geom)` |

Backlog (not shipped): `ST_AsMVT`, `ST_AsTWKB`, `ST_AsKML` — use `ST_AsGeoJSON`
or `ST_AsBinary` today (see COMPATIBILITY.md). `ST_AsSVG` shipped in
Milestone 16.

## 7. Geography type

PostGIS has a separate `geography` type. This extension uses `geometry` (WKB
BLOB) with explicit sphere/spheroid functions:

```sql
-- PostGIS
SELECT ST_Distance(a::geography, b::geography);

-- DuckDB
SELECT st_distancesphere(a, b);           -- haversine on sphere
SELECT st_distancespheroid(a, b);         -- WGS84 (Karney/GeographicLib)
```

Custom spheroids use the PostGIS string form:
`st_distancespheroid(a, b, 'SPHEROID["GRS 1980",6378137,298.257222101]')`
(also `st_lengthspheroid`/`st_areaspheroid`; `rf = 0` gives a sphere).

## What does NOT port

| Feature | Reason |
|---|---|
| `&&`, `<->`, `<#>` operators | DuckDB C-API extensions cannot register operators. |
| GiST / R-tree planner hooks | DuckDB has no extension-visible join planner. |
| `geometry(T, SRID)` typmods | DuckDB has no runtime geometric type system. |
| Topology schema | PostgreSQL-specific subsystem. |
| Tiger geocoder / address standardizer | PostgreSQL-specific. |
| SFCGAL 3D solids | No mature Rust binding. |
| Raster `ST_MapAlgebra` expression language | DuckDB SQL is the expression engine. |

## 8. End-to-end migration workbook

### 8.1 Complete DDL + query migration

**PostGIS source:**

```sql
CREATE TABLE parcels (
    id   SERIAL PRIMARY KEY,
    geom geometry(Polygon, 4326),
    zone VARCHAR
);
CREATE INDEX idx_parcels_geom ON parcels USING gist(geom);

-- Find parcels in zone R1 within 500m of a point
SELECT p.id
FROM parcels p
WHERE p.zone = 'R1'
  AND st_dwithin(p.geom::geography, st_setsrid(st_point(-122.4, 37.7), 4326)::geography, 500);
```

**DuckDB + sedonadb rewrite:**

```sql
-- DDL: WKB BLOB + materialized layout columns
CREATE TABLE parcels AS
SELECT
    id,
    st_geomfromtext(wkt) AS geom,         -- geometry is BLOB (ISO WKB)
    zone,
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
    st_quadkey(geom, 10) AS spatial_cell,
    st_hilbert(geom, 12) AS spatial_sort
FROM source;

-- Query: geography cast → explicit spheroid distance (add the three-stage
-- cell/bbox prefilter from §8.2 when the table is DuckLake-partitioned)
SELECT p.id
FROM parcels p
WHERE p.zone = 'R1'
  AND st_distancespheroid(p.geom,
        st_setsrid(st_point(-122.4, 37.7), 4326)) <= 500;
```

Key changes: `geometry(Polygon, 4326)` → BLOB; `USING gist` → layout columns;
`::geography` → `st_distancespheroid`; `st_dwithin` on geography →
`st_distancespheroid(...) <= threshold`.

### 8.2 DuckLake layout migration

**PostGIS source:**

```sql
CREATE TABLE trips (
    id   SERIAL PRIMARY KEY,
    geom geometry(LineString, 4326)
);
CREATE INDEX idx_trips_geom ON trips USING gist(geom);

-- KNN: 5 nearest trips to a depot
SELECT * FROM trips
ORDER BY geom <-> st_setsrid(st_point(-73.9, 40.7), 4326)
LIMIT 5;
```

**DuckDB + DuckLake rewrite:**

```sql
ATTACH 'ducklake:warehouse.ducklake' AS dl (DATA_PATH 'warehouse/');

CREATE TABLE dl.trips AS
SELECT
    id, geom,
    st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
    st_xmax(geom) AS xmax, st_ymax(geom) AS ymax,
    st_quadkey(geom, 10) AS spatial_cell
FROM source
ORDER BY st_hilbert(geom, 12);                   -- cluster files spatially

ALTER TABLE dl.trips SET PARTITIONED BY (spatial_cell);

-- KNN: bbox prefilter + exact distance sort
SELECT *
FROM dl.trips
WHERE xmin BETWEEN -73.95 AND -73.85
  AND ymin BETWEEN  40.65 AND  40.75             -- bbox prefilter
ORDER BY st_distance(geom, st_setsrid(st_point(-73.9, 40.7), 4326))
LIMIT 5;
```

### 8.3 Invalid geometry handling

**PostGIS source:**

```sql
-- Find and repair invalid polygons
SELECT id, st_isvalid(geom), st_isvalidreason(geom), st_makevalid(geom)
FROM parcels
WHERE NOT st_isvalid(geom);
```

**DuckDB rewrite:**

```sql
-- Same function names; st_isvaliddetail returns a table
SELECT id, st_isvalid(geom), st_isvalidreason(geom), st_makevalid(geom)
FROM parcels
WHERE NOT st_isvalid(geom);

-- Or use the table function for detailed validity info:
SELECT p.id, v.valid, v.reason, v.geom
FROM parcels p,
     st_isvaliddetail(p.geom) AS v(valid, reason, geom)
WHERE NOT v.valid;
```

### 8.4 Raster sampling workflow

**PostGIS source:**

```sql
-- Sample elevation at point locations
SELECT p.id, st_value(rast, p.geom) AS elevation
FROM points p
JOIN rasters r ON st_intersects(p.geom, r.rast);
```

**DuckDB rewrite (SQL-native, no raster-returning facade):**

```sql
-- st_value is a scalar function: (path, band, x, y) → DOUBLE
SELECT p.id, st_value('elevation.tif', 1, st_x(p.geom), st_y(p.geom)) AS elevation
FROM points p;
```

For bulk pixel extraction, use `st_pixeldata(path, band)` which streams
`(row, col, value)` rows that can be filtered by computed geographic bbox.

### 8.5 Automated rewriter

Use `tools/postgis_rewriter.py` to scan SQL files for PostGIS-specific patterns:

```bash
python3 tools/postgis_rewriter.py my_query.sql
```

Output is annotated with line-level warnings and suggested rewrites.
High-confidence patterns (operators, casts, GiST) have mechanical rewrites;
low-confidence patterns (`ST_Union` aggregate vs scalar, output formats) are
flagged for human review.

Roadmap direction: move this logic into a shared Rust `sqlparser-rs` AST
rewriter exposed as a CLI (`sedonadb-rewrite`) and SQL helper functions. The
regex tool is useful for quick linting, but AST rewriting is the intended path
for complex operands like `a.geom && b.geom` and nested expressions.
