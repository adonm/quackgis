-- PostGIS fixture: SRID handling and EWKT
-- Tests SRID tags, EWKT format, and CRS-aware operations through the facade.

SELECT CASE WHEN st_srid(st_geomfromtext('POINT(0 0)')) = 0
            THEN 'PASS srid default zero' ELSE 'FAIL srid default zero' END;

SELECT CASE WHEN st_srid(st_setsrid(st_point(1,2), 4326)) = 4326
            THEN 'PASS srid set-and-get' ELSE 'FAIL srid set-and-get' END;

SELECT CASE WHEN st_asewkt(st_setsrid(st_point(1,2), 4326)) = 'SRID=4326;POINT(1 2)'
            THEN 'PASS srid asewkt' ELSE 'FAIL srid asewkt' END;

SELECT CASE WHEN st_srid(st_geomfromtext('POINT(0 0)', 4326)) = 4326
            THEN 'PASS srid from-text-2arg' ELSE 'FAIL srid from-text-2arg' END;

-- SRID propagation through geometry-producing functions
SELECT CASE WHEN st_srid(st_centroid(st_setsrid(
    st_geomfromtext('POLYGON((0 0,2 0,2 2,0 2,0 0))'), 4326))) = 4326
            THEN 'PASS srid propagation centroid' ELSE 'FAIL srid propagation centroid' END;
