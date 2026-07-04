-- PostGIS fixture: constructors and WKT I/O
-- Tests PostGIS-style constructors through the QuackGIS facade.

SELECT CASE WHEN st_astext(st_geomfromtext('POINT(1 2)')) = 'POINT(1 2)'
            THEN 'PASS constructor geomfromtext' ELSE 'FAIL constructor geomfromtext' END;

SELECT CASE WHEN st_astext(st_point(3, 4)) = 'POINT(3 4)'
            THEN 'PASS constructor point' ELSE 'FAIL constructor point' END;

SELECT CASE WHEN st_astext('POINT(5 6)'::geometry) = 'POINT(5 6)'
            THEN 'PASS cast text-to-geometry' ELSE 'FAIL cast text-to-geometry' END;

SELECT CASE WHEN st_area(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))')) = 16
            THEN 'PASS constructor polygon area' ELSE 'FAIL constructor polygon area' END;

SELECT CASE WHEN st_astext(st_makeenvelope(0, 0, 2, 2)) = 'POLYGON((0 0,2 0,2 2,0 2,0 0))'
            THEN 'PASS constructor makeenvelope' ELSE 'FAIL constructor makeenvelope' END;

SELECT CASE WHEN abs(st_x(st_transform(st_geomfromtext('POINT(-0.1278 51.5074)'), 4326, 3857))
                      - (-14227.16)) < 2.0
            THEN 'PASS constructor transform' ELSE 'FAIL constructor transform' END;
