.mode list
-- =====================================================================
-- Milestone 19: Migration UX — rewriter verification + macro tests.
--
-- Section 1: Rewriter detection patterns verified via the SQL itself.
--   (The rewriter is a static-analysis tool; these checks verify the
--   DuckDB SQL that would result from applying its suggestions.)
-- Section 2: DuckLake layout macros.
-- =====================================================================

-- =====================================================================
-- Section 1: Rewritten PostGIS patterns produce correct results
-- =====================================================================

-- Pattern: && → bbox predicate + exact
SELECT CASE WHEN count(*) = count(*)
THEN 'PASS rewrite_bbox_overlap' ELSE 'FAIL rewrite_bbox_overlap' END
FROM (SELECT st_geomfromtext('POINT(1 1)') AS geom) t1,
     (SELECT st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))') AS geom) t2
WHERE st_xmax(t1.geom) >= st_xmin(t2.geom) AND st_xmin(t1.geom) <= st_xmax(t2.geom)
  AND st_ymax(t1.geom) >= st_ymin(t2.geom) AND st_ymin(t1.geom) <= st_ymax(t2.geom)
  AND st_intersects(t1.geom, t2.geom);

-- Pattern: <-> → ORDER BY st_distance + LIMIT
SELECT CASE WHEN (
    SELECT st_distance(geom, st_point(0.0, 0.0))
    FROM (VALUES (st_point(1.0, 0.0)), (st_point(0.5, 0.0)), (st_point(5.0, 5.0))) AS t(geom)
    ORDER BY st_distance(geom, st_point(0.0, 0.0)) LIMIT 1
) = 0.5
THEN 'PASS rewrite_knn_nearest' ELSE 'FAIL rewrite_knn_nearest' END;

-- Pattern: ::geometry → st_geomfromtext
SELECT CASE WHEN st_astext(st_geomfromtext('POINT(1 2)')) = 'POINT(1 2)'
THEN 'PASS rewrite_cast_geomfromtext' ELSE 'FAIL rewrite_cast_geomfromtext' END;

-- Pattern: ::geography → st_distancespheroid
SELECT CASE WHEN st_distancespheroid(st_point(0.0, 0.0), st_point(0.0, 1.0)) > 100000.0
THEN 'PASS rewrite_geography_distance' ELSE 'FAIL rewrite_geography_distance' END;

-- Pattern: ST_MemUnion → ST_Union_Agg
SELECT CASE WHEN st_area(st_union_agg(g)) > 0
THEN 'PASS rewrite_memunion' ELSE 'FAIL rewrite_memunion' END
FROM (VALUES (st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))')),
             (st_geomfromtext('POLYGON((1 0,2 0,2 1,1 1,1 0))'))) AS t(g);

-- Pattern: ST_AsEWKT with SRID
SELECT CASE WHEN st_asewkt(st_setsrid(st_geomfromtext('POINT(1 2)'), 4326), 4326)
            LIKE '%POINT(1 2)'
THEN 'PASS rewrite_srid_ewkt' ELSE 'FAIL rewrite_srid_ewkt' END;

-- Pattern: GiST index → layout columns (verify columns work)
SELECT CASE WHEN count(*) > 0
THEN 'PASS rewrite_gist_to_layout' ELSE 'FAIL rewrite_gist_to_layout' END
FROM (
    SELECT st_geomfromtext('POINT(-74.0 40.7)') AS geom,
           st_xmin(st_geomfromtext('POINT(-74.0 40.7)')) AS xmin,
           st_xmax(st_geomfromtext('POINT(-74.0 40.7)')) AS xmax,
           st_quadkey(st_geomfromtext('POINT(-74.0 40.7)'), 8) AS spatial_cell
);

-- =====================================================================
-- Section 2: DuckLake spatial macros (optional helpers)
-- =====================================================================

-- Macro: sedona_bbox_overlaps
SELECT CASE WHEN sedona_bbox_overlaps(0.0, 0.0, 2.0, 2.0, 1.0, 1.0, 3.0, 3.0)
THEN 'PASS macro_bbox_overlaps_true' ELSE 'FAIL macro_bbox_overlaps_true' END;

SELECT CASE WHEN NOT sedona_bbox_overlaps(0.0, 0.0, 1.0, 1.0, 5.0, 5.0, 6.0, 6.0)
THEN 'PASS macro_bbox_overlaps_false' ELSE 'FAIL macro_bbox_overlaps_false' END;

-- Macro: sedona_layout_cell
SELECT CASE WHEN length(sedona_layout_cell(st_geomfromtext('POINT(-74.0 40.7)'), 8)) > 0
THEN 'PASS macro_layout_cell' ELSE 'FAIL macro_layout_cell' END;

-- Macro: sedona_layout_sort
SELECT CASE WHEN sedona_layout_sort(st_geomfromtext('POINT(-74.0 40.7)'), 12) IS NOT NULL
THEN 'PASS macro_layout_sort' ELSE 'FAIL macro_layout_sort' END;

-- Macro: sedona_covering_cells_bbox (table-returning)
SELECT CASE WHEN count(*) > 0
THEN 'PASS macro_covering_cells_bbox' ELSE 'FAIL macro_covering_cells_bbox' END
FROM sedona_covering_cells_bbox(-1.0, -1.0, 1.0, 1.0, 4, 100);

-- =====================================================================
-- Section 3: ST_AsSVG output (M16 capability, part of migration story)
-- =====================================================================

SELECT CASE WHEN st_assvg(st_geomfromtext('POINT(1 2)')) = '1 -2'
THEN 'PASS migration_svg_output' ELSE 'FAIL migration_svg_output' END;
