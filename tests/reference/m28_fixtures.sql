-- m28_fixtures.sql — migration assistant round-trip: rewrite PostGIS SQL via
-- sedonadb_rewrite_postgis(), verify the rewritten SQL is textually correct,
-- then execute equivalent DuckDB SQL to prove the semantics are preserved.
.mode list

-- ======================================================================
-- 1. Operator overlap (&&) → st_intersects
-- ======================================================================
-- Rewriter must transform `&&` into `st_intersects()`.
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT * FROM a JOIN b ON a.geom && b.geom'
) LIKE '%st_intersects(a.geom, b.geom)%'
THEN 'PASS rewrite overlap' ELSE 'FAIL rewrite overlap' END;

-- Execute the rewritten semantics: two overlapping polygons must intersect.
WITH a(geom) AS (SELECT st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))')),
     b(geom) AS (SELECT st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))'))
SELECT CASE WHEN st_intersects(a.geom, b.geom) = true
            THEN 'PASS overlap semantic' ELSE 'FAIL overlap semantic' END
FROM a, b;

-- ======================================================================
-- 2. Geometry cast (::geometry) removal
-- ======================================================================
SELECT CASE WHEN NOT (sedonadb_rewrite_postgis(
    'SELECT ''POINT(1 2)''::geometry'
) LIKE '%::geometry%')
THEN 'PASS rewrite cast' ELSE 'FAIL rewrite cast' END;

-- Execute: st_geomfromtext (the DuckDB equivalent of a geometry cast).
SELECT CASE WHEN st_x(st_geomfromtext('POINT(1 2)')) = 1.0
            THEN 'PASS cast semantic' ELSE 'FAIL cast semantic' END;

-- ======================================================================
-- 3. Geography cast (::geography) → removed with warning
-- ======================================================================
SELECT CASE WHEN NOT (sedonadb_rewrite_postgis(
    'SELECT st_distance(a::geography, b::geography) FROM t'
) LIKE '%::geography%')
             AND sedonadb_rewrite_postgis(
    'SELECT st_distance(a::geography, b::geography) FROM t'
) LIKE '%WARNING%'
THEN 'PASS rewrite geog warn' ELSE 'FAIL rewrite geog warn' END;

-- Execute: st_distancespheroid is the DuckDB equivalent of geography distance.
SELECT CASE WHEN st_distancespheroid(st_point(0,0), st_point(0,1)) > 100000.0
            THEN 'PASS geog semantic' ELSE 'FAIL geog semantic' END;

-- ======================================================================
-- 4. ST_MemUnion → ST_Union_Agg
-- ======================================================================
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT st_memunion(geom) FROM t'
) LIKE '%ST_Union_Agg(%'
THEN 'PASS rewrite memunion' ELSE 'FAIL rewrite memunion' END;

-- Execute: union_agg merges overlapping polygons.
SELECT CASE WHEN abs(st_area(st_union_agg(g)) - 6.0) < 0.001
            THEN 'PASS memunion semantic' ELSE 'FAIL memunion semantic' END
FROM (VALUES (st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))')),
             (st_geomfromtext('POLYGON((1 0,3 0,3 2,1 2,1 0))'))) AS t(g);

-- ======================================================================
-- 5. Scalar ST_Collect(a, b) → st_collect_scalar
-- ======================================================================
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT ST_Collect(a.geom, b.geom) FROM a, b'
) LIKE '%st_collect_scalar(a.geom, b.geom)%'
THEN 'PASS rewrite collect' ELSE 'FAIL rewrite collect' END;

-- Execute: st_collect_scalar merges two points into a MULTIPOINT.
SELECT CASE WHEN st_astext(st_collect_scalar(st_point(1,2), st_point(3,4)))
                 = 'MULTIPOINT((1 2),(3 4))'
            THEN 'PASS collect semantic' ELSE 'FAIL collect semantic' END;

-- ======================================================================
-- 6. Aggregate ST_Collect(geom) — left untouched (1-arg form)
-- ======================================================================
SELECT CASE WHEN NOT (sedonadb_rewrite_postgis(
    'SELECT ST_Collect(geom) FROM t'
) LIKE '%st_collect_scalar%')
THEN 'PASS collect agg untouched' ELSE 'FAIL collect agg untouched' END;

-- ======================================================================
-- 7. Clean SQL is returned unchanged (no false rewrites)
-- ======================================================================
SELECT CASE WHEN NOT (sedonadb_rewrite_postgis(
    'SELECT st_intersects(a.geom, b.geom) FROM a, b'
) LIKE '%rewrite:%')
THEN 'PASS clean passthrough' ELSE 'FAIL clean passthrough' END;

-- ======================================================================
-- 8. Complex multi-rewrite statement
-- ======================================================================
-- && + ::geometry + ST_Collect in one statement.
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT ST_Collect(a.geom, b.geom) FROM a JOIN b ON a.geom && b.geom'
) LIKE '%st_intersects(a.geom, b.geom)%'
             AND sedonadb_rewrite_postgis(
    'SELECT ST_Collect(a.geom, b.geom) FROM a JOIN b ON a.geom && b.geom'
) LIKE '%st_collect_scalar%'
THEN 'PASS multi rewrite' ELSE 'FAIL multi rewrite' END;
