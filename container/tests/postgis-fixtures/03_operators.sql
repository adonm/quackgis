-- PostGIS fixture: operators and KNN
-- Tests PostGIS operators (&&, <->) through the QuackGIS facade.

-- && bbox overlap: overlapping polygons
SELECT CASE WHEN st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))') &&
                 st_geomfromtext('POLYGON((1 1,3 1,3 3,1 3,1 1))')
            THEN 'PASS operator overlap-true' ELSE 'FAIL operator overlap-true' END;

-- && bbox overlap: disjoint polygons
SELECT CASE WHEN NOT (
                 st_geomfromtext('POLYGON((0 0,1 0,1 1,0 1,0 0))') &&
                 st_geomfromtext('POLYGON((5 5,6 5,6 6,5 6,5 5))'))
            THEN 'PASS operator overlap-false' ELSE 'FAIL operator overlap-false' END;

-- <-> distance operator
SELECT CASE WHEN (st_geomfromtext('POINT(0 0)') <->
                 st_geomfromtext('POINT(3 4)')) = 5
            THEN 'PASS operator distance' ELSE 'FAIL operator distance' END;

-- KNN-style query: ORDER BY geom <-> point LIMIT 1
SELECT CASE WHEN (
    SELECT st_astext(geom) FROM (
        VALUES (st_geomfromtext('POINT(0 0)')),
               (st_geomfromtext('POINT(10 0)')),
               (st_geomfromtext('POINT(5 0)'))
    ) AS t(geom)
    ORDER BY geom <-> st_geomfromtext('POINT(4 0)')
    LIMIT 1
) = 'POINT(5 0)'
            THEN 'PASS operator knn-nearest' ELSE 'FAIL operator knn-nearest' END;
