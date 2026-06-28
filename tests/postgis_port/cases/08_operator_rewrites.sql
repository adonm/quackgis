.mode list
-- Port family: operator and PostgreSQL-ism rewrites
-- Tests the documented mechanical rewrites from PostGIS operators to DuckDB SQL.
-- See docs/MIGRATION.md for the full cookbook.

-- ── && (bbox overlaps operator) → bbox column predicate ──────────────
-- PG:  SELECT * FROM t1, t2 WHERE t1.geom && t2.geom;
-- DuckDB: materialize bbox columns and join on overlapping ranges.
--
-- This case verifies the bbox-overlap predicate produces correct results.
CREATE TEMP TABLE poly_a AS
    SELECT st_geomfromtext('POLYGON((0 0,3 0,3 3,0 3,0 0))') AS geom,
           st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
           st_xmax(geom) AS xmax, st_ymax(geom) AS ymax;

CREATE TEMP TABLE poly_b AS
    SELECT st_geomfromtext('POLYGON((2 2,5 2,5 5,2 5,2 2))') AS geom,
           st_xmin(geom) AS xmin, st_ymin(geom) AS ymin,
           st_xmax(geom) AS xmax, st_ymax(geom) AS ymax;

-- bbox overlap (the && rewrite)
SELECT CASE WHEN EXISTS (
    SELECT 1 FROM poly_a a, poly_b b
    WHERE a.xmax >= b.xmin AND a.xmin <= b.xmax
      AND a.ymax >= b.ymin AND a.ymin <= b.ymax
)
THEN 'PASS rewrite_bbox_overlap' ELSE 'FAIL rewrite_bbox_overlap' END;

-- exact predicate agrees
SELECT CASE WHEN EXISTS (
    SELECT 1 FROM poly_a a, poly_b b WHERE st_intersects(a.geom, b.geom)
)
THEN 'PASS rewrite_exact_agrees' ELSE 'FAIL rewrite_exact_agrees' END;

-- ── <-> (KNN distance operator) → ORDER BY st_distance + LIMIT ────────
-- PG:  SELECT * FROM pts ORDER BY geom <-> ST_Point(5,5) LIMIT 2;
-- DuckDB: ORDER BY st_distance(geom, query_point) + LIMIT
CREATE TEMP TABLE pts AS
    SELECT st_point(x::double, y::double) AS geom
    FROM (VALUES (0,0), (1,1), (10,10), (4,4)) v(x, y);

SELECT CASE WHEN (
    SELECT st_astext(geom) FROM pts
    ORDER BY st_distance(geom, st_point(5.0, 5.0))
    LIMIT 1
) LIKE 'POINT(4 4)'
THEN 'PASS rewrite_knn_nearest' ELSE 'FAIL rewrite_knn_nearest' END;

-- ── ::geometry cast → WKB BLOB ───────────────────────────────────────
-- PG:  SELECT ST_AsText('POINT(1 2)'::geometry);
-- DuckDB: SELECT ST_AsText(ST_GeomFromText('POINT(1 2)'));
SELECT CASE WHEN st_astext(st_geomfromtext('POINT(1 2)')) = 'POINT(1 2)'
THEN 'PASS rewrite_cast_geomfromtext' ELSE 'FAIL rewrite_cast_geomfromtext' END;

-- ── SRID typmod → explicit ST_SetSRID ─────────────────────────────────
-- PG:  SELECT ST_AsEWKT(ST_SetSRID(ST_Point(1,2), 4326));
-- Expected: SRID=4326;POINT(1 2)
-- DuckDB: SRID is type-level in SedonaDB bridge, not embedded in ISO WKB.
-- The geometry column is SRID-less WKB; use ST_AsEWKT for EWKT output.
SELECT CASE WHEN st_asewkt(st_geomfromtext('POINT(1 2)'), 0) LIKE '%POINT(1 2)'
THEN 'PASS rewrite_srid_ewkt' ELSE 'FAIL rewrite_srid_ewkt' END;

-- ── ST_Distance sphere ───────────────────────────────────────────────
-- PG:  SELECT ST_DistanceSphere(ST_Point(0,0), ST_Point(0,1));
-- Expected: ≈ 111195 (1 degree of latitude ≈ ~111km)
SELECT CASE WHEN abs(st_distancesphere(st_point(0.0, 0.0), st_point(0.0, 1.0)) - 111195.0) < 500
THEN 'PASS rewrite_distance_sphere' ELSE 'FAIL rewrite_distance_sphere' END;
