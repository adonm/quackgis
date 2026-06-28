.mode list
-- M25: sedonadb_rewrite_postgis() AST-based PostGIS→DuckDB SQL rewriter.
--
-- Validates that the sqlparser-rs AST rewriter correctly transforms
-- PostGIS-isms into DuckDB-compatible SQL at runtime.

-- && → st_intersects
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT * FROM a JOIN b ON a.geom && b.geom'
) LIKE '%st_intersects(a.geom, b.geom)%'
THEN 'PASS m25_rewrite_overlap' ELSE 'FAIL m25_rewrite_overlap' END;

-- ::geometry → removed
SELECT CASE WHEN NOT sedonadb_rewrite_postgis(
    'SELECT ''POINT(1 2)''::geometry'
) LIKE '%::geometry%'
THEN 'PASS m25_rewrite_geom_cast' ELSE 'FAIL m25_rewrite_geom_cast' END;

-- ::geography → removed + warning
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT st_distance(a::geography, b::geography) FROM t'
) LIKE '%WARNING%geography%'
THEN 'PASS m25_rewrite_geog_cast' ELSE 'FAIL m25_rewrite_geog_cast' END;

-- ST_MemUnion → ST_Union_Agg
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT st_memunion(geom) FROM t'
) LIKE '%ST_Union_Agg(%'
THEN 'PASS m25_rewrite_memunion' ELSE 'FAIL m25_rewrite_memunion' END;

-- Clean SQL not rewritten
SELECT CASE WHEN NOT sedonadb_rewrite_postgis(
    'SELECT st_area(geom) FROM t'
) LIKE '%rewrite:%'
THEN 'PASS m25_clean_sql' ELSE 'FAIL m25_clean_sql' END;

-- NULL propagation
SELECT CASE WHEN sedonadb_rewrite_postgis(NULL) IS NULL
THEN 'PASS m25_null' ELSE 'FAIL m25_null' END;

-- Complex expression: join with && and where clause
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT * FROM a JOIN b ON a.geom && b.geom WHERE st_dwithin(a.geom, b.geom, 100)'
) LIKE '%st_intersects(a.geom, b.geom)%'
THEN 'PASS m25_complex_join' ELSE 'FAIL m25_complex_join' END;

-- Rewrite count appears
SELECT CASE WHEN sedonadb_rewrite_postgis(
    'SELECT st_memunion(geom) FROM t'
) LIKE '%mechanical rewrite%'
THEN 'PASS m25_rewrite_count' ELSE 'FAIL m25_rewrite_count' END;
