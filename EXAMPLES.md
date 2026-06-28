# Copy-pasteable workflows

Every example below works against a DuckDB session with the sedonadb extension
loaded. Replace `'/abs/path/sedonadb.duckdb_extension'` with your build path.

## Quick start

```sql
LOAD '/abs/path/sedonadb.duckdb_extension';

-- All ST_* functions consume and return BLOB (ISO WKB).
SELECT st_astext(st_geomfromtext('POINT(1 2)'));          -- POINT(1 2)
SELECT st_area(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))'));  -- 16.0
SELECT st_distance(st_geomfromtext('POINT(0 0)'),
                   st_geomfromtext('POINT(3 4)'));          -- 5.0
```

## 1. GeoParquet ingest

DuckDB reads Parquet natively; geometry columns are WKB BLOB.

```sql
-- Create a persistent database and load GeoParquet data.
ATTACH 'spatial.db' AS db;
CREATE TABLE db.buildings AS
    SELECT * FROM read_parquet('buildings.parquet');
-- The geometry column is now a BLOB containing ISO-WKB.

-- Verify the data loads.
SELECT count(*), st_geometrytype(geom) AS geom_type
FROM   db.buildings
GROUP  BY geom_type
LIMIT  5;
```

## 2. CRS transform + spatial join (bbox prefilter)

No GiST index — use a bounding-box prefilter so DuckDB's IEJoin planner can
prune the candidate set, then apply the exact predicate.

```sql
-- Step 1: materialize bbox columns once per table.
CREATE TABLE db.buildings_bbox AS
    SELECT *, st_xmin(geom) AS _xmin, st_xmax(geom) AS _xmax,
              st_ymin(geom) AS _ymin, st_ymax(geom) AS _ymax
    FROM   db.buildings;

CREATE TABLE db.zones_bbox AS
    SELECT *, st_xmin(geom) AS _xmin, st_xmax(geom) AS _xmax,
              st_ymin(geom) AS _ymin, st_ymax(geom) AS _ymax
    FROM   db.zones;

-- Step 2: bbox prefilter + exact predicate.
SELECT    b.id AS building_id, z.id AS zone_id
FROM      db.buildings_bbox b
JOIN      db.zones_bbox     z
  ON      b._xmin <= z._xmax AND b._xmax >= z._xmin
  AND     b._ymin <= z._ymax AND b._ymax >= z._ymin
WHERE     st_intersects(st_transform(b.geom, 4326, 3857), z.geom);
```

## 3. Geodesic distance (metres)

No projection needed — lon/lat points go straight to the sphere/spheroid
functions.

```sql
-- London → Paris, great-circle distance in metres.
SELECT st_distancesphere(st_point(-0.1278, 51.5074),
                         st_point(2.3522, 48.8566));
-- → ~343,520 m

-- WGS84 spheroid (antipodal-safe, Karney/GeographicLib).
SELECT st_distancespheroid(st_point(-0.1278, 51.5074),
                           st_point(2.3522, 48.8566));
-- → ~343,483 m
```

## 4. Dissolve by category

```sql
-- Merge all polygons per category into one geometry.
SELECT category,
       st_union_agg(geom) AS merged_geom
FROM   db.parcels
GROUP  BY category;
```

## 5. Dump to atomic geometries

```sql
-- Explode multi-geometries into one row per atomic geometry.
SELECT t.id, (d).path, st_astext((d).geom) AS wkt
FROM   (SELECT id, st_dump(geom) AS d FROM db.collections) t;

-- One row per vertex (with navigation paths).
SELECT t.id, d.path, st_astext(d.geom) AS point
FROM   my_table t, st_dumppoints(t.geom) d;
```

## 6. Raster point sampling

```sql
-- Sample the pixel value at a geographic coordinate.
SELECT st_value('elevation.tif', 1, 12.5, 42.3);
-- → e.g. 342.0

-- Sample many points from a table.
SELECT p.id, st_value('elevation.tif', 1, p.x, p.y) AS elev
FROM   sample_points p;
```

## 7. Raster reclassification (map algebra via SQL)

```sql
-- Reclassify pixels: low/mid/high based on value thresholds.
SELECT row, col,
       CASE WHEN value > 80 THEN 3   -- high
            WHEN value > 40 THEN 2   -- mid
            ELSE 1                   -- low
       END AS reclass
FROM   st_pixeldata('elevation.tif', 1)
WHERE  value IS NOT NULL;
```

## 8. Raster clip by bounding box

```sql
-- Select pixels within a geographic bounding box.
WITH transform AS (
    SELECT origin_x, origin_y, pixel_w, pixel_h, row_rot, col_rot
    FROM   st_raster_transform('elevation.tif')
),
clipped AS (
    SELECT p.row, p.col, p.value,
           t.origin_x + p.col * t.pixel_w + p.row * t.row_rot AS x,
           t.origin_y + p.col * t.col_rot + p.row * t.pixel_h AS y
    FROM   st_pixeldata('elevation.tif', 1) p
    CROSS  JOIN transform t
    WHERE  t.origin_x + p.col * t.pixel_w + p.row * t.row_rot BETWEEN 10 AND 20
      AND  t.origin_y + p.col * t.col_rot + p.row * t.pixel_h BETWEEN 40 AND 50
      AND  p.value IS NOT NULL
)
SELECT min(value), max(value), avg(value), count(*) FROM clipped;
```

## 9. sedona_join — R-tree spatial join over Parquet

When the bbox-prefilter approach is too slow (or you want the SedonaDB
disk-spill model), spill both tables to Parquet and use `sedona_join`.

```sql
-- Spill tables to Parquet (geometry must be the last BLOB column).
COPY (SELECT id, geom FROM a) TO 'a.parquet';
COPY (SELECT id, geom FROM b) TO 'b.parquet';

-- R-tree join — predicates: intersects | contains | within | covers
--               | disjoint | equals | touches | crosses | overlaps | dwithin
SELECT * FROM sedona_join('a.parquet', 'b.parquet', 'intersects');
```

## 10. Topology workflows (GEOS-backed)

```sql
-- Node a set of crossing lines.
SELECT st_node(geom) FROM crossings;

-- Polygonize noded lines.
SELECT st_polygonize(st_node(geom)) FROM crossings;

-- Build area from lines (holes preserved).
SELECT st_buildarea(geom) FROM rings;

-- Voronoi diagram from points.
SELECT st_voronoipolygons(geom)
FROM   (SELECT st_collect(geom) AS geom FROM points) t;

-- Snap one geometry to another within tolerance.
SELECT st_snap(geom, ref_geom, 0.001) FROM survey;

-- Repair invalid geometry (canonical PostGIS engine).
SELECT st_makevalid(geom) FROM invalid_polygons;
```

## 11. Literal SedonaDB bridge — provenance comparison

```sql
-- For routed functions, both names use the same literal SedonaDB kernel.
SELECT st_dimension(geom);
SELECT sedona_st_dimension(geom);
-- Both return identical results — the literal kernel IS the implementation.

-- For unrouted functions, sedona_st_* is the literal kernel and st_* is the
-- local geo-crate reimplementation.
SELECT st_centroid(geom) AS local;        -- geo::Centroid
SELECT sedona_st_envelope(geom) AS literal; -- SedonaDB kernel
```
