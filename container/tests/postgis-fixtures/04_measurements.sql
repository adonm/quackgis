-- PostGIS fixture: measurements and geography
-- Tests distance, area, length, and geodesic functions through the facade.

SELECT CASE WHEN st_distance(st_point(0,0), st_point(3,4)) = 5
            THEN 'PASS measurement distance' ELSE 'FAIL measurement distance' END;

SELECT CASE WHEN st_area(st_geomfromtext('POLYGON((0 0,4 0,4 4,0 4,0 0))')) = 16
            THEN 'PASS measurement area' ELSE 'FAIL measurement area' END;

SELECT CASE WHEN st_length(st_geomfromtext('LINESTRING(0 0,3 0,3 4)')) = 7
            THEN 'PASS measurement length' ELSE 'FAIL measurement length' END;

SELECT CASE WHEN st_distancesphere(st_point(0,0), st_point(0,1)) > 100000
            THEN 'PASS measurement distancesphere' ELSE 'FAIL measurement distancesphere' END;

SELECT CASE WHEN st_distancespheroid(st_point(0,0), st_point(0,1)) > 100000
            THEN 'PASS measurement distancespheroid' ELSE 'FAIL measurement distancespheroid' END;
